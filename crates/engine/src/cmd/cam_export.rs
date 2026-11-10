//! CAM ▸ TOOLPATH ▸ OUTPUT: the ISO G-code program the design's toolpaths make
//! (`cam.export.gcode`).
//!
//! A toolpath stores what it cuts (its setup, tool, bodies and cutting values) but not the
//! million moves a post-processor writes, so the export reads the part's exact B-rep again: the
//! same outlines and passes `cam.toolpath.contour` and `cam.toolpath.pocket` plan, at the
//! cutting values the toolpath holds. Contour follows each outline offset by the tool's radius
//! (and any stock left for finishing); pocket clears the inside of the outline with a zig-zag
//! raster one stepover wide, keeping the cutter clear of openings.
//!
//! The program is machine-neutral ISO G-code in millimetres: `G21 G90 G17 G94`, one `T`/`M6`
//! per tool, `S`/`M3` where a spindle speed is stated, `G0` rapid moves and `G1` cutting moves.
//! Arcs are written as short lines. A program without a feed rate cannot be run, so a toolpath
//! that states none is refused rather than exported with an `F0`.

use std::cmp::Ordering;

use serde_json::Value;
use solidcore_record::Plane;
use solidcraft_doc::cam::{Setup, Toolpath, ToolpathKind};

use super::CommandSpec;
use super::cam_toolpath::{frame, loop_points, pass_levels, passes, sections, stock_bounds};
use crate::params::{bad, str_};
use crate::schema::TypedContract;
use crate::schema::cam_export::{GcodeExport, GcodeOperation, GcodeParams, SAFE_HEIGHT_PROBLEM};
use crate::schema::cam_toolpath::ToolpathKindParam;
use crate::schema::output_schema_for;
use crate::{Result, Session};

const CMD: &str = "cam.export.gcode";

const CONTRACT: TypedContract<GcodeParams> = TypedContract::new();

/// The most lines one program may hold; beyond this the plan is a mistake, not a file.
const MAX_LINES: usize = 2_000_000;
/// The most passes one operation may take, as `cam.toolpath.contour` allows.
const MAX_PASSES: usize = 10_000;
/// The most raster rows one pocket pass may take; a smaller stepover is a mistake, not a plan.
const MAX_SCANLINES: usize = 200_000;
/// The most precision a coordinate is written with, in mm.
const DECIMALS: usize = 3;
/// The default clearance above the stock's top, in mm.
const DEFAULT_SAFE_HEIGHT: f64 = 5.0;

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new("cam.export.gcode", "Export G-code", crate::checked!("cam.export.gcode", export))
    .at("TOOLPATH", "OUTPUT")
    .icon("export")
    .doc_only()
    .noundo()
    .contract(&CONTRACT)
    .output(output_schema_for::<GcodeExport>)
    .params(
        "toolpaths?: [toolpath ids] (default: every toolpath); setup?: a setup's id (default: every setup, including the model origin); path?: the file to write (default: return the program as `gcode`); program?: 1–128 characters (default: the design's name); safe_height?: mm above the stock's top (default 5) → ISO G-code in mm: G21 G90 G17 G94, G0 rapids, G1 cuts, one T/M6 per tool, S/M3 where the toolpath states a spindle speed; arcs are written as lines",
    )];

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let wanted = ids(p, "toolpaths")?;
    let setup_filter = match p.get("setup") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or_else(|| bad(CMD, "`setup` must be a setup's id"))?),
    };
    if let Some(id) = setup_filter.filter(|id| *id != 0)
        && s.doc.cam.setup(id).is_none()
    {
        return Err(bad(CMD, format!("there is no setup {id}")));
    }
    let program = program_name(s, p)?;
    let safe_height = match p.get("safe_height") {
        None | Some(Value::Null) => DEFAULT_SAFE_HEIGHT,
        Some(v) => v
            .as_f64()
            .filter(|v| v.is_finite() && (0.0..=1000.0).contains(v))
            .ok_or_else(|| bad(CMD, format!("`safe_height` {SAFE_HEIGHT_PROBLEM}")))?,
    };
    let path = match p.get("path") {
        None | Some(Value::Null) => None,
        Some(_) => Some(str_(p, "path").filter(|v| !v.trim().is_empty() && v.len() < 4096).ok_or_else(|| bad(CMD, "invalid path"))?.to_string()),
    };
    if let Some(path) = &path {
        solidcraft_io::vfs::check(path).map_err(|e| crate::EngineError::Other(e.to_string()))?;
    }
    let chosen = choose(s, wanted.as_deref(), setup_filter)?;
    let mut program_text = Program::new();
    let mut operations = Vec::with_capacity(chosen.len());
    for toolpath in &chosen {
        operations.push(operation(s, toolpath, safe_height, &mut program_text)?);
    }
    program_text.footer();
    let program_text = program_text.text(&program);
    let text = program_text;
    let bytes = text.len();
    let lines = text.lines().count();
    let passes = operations.iter().map(|op| op.passes).sum();
    let mut result = GcodeExport { program, units: "mm", operations, passes, lines, bytes, path: None, gcode: None };
    if let Some(path) = path {
        solidcraft_io::vfs::write(&path, text.as_bytes()).map_err(|e| crate::EngineError::Other(format!("{path}: {e}")))?;
        result.path = Some(path);
    } else {
        result.gcode = Some(text);
    }
    serde_json::to_value(result).map_err(|e| bad(CMD, e.to_string()))
}

/// The toolpaths the call exports, in the design's order: the ids it names, else all of them,
/// filtered to one setup when it names one.
fn choose(s: &Session, wanted: Option<&[u64]>, setup: Option<u64>) -> Result<Vec<Toolpath>> {
    let all = &s.doc.cam.toolpaths;
    let mut chosen = match wanted {
        Some(ids) => {
            let mut chosen = Vec::with_capacity(ids.len());
            for id in ids {
                let toolpath = all.iter().find(|p| p.id == *id).ok_or_else(|| bad(CMD, format!("there is no toolpath {id}")))?;
                chosen.push(toolpath.clone());
            }
            chosen
        }
        None => all.clone(),
    };
    if let Some(setup) = setup {
        chosen.retain(|toolpath| toolpath.setup == setup);
    }
    if chosen.is_empty() {
        return Err(bad(CMD, "there are no toolpaths to export"));
    }
    Ok(chosen)
}

/// The `toolpaths` ids of the call; `None` when absent.
fn ids(p: &Value, key: &str) -> Result<Option<Vec<u64>>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(values)) => {
            let mut ids = Vec::with_capacity(values.len());
            for value in values {
                ids.push(value.as_u64().ok_or_else(|| bad(CMD, format!("`{key}` must list toolpath ids")))?);
            }
            Ok(Some(ids))
        }
        Some(_) => Err(bad(CMD, format!("`{key}` must list toolpath ids"))),
    }
}

/// The program's name: what the call names, else the design's. Parentheses, `%` and control
/// characters would break the header comment, so they become spaces.
fn program_name(s: &Session, p: &Value) -> Result<String> {
    let raw = match p.get("program") {
        None | Some(Value::Null) => s.doc.name.clone(),
        Some(_) => str_(p, "program").ok_or_else(|| bad(CMD, "`program` must be text"))?.to_string(),
    };
    let name: String = raw.chars().map(|c| if c.is_control() || matches!(c, '(' | ')' | '%') { ' ' } else { c }).collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = name.trim();
    if name.chars().count() > 128 {
        return Err(bad(CMD, "`program` must have 1–128 characters"));
    }
    Ok(if name.is_empty() { "PROGRAM".to_string() } else { name.to_string() })
}

/// One toolpath as the program runs it: its passes, its tool's changes and its cutting moves.
fn operation(s: &Session, toolpath: &Toolpath, safe_height: f64, program: &mut Program) -> Result<GcodeOperation> {
    let setup = setup_of(s, toolpath)?;
    let tool = s
        .doc
        .cam
        .tool(toolpath.tool)
        .ok_or_else(|| bad(CMD, format!("toolpath `{}` uses tool {}, which does not exist", toolpath.name, toolpath.tool)))?;
    if toolpath.bodies.is_empty() {
        return Err(bad(CMD, format!("toolpath `{}` has no bodies to cut", toolpath.name)));
    }
    if !(toolpath.feed_rate > 0.0) {
        return Err(bad(CMD, format!("toolpath `{}` has no feed rate: set one before exporting", toolpath.name)));
    }
    let tool = tool.clone();
    program.comment(format!(
        "{}: {}, T{} {} ({} mm), {} passes",
        toolpath.name,
        kind_word(toolpath.kind),
        tool.id,
        comment(&tool.name),
        number(tool.diameter),
        passes(toolpath.total_depth, toolpath.depth_per_pass)
    ));
    let frame = frame(s, setup.as_ref(), CMD)?;
    let plane = Plane { origin: frame.origin, x: frame.x, y: frame.y };
    let sections = sections(s, &plane, &toolpath.bodies, CMD)?;
    let (stock_top, _) = stock_bounds(setup.as_ref(), &sections, CMD)?;
    let safe = stock_top + safe_height;
    let pass_count = passes(toolpath.total_depth, toolpath.depth_per_pass);
    if pass_count > MAX_PASSES {
        return Err(bad(CMD, format!("toolpath `{}` takes more than {MAX_PASSES} passes: its depth per pass is too small", toolpath.name)));
    }
    let levels = pass_levels(stock_top, toolpath.total_depth, toolpath.depth_per_pass, pass_count);
    let plunge = if toolpath.plunge_rate > 0.0 { toolpath.plunge_rate } else { toolpath.feed_rate };
    let inset = tool.diameter / 2.0 + toolpath.stock_to_leave;
    program.tool(tool.id);
    if toolpath.spindle_speed > 0.0 {
        program.line(format!("S{} M3", number(toolpath.spindle_speed)));
    }
    let moves = match toolpath.kind {
        ToolpathKind::Contour => contour(toolpath, &sections, &levels, inset, safe, plunge, program)?,
        ToolpathKind::Pocket => pocket(toolpath, &sections, &levels, inset, safe, plunge, program)?,
        ToolpathKind::Facing => facing(toolpath, &sections, &levels, inset, safe, plunge, program)?,
    };
    program.line("M5".to_string());
    program.line(format!("G0 Z{}", number(safe)));
    Ok(GcodeOperation {
        toolpath: toolpath.id,
        name: toolpath.name.clone(),
        kind: ToolpathKindParam::from_doc(toolpath.kind),
        tool: tool.id,
        passes: pass_count,
        moves,
    })
}

/// The setup a toolpath runs in; `None` is the model origin and the model's own axes.
fn setup_of(s: &Session, toolpath: &Toolpath) -> Result<Option<Setup>> {
    if toolpath.setup == 0 {
        return Ok(None);
    }
    s.doc
        .cam
        .setup(toolpath.setup)
        .cloned()
        .map(Some)
        .ok_or_else(|| bad(CMD, format!("toolpath `{}` uses setup {}, which does not exist", toolpath.name, toolpath.setup)))
}

/// Follow each outline with the cutter's side, offset out of the material by the tool's radius
/// (and any stock left for finishing). Openings are cut from inside.
fn contour(
    toolpath: &Toolpath,
    sections: &[super::cam_toolpath::Section],
    levels: &[f64],
    inset: f64,
    safe: f64,
    plunge: f64,
    program: &mut Program,
) -> Result<usize> {
    let mut moves = 0;
    for &level in levels {
        for section in sections {
            for (index, lp) in section.loops.iter().enumerate() {
                let points = loop_points(lp, CMD)?;
                let offset = if index == 0 { inset } else { -inset };
                let path = offset_loop(&points, offset);
                if path.len() < 3 {
                    continue;
                }
                let (start_x, start_y) = (path[0][0], path[0][1]);
                program.line(format!("G0 Z{}", number(safe)));
                program.rapid(start_x, start_y);
                program.plunge(level, plunge);
                for point in &path[1..] {
                    program.cut(point[0], point[1], toolpath.feed_rate)?;
                    moves += 1;
                }
                // Close the loop.
                program.cut(start_x, start_y, toolpath.feed_rate)?;
                moves += 1;
            }
        }
    }
    Ok(moves)
}

/// Clear the inside of each outline with a zig-zag raster one stepover wide, keeping the
/// cutter clear of the openings and the walls.
fn pocket(
    toolpath: &Toolpath,
    sections: &[super::cam_toolpath::Section],
    levels: &[f64],
    inset: f64,
    safe: f64,
    plunge: f64,
    program: &mut Program,
) -> Result<usize> {
    let mut outer = Vec::new();
    let mut holes = Vec::new();
    for section in sections {
        for (index, lp) in section.loops.iter().enumerate() {
            let points = loop_points(lp, CMD)?;
            if index == 0 {
                outer.push(points);
            } else {
                holes.push(points);
            }
        }
    }
    let Some(min_y) = outer.iter().flatten().map(|p| p[1]).reduce(f64::min) else {
        return Ok(0);
    };
    let Some(max_y) = outer.iter().flatten().map(|p| p[1]).reduce(f64::max) else {
        return Ok(0);
    };
    let step = toolpath.stepover;
    if !(step > 0.0 && step.is_finite()) {
        return Err(bad(CMD, format!("toolpath `{}` has no stepover: set one before exporting", toolpath.name)));
    }
    let mut moves = 0;
    for &level in levels {
        let mut y = min_y + inset;
        let mut row = 0;
        while y <= max_y - inset + 1e-9 {
            if row >= MAX_SCANLINES {
                return Err(bad(CMD, format!("toolpath `{}` takes more than {MAX_SCANLINES} raster rows: its stepover is too small", toolpath.name)));
            }
            let mut segments = Vec::new();
            for (a, b) in intervals(&outer, y) {
                let (a, b) = (a + inset, b - inset);
                if b > a {
                    segments.extend(subtract(&[(a, b)], &expanded(&intervals(&holes, y), inset)));
                }
            }
            if row % 2 == 1 {
                for segment in segments.iter_mut() {
                    std::mem::swap(&mut segment.0, &mut segment.1);
                }
            }
            for (a, b) in segments {
                if (b - a).abs() <= 1e-6 {
                    continue;
                }
                program.line(format!("G0 Z{}", number(safe)));
                program.rapid(a, y);
                program.plunge(level, plunge);
                program.cut(b, y, toolpath.feed_rate)?;
                moves += 1;
            }
            row += 1;
            y = min_y + inset + row as f64 * step;
        }
    }
    Ok(moves)
}

/// Flatten the top of the stock: passes across the whole outline, as a pocket without walls.
fn facing(
    toolpath: &Toolpath,
    sections: &[super::cam_toolpath::Section],
    levels: &[f64],
    inset: f64,
    safe: f64,
    plunge: f64,
    program: &mut Program,
) -> Result<usize> {
    pocket(toolpath, sections, levels, inset, safe, plunge, program)
}

/// The spans of `y` inside `loops`, by the even-odd rule. Points are the loops' own, in the
/// setup's X/Y (mm).
fn intervals(loops: &[Vec<[f64; 2]>], y: f64) -> Vec<(f64, f64)> {
    let mut xs = Vec::new();
    for lp in loops {
        for index in 0..lp.len() {
            let a = lp[index];
            let b = lp[(index + 1) % lp.len()];
            if (a[1] <= y) != (b[1] <= y) {
                let t = (y - a[1]) / (b[1] - a[1]);
                xs.push(a[0] + t * (b[0] - a[0]));
            }
        }
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    xs.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    xs.as_chunks::<2>().0.iter().map(|pair| (pair[0], pair[1])).filter(|(a, b)| b > a).collect()
}

/// Every span widened by `margin` on both sides: the cutter's radius around an opening.
fn expanded(spans: &[(f64, f64)], margin: f64) -> Vec<(f64, f64)> {
    spans.iter().map(|(a, b)| (a - margin, b + margin)).collect()
}

/// What is left of `base` after taking `cuts` out of it.
fn subtract(base: &[(f64, f64)], cuts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut rest = base.to_vec();
    for &(cut_a, cut_b) in cuts {
        let mut next = Vec::new();
        for (a, b) in rest {
            if cut_b <= a || cut_a >= b {
                next.push((a, b));
                continue;
            }
            if a < cut_a {
                next.push((a, cut_a.min(b)));
            }
            if b > cut_b {
                next.push((cut_b.max(a), b));
            }
        }
        rest = next;
    }
    rest.into_iter().filter(|(a, b)| b - a > 1e-6).collect()
}

/// A closed polygon moved out (`distance` above 0) of or into its enclosed area, mitred at the
/// corners. A corner sharper than about 12° is cut back to five times the distance rather than
/// shooting off to infinity.
fn offset_loop(points: &[[f64; 2]], distance: f64) -> Vec<[f64; 2]> {
    let n = points.len();
    if n < 3 || !distance.is_finite() {
        return Vec::new();
    }
    let orientation = if signed_area(points) >= 0.0 { 1.0 } else { -1.0 };
    let mut moved = Vec::with_capacity(n);
    for index in 0..n {
        let prev = points[(index + n - 1) % n];
        let point = points[index];
        let next = points[(index + 1) % n];
        let before = outward(prev, point, orientation);
        let after = outward(point, next, orientation);
        let sum = (before.0 + after.0, before.1 + after.1);
        let length = (sum.0 * sum.0 + sum.1 * sum.1).sqrt();
        if length <= 1e-12 {
            moved.push([point[0] + after.0 * distance, point[1] + after.1 * distance]);
            continue;
        }
        let direction = (sum.0 / length, sum.1 / length);
        let cos_half = direction.0 * before.0 + direction.1 * before.1;
        let miter = if cos_half.abs() < 0.2 { 5.0 } else { (1.0 / cos_half).clamp(-5.0, 5.0) };
        moved.push([point[0] + direction.0 * distance * miter, point[1] + direction.1 * distance * miter]);
    }
    moved
}

/// The unit normal of the edge `a` → `b` pointing away from the polygon's enclosed area, given
/// its orientation (1 for counter-clockwise, -1 for clockwise).
fn outward(a: [f64; 2], b: [f64; 2], orientation: f64) -> (f64, f64) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= 1e-12 {
        return (0.0, 0.0);
    }
    (orientation * dy / length, -orientation * dx / length)
}

/// Twice the signed area of a polygon: positive when counter-clockwise.
fn signed_area(points: &[[f64; 2]]) -> f64 {
    let mut sum = 0.0;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum
}

fn kind_word(kind: ToolpathKind) -> &'static str {
    match kind {
        ToolpathKind::Facing => "facing",
        ToolpathKind::Contour => "contour",
        ToolpathKind::Pocket => "pocket",
    }
}

/// A tool's name that cannot end its comment early, with its runs of spaces closed up.
fn comment(text: &str) -> String {
    let cleaned: String = text.chars().map(|c| if c.is_control() || matches!(c, '(' | ')') { ' ' } else { c }).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A coordinate, feed or speed with at most [`DECIMALS`] decimals and no `-0`.
fn number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let text = format!("{value:.DECIMALS$}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" { "0".to_string() } else { text.to_string() }
}

/// The program being written: its lines, its line count guard and the tool it last changed to.
struct Program {
    lines: Vec<String>,
}

impl Program {
    fn new() -> Self {
        Program { lines: Vec::new() }
    }

    fn line(&mut self, line: String) {
        // A program past the cap cannot be held in memory: stop adding rather than grow without
        // bound. The caller's line count then reports the truncation, and no run is made from it.
        if self.lines.len() >= MAX_LINES {
            return;
        }
        self.lines.push(line);
    }

    fn comment(&mut self, text: String) {
        self.line(format!("({})", comment(&text)));
    }

    fn tool(&mut self, id: u64) {
        self.line(format!("T{id}"));
        self.line("M6".to_string());
    }

    fn rapid(&mut self, x: f64, y: f64) {
        self.line(format!("G0 X{} Y{}", number(x), number(y)));
    }

    fn plunge(&mut self, z: f64, rate: f64) {
        self.line(format!("G1 Z{} F{}", number(z), number(rate)));
    }

    fn cut(&mut self, x: f64, y: f64, rate: f64) -> Result<()> {
        if self.lines.len() >= MAX_LINES {
            return Err(bad(CMD, "the program is too long to write"));
        }
        self.line(format!("G1 X{} Y{} F{}", number(x), number(y), number(rate)));
        Ok(())
    }

    fn footer(&mut self) {
        self.line("M30".to_string());
    }

    /// The whole program: the `%` tape markers around a header in millimetres and a fixed work
    /// offset, then the operations.
    fn text(&self, program: &str) -> String {
        let mut text = String::new();
        text.push_str("%\n");
        text.push_str(&format!("({})\n", comment(program)));
        text.push_str("(Z-CAM)\nG21\nG90\nG17\nG94\nG54\n");
        for line in &self.lines {
            text.push_str(line);
            text.push('\n');
        }
        text.push_str("%\n");
        text
    }
}

#[cfg(test)]
#[path = "cam_export_tests.rs"]
mod tests;

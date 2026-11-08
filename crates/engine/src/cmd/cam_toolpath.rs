//! CAM ▸ TOOLPATH: contour and pocket toolpaths (`cam.toolpath.contour` and
//! `cam.toolpath.pocket`), the operations that cut a part's outline and clear what is inside it.
//!
//! A toolpath names its setup, its tool, its bodies and the cutting values it runs at; the engine
//! reads the bodies' exact B-rep to work out the outline it cuts. Contour and pocket take a
//! uniform prism: a body whose section down the setup's Z does not change between its top and
//! bottom cap, which is what 2.5D machining cuts. The section comes from the kernel's certificate
//! (`certified_prism_section`), so the boundary loops are the part's own lines and arcs, never a
//! mesh. Creating a toolpath is undoable like any other design change, and a refused call changes
//! nothing.

use std::collections::BTreeSet;

use serde_json::Value;
use solidcore_record::{Loop2, Plane, Vec2, Vec3};
use solidcraft_doc::cam::{Setup, Tool, Toolpath, ToolpathKind};
use solidcraft_geom::prelude::*;
use solidcraft_kernel::certified_prism_section;

use super::CommandSpec;
use crate::params::{bad, num, str_, string_list};
use crate::schema::TypedContract;
use crate::schema::cam_toolpath::{
    BoundaryLoop, BoundaryRole, DEPTH_PROBLEM, RATE_PROBLEM, STOCK_TO_LEAVE_PROBLEM, ToolpathParams, ToolpathReply, ToolpathSummary,
};
use crate::schema::output_schema_for;
use crate::{Result, Session};

const CONTRACT: TypedContract<ToolpathParams> = TypedContract::new();

/// The most toolpaths a design may hold.
const MAX_TOOLPATHS: usize = 1000;
/// The most passes one operation may take; more is a parameter mistake, not a plan.
const MAX_PASSES: usize = 10_000;
/// The most points one boundary loop may report.
const MAX_BOUNDARY_POINTS: usize = 65_536;
/// How closely a reported boundary follows the exact curve, in mm.
const BOUNDARY_TOL: f64 = 0.01;
/// How far a cut may sit outside a body's extent, in mm.
const DEPTH_TOL: f64 = 1e-6;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cam.toolpath.contour", "Contour", crate::checked!("cam.toolpath.contour", contour))
        .at("TOOLPATH", "2D MILLING")
        .icon("cam")
        .doc_only()
        .contract(&CONTRACT)
        .output(output_schema_for::<ToolpathReply>)
        .params(
            "tool: a tool's id in this design; setup?: a setup's id (default 0: the model origin and axes, with the stock read from the machined bodies); bodies?: [body names] (default: the setup's bodies, else every solid body); name?: 1–128 characters (default: \"Contour\" and the first free number); feed_rate?, plunge_rate?: mm per minute; spindle_speed?: rpm (each default 0: not stated); stepover?: mm (default half the tool's diameter); depth_per_pass?: mm (default: the whole total depth, one pass); total_depth?: mm (default: through the machined bodies); stock_to_leave?: mm (default 0)",
        ),
    CommandSpec::new("cam.toolpath.pocket", "Pocket", crate::checked!("cam.toolpath.pocket", pocket))
        .at("TOOLPATH", "2D MILLING")
        .icon("cam")
        .doc_only()
        .contract(&CONTRACT)
        .output(output_schema_for::<ToolpathReply>)
        .params(
            "as cam.toolpath.contour, clearing inside the outline: bodies that are a uniform prism down the setup's Z, cut down to total_depth, with stepover above 0 and at most the tool's diameter",
        ),
];

fn contour(s: &mut Session, p: &Value) -> Result<Value> {
    create(s, p, "cam.toolpath.contour", ToolpathKind::Contour)
}

fn pocket(s: &mut Session, p: &Value) -> Result<Value> {
    create(s, p, "cam.toolpath.pocket", ToolpathKind::Pocket)
}

/// The frame a setup works in: the machine zero and its X, Y and Z directions in model space.
pub(crate) struct Frame {
    pub(crate) origin: Vec3,
    pub(crate) x: Vec3,
    pub(crate) y: Vec3,
    pub(crate) z: Vec3,
}

/// One machined body's exact section: its extent along the setup's Z and its outline loops.
pub(crate) struct Section {
    pub(crate) body: String,
    pub(crate) lo: f64,
    pub(crate) hi: f64,
    pub(crate) loops: Vec<Loop2>,
}

fn create(s: &mut Session, p: &Value, cmd: &str, kind: ToolpathKind) -> Result<Value> {
    if s.doc.cam.toolpaths.len() >= MAX_TOOLPATHS {
        return Err(bad(cmd, "too many toolpaths"));
    }
    let tool = tool(s, p, cmd)?;
    let setup = setup(s, p, cmd)?;
    let frame = frame(s, setup.as_ref(), cmd)?;
    // A plane through the machine zero along the setup's Z: the prism sections are coordinates in
    // its X and Y, and heights above it point along the setup's Z.
    let plane = Plane { origin: frame.origin, x: frame.x, y: frame.y };
    let flat = frame.x.cross(frame.y).dot(frame.z);
    if !(flat > 0.999) {
        return Err(bad(cmd, "the setup's coordinate system is not right-handed: X cross Y must point along Z"));
    }
    let names = bodies(s, p, setup.as_ref(), cmd)?;
    let sections = sections(s, &plane, &names, cmd)?;
    let (total_depth, stock_top) = depths(p, setup.as_ref(), &sections, cmd)?;
    let cut = stock_top - total_depth;
    for section in &sections {
        if cut < section.lo - DEPTH_TOL || cut > section.hi + DEPTH_TOL {
            return Err(bad(
                cmd,
                format!(
                    "the cut reaches {cut:.3} mm above the machine zero, outside body `{}` (it spans {:.3} to {:.3} mm)",
                    section.body, section.lo, section.hi
                ),
            ));
        }
    }
    let stepover = match depth_param(p, "stepover", cmd)? {
        Some(stepover) => stepover,
        None => tool.diameter / 2.0,
    };
    if kind == ToolpathKind::Pocket && stepover > tool.diameter + DEPTH_TOL {
        return Err(bad(cmd, format!("`stepover` {stepover} mm is wider than the tool's diameter ({} mm)", tool.diameter)));
    }
    let depth_per_pass = depth_param(p, "depth_per_pass", cmd)?.unwrap_or(total_depth);
    let passes = passes(total_depth, depth_per_pass);
    if passes > MAX_PASSES {
        return Err(bad(cmd, format!("`depth_per_pass` {depth_per_pass} mm takes more than {MAX_PASSES} passes")));
    }
    let cut_levels = pass_levels(stock_top, total_depth, depth_per_pass, passes);
    let boundaries = boundaries(&sections, cmd)?;
    let name = new_name(s, cmd, str_(p, "name"), kind)?;
    let toolpath = Toolpath {
        id: 0,
        name,
        kind,
        setup: setup.as_ref().map(|x| x.id).unwrap_or(0),
        tool: tool.id,
        bodies: names,
        feed_rate: rate(p, "feed_rate", cmd)?.unwrap_or(0.0),
        plunge_rate: rate(p, "plunge_rate", cmd)?.unwrap_or(0.0),
        spindle_speed: rate(p, "spindle_speed", cmd)?.unwrap_or(0.0),
        stepover,
        depth_per_pass,
        total_depth,
        stock_to_leave: stock_to_leave(p, cmd)?.unwrap_or(0.0),
    };
    let (toolpath, count) = {
        let doc = s.doc_mut();
        let id = doc.allocate_id()?;
        let mut stored = toolpath;
        stored.id = id;
        doc.cam.toolpaths.push(stored.clone());
        (stored, doc.cam.toolpaths.len())
    };
    serde_json::to_value(ToolpathReply { toolpath: summary(&toolpath), count, passes, cut_levels, boundaries, param_warnings: Vec::new() })
        .map_err(|e| bad(cmd, e.to_string()))
}

/// The exact section of each named body down the setup's Z. Contour and pocket cut a constant
/// cross-section, so a body that is not a uniform prism is refused.
pub(crate) fn sections(s: &Session, plane: &Plane, names: &[String], cmd: &str) -> Result<Vec<Section>> {
    let state = s.world_state()?;
    let mut sections = Vec::with_capacity(names.len());
    for name in names {
        let found = state.bodies.iter().find(|b| b.name == *name).ok_or_else(|| bad(cmd, format!("unknown body name `{name}`")))?;
        let section = certified_prism_section(&found.body, plane)
            .map_err(|e| bad(cmd, format!("cannot read the exact geometry of body `{name}`: {e}")))?;
        let Some(section) = section else {
            return Err(bad(
                cmd,
                format!("body `{name}` is not a uniform prism along the setup's Z axis: contour and pocket cut a constant cross-section (2.5D machining)"),
            ));
        };
        let loops = section.region.loops().cloned().collect();
        sections.push(Section { body: name.clone(), lo: section.lo, hi: section.hi, loops });
    }
    Ok(sections)
}

/// The tool the call cuts with: a tool of this design.
fn tool(s: &Session, p: &Value, cmd: &str) -> Result<Tool> {
    let id = p.get("tool").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "`tool` must be a tool's id in this design"))?;
    s.doc.cam.tool(id).cloned().ok_or_else(|| bad(cmd, format!("there is no tool {id}")))
}

/// The setup the call runs in; `None` is the model origin and the model's own axes.
pub(crate) fn setup(s: &Session, p: &Value, cmd: &str) -> Result<Option<Setup>> {
    match p.get("setup") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_u64() {
            Some(0) => Ok(None),
            Some(id) => s.doc.cam.setup(id).cloned().map(Some).ok_or_else(|| bad(cmd, format!("there is no setup {id}"))),
            None => Err(bad(cmd, "`setup` must be a setup's id in this design")),
        },
        Some(_) => Err(bad(cmd, "`setup` must be a setup's id in this design")),
    }
}

/// The machine zero and axes of `setup`, or the model origin and its own axes.
pub(crate) fn frame(s: &Session, setup: Option<&Setup>, cmd: &str) -> Result<Frame> {
    let coordinate_system = setup.map(|x| x.coordinate_system).unwrap_or(0);
    if coordinate_system == 0 {
        return Ok(Frame { origin: Vec3::ZERO, x: Vec3::X, y: Vec3::Y, z: Vec3::Z });
    }
    let cs = s
        .doc
        .coordinate_systems
        .iter()
        .find(|c| c.id == coordinate_system)
        .ok_or_else(|| bad(cmd, format!("the setup uses coordinate system {coordinate_system}, which does not exist")))?;
    let v = |a: [f64; 3]| Vec3::new(a[0], a[1], a[2]);
    Ok(Frame { origin: v(cs.origin), x: v(cs.x_axis), y: v(cs.y_axis), z: v(cs.z_axis) })
}

/// The bodies the call machines: the names it gives, else the setup's bodies, else every solid
/// body of the model. Mesh bodies are refused: a toolpath reads exact geometry.
fn bodies(s: &Session, p: &Value, setup: Option<&Setup>, cmd: &str) -> Result<Vec<String>> {
    let mut wanted = string_list(cmd, p, "bodies")?;
    if wanted.is_empty() {
        if let Some(setup) = setup {
            wanted.clone_from(&setup.bodies);
        }
    }
    if wanted.is_empty() {
        let state = s.world_state()?;
        wanted = state.bodies.iter().filter(|b| !b.body.is_mesh()).map(|b| b.name.clone()).collect();
    }
    if wanted.is_empty() {
        return Err(bad(cmd, "there is nothing to machine: the design has no solid body"));
    }
    let state = s.world_state()?;
    let mut seen = BTreeSet::new();
    let mut names = Vec::with_capacity(wanted.len());
    for name in wanted {
        if !seen.insert(name.clone()) {
            continue;
        }
        let found = state.bodies.iter().find(|b| b.name == name).ok_or_else(|| bad(cmd, format!("unknown body name `{name}`")))?;
        if found.body.is_mesh() {
            return Err(bad(cmd, format!("body `{name}` is a mesh body: a toolpath reads exact B-rep geometry")));
        }
        names.push(name);
    }
    Ok(names)
}

/// The stock's top and bottom along the setup's Z.
pub(crate) fn stock_bounds(setup: Option<&Setup>, sections: &[Section], cmd: &str) -> Result<(f64, f64)> {
    let Some(lo) = sections.iter().map(|s| s.lo).reduce(f64::min) else {
        return Err(bad(cmd, "there is nothing to machine: the design has no solid body"));
    };
    let Some(hi) = sections.iter().map(|s| s.hi).reduce(f64::max) else {
        return Err(bad(cmd, "there is nothing to machine: the design has no solid body"));
    };
    let stock = setup.map(|x| x.stock.clone()).unwrap_or_default();
    let (stock_top, stock_bottom) =
        if stock.size[2].is_finite() && stock.size[2] > 0.0 { (stock.offset[2] + stock.size[2], stock.offset[2]) } else { (hi, lo) };
    if !(stock_top.is_finite() && stock_bottom.is_finite() && stock_top > stock_bottom) {
        return Err(bad(cmd, "the stock has no height"));
    }
    Ok((stock_top, stock_bottom))
}

/// The total depth below the stock top, and the stock's top and bottom along the setup's Z.
fn depths(p: &Value, setup: Option<&Setup>, sections: &[Section], cmd: &str) -> Result<(f64, f64)> {
    let (stock_top, stock_bottom) = stock_bounds(setup, sections, cmd)?;
    let Some(lo) = sections.iter().map(|s| s.lo).reduce(f64::min) else {
        return Err(bad(cmd, "there is nothing to machine: the design has no solid body"));
    };
    let total_depth = match depth_param(p, "total_depth", cmd)? {
        Some(depth) => depth,
        None => stock_top - lo,
    };
    if !(total_depth > 0.0) {
        return Err(bad(cmd, "`total_depth` must reach below the stock's top"));
    }
    if total_depth > stock_top - stock_bottom + DEPTH_TOL {
        return Err(bad(cmd, format!("`total_depth` {total_depth} mm is deeper than the stock ({} mm)", stock_top - stock_bottom)));
    }
    Ok((total_depth, stock_top))
}

/// `depth_per_pass`, `total_depth` or `stepover` of the call; `None` when absent.
fn depth_param(p: &Value, k: &str, cmd: &str) -> Result<Option<f64>> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => num(p, k).filter(|v| *v > 0.0 && *v <= 1000.0).map(Some).ok_or_else(|| bad(cmd, format!("`{k}` {DEPTH_PROBLEM}"))),
    }
}

/// `feed_rate`, `plunge_rate` or `spindle_speed` of the call; `None` when absent.
fn rate(p: &Value, k: &str, cmd: &str) -> Result<Option<f64>> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => num(p, k).filter(|v| (0.0..=1_000_000.0).contains(v)).map(Some).ok_or_else(|| bad(cmd, format!("`{k}` {RATE_PROBLEM}"))),
    }
}

/// `stock_to_leave` of the call; `None` when absent.
fn stock_to_leave(p: &Value, cmd: &str) -> Result<Option<f64>> {
    match p.get("stock_to_leave") {
        None | Some(Value::Null) => Ok(None),
        Some(_) => num(p, "stock_to_leave")
            .filter(|v| (0.0..=1000.0).contains(v))
            .map(Some)
            .ok_or_else(|| bad(cmd, format!("`stock_to_leave` {STOCK_TO_LEAVE_PROBLEM}"))),
    }
}

/// How many passes cut `total_depth` at `depth_per_pass` each; the last pass takes the rest.
pub(crate) fn passes(total_depth: f64, depth_per_pass: f64) -> usize {
    let exact = (total_depth / depth_per_pass - 1e-9).ceil();
    if exact.is_finite() && exact >= 1.0 { exact as usize } else { 1 }
}

/// The height of each pass above the machine zero, deepest last.
pub(crate) fn pass_levels(stock_top: f64, total_depth: f64, depth_per_pass: f64, passes: usize) -> Vec<f64> {
    (1..=passes).map(|pass| stock_top - (depth_per_pass * pass as f64).min(total_depth)).collect()
}

/// The boundary loops of every section, the outer outlines before the openings inside them.
fn boundaries(sections: &[Section], cmd: &str) -> Result<Vec<BoundaryLoop>> {
    let mut outer = Vec::new();
    let mut holes = Vec::new();
    for section in sections {
        for (index, lp) in section.loops.iter().enumerate() {
            let points = loop_points(lp, cmd)?;
            let boundary = BoundaryLoop {
                body: section.body.clone(),
                role: if index == 0 { BoundaryRole::Outer } else { BoundaryRole::Hole },
                length: lp.segs.iter().map(|seg| seg.length()).sum(),
                points,
            };
            if index == 0 { outer.push(boundary) } else { holes.push(boundary) }
        }
    }
    outer.extend(holes);
    Ok(outer)
}

/// A loop's points, in order, without the repeated closing point.
pub(crate) fn loop_points(lp: &Loop2, cmd: &str) -> Result<Vec<[f64; 2]>> {
    let mut points: Vec<Vec2> = Vec::new();
    for seg in &lp.segs {
        for point in seg.polyline(BOUNDARY_TOL) {
            if points.last().is_none_or(|last| last.dist(point) > 1e-9) {
                points.push(point);
            }
        }
        if points.len() > MAX_BOUNDARY_POINTS {
            return Err(bad(cmd, format!("a boundary loop has more than {MAX_BOUNDARY_POINTS} points: machining it needs a simpler outline")));
        }
    }
    if points.len() > 1 && points.first().zip(points.last()).is_some_and(|(a, b)| a.dist(*b) <= 1e-9) {
        points.pop();
    }
    if points.len() < 3 {
        return Err(bad(cmd, "a boundary loop has fewer than 3 points"));
    }
    Ok(points.iter().map(|p| [p.x, p.y]).collect())
}

fn summary(toolpath: &Toolpath) -> ToolpathSummary {
    ToolpathSummary {
        id: toolpath.id,
        name: toolpath.name.clone(),
        kind: crate::schema::cam_toolpath::ToolpathKindParam::from_doc(toolpath.kind),
        setup: toolpath.setup,
        tool: toolpath.tool,
        bodies: toolpath.bodies.clone(),
        feed_rate: toolpath.feed_rate,
        plunge_rate: toolpath.plunge_rate,
        spindle_speed: toolpath.spindle_speed,
        stepover: toolpath.stepover,
        depth_per_pass: toolpath.depth_per_pass,
        total_depth: toolpath.total_depth,
        stock_to_leave: toolpath.stock_to_leave,
    }
}

/// A new toolpath name: 1–128 trimmed characters, not another toolpath's. A blank or absent name
/// takes "Contour" or "Pocket" and the first number no toolpath's name has.
fn new_name(s: &Session, cmd: &str, given: Option<&str>, kind: ToolpathKind) -> Result<String> {
    let given = given.map(str::trim).filter(|n| !n.is_empty());
    let stem = match kind {
        ToolpathKind::Facing => "Facing",
        ToolpathKind::Contour => "Contour",
        ToolpathKind::Pocket => "Pocket",
    };
    let name = match given {
        Some(name) => name.to_string(),
        None => {
            let names: BTreeSet<&str> = s.doc.cam.toolpaths.iter().map(|p| p.name.as_str()).collect();
            let number = (1..=s.doc.cam.toolpaths.len().saturating_add(1)).find(|n| !names.contains(format!("{stem} {n}").as_str())).unwrap_or(1);
            format!("{stem} {number}")
        }
    };
    if name.chars().count() > 128 {
        return Err(bad(cmd, "`name` must have 1–128 characters"));
    }
    if s.doc.cam.toolpaths.iter().any(|p| p.name == name) {
        return Err(bad(cmd, format!("a toolpath named `{name}` already exists")));
    }
    Ok(name)
}

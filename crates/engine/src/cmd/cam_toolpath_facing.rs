//! CAM ▸ TOOLPATH: a 2D facing operation (`cam.toolpath.facing`).
//!
//! A facing call names a setup, a tool and the depth it flattens the stock to. The cutter sweeps
//! back and forth across the top of the stock, one pass beside the next, and each pass runs
//! `pass_extension` beyond the stock's edge so the edges are cleared too. The stock is the setup's
//! own when it states a size; otherwise it is the exact extent of the machined bodies, enlarged by
//! `stock_offset` on each side. The bodies' exact bounds are read from the B-rep, so nothing is
//! measured on a mesh and a body that is a mesh is refused.
//!
//! The passes are evenly spaced, no further apart than `stepover` times the tool's diameter, and
//! the toolpath is generated at once in the setup's machine coordinates: a boustrophedon from one
//! corner of the stock plus its extension to the other, at the cut level. Creating a facing
//! operation is undoable like any other design change, and a refused call changes nothing.

use std::collections::BTreeSet;

use serde_json::Value;
use solidcore_record::Plane;
use solidcraft_doc::ModelState;
use solidcraft_doc::cam::{CamFacing, Setup, Tool};
use solidcraft_doc::expr::Kind;
use solidcraft_geom::Aabb3;
use solidcraft_geom::prelude::*;

use super::CommandSpec;
use super::cam_toolpath::{frame, setup};
use crate::params::{bad, expr};
use crate::schema::TypedContract;
use crate::schema::cam_toolpath_facing::{
    CamToolpathFacingOutput, CamToolpathFacingParams, CamToolpathFacingSummary, FACING_DEPTH_PROBLEM, FACING_OFFSET_PROBLEM, FACING_STEPOVER_PROBLEM,
    FacingBounds,
};
use crate::schema::output_schema_for;
use crate::{Result, Session};

const CMD: &str = "cam.toolpath.facing";

const CONTRACT: TypedContract<CamToolpathFacingParams> = TypedContract::new();

/// The most facing operations a design may hold.
const MAX_FACINGS: usize = 1000;
/// The most passes one operation may run across the stock; more is a parameter mistake.
const MAX_PASSES: usize = 100_000;
/// The most points one toolpath may report.
const MAX_WAYPOINTS: usize = 1_000_000;
/// How far a pass moves over, as a fraction of the tool's diameter, when the call states none.
const DEFAULT_STEPOVER: f64 = 0.7;
/// A coordinate or depth this close to its own value is that value, in mm.
const DIM_TOL: f64 = 1e-6;

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new(
    "cam.toolpath.facing",
    "2D Facing",
    crate::checked!("cam.toolpath.facing", facing),
)
.at("TOOLPATH", "2D MILLING")
.icon("cam")
.doc_only()
.contract(&CONTRACT)
.output(output_schema_for::<CamToolpathFacingOutput>)
.params(
    "tool: a tool's id in this design (its diameter sets the passes' spacing); setup?: a setup's id (default 0: the model origin and axes, with the stock read from the machined bodies); stepover?: how far the cutter moves over between passes, as a fraction of its diameter (0.1–1.0, default 0.7); pass_extension?: mm each pass runs beyond the stock's edge (default 0); stock_offset?: mm the raw stock extends beyond the machined bodies on each side (default 0, used when the setup states no stock size); depth: mm below the stock's top, a number or an expression; name?: 1–128 characters (default: \"2D Facing\" and the first free number)",
)];

fn facing(s: &mut Session, p: &Value) -> Result<Value> {
    if s.doc.cam.facings.len() >= MAX_FACINGS {
        return Err(bad(CMD, "too many facing operations"));
    }
    let tool = tool(s, p)?;
    let diameter = diameter(&tool)?;
    let setup = setup(s, p, CMD)?;
    let frame = frame(s, setup.as_ref(), CMD)?;
    let plane = plane(&frame)?;
    let stepover = stepover(p)?;
    let pass_extension = length(s, p, "pass_extension", false)?.unwrap_or(0.0);
    let stock_offset = length(s, p, "stock_offset", false)?.unwrap_or(0.0);
    let depth = length(s, p, "depth", true)?.ok_or_else(|| bad(CMD, format!("`depth` {FACING_DEPTH_PROBLEM}")))?;
    let names = body_names(s, setup.as_ref())?;
    let state = s.world_state()?;
    let stock = stock(&state, setup.as_ref(), &plane, &names, stock_offset)?;
    let height = stock.top - stock.bottom;
    if !(height.is_finite() && height > 0.0) {
        return Err(bad(CMD, "the stock has no height to cut into"));
    }
    if depth > height + DIM_TOL {
        return Err(bad(CMD, format!("`depth` {depth} mm is deeper than the stock ({height} mm)")));
    }
    let spacing = stepover * diameter;
    if !(spacing.is_finite() && spacing > 0.0) {
        return Err(bad(CMD, format!("`stepover` {stepover} of a {diameter} mm tool spaces the passes by no distance")));
    }
    let span_x = (stock.max.x - stock.min.x) + 2.0 * pass_extension;
    let span_y = (stock.max.y - stock.min.y) + 2.0 * pass_extension;
    if !(span_x.is_finite() && span_y.is_finite() && span_x > 0.0 && span_y > 0.0) {
        return Err(bad(CMD, "the stock has no area to face"));
    }
    let intervals = (span_y / spacing - 1e-9).ceil().max(1.0);
    if !intervals.is_finite() || intervals > MAX_PASSES as f64 {
        return Err(bad(CMD, format!("`stepover` {stepover} takes more than {MAX_PASSES} passes: a wider stepover is needed")));
    }
    let intervals = intervals as usize;
    let lines = intervals.saturating_add(1);
    if lines.checked_mul(2).is_none_or(|points| points > MAX_WAYPOINTS) {
        return Err(bad(CMD, format!("the toolpath has more than {MAX_WAYPOINTS} points: a wider stepover is needed")));
    }
    let level = stock.top - depth;
    let (waypoints, cutting_length, rapid_length) = path(&plane, &stock, level, span_x, pass_extension, lines, intervals);
    let bounds = FacingBounds {
        min: [stock.min.x, stock.min.y, stock.bottom],
        max: [stock.max.x, stock.max.y, stock.top],
        area: (stock.max.x - stock.min.x) * (stock.max.y - stock.min.y),
    };
    let name = new_name(s, p.get("name").and_then(Value::as_str))?;
    let facing = CamFacing {
        id: 0,
        name,
        setup: setup.as_ref().map(|setup| setup.id).unwrap_or(0),
        tool: tool.id,
        stepover,
        pass_extension,
        stock_offset,
        depth,
    };
    let (facing, count) = {
        let doc = s.doc_mut();
        let id = doc.allocate_id()?;
        let mut stored = facing;
        stored.id = id;
        doc.cam.facings.push(stored.clone());
        (stored, doc.cam.facings.len())
    };
    serde_json::to_value(CamToolpathFacingOutput {
        facing: summary(&facing),
        count,
        passes: lines,
        cut_levels: vec![level],
        bounds,
        waypoints,
        cutting_length,
        rapid_length,
        param_warnings: Vec::new(),
    })
    .map_err(|e| bad(CMD, e.to_string()))
}

/// The tool the call cuts with: a tool of this design.
fn tool(s: &Session, p: &Value) -> Result<Tool> {
    let id = p.get("tool").and_then(Value::as_u64).ok_or_else(|| bad(CMD, "`tool` must be a tool's id in this design"))?;
    s.doc.cam.tool(id).cloned().ok_or_else(|| bad(CMD, format!("there is no tool {id}")))
}

/// The diameter that, times the stepover, spaces the passes: a cutter with a real size.
fn diameter(tool: &Tool) -> Result<f64> {
    if !(tool.diameter.is_finite() && tool.diameter > 0.0 && tool.diameter <= 1000.0) {
        return Err(bad(CMD, format!("tool {} has no diameter ({} mm) to space the passes by", tool.id, tool.diameter)));
    }
    Ok(tool.diameter)
}

/// `stepover` of the call: a fraction of the tool's diameter from 0.1 to 1.
fn stepover(p: &Value) -> Result<f64> {
    match p.get("stepover") {
        None | Some(Value::Null) => Ok(DEFAULT_STEPOVER),
        Some(_) => p
            .get("stepover")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && (0.1..=1.0).contains(value))
            .ok_or_else(|| bad(CMD, format!("`stepover` {FACING_STEPOVER_PROBLEM}"))),
    }
}

/// `pass_extension`, `stock_offset` or `depth` of the call, in mm: a number or an expression,
/// from 0 (or above 0, for `depth`) to 1000.
fn length(s: &Session, p: &Value, key: &str, above_zero: bool) -> Result<Option<f64>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => {
            let given = expr(p, key).ok_or_else(|| bad(CMD, format!("`{key}` must be a number or an expression")))?;
            let text = s.doc.with_design_unit(given.as_str(), Kind::Length);
            let value = s.doc.eval(&text, Kind::Length).map_err(|e| bad(CMD, format!("`{key}`: {e}")))?;
            let problem = if above_zero { FACING_DEPTH_PROBLEM } else { FACING_OFFSET_PROBLEM };
            let ok = value.is_finite() && if above_zero { value > 0.0 } else { value >= 0.0 } && value <= 1000.0;
            if !ok {
                return Err(bad(CMD, format!("`{key}` {problem}")));
            }
            Ok(Some(value))
        }
    }
}

/// The setup's X/Y plane, after checking that its frame is a set of unit, perpendicular axes.
fn plane(frame: &super::cam_toolpath::Frame) -> Result<Plane> {
    let unit = |v: solidcore_record::Vec3| v.is_finite() && (v.len() - 1.0).abs() <= DIM_TOL;
    if !(unit(frame.x) && unit(frame.y) && unit(frame.z)) {
        return Err(bad(CMD, "the setup's coordinate system axes must be unit vectors"));
    }
    if frame.x.dot(frame.y).abs() > DIM_TOL {
        return Err(bad(CMD, "the setup's X and Y axes must be perpendicular"));
    }
    if frame.x.cross(frame.y).dot(frame.z) <= 1.0 - DIM_TOL {
        return Err(bad(CMD, "the setup's coordinate system is not right-handed: X cross Y must point along Z"));
    }
    Ok(Plane { origin: frame.origin, x: frame.x, y: frame.y })
}

/// The bodies the operation faces: the setup's bodies, or every solid body when it names none.
fn body_names(s: &Session, setup: Option<&Setup>) -> Result<Vec<String>> {
    let mut names = setup.map(|setup| setup.bodies.clone()).unwrap_or_default();
    if names.is_empty() {
        let state = s.world_state()?;
        names = state.bodies.iter().filter(|found| !found.body.is_mesh()).map(|found| found.name.clone()).collect();
    }
    if names.is_empty() {
        return Err(bad(CMD, "there is nothing to face: the design has no solid body"));
    }
    Ok(names)
}

/// The box the operation clears: the setup's stock where it states a size, else the exact extent
/// of the machined bodies enlarged by `stock_offset` on each side.
struct Stock {
    /// Its corner nearest the machine zero, in the setup's X/Y.
    min: solidcore_record::Vec2,
    /// Its corner furthest from the machine zero, in the setup's X/Y.
    max: solidcore_record::Vec2,
    /// Its top along the setup's Z, in mm above the machine zero.
    top: f64,
    /// Its bottom along the setup's Z, in mm above the machine zero.
    bottom: f64,
}

fn stock(state: &ModelState, setup: Option<&Setup>, plane: &Plane, names: &[String], stock_offset: f64) -> Result<Stock> {
    let mut lowest = f64::INFINITY;
    let mut highest = f64::NEG_INFINITY;
    let mut local = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name.as_str()) {
            continue;
        }
        let found = state.body(name).ok_or_else(|| bad(CMD, format!("no body `{name}`")))?;
        if found.body.is_mesh() {
            return Err(bad(CMD, format!("body `{name}` is a mesh body: a toolpath reads exact B-rep geometry")));
        }
        let bounds = found.body.exact_bounds().map_err(|e| bad(CMD, format!("cannot read the exact extent of body `{name}`: {e}")))?;
        for corner in corners(&bounds) {
            let height = plane.height(corner);
            lowest = lowest.min(height);
            highest = highest.max(height);
            let point = plane.to_local(corner);
            local = [local[0].min(point.x), local[1].min(point.y), local[2].max(point.x), local[3].max(point.y)];
        }
    }
    if !(lowest.is_finite() && highest.is_finite()) {
        return Err(bad(CMD, "there is nothing to face: the machined bodies have no extent"));
    }
    let [body_min_x, body_min_y, body_max_x, body_max_y] = local;
    let (min_x, max_x) = match setup {
        Some(setup) if setup.stock.size[0].is_finite() && setup.stock.size[0] > 0.0 => {
            (setup.stock.offset[0], setup.stock.offset[0] + setup.stock.size[0])
        }
        _ => (body_min_x - stock_offset, body_max_x + stock_offset),
    };
    let (min_y, max_y) = match setup {
        Some(setup) if setup.stock.size[1].is_finite() && setup.stock.size[1] > 0.0 => {
            (setup.stock.offset[1], setup.stock.offset[1] + setup.stock.size[1])
        }
        _ => (body_min_y - stock_offset, body_max_y + stock_offset),
    };
    let (bottom, top) = match setup {
        Some(setup) if setup.stock.size[2].is_finite() && setup.stock.size[2] > 0.0 => {
            let bottom = setup.stock.offset[2];
            (bottom, bottom + setup.stock.size[2])
        }
        _ => (lowest, highest),
    };
    if !(min_x.is_finite() && max_x.is_finite() && min_y.is_finite() && max_y.is_finite() && bottom.is_finite() && top.is_finite()) {
        return Err(bad(CMD, "the stock has no finite extent"));
    }
    Ok(Stock { min: solidcore_record::Vec2::new(min_x, min_y), max: solidcore_record::Vec2::new(max_x, max_y), top, bottom })
}

/// The eight corners of `bounds`.
fn corners(bounds: &Aabb3) -> [solidcore_record::Vec3; 8] {
    let (min, max) = (bounds.min, bounds.max);
    [
        solidcore_record::Vec3::new(min.x, min.y, min.z),
        solidcore_record::Vec3::new(max.x, min.y, min.z),
        solidcore_record::Vec3::new(min.x, max.y, min.z),
        solidcore_record::Vec3::new(max.x, max.y, min.z),
        solidcore_record::Vec3::new(min.x, min.y, max.z),
        solidcore_record::Vec3::new(max.x, min.y, max.z),
        solidcore_record::Vec3::new(min.x, max.y, max.z),
        solidcore_record::Vec3::new(max.x, max.y, max.z),
    ]
}

/// The boustrophedon at one cut level: its points in machine coordinates, its cutting length and
/// the rapid length between passes.
fn path(plane: &Plane, stock: &Stock, level: f64, span_x: f64, extension: f64, lines: usize, intervals: usize) -> (Vec<[f64; 3]>, f64, f64) {
    let normal = plane.normal();
    let at = |point: solidcore_record::Vec2| {
        let world = plane.to_world(point) + normal * level;
        [world.x, world.y, world.z]
    };
    let left = stock.min.x - extension;
    let right = stock.max.x + extension;
    let bottom = stock.min.y - extension;
    let dy = (stock.max.y - stock.min.y + 2.0 * extension) / intervals as f64;
    let mut waypoints = Vec::with_capacity(lines.saturating_mul(2));
    let mut cutting_length = 0.0;
    let mut rapid_length = 0.0;
    for index in 0..lines {
        let y = bottom + index as f64 * dy;
        let (from, to) = if index % 2 == 0 { (left, right) } else { (right, left) };
        if index > 0 {
            rapid_length += dy;
        }
        cutting_length += span_x;
        waypoints.push(at(solidcore_record::Vec2::new(from, y)));
        waypoints.push(at(solidcore_record::Vec2::new(to, y)));
    }
    (waypoints, cutting_length, rapid_length)
}

/// A new facing operation name: 1–128 trimmed characters, not another facing operation's. A blank
/// or absent name takes "2D Facing" and the first number no facing operation's name has.
fn new_name(s: &Session, given: Option<&str>) -> Result<String> {
    let given = given.map(str::trim).filter(|name| !name.is_empty());
    let name = match given {
        Some(name) => name.to_string(),
        None => {
            let names: BTreeSet<&str> = s.doc.cam.facings.iter().map(|facing| facing.name.as_str()).collect();
            let number = (1..=s.doc.cam.facings.len().saturating_add(1)).find(|n| !names.contains(format!("2D Facing {n}").as_str())).unwrap_or(1);
            format!("2D Facing {number}")
        }
    };
    if name.chars().count() > 128 {
        return Err(bad(CMD, "`name` must have 1–128 characters"));
    }
    if s.doc.cam.facings.iter().any(|facing| facing.name == name) {
        return Err(bad(CMD, format!("a facing operation named `{name}` already exists")));
    }
    Ok(name)
}

fn summary(facing: &CamFacing) -> CamToolpathFacingSummary {
    CamToolpathFacingSummary {
        id: facing.id,
        name: facing.name.clone(),
        setup: facing.setup,
        tool: facing.tool,
        stepover: facing.stepover,
        pass_extension: facing.pass_extension,
        stock_offset: facing.stock_offset,
        depth: facing.depth,
    }
}

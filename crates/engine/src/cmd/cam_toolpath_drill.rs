//! CAM ▸ TOOLPATH: a drilling operation (`cam.toolpath.drill`).
//!
//! A drilling call names a setup, a tool, the hole centers it drills and the cycle and depth it
//! runs at. A hole center is the persistent name of a hole's rim (the name `model.edges` lists
//! for it), and a rim is an exact circular edge of a solid body: the kernel reads its exact
//! B-rep curve ([`solidcraft_kernel::edge_circle`]) for the hole's center, its axis and its
//! radius, so nothing here is measured on a mesh and the hole follows the part as it changes.
//!
//! The bore's other end is its other exact rim: a circular edge of the same body, on the same
//! axis and of the same radius, at a lower level. `hole_bottom` cuts to the lowest such rim, so a
//! through hole is drilled right through and a flat-bottomed blind hole stops on its floor. Only
//! a hole with a single rim (a drill point, say) has no exact bottom, and it is refused rather
//! than guessed: the call gives `depth_reference: absolute` instead.
//!
//! The toolpath is generated at once, in the setup's machine coordinates: for each hole the
//! cutter rapid-moves to the clearance height above it, plunges in the cycle's pecks down to the
//! depth, and retracts. A `standard` cycle plunges in one move, a `peck` cycle retracts clear of
//! the hole between pecks, and a `deep_hole` cycle retracts only to the hole's top. Creating a
//! drilling operation is undoable like any other design change, and a refused call changes
//! nothing.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;
use solidcore_record::{Plane, Vec3};
use solidcraft_doc::ModelState;
use solidcraft_doc::cam::{CamDrill, Setup, Tool};
use solidcraft_doc::naming::edge_names;
use solidcraft_geom::Aabb3;
use solidcraft_geom::prelude::*;
use solidcraft_kernel::{EdgeCircle, edge_circle};

use super::CommandSpec;
use super::cam_toolpath::{Frame, frame, setup};
use crate::params::{bad, num, str_};
use crate::schema::TypedContract;
use crate::schema::cam_toolpath::{CLEARANCE_HEIGHT_PROBLEM, DEPTH_PROBLEM, RATE_PROBLEM};
use crate::schema::cam_toolpath_drill::{
    CamToolpathDrillOutput, CamToolpathDrillParams, CamToolpathDrillSummary, DrillCycleParam, DrillDepthReferenceParam, HOLE_CENTERS_PROBLEM,
    HOLE_NAME_PROBLEM, PECK_DEPTH_PROBLEM,
};
use crate::schema::output_schema_for;
use crate::{Result, Session};

const CMD: &str = "cam.toolpath.drill";

const CONTRACT: TypedContract<CamToolpathDrillParams> = TypedContract::new();

/// The most drilling operations a design may hold.
const MAX_DRILLS: usize = 1000;
/// The most hole centers one call may drill, as the contract's list allows.
const MAX_HOLES: usize = 1000;
/// The most points one toolpath may report.
const MAX_WAYPOINTS: usize = 1_000_000;
/// A coordinate, depth or axis this close to its own value is that value.
const DIM_TOL: f64 = 1e-6;
/// How far a rim's sweep may fall short of a whole turn and still be a whole hole's rim.
const ANGLE_TOL: f64 = 1e-4;
/// How far two axes may be from parallel to be the same axis.
const AXIS_TOL: f64 = 1e-6;
/// How far a rim may sit off the bore it names, as a fraction of the body's size.
const RIM_FRACTION: f64 = 1e-4;
/// The least tolerance a rim is matched with, in mm.
const RIM_TOL_MIN: f64 = 1e-6;
/// How far the cutter travels in a minute to the second.
const SECONDS_PER_MINUTE: f64 = 60.0;

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new("cam.toolpath.drill", "Drilling", crate::checked!("cam.toolpath.drill", drill))
    .at("TOOLPATH", "2D MILLING")
    .icon("cam")
    .doc_only()
    .contract(&CONTRACT)
    .output(output_schema_for::<CamToolpathDrillOutput>)
    .params(
        "tool: a tool's id in this design; hole_centers: [the persistent name of each hole's rim, as model.edges lists it] (1–1000 whole circular edges of solid bodies, each a hole along the setup's Z); setup?: a setup's id (default 0: the model origin and axes, with the stock read from the holes' bodies); cycle_type?: \"standard\" | \"peck\" | \"deep_hole\" (default \"standard\"); depth_reference?: \"hole_bottom\" | \"model_bottom\" | \"absolute\" (default \"hole_bottom\"); clearance_height?: mm above the stock's top (default 0); peck_depth?: mm per peck (default 0: the tool's flute length); depth?: mm below the stock's top (needed by \"absolute\"); feed_rate?, plunge_rate?: mm per minute; spindle_speed?: rpm (each default 0: not stated); name?: 1–128 characters (default: \"Drilling\" and the first free number)",
    )];

fn drill(s: &mut Session, p: &Value) -> Result<Value> {
    if s.doc.cam.drills.len() >= MAX_DRILLS {
        return Err(bad(CMD, "too many drilling operations"));
    }
    let tool = tool(s, p)?;
    let diameter = tool_diameter(&tool)?;
    let setup = setup(s, p, CMD)?;
    let frame = frame(s, setup.as_ref(), CMD)?;
    let plane = plane(&frame)?;
    let cycle = cycle_type(p)?;
    let reference = depth_reference(p)?;
    let clearance_height = clearance_height(p)?;
    let peck_depth = peck_depth(p)?;
    let depth = depth(p)?;
    if reference == DrillDepthReferenceParam::Absolute && depth.is_none() {
        return Err(bad(CMD, "`depth` is needed when `depth_reference` is `absolute`"));
    }
    let feed_rate = rate(p, "feed_rate")?;
    let plunge_rate = rate(p, "plunge_rate")?;
    // A drilling operation stores no spindle speed yet, but a call that states a hostile one is
    // refused like any other value out of range.
    rate(p, "spindle_speed")?;
    let names = hole_center_names(p)?;
    let state = s.world_state()?;
    let holes = resolve(&state, &plane, &frame, &names)?;
    let (stock_top, stock_bottom) = stock(&state, setup.as_ref(), &plane, &holes)?;
    let clearance_level = stock_top + clearance_height;
    let model_bottom = match reference {
        DrillDepthReferenceParam::ModelBottom => Some(model_bottom(&state, setup.as_ref(), &plane)?),
        _ => None,
    };
    let mut plan: Vec<Move> = Vec::new();
    let mut pecks = 0usize;
    for hole in &holes {
        if diameter > 2.0 * hole.radius + DIM_TOL {
            return Err(bad(
                CMD,
                format!("tool {} is {diameter} mm wide, wider than the {:.3} mm hole at [{}, {}]", tool.id, 2.0 * hole.radius, hole.x, hole.y),
            ));
        }
        let cut = match reference {
            DrillDepthReferenceParam::HoleBottom => hole.bottom,
            DrillDepthReferenceParam::ModelBottom => model_bottom.unwrap_or(hole.bottom),
            DrillDepthReferenceParam::Absolute => stock_top - depth.unwrap_or(0.0),
        };
        if !(cut < hole.top - DIM_TOL) {
            return Err(bad(
                CMD,
                format!("the hole at [{}, {}] has no depth to drill: its top is at {:.3} mm and its bottom at {cut:.3} mm", hole.x, hole.y, hole.top),
            ));
        }
        if cut < stock_bottom - DIM_TOL {
            return Err(bad(CMD, format!("a cut to {cut:.3} mm is deeper than the stock (it spans {stock_bottom:.3} to {stock_top:.3} mm)")));
        }
        if clearance_level < hole.top - DIM_TOL {
            return Err(bad(
                CMD,
                format!(
                    "`clearance_height` {clearance_height} mm above the stock's top is below the hole at [{}, {}], whose top is {:.3} mm",
                    hole.x, hole.y, hole.top
                ),
            ));
        }
        let to_cut = hole.top - cut;
        let peck = match cycle {
            DrillCycleParam::Standard => None,
            DrillCycleParam::Peck | DrillCycleParam::DeepHole => {
                let peck = match peck_depth {
                    Some(peck) if peck > 0.0 => peck,
                    _ => tool.flute_length,
                };
                if !(peck.is_finite() && peck > 0.0) {
                    return Err(bad(CMD, format!("tool {} states no flute length to peck by: give `peck_depth`", tool.id)));
                }
                if peck > to_cut + DIM_TOL {
                    return Err(bad(CMD, format!("`peck_depth` {peck} mm is deeper than the hole at [{}, {}] ({to_cut} mm)", hole.x, hole.y)));
                }
                Some(peck)
            }
        };
        let peck_count = peck.map_or(1, |peck| pecks_for(to_cut, peck));
        pecks = pecks.checked_add(peck_count).filter(|count| *count <= MAX_WAYPOINTS).ok_or_else(|| {
            bad(CMD, format!("the operation takes more than {MAX_WAYPOINTS} pecks: a deeper `peck_depth`, or fewer holes, is needed"))
        })?;
        plan.reserve(peck_count * 3 + 2);
        plan.push(Move { point: [hole.x, hole.y, clearance_level], cutting: false });
        match (cycle, peck) {
            (DrillCycleParam::Standard, _) => {
                plan.push(Move { point: [hole.x, hole.y, hole.top], cutting: false });
                plan.push(Move { point: [hole.x, hole.y, cut], cutting: true });
                plan.push(Move { point: [hole.x, hole.y, clearance_level], cutting: false });
            }
            (DrillCycleParam::Peck | DrillCycleParam::DeepHole, Some(peck)) => {
                let inside = if cycle == DrillCycleParam::DeepHole { hole.top } else { clearance_level };
                let mut start = hole.top;
                for number in 1..=peck_count {
                    let level = (hole.top - peck * number as f64).max(cut);
                    plan.push(Move { point: [hole.x, hole.y, start], cutting: false });
                    plan.push(Move { point: [hole.x, hole.y, level], cutting: true });
                    plan.push(Move { point: [hole.x, hole.y, inside], cutting: false });
                    start = level;
                }
                if cycle == DrillCycleParam::DeepHole {
                    plan.push(Move { point: [hole.x, hole.y, clearance_level], cutting: false });
                }
            }
            (DrillCycleParam::Peck | DrillCycleParam::DeepHole, None) => {
                return Err(bad(CMD, "the cycle needs a `peck_depth` above 0"));
            }
        }
        if plan.len() > MAX_WAYPOINTS {
            return Err(bad(CMD, format!("the toolpath has more than {MAX_WAYPOINTS} points: a deeper `peck_depth`, or fewer holes, is needed")));
        }
    }
    let plan = dedupe(plan);
    let (cutting_length, rapid_length) = lengths(&plan);
    let cycle_time = cycle_time(cutting_length, rapid_length, plunge_rate, feed_rate);
    let name = new_name(s, str_(p, "name"))?;
    let drill = CamDrill {
        id: 0,
        name,
        setup: setup.as_ref().map(|setup| setup.id).unwrap_or(0),
        tool: tool.id,
        hole_centers: holes.iter().map(|hole| hole.name.clone()).collect(),
        cycle_type: cycle.to_doc(),
        depth_reference: reference.to_doc(),
        clearance_height,
        peck_depth: peck_depth.unwrap_or(0.0),
        depth: depth.unwrap_or(0.0),
    };
    let (drill, count) = {
        let doc = s.doc_mut();
        let id = doc.allocate_id()?;
        let mut stored = drill;
        stored.id = id;
        doc.cam.drills.push(stored.clone());
        (stored, doc.cam.drills.len())
    };
    serde_json::to_value(CamToolpathDrillOutput {
        drill: summary(&drill),
        count,
        holes: holes.len(),
        pecks,
        waypoints: plan.iter().map(|step| step.point).collect(),
        cutting_length,
        rapid_length,
        cycle_time,
        param_warnings: Vec::new(),
    })
    .map_err(|e| bad(CMD, e.to_string()))
}

/// One move of the generated toolpath: its point in the setup's machine coordinates, in mm, and
/// whether the cutter is in material (else it rapid-moves there).
struct Move {
    point: [f64; 3],
    cutting: bool,
}

/// The moves without the ones that stay where the cutter already is.
fn dedupe(plan: Vec<Move>) -> Vec<Move> {
    let mut out: Vec<Move> = Vec::with_capacity(plan.len());
    for step in plan {
        if let Some(last) = out.last()
            && distance(last.point, step.point) <= DIM_TOL
        {
            continue;
        }
        out.push(step);
    }
    out
}

/// The cutting length and the rapid length of the moves, in mm.
fn lengths(plan: &[Move]) -> (f64, f64) {
    let mut cutting = 0.0;
    let mut rapid = 0.0;
    for pair in plan.windows(2) {
        let (Some(from), Some(to)) = (pair.first(), pair.get(1)) else { continue };
        let length = distance(from.point, to.point);
        if to.cutting {
            cutting += length;
        } else {
            rapid += length;
        }
    }
    (cutting, rapid)
}

/// How far apart two points of the toolpath are, in mm.
fn distance(from: [f64; 3], to: [f64; 3]) -> f64 {
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// How many pecks cut `depth` at `peck` each; the last takes the rest.
fn pecks_for(depth: f64, peck: f64) -> usize {
    let exact = (depth / peck - 1e-9).ceil();
    if exact.is_finite() && exact >= 1.0 { exact as usize } else { 1 }
}

/// How long the moves take, in seconds: the cutting at the plunge rate, the rapids at the feed
/// rate, each counted only when it is stated (0: unknown, and the operation takes no time).
fn cycle_time(cutting: f64, rapid: f64, plunge_rate: f64, feed_rate: f64) -> f64 {
    let mut seconds = 0.0;
    if cutting > 0.0 && plunge_rate > 0.0 {
        seconds += cutting / plunge_rate * SECONDS_PER_MINUTE;
    }
    if rapid > 0.0 && feed_rate > 0.0 {
        seconds += rapid / feed_rate * SECONDS_PER_MINUTE;
    }
    seconds
}

/// The tool the call drills with: a tool of this design.
fn tool(s: &Session, p: &Value) -> Result<Tool> {
    let id = p.get("tool").and_then(Value::as_u64).ok_or_else(|| bad(CMD, "`tool` must be a tool's id in this design"))?;
    s.doc.cam.tool(id).cloned().ok_or_else(|| bad(CMD, format!("there is no tool {id}")))
}

/// The diameter the tool cuts, which must fit the holes it drills.
fn tool_diameter(tool: &Tool) -> Result<f64> {
    let diameter = tool.diameter;
    if !(diameter.is_finite() && diameter > 0.0 && diameter <= 1000.0) {
        return Err(bad(CMD, format!("tool {} has no diameter ({} mm)", tool.id, diameter)));
    }
    Ok(diameter)
}

/// The setup's X/Y plane, after checking that its frame is a set of unit, perpendicular axes.
fn plane(frame: &Frame) -> Result<Plane> {
    let unit = |v: Vec3| v.is_finite() && (v.len() - 1.0).abs() <= AXIS_TOL;
    if !(unit(frame.x) && unit(frame.y) && unit(frame.z)) {
        return Err(bad(CMD, "the setup's coordinate system axes must be unit vectors"));
    }
    if frame.x.dot(frame.y).abs() > AXIS_TOL {
        return Err(bad(CMD, "the setup's X and Y axes must be perpendicular"));
    }
    if frame.x.cross(frame.y).dot(frame.z) <= 1.0 - AXIS_TOL {
        return Err(bad(CMD, "the setup's coordinate system is not right-handed: X cross Y must point along Z"));
    }
    Ok(Plane { origin: frame.origin, x: frame.x, y: frame.y })
}

/// `cycle_type` of the call: `standard` unless it says otherwise.
fn cycle_type(p: &Value) -> Result<DrillCycleParam> {
    match p.get("cycle_type") {
        None | Some(Value::Null) => Ok(DrillCycleParam::Standard),
        Some(value) => serde_json::from_value(value.clone()).map_err(|e| bad(CMD, format!("`cycle_type` {e}"))),
    }
}

/// `depth_reference` of the call: `hole_bottom` unless it says otherwise.
fn depth_reference(p: &Value) -> Result<DrillDepthReferenceParam> {
    match p.get("depth_reference") {
        None | Some(Value::Null) => Ok(DrillDepthReferenceParam::HoleBottom),
        Some(value) => serde_json::from_value(value.clone()).map_err(|e| bad(CMD, format!("`depth_reference` {e}"))),
    }
}

/// `clearance_height` of the call: 0 to 1000 mm above the stock's top; absent is 0.
fn clearance_height(p: &Value) -> Result<f64> {
    match p.get("clearance_height") {
        None | Some(Value::Null) => Ok(0.0),
        Some(_) => num(p, "clearance_height")
            .filter(|value| (0.0..=1000.0).contains(value))
            .ok_or_else(|| bad(CMD, format!("`clearance_height` {CLEARANCE_HEIGHT_PROBLEM}"))),
    }
}

/// `peck_depth` of the call: 0 (the tool's flute length) to 1000 mm; absent is none.
fn peck_depth(p: &Value) -> Result<Option<f64>> {
    match p.get("peck_depth") {
        None | Some(Value::Null) => Ok(None),
        Some(_) => num(p, "peck_depth")
            .filter(|value| (0.0..=1000.0).contains(value))
            .map(Some)
            .ok_or_else(|| bad(CMD, format!("`peck_depth` {PECK_DEPTH_PROBLEM}"))),
    }
}

/// `depth` of the call: above 0 and at most 1000 mm; absent is none.
fn depth(p: &Value) -> Result<Option<f64>> {
    match p.get("depth") {
        None | Some(Value::Null) => Ok(None),
        Some(_) => {
            num(p, "depth").filter(|value| *value > 0.0 && *value <= 1000.0).map(Some).ok_or_else(|| bad(CMD, format!("`depth` {DEPTH_PROBLEM}")))
        }
    }
}

/// `feed_rate`, `plunge_rate` or `spindle_speed` of the call: 0 to 1,000,000; absent is 0.
fn rate(p: &Value, key: &str) -> Result<f64> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(0.0),
        Some(_) => num(p, key).filter(|value| (0.0..=1_000_000.0).contains(value)).ok_or_else(|| bad(CMD, format!("`{key}` {RATE_PROBLEM}"))),
    }
}

/// The `hole_centers` list of the call: 1 to 1000 persistent rim names.
fn hole_center_names(p: &Value) -> Result<Vec<String>> {
    let items = p.get("hole_centers").and_then(Value::as_array).ok_or_else(|| bad(CMD, format!("`hole_centers` {HOLE_CENTERS_PROBLEM}")))?;
    if items.is_empty() || items.len() > MAX_HOLES {
        return Err(bad(CMD, format!("`hole_centers` {HOLE_CENTERS_PROBLEM}")));
    }
    items
        .iter()
        .map(|item| match item.as_str() {
            Some(name) if !name.trim().is_empty() && name.len() <= 512 => Ok(name.to_string()),
            _ => Err(bad(CMD, format!("a hole center {HOLE_NAME_PROBLEM}"))),
        })
        .collect()
}

/// One hole a call drills: the body it is in, the persistent name of its picked rim, its center
/// in the setup's X/Y, its radius, and the levels its top and bottom lie at.
struct Hole {
    body: String,
    name: String,
    x: f64,
    y: f64,
    radius: f64,
    top: f64,
    bottom: f64,
}

/// The holes the call names, each read from its exact rim.
fn resolve(state: &ModelState, plane: &Plane, frame: &Frame, names: &[String]) -> Result<Vec<Hole>> {
    let index = edge_index(state);
    let mut circles: HashMap<usize, Vec<EdgeCircle>> = HashMap::new();
    let mut seen = BTreeSet::new();
    let mut holes = Vec::with_capacity(names.len());
    for name in names {
        if !seen.insert(name.as_str()) {
            return Err(bad(CMD, format!("hole center `{name}` is named twice")));
        }
        let (body, edge) = match index.get(name.as_str()) {
            Some(Some(hit)) => *hit,
            Some(None) => return Err(bad(CMD, format!("hole center `{name}` names an edge of more than one body"))),
            None => return Err(bad(CMD, format!("no hole center `{name}`: no edge of the model has that name"))),
        };
        let Some(found) = state.bodies.get(body) else {
            return Err(bad(CMD, format!("no body holds hole center `{name}`")));
        };
        if found.body.is_mesh() {
            return Err(bad(CMD, format!("hole center `{name}` is on a mesh body: a toolpath reads exact B-rep geometry")));
        }
        let edges = circles.entry(body).or_insert_with(|| all_circles(found));
        holes.push(hole(found, edge, edges, name, plane, frame)?);
    }
    Ok(holes)
}

/// Every edge name of the model's solid bodies, and the body and edge each names. A name two
/// edges have is kept as ambiguous (`None`), so a call naming it is refused rather than guessed.
fn edge_index(state: &ModelState) -> HashMap<String, Option<(usize, usize)>> {
    let mut index: HashMap<String, Option<(usize, usize)>> = HashMap::new();
    for (body, found) in state.bodies.iter().enumerate() {
        if found.body.is_mesh() {
            continue;
        }
        for (edge, name) in edge_names(found).into_iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            index.entry(name).and_modify(|hit| *hit = None).or_insert(Some((body, edge)));
        }
    }
    index
}

/// The exact circles the edge curves of `found` lie on.
fn all_circles(found: &solidcraft_doc::ModelBody) -> Vec<EdgeCircle> {
    (0..found.body.record().edges.len()).filter_map(|edge| edge_circle(&found.body, edge)).collect()
}

/// One named hole, read from the exact circle its rim lies on and the exact rims of its bore.
fn hole(found: &solidcraft_doc::ModelBody, edge: usize, circles: &[EdgeCircle], name: &str, plane: &Plane, frame: &Frame) -> Result<Hole> {
    let Some(circle) = edge_circle(&found.body, edge) else {
        return Err(bad(CMD, format!("hole center `{name}` is not on a circle: give a hole's rim")));
    };
    if circle.axis.dot(frame.z).abs() < 1.0 - AXIS_TOL {
        return Err(bad(CMD, format!("hole center `{name}` is not a hole along the setup's Z axis")));
    }
    if !(circle.radius.is_finite() && circle.radius > 0.0) {
        return Err(bad(CMD, format!("hole center `{name}` has no radius to drill")));
    }
    let tolerance = rim_tolerance(found);
    // The picked rim must be the whole circle: an arc of one is a piece of a rim, not a hole.
    let swept: f64 = circles.iter().filter(|other| same_rim(other, &circle, tolerance)).map(|other| other.angle).sum();
    if swept < std::f64::consts::TAU - ANGLE_TOL {
        return Err(bad(CMD, format!("hole center `{name}` is an arc, not the whole rim of a hole")));
    }
    // The bore's two ends: its highest and lowest exact rims, which are the same circle again
    // somewhere along the same axis.
    let (mut top, mut bottom) = (f64::NEG_INFINITY, f64::INFINITY);
    for other in circles.iter().filter(|other| on_bore(other, &circle, tolerance)) {
        let level = plane.height(other.center);
        top = top.max(level);
        bottom = bottom.min(level);
    }
    let center = plane.to_local(circle.center);
    if !(center.x.is_finite() && center.y.is_finite()) {
        return Err(bad(CMD, format!("hole center `{name}` is not a finite point")));
    }
    if !(top > bottom) {
        return Err(bad(CMD, format!("hole center `{name}` has no bottom rim to drill to: give `depth_reference: absolute` and its `depth`")));
    }
    Ok(Hole { body: found.name.clone(), name: name.to_string(), x: center.x, y: center.y, radius: circle.radius, top, bottom })
}

/// How far a rim may sit off the bore it names, in mm.
fn rim_tolerance(found: &solidcraft_doc::ModelBody) -> f64 {
    (found.body.size() * RIM_FRACTION).max(RIM_TOL_MIN)
}

/// Whether `other` is the very circle `picked` lies on (another piece of the same rim).
fn same_rim(other: &EdgeCircle, picked: &EdgeCircle, tolerance: f64) -> bool {
    other.center.dist(picked.center) <= tolerance && (other.radius - picked.radius).abs() <= tolerance
}

/// Whether `other` circles the same bore as `picked`: on its axis, of its radius, anywhere along
/// it.
fn on_bore(other: &EdgeCircle, picked: &EdgeCircle, tolerance: f64) -> bool {
    if (other.radius - picked.radius).abs() > tolerance || other.axis.dot(*picked.axis).abs() < 1.0 - AXIS_TOL {
        return false;
    }
    let offset = other.center - picked.center;
    (offset - *picked.axis * offset.dot(*picked.axis)).len() <= tolerance
}

/// The stock's top and bottom along the setup's Z: its own stock when it states a height, else
/// the exact extent of the bodies the holes lie in.
fn stock(state: &ModelState, setup: Option<&Setup>, plane: &Plane, holes: &[Hole]) -> Result<(f64, f64)> {
    if let Some(stock) = setup.map(|setup| &setup.stock)
        && stock.size[2].is_finite()
        && stock.size[2] > 0.0
    {
        let (top, bottom) = (stock.offset[2] + stock.size[2], stock.offset[2]);
        if top.is_finite() && bottom.is_finite() && top > bottom {
            return Ok((top, bottom));
        }
    }
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    let mut seen = BTreeSet::new();
    for hole in holes {
        if !seen.insert(hole.body.as_str()) {
            continue;
        }
        let found = state.body(&hole.body).ok_or_else(|| bad(CMD, format!("no body `{}`", hole.body)))?;
        let bounds = found.body.exact_bounds().map_err(|e| bad(CMD, format!("cannot read the exact extent of body `{}`: {e}", hole.body)))?;
        for corner in corners(&bounds) {
            let height = plane.height(corner);
            low = low.min(height);
            high = high.max(height);
        }
    }
    if !(high.is_finite() && low.is_finite() && high > low) {
        return Err(bad(CMD, "the bodies the hole centers lie in have no height to drill into"));
    }
    Ok((high, low))
}

/// The lowest exact point of the machined bodies along the setup's Z: the setup's bodies, else
/// every solid body of the design.
fn model_bottom(state: &ModelState, setup: Option<&Setup>, plane: &Plane) -> Result<f64> {
    let wanted = setup.map(|setup| setup.bodies.clone()).unwrap_or_default();
    let mut low = f64::INFINITY;
    let mut seen = false;
    for found in state.bodies.iter().filter(|found| !found.body.is_mesh()) {
        if !wanted.is_empty() && !wanted.contains(&found.name) {
            continue;
        }
        let bounds = found.body.exact_bounds().map_err(|e| bad(CMD, format!("cannot read the exact extent of body `{}`: {e}", found.name)))?;
        for corner in corners(&bounds) {
            low = low.min(plane.height(corner));
        }
        seen = true;
    }
    if !(seen && low.is_finite()) {
        return Err(bad(CMD, "there is nothing to drill: the design has no solid body"));
    }
    Ok(low)
}

/// The eight corners of `bounds`.
fn corners(bounds: &Aabb3) -> [Vec3; 8] {
    let (min, max) = (bounds.min, bounds.max);
    [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, max.y, max.z),
        Vec3::new(max.x, max.y, max.z),
    ]
}

fn summary(drill: &CamDrill) -> CamToolpathDrillSummary {
    CamToolpathDrillSummary {
        id: drill.id,
        name: drill.name.clone(),
        setup: drill.setup,
        tool: drill.tool,
        hole_centers: drill.hole_centers.clone(),
        cycle_type: DrillCycleParam::from_doc(drill.cycle_type),
        depth_reference: DrillDepthReferenceParam::from_doc(drill.depth_reference),
        clearance_height: drill.clearance_height,
        peck_depth: drill.peck_depth,
        depth: drill.depth,
    }
}

/// A new drilling name: 1–128 trimmed characters, not another operation's. A blank or absent
/// name takes "Drilling" and the first number no operation's name has.
fn new_name(s: &Session, given: Option<&str>) -> Result<String> {
    let given = given.map(str::trim).filter(|name| !name.is_empty());
    let name = match given {
        Some(name) => name.to_string(),
        None => {
            let names: BTreeSet<&str> = s
                .doc
                .cam
                .toolpaths
                .iter()
                .map(|toolpath| toolpath.name.as_str())
                .chain(s.doc.cam.contours.iter().map(|contour| contour.name.as_str()))
                .chain(s.doc.cam.drills.iter().map(|drill| drill.name.as_str()))
                .collect();
            let count = s.doc.cam.drills.len().saturating_add(1);
            let number = (1..=count).find(|number| !names.contains(format!("Drilling {number}").as_str())).unwrap_or(1);
            format!("Drilling {number}")
        }
    };
    if name.chars().count() > 128 {
        return Err(bad(CMD, "`name` must have 1–128 characters"));
    }
    if s.doc.cam.toolpaths.iter().any(|toolpath| toolpath.name == name)
        || s.doc.cam.contours.iter().any(|contour| contour.name == name)
        || s.doc.cam.drills.iter().any(|drill| drill.name == name)
    {
        return Err(bad(CMD, format!("an operation named `{name}` already exists")));
    }
    Ok(name)
}

#[cfg(test)]
#[path = "cam_toolpath_drill_tests.rs"]
mod tests;

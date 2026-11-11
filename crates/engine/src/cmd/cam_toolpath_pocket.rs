//! CAM ▸ TOOLPATH: a 2D pocket that clears the region of the faces a call picks
//! (`cam.toolpath.pocket_2d`).
//!
//! A 2D pocket names a setup, a tool, the faces whose region it clears and the values it cuts
//! at. Each picked face must be a planar face of a solid body, parallel to the setup's X/Y; its
//! exact B-rep loops (`Body::planar_edges` on the record's face loops) are the region to clear,
//! its outer loop and the openings inside it. A mesh is never read as the shape: a pick at a
//! point is decided by the exact face planes and the exact loops, and a body that is a mesh is
//! refused.
//!
//! The cutter clears each region with concentric offset passes: the boundary inset by the tool's
//! radius, then by one stepover more, until the next pass no longer fits (the region closes up or
//! splits). The openings inside a region are offset outwards and the passes stop before the
//! cutter reaches them. Every pass rings the boundary at each cut level from the stock's top down
//! to `depth` below it, one `depth_per_pass` apart, deepest last, and a refused call changes
//! nothing: this command stores nothing in the design yet.

use std::collections::BTreeSet;

use serde_json::Value;
use solidcore_record::Vec2;
use solidcore_record::brep::Loop;
use solidcraft_doc::ModelState;
use solidcraft_doc::cam::{Setup, Tool};
use solidcraft_doc::expr::Kind;
use solidcraft_geom::Aabb3;
use solidcraft_geom::prelude::*;

use super::CommandSpec;
use super::cam_toolpath::{frame, pass_levels, passes, setup};
use super::cam_toolpath_contour::path;
use crate::params::{bad, expr};
use crate::schema::TypedContract;
use crate::schema::cam_toolpath::{BoundaryLoop, BoundaryRole, DEPTH_PROBLEM, Pocket2DParams, Pocket2DResult};
use crate::schema::output_schema_for;
use crate::{Result, Session};

const CMD: &str = "cam.toolpath.pocket_2d";

const CONTRACT: TypedContract<Pocket2DParams> = TypedContract::new();

/// The most faces one call may clear, as the contract's list allows.
const MAX_FACES: usize = 1000;
/// The most loops one face may have (its outline and its openings).
const MAX_LOOPS: usize = 1000;
/// The most edges one loop may have.
const MAX_LOOP_EDGES: usize = 65_536;
/// The most points one loop may be read as.
const MAX_PATH_POINTS: usize = 65_536;
/// The most passes one operation may take down to `depth`; more is a parameter mistake.
const MAX_PASSES: usize = 10_000;
/// The most offset passes one region may take; more means the passes never close up.
const MAX_OFFSET_PASSES: usize = 10_000;
/// The most points all the passes of one call may report.
const MAX_WAYPOINTS: usize = 1_000_000;
/// Two loop points closer than this are one point, in mm.
const POINT_TOL: f64 = 1e-9;
/// How far a loop's last point may lie from its first to be the one closed loop, in mm.
const CLOSE_TOL: f64 = 1e-6;
/// A coordinate or depth this close to its own value is that value.
const DIM_TOL: f64 = 1e-6;
/// How far from parallel the setup's Z and a face's normal may be to be the same direction.
const PARALLEL_TOL: f64 = 1e-6;
/// How far a pick may lie from the face it means, as a fraction of the body's size.
const PICK_FRACTION: f64 = 2e-3;
/// How much nearer two faces must be for a pick between them to mean neither, in mm.
const PICK_TIE: f64 = 1e-6;
/// A corner sharper than about 12° is cut back to five times the offset rather than shooting off
/// to infinity (as `cam.export.gcode` offsets its loops).
const MITER_LIMIT: f64 = 5.0;

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new(
    "cam.toolpath.pocket_2d",
    "2D Pocket",
    crate::checked!("cam.toolpath.pocket_2d", pocket_2d),
)
.at("TOOLPATH", "2D MILLING")
.icon("cam")
.doc_only()
.contract(&CONTRACT)
.output(output_schema_for::<Pocket2DResult>)
.params(
    "tool: a tool's id in this design (its radius and the stepover decide the offset passes); boundary_faces: [a point on each face, or its name from model.faces] (1–1000 planar faces, each parallel to the setup's X/Y); setup?: a setup's id (default 0: the model origin and axes, with the stock read from the faces' bodies); stepover: mm between passes, a number or an expression (above 0, at most the tool's diameter); depth: mm below the stock's top, a number or an expression; depth_per_pass?: mm per pass, a number or an expression (default: the whole depth, one pass)",
)];

fn pocket_2d(s: &mut Session, p: &Value) -> Result<Value> {
    let tool = tool(s, p)?;
    let radius = radius(&tool)?;
    let setup = setup(s, p, CMD)?;
    let frame = frame(s, setup.as_ref(), CMD)?;
    let plane = plane(&frame)?;
    let stepover = length(s, p, "stepover")?;
    if stepover > tool.diameter + DIM_TOL {
        return Err(bad(CMD, format!("`stepover` {stepover} mm is wider than the tool's diameter ({} mm)", tool.diameter)));
    }
    let depth = length(s, p, "depth")?;
    let depth_per_pass = match p.get("depth_per_pass") {
        None | Some(Value::Null) => depth,
        Some(_) => length(s, p, "depth_per_pass")?,
    };
    let picks = picks(p)?;
    let state = s.world_state()?;
    let regions = regions(&state, &plane, picks)?;
    let (stock_top, stock_bottom) = stock(&state, setup.as_ref(), &plane, &regions)?;
    if depth > stock_top - stock_bottom + DIM_TOL {
        return Err(bad(CMD, format!("`depth` {depth} mm is deeper than the stock ({} mm)", stock_top - stock_bottom)));
    }
    for region in &regions {
        if !(stock_bottom - DIM_TOL..=stock_top + DIM_TOL).contains(&region.level) {
            return Err(bad(
                CMD,
                format!(
                    "face {} of body `{}` lies {:.3} mm above the machine zero, outside the stock ({stock_bottom:.3} to {stock_top:.3} mm)",
                    region.face, region.body, region.level
                ),
            ));
        }
    }
    let cut = stock_top - depth;
    let lowest = regions.iter().map(|region| region.body.as_str()).collect::<BTreeSet<_>>().into_iter().try_fold(f64::INFINITY, |low, name| {
        let found = state.body(name).ok_or_else(|| bad(CMD, format!("no body `{name}`")))?;
        let bounds = found.body.exact_bounds().map_err(|e| bad(CMD, format!("cannot read the exact extent of body `{name}`: {e}")))?;
        Ok::<f64, crate::EngineError>(corners(&bounds).iter().map(|corner| plane.height(*corner)).fold(low, f64::min))
    })?;
    if cut < lowest - DIM_TOL {
        return Err(bad(
            CMD,
            format!("`depth` {depth} mm reaches {cut:.3} mm above the machine zero, below the machined bodies (they start at {lowest:.3} mm)"),
        ));
    }
    let pass_count = passes(depth, depth_per_pass);
    if pass_count > MAX_PASSES {
        return Err(bad(CMD, format!("`depth_per_pass` {depth_per_pass} mm takes more than {MAX_PASSES} passes")));
    }
    let cut_levels = pass_levels(stock_top, depth, depth_per_pass, pass_count);
    let mut planned = Vec::with_capacity(regions.len());
    for region in &regions {
        let cuts = rings(region, radius, stepover);
        if cuts.is_empty() {
            return Err(bad(
                CMD,
                format!(
                    "the cutter of radius {radius} mm does not fit inside face {} of body `{}`: a smaller tool, or a wider boundary, is needed",
                    region.face, region.body
                ),
            ));
        }
        planned.push(cuts);
    }
    let per_level = planned.iter().flat_map(|cuts| cuts.iter()).map(Pass::points).sum::<usize>();
    if per_level.checked_mul(pass_count).is_none_or(|count| count > MAX_WAYPOINTS) {
        return Err(bad(CMD, format!("the toolpath has more than {MAX_WAYPOINTS} points: a wider stepover, or simpler boundaries, is needed")));
    }
    let mut waypoints = Vec::with_capacity(per_level * pass_count);
    let mut length_out = 0.0;
    for &level in &cut_levels {
        for cuts in &planned {
            for pass in cuts {
                for ring in std::iter::once(&pass.outer).chain(&pass.holes) {
                    length_out += ring_length(ring);
                    waypoints.extend(ring.iter().map(|point| [point.x, point.y, level]));
                    if let Some(first) = ring.first() {
                        waypoints.push([first.x, first.y, level]);
                    }
                }
            }
        }
    }
    let count = planned.iter().flat_map(|cuts| cuts.iter()).map(Pass::count).sum();
    let boundaries = boundaries(&regions);
    serde_json::to_value(Pocket2DResult {
        boundaries,
        passes: pass_count,
        cut_levels,
        loops: count,
        waypoints,
        length: length_out,
        param_warnings: Vec::new(),
    })
    .map_err(|e| bad(CMD, e.to_string()))
}

/// The tool the call cuts with: a tool of this design.
fn tool(s: &Session, p: &Value) -> Result<Tool> {
    let id = p.get("tool").and_then(Value::as_u64).ok_or_else(|| bad(CMD, "`tool` must be a tool's id in this design"))?;
    s.doc.cam.tool(id).cloned().ok_or_else(|| bad(CMD, format!("there is no tool {id}")))
}

/// The radius the cutter offsets the boundaries by.
fn radius(tool: &Tool) -> Result<f64> {
    let radius = tool.diameter / 2.0;
    if !(radius.is_finite() && radius > 0.0 && radius <= 500.0) {
        return Err(bad(CMD, format!("tool {} has no diameter ({} mm) to offset the boundaries by", tool.id, tool.diameter)));
    }
    Ok(radius)
}

/// The setup's X/Y plane, after checking that its frame is a set of unit, perpendicular axes: the
/// offsets and the depths are millimetres along them.
fn plane(frame: &super::cam_toolpath::Frame) -> Result<solidcore_record::Plane> {
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
    Ok(solidcore_record::Plane { origin: frame.origin, x: frame.x, y: frame.y })
}

/// `stepover`, `depth` or `depth_per_pass` of the call, in mm: a number or an expression, above 0
/// and at most 1000 mm.
fn length(s: &Session, p: &Value, key: &str) -> Result<f64> {
    let given = expr(p, key).ok_or_else(|| bad(CMD, format!("`{key}` must be a number or an expression")))?;
    let text = s.doc.with_design_unit(given.as_str(), Kind::Length);
    let value = s.doc.eval(&text, Kind::Length).map_err(|e| bad(CMD, format!("`{key}`: {e}")))?;
    if !(value.is_finite() && value > 0.0 && value <= 1000.0) {
        return Err(bad(CMD, format!("`{key}` {DEPTH_PROBLEM}")));
    }
    Ok(value)
}

/// The `boundary_faces` list of the call: 1 to 1000 face picks, as the contract allows.
fn picks(p: &Value) -> Result<&[Value]> {
    match p.get("boundary_faces") {
        Some(Value::Array(items)) if !items.is_empty() && items.len() <= MAX_FACES => Ok(items),
        Some(Value::Array(_)) => Err(bad(CMD, format!("select 1…{MAX_FACES} faces"))),
        _ => Err(bad(CMD, "`boundary_faces` must be a list of face picks, a point on each face or its name from model.faces")),
    }
}

/// One picked face's region: its exact boundary in the setup's X/Y, its openings, and the height
/// of its plane above the machine zero.
struct Region {
    body: String,
    face: usize,
    /// The face's outer loop, counter-clockwise in the setup's X/Y.
    outer: Vec<Vec2>,
    /// The openings inside it, clockwise.
    holes: Vec<Vec<Vec2>>,
    /// The height of the face's plane above the machine zero, in mm.
    level: f64,
}

/// The regions of the picked faces, each read exactly from the B-rep.
fn regions(state: &ModelState, plane: &solidcore_record::Plane, picks: &[Value]) -> Result<Vec<Region>> {
    let mut out: Vec<Region> = Vec::with_capacity(picks.len());
    for pick in picks {
        let picked = pick_face(state, plane, pick)?;
        if out.iter().any(|region| region.body == picked.body && region.face == picked.face) {
            return Err(bad(CMD, format!("face {} of body `{}` is picked twice", picked.face, picked.body)));
        }
        out.push(picked);
    }
    Ok(out)
}

/// The face a pick names: a point on it, or its name looked up as `face::face_ref` does, then the
/// exact planar face of a solid body nearest that point.
fn pick_face(state: &ModelState, plane: &solidcore_record::Plane, pick: &Value) -> Result<Region> {
    let point = match super::face_ref_form(pick).map_err(|why| bad(CMD, why))? {
        super::FaceRefForm::Point(point) => point,
        super::FaceRefForm::Name(name) => solidcraft_doc::naming::named_face_point(state, None, name).map_err(|why| bad(CMD, why))?,
    };
    nearest(state, plane, point)
}

/// The planar face of a solid body nearest the picked point, among the faces parallel to the
/// setup's X/Y that the point lies on. A point where faces meet means none of them: the call
/// names one instead.
fn nearest(state: &ModelState, plane: &solidcore_record::Plane, point: solidcore_record::Vec3) -> Result<Region> {
    if !point.is_finite() {
        return Err(bad(CMD, "a face pick must be a point with finite coordinates"));
    }
    let local = plane.to_local(point);
    let mut candidates: Vec<(f64, Region)> = Vec::new();
    for found in state.bodies.iter().filter(|found| !found.body.is_mesh()) {
        let tolerance = pick_tolerance(found);
        for face in 0..found.body.face_count() {
            let Some(face_plane) = found.body.planar_face(face) else { continue };
            if face_plane.normal().dot(plane.normal()).abs() < 1.0 - PARALLEL_TOL {
                continue;
            }
            let distance = (point - face_plane.origin).dot(face_plane.normal()).abs();
            if distance > tolerance {
                continue;
            }
            let Ok(region) = face_region(state, plane, &found.name, face) else { continue };
            if !on_face(&region, local) {
                continue;
            }
            candidates.push((distance, region));
        }
    }
    let Some(nearest) = candidates.iter().map(|candidate| candidate.0).reduce(f64::min) else {
        return Err(bad(
            CMD,
            format!("no planar face of a solid body, parallel to the setup's X/Y, lies at [{}, {}, {}]", point.x, point.y, point.z),
        ));
    };
    let mut near = candidates.into_iter().filter(|candidate| candidate.0 <= nearest + PICK_TIE);
    let Some((_, region)) = near.next() else {
        return Err(bad(CMD, "no face lies at the picked point"));
    };
    let meeting = 1 + near.count();
    if meeting > 1 {
        return Err(bad(CMD, format!("[{}, {}, {}] is where {meeting} faces meet: give a point inside one face", point.x, point.y, point.z)));
    }
    Ok(region)
}

/// How near a pick must be to a body's face to mean it.
fn pick_tolerance(found: &solidcraft_doc::ModelBody) -> f64 {
    (found.body.size() * PICK_FRACTION).max(1e-3)
}

/// Whether the point is on the region's face: inside its outer loop and outside its openings, or
/// within [`POINT_TOL`] of one of their edges.
fn on_face(region: &Region, point: Vec2) -> bool {
    let outer = contains(&region.outer, point);
    let hole = region.holes.iter().any(|hole| contains(hole, point));
    outer && !hole || on_loop(&region.outer, point)
}

/// Whether the point lies on the loop's boundary.
fn on_loop(points: &[Vec2], point: Vec2) -> bool {
    (0..points.len()).any(|index| segment_distance(points[index], points[(index + 1) % points.len()], point) <= POINT_TOL)
}

/// The exact region of one face of one body: its outer loop and its openings, in the setup's X/Y.
fn face_region(state: &ModelState, plane: &solidcore_record::Plane, body: &str, face: usize) -> Result<Region> {
    let found = state.body(body).ok_or_else(|| bad(CMD, format!("no body `{body}`")))?;
    if found.body.is_mesh() {
        return Err(bad(CMD, format!("body `{body}` is a mesh body: a toolpath reads exact B-rep geometry")));
    }
    let face_plane = found
        .body
        .planar_face(face)
        .ok_or_else(|| bad(CMD, format!("face {face} of body `{body}` is not planar: a 2D pocket clears planar faces")))?;
    if face_plane.normal().dot(plane.normal()).abs() < 1.0 - PARALLEL_TOL {
        return Err(bad(
            CMD,
            format!("face {face} of body `{body}` is not parallel to the setup's X/Y: a 2D pocket clears faces that look along the setup's Z"),
        ));
    }
    let record = found.body.record();
    let record_face = record.faces.get(face).ok_or_else(|| bad(CMD, format!("body `{body}` has no face {face}")))?;
    if record_face.loops.is_empty() {
        return Err(bad(CMD, format!("face {face} of body `{body}` has no boundary to clear")));
    }
    if record_face.loops.len() > MAX_LOOPS {
        return Err(bad(CMD, format!("face {face} of body `{body}` has more than {MAX_LOOPS} loops")));
    }
    let mut loops = Vec::with_capacity(record_face.loops.len());
    for (index, lp) in record_face.loops.iter().enumerate() {
        loops.push(loop_points(found, plane, face, index, lp)?);
    }
    // The record's first loop is the outer one; seen from the face's own normal it runs
    // counter-clockwise and the openings run clockwise. In the setup's X/Y the face's normal may
    // point either way, so the orientation is set here.
    let mut outer = loops.remove(0);
    orient(&mut outer, true);
    let mut holes = loops;
    for hole in &mut holes {
        orient(hole, false);
    }
    Ok(Region { body: body.to_string(), face, outer, holes, level: plane.height(face_plane.origin) })
}

/// One loop of a face as the closed polyline the cutter follows, in the plan's X/Y, read exactly.
fn loop_points(found: &solidcraft_doc::ModelBody, plane: &solidcore_record::Plane, face: usize, index: usize, lp: &Loop) -> Result<Vec<Vec2>> {
    let Loop::Edges { edges } = lp else {
        return Err(bad(CMD, format!("a loop of face {face} of body `{}` is a single vertex: a 2D pocket needs closed boundaries", found.name)));
    };
    if edges.len() > MAX_LOOP_EDGES {
        return Err(bad(CMD, format!("loop {index} of face {face} of body `{}` has more than {MAX_LOOP_EDGES} edges", found.name)));
    }
    let indices: Vec<usize> = edges.iter().map(|edge| edge.edge as usize).collect();
    let pieces = found.body.planar_edges(plane, &indices);
    let mut points: Vec<Vec2> = Vec::new();
    for (edge, pieces) in edges.iter().zip(pieces) {
        let pieces = pieces.ok_or_else(|| {
            bad(CMD, format!("edge {} of loop {index} of face {face} of body `{}` cannot be read exactly in the setup's X/Y", edge.edge, found.name))
        })?;
        let mut sampled = path(&pieces, edge.edge as usize, &found.name)?;
        if !edge.forward {
            sampled.reverse();
        }
        if let Some(last) = points.last().copied() {
            let Some(first) = sampled.first() else {
                return Err(bad(CMD, format!("edge {} of loop {index} of face {face} of body `{}` has no length", edge.edge, found.name)));
            };
            if last.dist(*first) > CLOSE_TOL {
                return Err(bad(CMD, format!("loop {index} of face {face} of body `{}` does not close", found.name)));
            }
            sampled.remove(0);
        }
        for point in sampled {
            if !point.is_finite() {
                return Err(bad(CMD, format!("loop {index} of face {face} of body `{}` has a curve that is not a finite point", found.name)));
            }
            if points.last().is_none_or(|last| last.dist(point) > POINT_TOL) {
                points.push(point);
            }
        }
        if points.len() > MAX_PATH_POINTS {
            return Err(bad(CMD, format!("loop {index} of face {face} of body `{}` has more than {MAX_PATH_POINTS} points", found.name)));
        }
    }
    if points.len() > 1 && points.first().zip(points.last()).is_some_and(|(a, b)| a.dist(*b) <= CLOSE_TOL) {
        points.pop();
    }
    if points.len() < 3 {
        return Err(bad(CMD, format!("loop {index} of face {face} of body `{}` encloses no area", found.name)));
    }
    Ok(points)
}

/// The loop running counter-clockwise (or clockwise) in the setup's X/Y.
fn orient(points: &mut [Vec2], counter_clockwise: bool) {
    let area = signed_area(points);
    if area != 0.0 && (area > 0.0) != counter_clockwise {
        points.reverse();
    }
}

/// The stock's top and bottom along the setup's Z: its own stock when it states a height, else the
/// exact extent of the bodies the picked faces lie on.
fn stock(state: &ModelState, setup: Option<&Setup>, plane: &solidcore_record::Plane, regions: &[Region]) -> Result<(f64, f64)> {
    if let Some(stock) = setup.map(|setup| &setup.stock)
        && stock.size[2].is_finite()
        && stock.size[2] > 0.0
    {
        let (top, bottom) = (stock.offset[2] + stock.size[2], stock.offset[2]);
        if top.is_finite() && bottom.is_finite() && top > bottom {
            return Ok((top, bottom));
        }
    }
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut seen = BTreeSet::new();
    for region in regions {
        if !seen.insert(region.body.as_str()) {
            continue;
        }
        let found = state.body(&region.body).ok_or_else(|| bad(CMD, format!("no body `{}`", region.body)))?;
        let bounds = found.body.exact_bounds().map_err(|e| bad(CMD, format!("cannot read the exact extent of body `{}`: {e}", region.body)))?;
        for corner in corners(&bounds) {
            let height = plane.height(corner);
            lo = lo.min(height);
            hi = hi.max(height);
        }
    }
    if !(hi.is_finite() && lo.is_finite() && hi > lo) {
        return Err(bad(CMD, "the bodies the picked faces lie on have no height to cut into"));
    }
    Ok((hi, lo))
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

/// One cut level's rings for one region: the boundary inset by the tool's radius and a whole
/// number of stepovers, and the openings inside it offset outwards by as much.
struct Pass {
    outer: Vec<Vec2>,
    holes: Vec<Vec<Vec2>>,
}

impl Pass {
    /// The rings this pass cuts: the boundary, then the openings.
    fn rings(&self) -> impl Iterator<Item = &Vec<Vec2>> {
        std::iter::once(&self.outer).chain(&self.holes)
    }

    /// How many rings it cuts.
    fn count(&self) -> usize {
        1 + self.holes.len()
    }

    /// How many points it cuts, each ring closed.
    fn points(&self) -> usize {
        self.rings().map(|ring| ring.len() + 1).sum()
    }
}

/// The concentric passes that clear `region`: the boundary inset by `radius`, then by one
/// `stepover` more, until the next pass no longer fits inside the region or reaches an opening.
fn rings(region: &Region, radius: f64, stepover: f64) -> Vec<Pass> {
    let mut out = Vec::new();
    let mut distance = radius;
    for _ in 0..MAX_OFFSET_PASSES {
        let Some(outer) = offset_polygon(&region.outer, distance) else { break };
        let mut holes = Vec::with_capacity(region.holes.len());
        let mut covered = false;
        for hole in &region.holes {
            let Some(grown) = offset_polygon(hole, -distance) else {
                covered = true;
                break;
            };
            if outer.iter().any(|point| contains(&grown, *point)) {
                covered = true;
                break;
            }
            holes.push(grown);
        }
        if covered {
            break;
        }
        out.push(Pass { outer, holes });
        distance += stepover;
    }
    out
}

/// The closed polygon `points` moved `distance` into its enclosed area (above 0) or out of it,
/// mitred at the corners; `None` when it does not survive: the next pass no longer fits.
fn offset_polygon(points: &[Vec2], distance: f64) -> Option<Vec<Vec2>> {
    let count = points.len();
    let area = signed_area(points);
    if count < 3 || !distance.is_finite() || distance == 0.0 || area == 0.0 {
        return None;
    }
    let orientation = if area > 0.0 { 1.0 } else { -1.0 };
    let mut moved = Vec::with_capacity(count);
    for index in 0..count {
        let previous = points[(index + count - 1) % count];
        let point = points[index];
        let next = points[(index + 1) % count];
        let before = inward(previous, point, orientation)?;
        let after = inward(point, next, orientation)?;
        let normal = match (before + after).normalized() {
            Some(direction) => {
                let cos_half = direction.dot(after);
                let miter = if cos_half.abs() < 1.0 / MITER_LIMIT { MITER_LIMIT } else { (1.0 / cos_half).clamp(-MITER_LIMIT, MITER_LIMIT) };
                direction * miter
            }
            None => after,
        };
        moved.push(point + normal * distance);
    }
    let moved_area = signed_area(&moved);
    if moved_area == 0.0 || (moved_area > 0.0) != (area > 0.0) {
        return None;
    }
    let fits = if distance > 0.0 { moved_area.abs() < area.abs() } else { moved_area.abs() > area.abs() };
    fits.then_some(moved)
}

/// The unit normal of the edge `a` → `b` pointing into the polygon's enclosed area, given the
/// polygon's orientation (1 for counter-clockwise, -1 for clockwise).
fn inward(a: Vec2, b: Vec2, orientation: f64) -> Option<Vec2> {
    let direction = (b - a).normalized()?;
    let left = direction.perp();
    Some(if orientation > 0.0 { left } else { -left })
}

/// Whether `point` lies inside the closed polygon, its boundary included.
fn contains(points: &[Vec2], point: Vec2) -> bool {
    let count = points.len();
    if count < 3 {
        return false;
    }
    let mut inside = false;
    for index in 0..count {
        let a = points[index];
        let b = points[(index + 1) % count];
        if segment_distance(a, b, point) <= POINT_TOL {
            return true;
        }
        if (a.y > point.y) != (b.y > point.y) {
            let x = a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if point.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// How far `point` lies from the segment `a` → `b`.
fn segment_distance(a: Vec2, b: Vec2, point: Vec2) -> f64 {
    let along = b - a;
    let len2 = along.len2();
    if len2 <= POINT_TOL * POINT_TOL {
        return point.dist(a);
    }
    let t = ((point - a).dot(along) / len2).clamp(0.0, 1.0);
    point.dist(a + along * t)
}

/// Twice the signed area of a closed polygon: positive when counter-clockwise.
fn signed_area(points: &[Vec2]) -> f64 {
    let count = points.len();
    if count < 3 {
        return 0.0;
    }
    (0..count).map(|index| points[index].x * points[(index + 1) % count].y - points[(index + 1) % count].x * points[index].y).sum()
}

/// How far round a ring runs, its closing segment included.
fn ring_length(points: &[Vec2]) -> f64 {
    let count = points.len();
    if count < 2 {
        return 0.0;
    }
    (0..count).map(|index| points[index].dist(points[(index + 1) % count])).sum()
}

/// The picked faces' exact loops, the outer ones before the holes.
fn boundaries(regions: &[Region]) -> Vec<BoundaryLoop> {
    let mut outer = Vec::new();
    let mut holes = Vec::new();
    for region in regions {
        outer.push(BoundaryLoop {
            body: region.body.clone(),
            role: BoundaryRole::Outer,
            length: ring_length(&region.outer),
            points: region.outer.iter().map(|point| [point.x, point.y]).collect(),
        });
        for hole in &region.holes {
            holes.push(BoundaryLoop {
                body: region.body.clone(),
                role: BoundaryRole::Hole,
                length: ring_length(hole),
                points: hole.iter().map(|point| [point.x, point.y]).collect(),
            });
        }
    }
    outer.extend(holes);
    outer
}

#[cfg(test)]
#[path = "cam_toolpath_pocket_tests.rs"]
mod tests;

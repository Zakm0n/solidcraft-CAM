//! CAM ▸ TOOLPATH: a 2D contour that follows the edges a call picks (`cam.toolpath.contour2d`).
//!
//! A 2D contour names a setup, a tool, the edges it follows and the depths it cuts to. The engine
//! reads each picked edge's exact B-rep curve (`Body::planar_edges`), keeps it in the setup's X/Y,
//! and offsets it by the tool's radius to the side the material is not on, so the cutter's side
//! runs along the picked edge. Which side is open is decided exactly, by classifying a point one
//! radius to either side against the body (`Body::classify_point`), never from a mesh.
//!
//! The path follows the exact curve as a polyline within [`PATH_TOL`], mitred at its corners, as
//! `cam.export.gcode` offsets the outlines it writes; the sampled points of an offset path lie
//! that close to the exact offset curve. The cutter then runs the same path at each level from
//! the stock's top down to `bottom_depth`, one `stepdown` apart, deepest last. Creating a contour
//! is undoable like any other design change, and a refused call changes nothing.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;
use solidcore_record::{Plane, Vec2, Vec3};
use solidcraft_doc::ModelState;
use solidcraft_doc::cam::{CamContour2d, Setup, Tool};
use solidcraft_doc::naming::edge_names;
use solidcraft_geom::Aabb3;
use solidcraft_geom::prelude::*;
use solidcraft_kernel::{PlanarPiece, PlanarSpline, PointClass};

use super::cam_toolpath::{Frame, frame, pass_levels, passes, setup};
use super::{CommandSpec, EdgeRefForm, edge_ref_form};
use crate::params::{bad, num, str_};
use crate::schema::TypedContract;
use crate::schema::cam_toolpath::{CLEARANCE_HEIGHT_PROBLEM, CamContour2dOutput, CamContour2dParams, CamContour2dSummary, DEPTH_PROBLEM};
use crate::schema::output_schema_for;
use crate::{Result, Session};

const CMD: &str = "cam.toolpath.contour2d";

const CONTRACT: TypedContract<CamContour2dParams> = TypedContract::new();

/// The most 2D contour operations a design may hold.
const MAX_CONTOURS: usize = 1000;
/// The most edges one call may follow, as the contract's list allows.
const MAX_EDGES: usize = 1000;
/// The most passes one operation may take; more is a parameter mistake, not a plan.
const MAX_PASSES: usize = 10_000;
/// The most points one edge's path may report.
const MAX_PATH_POINTS: usize = 65_536;
/// The most points all the passes of one call may report.
const MAX_WAYPOINTS: usize = 1_000_000;
/// How closely a path follows the exact curve it is read from, in mm.
const PATH_TOL: f64 = 1e-3;
/// How far round an edge's first point its last must lie to be the one closed ring, in mm.
const CLOSED_TOL: f64 = 1e-6;
/// Two path points closer than this are one point, in mm.
const POINT_TOL: f64 = 1e-9;
/// A coordinate, depth or axis this close to its own value is that value.
const DIM_TOL: f64 = 1e-6;
/// How far a pick may lie from the edge it means, as a fraction of the body's size (the distance
/// the kernel re-finds an edge from).
const PICK_FRACTION: f64 = 2e-3;
/// How much nearer two edges must be for a pick between them to mean neither, in mm.
const PICK_TIE: f64 = 1e-6;
/// How far a setup's axes may be from unit, perpendicular vectors.
const AXIS_TOL: f64 = 1e-6;
/// A corner sharper than about 12° is cut back to five times the offset rather than shooting off
/// to infinity (as `cam.export.gcode` offsets its loops).
const MITER_LIMIT: f64 = 5.0;

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new(
    "cam.toolpath.contour2d",
    "2D Contour",
    crate::checked!("cam.toolpath.contour2d", contour2d),
)
.at("TOOLPATH", "2D MILLING")
.icon("cam")
.doc_only()
.contract(&CONTRACT)
.output(output_schema_for::<CamContour2dOutput>)
.params(
    "tool: a tool's id in this design (its radius offsets the edges); target_edges: [a point on each edge, or {body, index}] (1–1000 edges, each in a plane parallel to the setup's X/Y); setup?: a setup's id (default 0: the model origin and axes, with the stock read from the edges' bodies); name?: 1–128 characters (default: \"2D Contour\" and the first free number); clearance_height?: mm above the stock's top (default 0); bottom_depth: mm below the stock's top; stepdown: mm per pass",
)];

fn contour2d(s: &mut Session, p: &Value) -> Result<Value> {
    if s.doc.cam.contours.len() >= MAX_CONTOURS {
        return Err(bad(CMD, "too many 2D contour operations"));
    }
    let tool = tool(s, p)?;
    let radius = radius(&tool)?;
    let setup = setup(s, p, CMD)?;
    let frame = frame(s, setup.as_ref(), CMD)?;
    let plane = plane(&frame)?;
    let state = s.world_state()?;
    let edges = resolve(&state, &plane, picks(p)?)?;
    let (stock_top, stock_bottom) = stock(&state, setup.as_ref(), &plane, &edges)?;
    let bottom_depth = depth(p, "bottom_depth")?;
    if bottom_depth > stock_top - stock_bottom + DIM_TOL {
        return Err(bad(CMD, format!("`bottom_depth` {bottom_depth} mm is deeper than the stock ({} mm)", stock_top - stock_bottom)));
    }
    let stepdown = depth(p, "stepdown")?;
    let clearance_height = clearance_height(p)?;
    let pass_count = passes(bottom_depth, stepdown);
    if pass_count > MAX_PASSES {
        return Err(bad(CMD, format!("`stepdown` {stepdown} mm takes more than {MAX_PASSES} passes")));
    }
    let cut_levels = pass_levels(stock_top, bottom_depth, stepdown, pass_count);
    let paths = offsets(&state, &plane, &edges, radius)?;
    let per_level = paths.iter().map(Vec::len).sum::<usize>();
    if per_level.checked_mul(pass_count).is_none_or(|count| count > MAX_WAYPOINTS) {
        return Err(bad(CMD, format!("the toolpath has more than {MAX_WAYPOINTS} points: a smaller stepdown, or simpler edges, is needed")));
    }
    let mut waypoints = Vec::with_capacity(per_level * pass_count);
    let mut length = 0.0;
    for &level in &cut_levels {
        for path in &paths {
            length += path_length(path);
            waypoints.extend(path.iter().map(|point| [point.x, point.y, level]));
        }
    }
    let name = new_name(s, str_(p, "name"))?;
    let contour = CamContour2d {
        id: 0,
        name,
        setup: setup.as_ref().map(|setup| setup.id).unwrap_or(0),
        tool: tool.id,
        target_edges: edges.iter().map(|edge| edge.name.clone()).collect(),
        clearance_height,
        bottom_depth,
        stepdown,
    };
    let (contour, count) = {
        let doc = s.doc_mut();
        let id = doc.allocate_id()?;
        let mut stored = contour;
        stored.id = id;
        doc.cam.contours.push(stored.clone());
        (stored, doc.cam.contours.len())
    };
    serde_json::to_value(CamContour2dOutput {
        contour: summary(&contour),
        count,
        passes: pass_count,
        cut_levels,
        waypoints,
        length,
        param_warnings: Vec::new(),
    })
    .map_err(|e| bad(CMD, e.to_string()))
}

/// The `target_edges` list of the call: 1 to 1000 picks, as the contract allows.
fn picks(p: &Value) -> Result<&[Value]> {
    match p.get("target_edges") {
        Some(Value::Array(items)) if !items.is_empty() && items.len() <= MAX_EDGES => Ok(items),
        Some(Value::Array(_)) => Err(bad(CMD, "select 1…1000 edges")),
        _ => Err(bad(CMD, "`target_edges` must be a list of edge picks, a point on each edge or {body, index}")),
    }
}

/// The tool the call cuts with: a tool of this design.
fn tool(s: &Session, p: &Value) -> Result<Tool> {
    let id = p.get("tool").and_then(Value::as_u64).ok_or_else(|| bad(CMD, "`tool` must be a tool's id in this design"))?;
    s.doc.cam.tool(id).cloned().ok_or_else(|| bad(CMD, format!("there is no tool {id}")))
}

/// The radius the cutter offsets the edges by.
fn radius(tool: &Tool) -> Result<f64> {
    let radius = tool.diameter / 2.0;
    if !(radius.is_finite() && radius > 0.0 && radius <= 500.0) {
        return Err(bad(CMD, format!("tool {} has no diameter ({} mm) to offset the edges by", tool.id, tool.diameter)));
    }
    Ok(radius)
}

/// The setup's X/Y plane, after checking that its frame is a set of unit, perpendicular axes: the
/// offset and the depths are millimetres along them.
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

/// `bottom_depth` or `stepdown` of the call: above 0 and at most 1000 mm.
fn depth(p: &Value, key: &str) -> Result<f64> {
    num(p, key).filter(|value| *value > 0.0 && *value <= 1000.0).ok_or_else(|| bad(CMD, format!("`{key}` {DEPTH_PROBLEM}")))
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

/// One picked edge: the body it belongs to, its index in that body's edge order, its persistent
/// name, its exact curve in the plane's coordinates, and its height above the plane.
struct Picked {
    body: String,
    index: usize,
    name: String,
    pieces: Vec<PlanarPiece>,
    level: f64,
}

/// The edges the call picks, each read exactly and named for the design to keep.
fn resolve(state: &ModelState, plane: &Plane, picks: &[Value]) -> Result<Vec<Picked>> {
    let mut out = Vec::with_capacity(picks.len());
    let mut names: HashMap<String, Vec<String>> = HashMap::new();
    for pick in picks {
        let (body, index) = pick_edge(state, pick)?;
        let found = state.body(&body).ok_or_else(|| bad(CMD, format!("no body `{body}`")))?;
        let edge = found
            .body
            .edges(edge_tolerance(found))
            .map_err(|e| bad(CMD, format!("cannot read the edges of body `{body}`: {e}")))?
            .into_iter()
            .find(|edge| edge.index == index)
            .ok_or_else(|| bad(CMD, format!("body `{body}` has no edge {index}")))?;
        let level = plane.height(*edge.points.first().ok_or_else(|| bad(CMD, format!("edge {index} of body `{body}` has no points")))?);
        let pieces = found.body.planar_edges(plane, &[index]).into_iter().next().flatten().ok_or_else(|| {
            bad(CMD, format!("edge {index} of body `{body}` does not lie in a plane parallel to the setup's X/Y: a 2D contour follows planar edges"))
        })?;
        let list = names.entry(body.clone()).or_insert_with(|| edge_names(found));
        let name = list.get(index).cloned().unwrap_or_default();
        if name.is_empty() {
            return Err(bad(CMD, format!("edge {index} of body `{body}` has no name a design can store")));
        }
        out.push(Picked { body, index, name, pieces, level });
    }
    Ok(out)
}

/// The body and edge index a pick names: `{body, index}` of a solid body as given, else the edge
/// nearest a point on one.
fn pick_edge(state: &ModelState, pick: &Value) -> Result<(String, usize)> {
    match edge_ref_form(pick).map_err(|why| bad(CMD, why))? {
        EdgeRefForm::Index { body, index } => {
            let index = usize::try_from(index).map_err(|_| bad(CMD, "an edge `index` must be a whole number"))?;
            let found = state.body(body).ok_or_else(|| bad(CMD, format!("no body `{body}`")))?;
            if found.body.is_mesh() {
                return Err(bad(CMD, format!("body `{body}` is a mesh body: a toolpath reads exact B-rep geometry")));
            }
            let count = found.body.edges(edge_tolerance(found)).map_err(|e| bad(CMD, format!("cannot read the edges of body `{body}`: {e}")))?.len();
            if index >= count {
                return Err(bad(CMD, format!("body `{body}` has no edge {index}")));
            }
            Ok((body.to_string(), index))
        }
        EdgeRefForm::Point(pt) => nearest(state, pt),
    }
}

/// The body and edge nearest a picked point, within the distance the kernel re-finds an edge from.
/// A point where several edges meet means none of them: the call names one instead.
fn nearest(state: &ModelState, pt: Vec3) -> Result<(String, usize)> {
    if !pt.is_finite() {
        return Err(bad(CMD, "an edge pick must be a point with finite coordinates"));
    }
    let mut candidates: Vec<(f64, &str, usize, f64)> = Vec::new();
    for found in state.bodies.iter().filter(|found| !found.body.is_mesh()) {
        let tolerance = edge_tolerance(found);
        let edges = found.body.edges(tolerance).map_err(|e| bad(CMD, format!("cannot read the edges of body `{}`: {e}", found.name)))?;
        for edge in &edges {
            candidates.push((polyline_distance(&edge.points, pt), found.name.as_str(), edge.index, tolerance));
        }
    }
    let Some(nearest) = candidates.iter().map(|candidate| candidate.0).reduce(f64::min) else {
        return Err(bad(CMD, "there is nothing to follow: the design has no solid body"));
    };
    let mut near = candidates.iter().filter(|candidate| candidate.0 <= nearest + PICK_TIE);
    let Some(&(distance, body, index, tolerance)) = near.next() else {
        return Err(bad(CMD, format!("no edge at [{}, {}, {}]", pt.x, pt.y, pt.z)));
    };
    let meeting = 1 + near.count();
    if meeting > 1 {
        return Err(bad(
            CMD,
            format!("[{}, {}, {}] is where {meeting} edges meet: give a point along one edge, such as its midpoint", pt.x, pt.y, pt.z),
        ));
    }
    if distance > tolerance {
        return Err(bad(CMD, format!("no edge at [{}, {}, {}]: the nearest edge is {distance:.3} mm away", pt.x, pt.y, pt.z)));
    }
    Ok((body.to_string(), index))
}

/// How far a body's edges are polylined by, and how near a pick must be to mean one.
fn edge_tolerance(found: &solidcraft_doc::ModelBody) -> f64 {
    (found.body.size() * PICK_FRACTION).max(1e-3)
}

/// How far a polyline runs from `p` at its nearest.
fn polyline_distance(points: &[Vec3], p: Vec3) -> f64 {
    points.windows(2).filter_map(|window| Some(p.dist_to_segment(*window.first()?, *window.get(1)?))).fold(f64::INFINITY, f64::min)
}

/// The stock's top and bottom along the setup's Z: its own stock when it states a height, else the
/// exact extent of the bodies the picked edges lie on.
fn stock(state: &ModelState, setup: Option<&Setup>, plane: &Plane, edges: &[Picked]) -> Result<(f64, f64)> {
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
    for edge in edges {
        if !seen.insert(edge.body.as_str()) {
            continue;
        }
        let found = state.body(&edge.body).ok_or_else(|| bad(CMD, format!("no body `{}`", edge.body)))?;
        let bounds = found.body.exact_bounds().map_err(|e| bad(CMD, format!("cannot read the exact extent of body `{}`: {e}", edge.body)))?;
        for corner in corners(&bounds) {
            let height = plane.height(corner);
            lo = lo.min(height);
            hi = hi.max(height);
        }
    }
    if !(hi.is_finite() && lo.is_finite() && hi > lo) {
        return Err(bad(CMD, "the bodies the picked edges lie on have no height to cut into"));
    }
    Ok((hi, lo))
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

/// Each picked edge's path: the cutter's centre a radius to the open side of the edge, in the
/// plane's coordinates, with a closed edge's path closed on itself.
fn offsets(state: &ModelState, plane: &Plane, edges: &[Picked], radius: f64) -> Result<Vec<Vec<Vec2>>> {
    let mut out = Vec::with_capacity(edges.len());
    for edge in edges {
        let found = state.body(&edge.body).ok_or_else(|| bad(CMD, format!("no body `{}`", edge.body)))?;
        let sampled = path(&edge.pieces, edge.index, &edge.body)?;
        let closed = sampled.len() >= 3 && sampled.first().zip(sampled.last()).is_some_and(|(a, b)| a.dist(*b) <= CLOSED_TOL);
        let points = if closed { sampled[..sampled.len() - 1].to_vec() } else { sampled };
        if points.len() < 2 {
            return Err(bad(CMD, format!("edge {} of body `{}` has no length to follow", edge.index, edge.body)));
        }
        let side = open_side(found, plane, edge, &points, radius)?;
        let offset = offset_path(&points, radius * side, closed);
        if offset.len() < 2 || doubles_back(&points, &offset, closed) {
            return Err(bad(
                CMD,
                format!("the cutter of radius {radius} mm is wider than the curve of edge {} of body `{}` follows", edge.index, edge.body),
            ));
        }
        out.push(offset);
    }
    Ok(out)
}

/// Which side of the edge is open, as the sign of the offset: +1 is the path's left, -1 its right.
/// A point one radius to either side of the edge's middle is classified against the body, exactly:
/// the open side is the one outside its material.
fn open_side(found: &solidcraft_doc::ModelBody, plane: &Plane, edge: &Picked, points: &[Vec2], radius: f64) -> Result<f64> {
    let middle = points.len() / 2;
    let (from, to) = (points[middle - 1], points[middle]);
    let direction =
        (to - from).normalized().ok_or_else(|| bad(CMD, format!("edge {} of body `{}` has no length to follow", edge.index, edge.body)))?;
    let at = plane.offset(edge.level).to_world((from + to) * 0.5);
    let normal = plane.dir_to_world(direction.perp()) * radius;
    let side = |point: Vec3| {
        found
            .body
            .classify_point(point)
            .map(|class| class == PointClass::Outside)
            .map_err(|e| bad(CMD, format!("cannot tell what lies beside edge {} of body `{}`: {e}", edge.index, edge.body)))
    };
    match (side(at + normal)?, side(at - normal)?) {
        (true, false) => Ok(1.0),
        (false, true) => Ok(-1.0),
        _ => Err(bad(
            CMD,
            format!(
                "cannot tell which side of edge {} of body `{}` is open: the cutter of radius {radius} mm has material on both sides or on neither",
                edge.index, edge.body
            ),
        )),
    }
}

/// An edge's exact pieces as the polyline the cutter follows, within [`PATH_TOL`].
pub(crate) fn path(pieces: &[PlanarPiece], index: usize, body: &str) -> Result<Vec<Vec2>> {
    let mut points: Vec<Vec2> = Vec::new();
    for piece in pieces {
        let sampled = match piece {
            PlanarPiece::Seg(seg) => seg.polyline(PATH_TOL),
            PlanarPiece::Spline(spline) => spline_points(spline)?,
        };
        for point in sampled {
            if !point.is_finite() {
                return Err(bad(CMD, format!("edge {index} of body `{body}` has a curve that is not a finite point")));
            }
            if points.last().is_none_or(|last| last.dist(point) > POINT_TOL) {
                points.push(point);
            }
            if points.len() > MAX_PATH_POINTS {
                return Err(bad(CMD, format!("edge {index} of body `{body}` has more than {MAX_PATH_POINTS} points: a simpler edge is needed")));
            }
        }
    }
    Ok(points)
}

/// Points along a free-form piece within `tol`, ends included: the curve is halved where the point
/// in the middle of a span stands further than `tol` off its chord.
fn spline_points(spline: &PlanarSpline) -> Result<Vec<Vec2>> {
    let degree = u8::try_from(spline.degree).map_err(|_| bad(CMD, "a free-form edge's spline has too high a degree to read"))?;
    let nurbs = solidcraft_sketch::Nurbs { knots: spline.knots.clone(), weights: spline.weights.clone() };
    nurbs.check(spline.points.len(), degree).map_err(|e| bad(CMD, format!("a free-form edge's spline cannot be read: {e}")))?;
    let at = |t: f64| nurbs.point(&spline.points, degree, t).ok_or_else(|| bad(CMD, "a free-form edge's spline cannot be sampled"));
    let mut points = vec![at(0.0)?];
    let mut stack = vec![(0.0f64, points[0], 1.0f64, at(1.0)?)];
    while let Some((t0, p0, t1, p1)) = stack.pop() {
        let tm = 0.5 * (t0 + t1);
        let pm = at(tm)?;
        let chord = p1 - p0;
        let flat = if chord.len2() <= POINT_TOL * POINT_TOL {
            pm.dist(p0) <= PATH_TOL
        } else {
            let t = (pm - p0).dot(chord) / chord.len2();
            (p0 + chord * t.clamp(0.0, 1.0)).dist(pm) <= PATH_TOL
        };
        if flat || points.len() >= MAX_PATH_POINTS || !(t1 - t0 > 1e-12) {
            points.push(p1);
        } else {
            stack.push((tm, pm, t1, p1));
            stack.push((t0, p0, tm, pm));
        }
    }
    Ok(points)
}

/// A polyline moved `distance` to its left (above 0) or right, mitred at the corners where it
/// turns; a closed path (its first point also its last) is offset as the ring it is and closed
/// again.
fn offset_path(points: &[Vec2], distance: f64, closed: bool) -> Vec<Vec2> {
    let count = points.len();
    if count < 2 || (closed && count < 3) || !distance.is_finite() {
        return Vec::new();
    }
    let left = |from: Vec2, to: Vec2| (to - from).normalized().map(Vec2::perp);
    let mut moved: Vec<Vec2> = Vec::with_capacity(count + usize::from(closed));
    for index in 0..count {
        let before = if closed || index > 0 { left(points[(index + count - 1) % count], points[index]) } else { None };
        let after = if closed || index + 1 < count { left(points[index], points[(index + 1) % count]) } else { None };
        let normal = match (before, after) {
            (Some(before), Some(after)) => {
                let sum = before + after;
                match sum.normalized() {
                    Some(direction) => {
                        let cos_half = direction.dot(after);
                        let miter = if cos_half.abs() < 1.0 / MITER_LIMIT { MITER_LIMIT } else { (1.0 / cos_half).clamp(-MITER_LIMIT, MITER_LIMIT) };
                        direction * miter
                    }
                    None => after,
                }
            }
            (Some(normal), None) | (None, Some(normal)) => normal,
            (None, None) => return Vec::new(),
        };
        moved.push(points[index] + normal * distance);
    }
    if closed && !moved.is_empty() {
        moved.push(moved[0]);
    }
    moved
}

/// Whether an offset path doubles back on the path it came from: a cutter wider than the curve it
/// follows, which would cut the wrong side of it.
fn doubles_back(points: &[Vec2], moved: &[Vec2], closed: bool) -> bool {
    let count = points.len();
    let pairs = if closed { count } else { count.saturating_sub(1) };
    (0..pairs).any(|index| {
        let next = (index + 1) % count;
        let source = points[next] - points[index];
        let offset = moved[next] - moved[index];
        if source.len2() <= POINT_TOL * POINT_TOL || offset.len2() <= POINT_TOL * POINT_TOL {
            return false;
        }
        source.dot(offset) <= 0.0
    })
}

/// How far along `points` the path runs.
fn path_length(points: &[Vec2]) -> f64 {
    points.windows(2).map(|window: &[Vec2]| window[0].dist(window[1])).sum()
}

fn summary(contour: &CamContour2d) -> CamContour2dSummary {
    CamContour2dSummary {
        id: contour.id,
        name: contour.name.clone(),
        setup: contour.setup,
        tool: contour.tool,
        target_edges: contour.target_edges.clone(),
        clearance_height: contour.clearance_height,
        bottom_depth: contour.bottom_depth,
        stepdown: contour.stepdown,
    }
}

/// A new contour name: 1-128 trimmed characters, not another operation's. A blank or absent name
/// takes "2D Contour" and the first number no operation's name has.
fn new_name(s: &Session, given: Option<&str>) -> Result<String> {
    let given = given.map(str::trim).filter(|name| !name.is_empty());
    let name = match given {
        Some(name) => name.to_string(),
        None => {
            let names: BTreeSet<&str> = s
                .doc
                .cam
                .contours
                .iter()
                .map(|contour| contour.name.as_str())
                .chain(s.doc.cam.toolpaths.iter().map(|path| path.name.as_str()))
                .collect();
            let count = s.doc.cam.contours.len().saturating_add(1);
            let number = (1..=count).find(|number| !names.contains(format!("2D Contour {number}").as_str())).unwrap_or(1);
            format!("2D Contour {number}")
        }
    };
    if name.chars().count() > 128 {
        return Err(bad(CMD, "`name` must have 1–128 characters"));
    }
    if s.doc.cam.contours.iter().any(|contour| contour.name == name) || s.doc.cam.toolpaths.iter().any(|path| path.name == name) {
        return Err(bad(CMD, format!("an operation named `{name}` already exists")));
    }
    Ok(name)
}

#[cfg(test)]
#[path = "cam_toolpath_contour_tests.rs"]
mod tests;

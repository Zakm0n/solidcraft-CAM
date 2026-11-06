//! Interactive wireframe creation: the state machine behind the `Wireframe` tab's clicking tools.
//!
//! The workflow this module encodes comes from the reference product's help book that ships with
//! this repository (`mcam-help-docs/`): a line is placed by two endpoints and, while its function
//! stays live, the next click starts another line where the last one ended, so a chain of
//! connected lines costs one click each after the first; an arc is placed by its two endpoints
//! and one point the curve passes through; a point takes a single click and the function stays
//! live for the next one. Positions may be typed instead of clicked (Length and Angle for a line,
//! X, Y and Z for any position), which locks that dimension of the next point.
//!
//! Positions are picked with the same detection order the reference product documents for its
//! cursor: point entities first, then the endpoints of curves, then their midpoints, then arc
//! quadrants and centres, then real curve intersections, then the grid. That picking and the
//! typed fields feed this machine from outside; this module owns the state — when a click
//! anchors a point, when the entity is complete, what the completed entity is — and the document
//! features a pointer snaps to. Painting the rubber-band preview and creating the geometry in the
//! document are the viewport's and the engine's jobs.

use solidcore_record::{Plane, Vec2, Vec3};
use solidcraft_geom::{PlaneGeom, Vec2Geom, Vec3Geom};

use crate::document::{CamDocument, WireTool};

/// Distance below which two positions are the same point, in document units. A line or an arc
/// needs two distinct ends, and an arc's third point must leave the chord by more than this.
pub const POINT_TOLERANCE: f64 = 0.001;

/// The screen-space reach of a position pick, in logical pixels: a click within this distance of
/// a snap candidate takes the candidate instead of the point under the cursor.
pub const PICK_RADIUS_PIXELS: f32 = 8.0;

/// The screen-space reach of a snap, in logical pixels: a pointer within this distance of a
/// document vertex takes that vertex as the position under the cursor.
pub const SNAP_RADIUS_PIXELS: f32 = PICK_RADIUS_PIXELS;

/// How many straight segments the preview of an arc is drawn with: one every 7.5 degrees.
pub const ARC_PREVIEW_SEGMENTS: usize = 48;

/// How close every point of a tessellated curve must lie to one circle, relative to its radius,
/// for that circle's centre to count as the curve's centre.
const CENTRE_TOLERANCE: f64 = 1e-6;

/// The kinds of document feature a pointer can snap to, in the order a tie prefers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
    /// The end of a curve: a square marker.
    Endpoint,
    /// The middle of a curve segment: a triangle marker.
    Midpoint,
    /// The centre of a circle or an arc: a circle marker.
    Centre,
}

impl SnapKind {
    /// How strongly the kind claims the pointer when two candidates are the same distance away:
    /// a lower number wins.
    pub const fn priority(self) -> u8 {
        match self {
            Self::Endpoint => 0,
            Self::Midpoint => 1,
            Self::Centre => 2,
        }
    }
}

/// One document feature a pointer can snap to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapCandidate {
    /// What the feature is, which decides the marker drawn over it.
    pub kind: SnapKind,
    /// Where the feature is, in world coordinates.
    pub at: Vec3,
}

/// Every feature of the document's visible wireframe a pointer may snap to: the ends and segment
/// midpoints of each curve, and the centre of each curve that is a whole circle or an arc.
///
/// The curves are the exact sketch geometry, tessellated only for this search: the vertices are
/// the same points the viewport draws the wireframe from.
pub fn snap_candidates(document: &CamDocument) -> Vec<SnapCandidate> {
    let mut candidates = Vec::new();
    for wire in document.wireframe_lines() {
        let points = wire.points;
        let (Some(first), Some(last)) = (points.first().copied(), points.last().copied()) else { continue };
        candidates.push(SnapCandidate { kind: SnapKind::Endpoint, at: first });
        if last.dist(first) > POINT_TOLERANCE {
            candidates.push(SnapCandidate { kind: SnapKind::Endpoint, at: last });
        }
        for pair in points.windows(2) {
            candidates.push(SnapCandidate { kind: SnapKind::Midpoint, at: pair[0].lerp(pair[1], 0.5) });
        }
        if let Some(at) = curve_centre(&points) {
            candidates.push(SnapCandidate { kind: SnapKind::Centre, at });
        }
    }
    candidates
}

/// The centre of the circle that every one of `points` lies on, when they lie on one: the centre
/// of a drawn circle or arc. `None` for a straight run of points or a free-form curve.
fn curve_centre(points: &[Vec3]) -> Option<Vec3> {
    if points.len() < 3 {
        return None;
    }
    // Three points spread around the curve, so a closed curve whose last point repeats its first
    // still gives a triangle to take the centre from.
    let (a, b, c) = (points[0], points[points.len() / 3], points[(points.len() * 2) / 3]);
    let u = b - a;
    let v = c - a;
    let normal = u.cross(v);
    let span = normal.len();
    if !(span > POINT_TOLERANCE) || !normal.is_finite() {
        return None;
    }
    // Work in the plane's own frame, with `a` at the origin and `b` on the first axis.
    let x_axis = u.normalized()?;
    let y_axis = normal.cross(x_axis) / span;
    let (bx, cy) = (u.len(), v.dot(y_axis));
    if !(cy.abs() > POINT_TOLERANCE) {
        return None;
    }
    let (ox, oy) = (bx / 2.0, ((v.dot(x_axis) * v.dot(x_axis)) + cy * cy - bx * v.dot(x_axis)) / (2.0 * cy));
    let centre = a + x_axis * ox + y_axis * oy;
    let radius = ox.hypot(oy);
    if !centre.is_finite() || !(radius > POINT_TOLERANCE) {
        return None;
    }
    // A free-form curve only passes near a circle; a drawn one lies exactly on it.
    let on_circle = points.iter().all(|point| {
        let distance = point.dist(centre);
        distance.is_finite() && (distance - radius).abs() <= CENTRE_TOLERANCE * radius
    });
    on_circle.then_some(centre)
}

/// Whether the tool is waiting for its first point at all.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LineState {
    /// The first endpoint, once the user has clicked it; the second click completes the line.
    pub p1: Option<Vec3>,
    /// Where the pointer currently projects onto the construction plane.
    pub current_pos: Vec3,
}

/// An arc is being placed through two endpoints and one point between them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ArcState {
    /// The start of the arc, once clicked.
    pub p1: Option<Vec3>,
    /// The end of the arc, once clicked; the third click supplies a point on the curve.
    pub p2: Option<Vec3>,
    /// Where the pointer currently projects onto the construction plane.
    pub current_pos: Vec3,
}

/// A point is being placed at the pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointState {
    /// Where the pointer currently projects onto the construction plane.
    pub current_pos: Vec3,
}

/// The clicking wireframe tool that is live, if any.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum WireframeTool {
    /// No clicking tool is live: the viewport is only a camera.
    #[default]
    None,
    /// Placing a line, or a chain of connected lines.
    Line(LineState),
    /// Placing an arc through three points.
    Arc(ArcState),
    /// Placing points.
    Point(PointState),
}

impl WireframeTool {
    /// How many of the tool's points the user has already clicked.
    pub const fn placed_points(self) -> usize {
        match self {
            Self::None => 0,
            Self::Line(state) => state.p1.is_some() as usize,
            Self::Arc(state) => state.p1.is_some() as usize + state.p2.is_some() as usize,
            Self::Point(_) => 0,
        }
    }

    /// The point the pointer is at, as the tool last saw it.
    pub const fn current_pos(self) -> Option<Vec3> {
        match self {
            Self::None => None,
            Self::Line(state) => Some(state.current_pos),
            Self::Arc(state) => Some(state.current_pos),
            Self::Point(state) => Some(state.current_pos),
        }
    }

    /// Moves the pointer-following position of a live tool; a tool that is not live ignores it.
    fn track(&mut self, at: Vec3) {
        match self {
            Self::None => {}
            Self::Line(state) => state.current_pos = at,
            Self::Arc(state) => state.current_pos = at,
            Self::Point(state) => state.current_pos = at,
        }
    }

    /// Drops the anchors already clicked, keeping the tool live.
    fn rewind(&mut self) {
        match self {
            Self::None | Self::Point(_) => {}
            Self::Line(state) => state.p1 = None,
            Self::Arc(state) => {
                state.p1 = None;
                state.p2 = None;
            }
        }
    }
}

/// A wireframe entity the user has finished placing, ready to be created in the document.
///
/// The coordinates are world points on the construction plane the tool started on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PendingWire {
    /// A single point.
    Point { at: Vec3 },
    /// A straight line between two endpoints.
    Line { start: Vec3, end: Vec3 },
    /// An arc from `start` to `end` through `through`.
    Arc { start: Vec3, end: Vec3, through: Vec3 },
}

/// What one click did to the live tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClickOutcome {
    /// No tool is live, or the click landed on a position the tool cannot use.
    Ignored,
    /// The click anchored a point, and the tool is waiting for another one.
    Progress,
    /// The click completed the entity; the caller creates it in the document.
    Complete(PendingWire),
}

/// The interactive wireframe state: the live tool, the plane it places on, and where the pointer
/// currently is on that plane.
///
/// The viewport owns one of these. Each frame it hands the pointer's picking ray to
/// [`track_ray`](Self::track_ray), and each click that the workflow accepts goes to
/// [`click`](Self::click); the completed entity it returns is what later phases create in the
/// document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractiveGeometry {
    /// The live tool.
    tool: WireframeTool,
    /// The construction plane clicked positions land on.
    plane: Plane,
    /// The last position the pointer projected to on `plane`, if it was over it.
    hover: Option<Vec3>,
    /// The document feature the pointer has snapped to, if any.
    snap: Option<SnapCandidate>,
}

impl Default for InteractiveGeometry {
    /// Nothing is live, and positions land on the TOP plane.
    fn default() -> Self {
        Self { tool: WireframeTool::None, plane: Plane::XY, hover: None, snap: None }
    }
}

impl InteractiveGeometry {
    /// A machine with no tool live whose positions land on `plane`.
    pub fn on_plane(plane: Plane) -> Self {
        Self { plane, ..Self::default() }
    }

    /// The live tool.
    pub const fn tool(&self) -> WireframeTool {
        self.tool
    }

    /// Whether a clicking tool is live.
    pub const fn is_active(&self) -> bool {
        !matches!(self.tool, WireframeTool::None)
    }

    /// The construction plane clicked positions land on.
    pub const fn plane(&self) -> Plane {
        self.plane
    }

    /// Where the pointer last projected onto the plane, if it was over it.
    pub const fn hover(&self) -> Option<Vec3> {
        self.hover
    }

    /// The document feature the pointer has snapped to this frame, if any.
    pub const fn snap(&self) -> Option<SnapCandidate> {
        self.snap
    }

    /// How many points the live tool has anchored so far.
    pub const fn placed_points(&self) -> usize {
        self.tool.placed_points()
    }

    /// The positions the live tool has anchored, in the order it took them.
    pub fn anchors(&self) -> Vec<Vec3> {
        match self.tool {
            WireframeTool::None | WireframeTool::Point(_) => Vec::new(),
            WireframeTool::Line(state) => state.p1.into_iter().collect(),
            WireframeTool::Arc(state) => state.p1.into_iter().chain(state.p2).collect(),
        }
    }

    /// The live tool's rubber band in world coordinates: one polyline per preview curve, running
    /// from the anchored point to the pointer, with `arc_segments` straight pieces in an arc.
    /// The point tool has no band — the viewport draws its cursor instead.
    pub fn preview(&self, arc_segments: usize) -> Vec<Vec<Vec3>> {
        match self.tool {
            WireframeTool::None | WireframeTool::Point(_) => Vec::new(),
            WireframeTool::Line(state) => state.p1.map(|start| vec![vec![start, state.current_pos]]).unwrap_or_default(),
            WireframeTool::Arc(state) => match (state.p1, state.p2) {
                (Some(start), Some(end)) => arc_preview_points(start, end, state.current_pos, self.plane, arc_segments)
                    .map_or_else(|| vec![vec![start, end]], |arc| vec![arc]),
                (Some(start), None) => vec![vec![start, state.current_pos]],
                (None, _) => Vec::new(),
            },
        }
    }

    /// Makes `tool` the live clicking tool on `plane`, dropping any points already anchored.
    ///
    /// Only the tools this module knows how to place by clicking start a workflow; the tools that
    /// draw at a fixed size (rectangle, circle, spline) answer `false` and leave the state as it
    /// was.
    pub fn begin(&mut self, tool: WireTool, plane: Plane) -> bool {
        let live = match tool {
            WireTool::Line => WireframeTool::Line(LineState { current_pos: self.current_pos(), ..LineState::default() }),
            WireTool::Arc => WireframeTool::Arc(ArcState { current_pos: self.current_pos(), ..ArcState::default() }),
            WireTool::Point => WireframeTool::Point(PointState { current_pos: self.current_pos() }),
            WireTool::Rectangle | WireTool::Circle | WireTool::Spline => return false,
        };
        self.plane = plane;
        self.tool = live;
        true
    }

    /// Ends the live tool. Anchored points are dropped; the pointer position is kept, because it
    /// is still where the cursor is.
    pub fn cancel(&mut self) {
        self.tool = WireframeTool::None;
    }

    /// Moves the placement plane. Anchored points are dropped, because they belong to the plane
    /// they were clicked on, and a line cannot span two.
    pub fn set_plane(&mut self, plane: Plane) {
        if self.plane == plane {
            return;
        }
        self.plane = plane;
        self.tool.rewind();
    }

    /// Projects the picking ray `origin + t·direction` onto the construction plane and remembers
    /// the point the pointer is at. `None` when the ray runs parallel to the plane or leaves it
    /// at a point that is not finite.
    pub fn track_ray(&mut self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        let point = self.plane.intersect_ray(origin, direction)?;
        if !point.is_finite() {
            return None;
        }
        self.track_point(point);
        Some(point)
    }

    /// Remembers `at` as the pointer's position, for a caller that already has the plane point.
    pub fn track_point(&mut self, at: Vec3) {
        if !at.is_finite() {
            return;
        }
        self.hover = Some(at);
        self.snap = None;
        self.tool.track(at);
    }

    /// Snaps the pointer to `snap`, or leaves it at the projected point when there is none: the
    /// rubber band ends on the candidate, so the anchor a click would take is the one drawn.
    pub fn set_snap(&mut self, snap: Option<SnapCandidate>) {
        if let Some(snap) = snap {
            self.track_point(snap.at);
        }
        self.snap = snap;
    }

    /// Forgets the pointer position, once it leaves the viewport.
    pub fn clear_hover(&mut self) {
        self.hover = None;
        self.snap = None;
    }

    /// Places one point of the live tool at `at`, and reports what that did.
    ///
    /// A click on a position that is not finite, or that would make a zero-length line or a
    /// degenerate arc, is ignored: the tool keeps the points it already has and waits.
    pub fn click(&mut self, at: Vec3) -> ClickOutcome {
        if !at.is_finite() {
            return ClickOutcome::Ignored;
        }
        self.track_point(at);
        match self.tool {
            WireframeTool::None => ClickOutcome::Ignored,
            WireframeTool::Point(_) => ClickOutcome::Complete(PendingWire::Point { at }),
            WireframeTool::Line(state) => match state.p1 {
                None => {
                    if let WireframeTool::Line(state) = &mut self.tool {
                        state.p1 = Some(at);
                    }
                    ClickOutcome::Progress
                }
                Some(start) => {
                    if start.dist(at) <= POINT_TOLERANCE {
                        return ClickOutcome::Ignored;
                    }
                    // The function stays live and the next line starts here, so a run of clicks
                    // draws connected lines.
                    if let WireframeTool::Line(state) = &mut self.tool {
                        state.p1 = Some(at);
                    }
                    ClickOutcome::Complete(PendingWire::Line { start, end: at })
                }
            },
            WireframeTool::Arc(state) => match (state.p1, state.p2) {
                (None, _) => {
                    if let WireframeTool::Arc(state) = &mut self.tool {
                        state.p1 = Some(at);
                    }
                    ClickOutcome::Progress
                }
                (Some(start), None) => {
                    if start.dist(at) <= POINT_TOLERANCE {
                        return ClickOutcome::Ignored;
                    }
                    if let WireframeTool::Arc(state) = &mut self.tool {
                        state.p2 = Some(at);
                    }
                    ClickOutcome::Progress
                }
                (Some(start), Some(end)) => {
                    if !arc_is_defined(start, end, at) {
                        return ClickOutcome::Ignored;
                    }
                    // The function stays live for the next arc.
                    if let WireframeTool::Arc(state) = &mut self.tool {
                        state.p1 = None;
                        state.p2 = None;
                    }
                    ClickOutcome::Complete(PendingWire::Arc { start, end, through: at })
                }
            },
        }
    }

    /// The last known pointer position, or the plane's origin before the pointer has been over
    /// the viewport: the seed for a tool whose state wants a position.
    const fn current_pos(&self) -> Vec3 {
        match self.hover {
            Some(point) => point,
            None => Vec3::ZERO,
        }
    }
}

/// Whether three points define a real arc: two distinct ends and a third point that is not on
/// their chord (an arc through a collinear point is a straight line, which the line tool draws).
pub fn arc_is_defined(start: Vec3, end: Vec3, through: Vec3) -> bool {
    let chord = end - start;
    let length = chord.len();
    if !(length > POINT_TOLERANCE) || !chord.is_finite() || !through.is_finite() {
        return false;
    }
    // The distance from the third point to the chord's line: |chord × leg| / |chord|.
    chord.cross(through - start).len() / length > POINT_TOLERANCE
}

/// The points of the arc that runs from `start` to `end` through `through`, on `plane`, sampled
/// as `segments` straight pieces. `None` when the three points do not define an arc.
pub fn arc_preview_points(start: Vec3, end: Vec3, through: Vec3, plane: Plane, segments: usize) -> Option<Vec<Vec3>> {
    if !arc_is_defined(start, end, through) {
        return None;
    }
    let (a, b, t) = (plane.to_local(start), plane.to_local(end), plane.to_local(through));
    // The circumcentre of the triangle in the plane: the centre of the arc's circle.
    let d = 2.0 * (a.x * (b.y - t.y) + b.x * (t.y - a.y) + t.x * (a.y - b.y));
    if !d.is_finite() || d.abs() <= POINT_TOLERANCE {
        return None;
    }
    let (aa, bb, tt) = (a.len2(), b.len2(), t.len2());
    let centre = Vec2::new(
        (aa * (b.y - t.y) + bb * (t.y - a.y) + tt * (a.y - b.y)) / d,
        (aa * (t.x - b.x) + bb * (a.x - t.x) + tt * (b.x - a.x)) / d,
    );
    let radius = centre.dist(a);
    if !centre.x.is_finite() || !centre.y.is_finite() || !(radius > POINT_TOLERANCE) {
        return None;
    }
    // Sweep the short way or the long way round, whichever passes through the third point.
    let turn = std::f64::consts::TAU;
    let angle = |point: Vec2| (point - centre).angle().rem_euclid(turn);
    let (from, to, via) = (angle(a), angle(b), angle(t));
    let forward = (to - from).rem_euclid(turn);
    let through_forward = (via - from).rem_euclid(turn);
    let sweep = if through_forward <= forward { forward } else { forward - turn };
    if !sweep.is_finite() || sweep.abs() <= 1e-9 {
        return None;
    }
    let steps = segments.max(2);
    Some(
        (0..=steps)
            .map(|step| {
                let angle = from + sweep * (step as f64 / steps as f64);
                plane.to_world(centre + Vec2::new(radius * angle.cos(), radius * angle.sin()))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live line tool on the TOP plane.
    fn line_tool() -> InteractiveGeometry {
        let mut geometry = InteractiveGeometry::default();
        assert!(geometry.begin(WireTool::Line, Plane::XY));
        geometry
    }

    /// A live arc tool on the TOP plane.
    fn arc_tool() -> InteractiveGeometry {
        let mut geometry = InteractiveGeometry::default();
        assert!(geometry.begin(WireTool::Arc, Plane::XY));
        geometry
    }

    #[test]
    fn a_line_is_complete_after_two_clicks() {
        let mut geometry = line_tool();
        let (start, end) = (Vec3::new(1.0, 2.0, 0.0), Vec3::new(41.0, 12.0, 0.0));
        assert_eq!(geometry.click(start), ClickOutcome::Progress);
        assert_eq!(geometry.placed_points(), 1);
        assert_eq!(geometry.click(end), ClickOutcome::Complete(PendingWire::Line { start, end }));
    }

    #[test]
    fn the_next_line_starts_where_the_last_one_ended() {
        let mut geometry = line_tool();
        let (a, b, c) = (Vec3::ZERO, Vec3::new(20.0, 0.0, 0.0), Vec3::new(20.0, 15.0, 0.0));
        assert!(matches!(geometry.click(a), ClickOutcome::Progress));
        assert!(matches!(geometry.click(b), ClickOutcome::Complete(_)));
        assert_eq!(geometry.placed_points(), 1, "the tool keeps the last endpoint as the next start");
        assert_eq!(geometry.click(c), ClickOutcome::Complete(PendingWire::Line { start: b, end: c }));
    }

    #[test]
    fn a_zero_length_line_is_refused() {
        let mut geometry = line_tool();
        let at = Vec3::new(5.0, 5.0, 0.0);
        assert_eq!(geometry.click(at), ClickOutcome::Progress);
        assert_eq!(geometry.click(at), ClickOutcome::Ignored);
        assert_eq!(geometry.placed_points(), 1, "the first endpoint is still there");
    }

    #[test]
    fn an_arc_takes_two_endpoints_and_a_third_point() {
        let mut geometry = arc_tool();
        let (start, end, through) = (Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), Vec3::new(5.0, 5.0, 0.0));
        assert_eq!(geometry.click(start), ClickOutcome::Progress);
        assert_eq!(geometry.click(end), ClickOutcome::Progress);
        assert_eq!(geometry.placed_points(), 2);
        assert_eq!(geometry.click(through), ClickOutcome::Complete(PendingWire::Arc { start, end, through }));
        assert_eq!(geometry.placed_points(), 0, "the tool is live for the next arc");
    }

    #[test]
    fn a_collinear_third_point_does_not_complete_an_arc() {
        let mut geometry = arc_tool();
        assert!(matches!(geometry.click(Vec3::ZERO), ClickOutcome::Progress));
        assert!(matches!(geometry.click(Vec3::new(10.0, 0.0, 0.0)), ClickOutcome::Progress));
        assert_eq!(geometry.click(Vec3::new(4.0, 0.0, 0.0)), ClickOutcome::Ignored);
        assert_eq!(geometry.placed_points(), 2, "the arc waits for a point that bends it");
        assert!(!arc_is_defined(Vec3::ZERO, Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0)), "no chord, no arc");
    }

    #[test]
    fn points_are_placed_one_per_click() {
        let mut geometry = InteractiveGeometry::default();
        assert!(geometry.begin(WireTool::Point, Plane::XY));
        let at = Vec3::new(-3.0, 7.5, 0.0);
        assert_eq!(geometry.click(at), ClickOutcome::Complete(PendingWire::Point { at }));
        assert_eq!(geometry.click(Vec3::new(1.0, 1.0, 0.0)), ClickOutcome::Complete(PendingWire::Point { at: Vec3::new(1.0, 1.0, 0.0) }));
        assert!(geometry.is_active(), "the point function stays live");
    }

    #[test]
    fn no_tool_ignores_clicks() {
        let mut geometry = InteractiveGeometry::default();
        assert!(!geometry.is_active());
        assert_eq!(geometry.click(Vec3::new(1.0, 1.0, 0.0)), ClickOutcome::Ignored);
        assert_eq!(geometry.tool().current_pos(), None);
    }

    #[test]
    fn fixed_size_tools_do_not_start_a_clicking_workflow() {
        for tool in [WireTool::Rectangle, WireTool::Circle, WireTool::Spline] {
            let mut geometry = line_tool();
            assert!(!geometry.begin(tool, Plane::XY), "{} is not placed by clicking", tool.label());
            assert!(geometry.is_active(), "the line tool it did not replace is still live");
        }
    }

    #[test]
    fn cancelling_ends_the_tool_and_its_anchors() {
        let mut geometry = line_tool();
        assert!(matches!(geometry.click(Vec3::new(1.0, 1.0, 0.0)), ClickOutcome::Progress));
        geometry.cancel();
        assert!(!geometry.is_active());
        assert_eq!(geometry.placed_points(), 0);
        assert!(geometry.hover().is_some(), "the cursor is still where it was");
    }

    #[test]
    fn changing_the_plane_drops_the_anchors() {
        let mut geometry = line_tool();
        assert!(matches!(geometry.click(Vec3::new(1.0, 1.0, 0.0)), ClickOutcome::Progress));
        geometry.set_plane(Plane::XZ);
        assert_eq!(geometry.placed_points(), 0, "a line cannot span two planes");
        assert_eq!(geometry.plane(), Plane::XZ);
        geometry.set_plane(Plane::XZ);
        assert!(geometry.is_active());
    }

    #[test]
    fn the_pointer_position_follows_the_tool() {
        let mut geometry = line_tool();
        geometry.track_point(Vec3::new(4.0, 5.0, 0.0));
        assert_eq!(geometry.tool().current_pos(), Some(Vec3::new(4.0, 5.0, 0.0)));
        geometry.clear_hover();
        assert_eq!(geometry.hover(), None);
    }

    #[test]
    fn a_ray_meets_the_construction_plane() {
        let mut geometry = InteractiveGeometry::default();
        let point = geometry.track_ray(Vec3::new(0.0, 0.0, 100.0), Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(point, Some(Vec3::ZERO));
        assert_eq!(geometry.hover(), Some(Vec3::ZERO));
        // A ray along the plane never meets it.
        assert_eq!(geometry.track_ray(Vec3::new(0.0, 0.0, 100.0), Vec3::X), None);
    }

    #[test]
    fn non_finite_clicks_are_ignored() {
        let mut geometry = line_tool();
        assert_eq!(geometry.click(Vec3::new(f64::NAN, 0.0, 0.0)), ClickOutcome::Ignored);
        geometry.track_point(Vec3::new(f64::INFINITY, 0.0, 0.0));
        assert_eq!(geometry.hover(), None);
        assert_eq!(geometry.placed_points(), 0);
    }

    #[test]
    fn the_tool_keeps_the_pointer_position_when_it_starts() {
        let mut geometry = InteractiveGeometry::default();
        geometry.track_point(Vec3::new(3.0, 4.0, 0.0));
        assert!(geometry.begin(WireTool::Line, Plane::XY));
        assert_eq!(geometry.tool().current_pos(), Some(Vec3::new(3.0, 4.0, 0.0)));
    }

    #[test]
    fn a_snap_takes_the_pointer_and_the_band_to_the_candidate() {
        let mut geometry = line_tool();
        assert!(matches!(geometry.click(Vec3::ZERO), ClickOutcome::Progress));
        let snap = SnapCandidate { kind: SnapKind::Endpoint, at: Vec3::new(25.0, 5.0, 0.0) };
        geometry.track_point(Vec3::new(24.0, 6.0, 0.0));
        geometry.set_snap(Some(snap));
        assert_eq!(geometry.snap(), Some(snap));
        assert_eq!(geometry.hover(), Some(snap.at), "the pointer sits on the snap, not beside it");
        assert_eq!(geometry.preview(ARC_PREVIEW_SEGMENTS), vec![vec![Vec3::ZERO, snap.at]], "the rubber band ends on the snap");
        geometry.track_point(Vec3::new(23.0, 7.0, 0.0));
        assert_eq!(geometry.snap(), None, "moving the pointer off the candidate drops the snap");
    }

    #[test]
    fn the_line_rubber_band_runs_from_its_anchor_to_the_pointer() {
        let mut geometry = line_tool();
        assert!(geometry.preview(ARC_PREVIEW_SEGMENTS).is_empty(), "a line with no anchor has no band");
        let start = Vec3::new(5.0, 5.0, 0.0);
        assert!(matches!(geometry.click(start), ClickOutcome::Progress));
        let pointer = Vec3::new(35.0, 12.0, 0.0);
        geometry.track_point(pointer);
        assert_eq!(geometry.preview(ARC_PREVIEW_SEGMENTS), vec![vec![start, pointer]]);
        assert_eq!(geometry.anchors(), vec![start]);
    }

    #[test]
    fn the_arc_rubber_band_bends_through_the_pointer() {
        let mut geometry = arc_tool();
        let (start, end) = (Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0));
        assert!(matches!(geometry.click(start), ClickOutcome::Progress));
        geometry.track_point(end);
        assert_eq!(geometry.preview(ARC_PREVIEW_SEGMENTS), vec![vec![start, end]], "two anchors preview the chord");
        assert!(matches!(geometry.click(end), ClickOutcome::Progress));
        let through = Vec3::new(5.0, 5.0, 0.0);
        geometry.track_point(through);
        let preview = geometry.preview(ARC_PREVIEW_SEGMENTS);
        assert_eq!(preview.len(), 1);
        let points = &preview[0];
        assert_eq!(points.len(), ARC_PREVIEW_SEGMENTS + 1);
        assert!(points[0].dist(start) < 1e-9 && points[points.len() - 1].dist(end) < 1e-9, "the arc starts and ends on its anchors");
        assert!(points.iter().any(|point| point.dist(through) < 1e-9), "the arc passes through the pointer");
        assert!(points.iter().all(|point| point.z.abs() < 1e-9), "the arc stays on the TOP plane");
    }

    #[test]
    fn an_arc_through_its_own_endpoints_has_no_preview() {
        assert!(arc_preview_points(Vec3::ZERO, Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0), Plane::XY, 16).is_none());
        assert!(arc_preview_points(Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0), Plane::XY, 16).is_none());
    }

    #[test]
    fn snap_candidates_are_the_curves_ends_midpoints_and_centres() {
        let mut document = CamDocument::default();
        document.draw(WireTool::Rectangle).expect("the rectangle draws on a new document");
        let candidates = snap_candidates(&document);
        let corners = [Vec3::new(10.0, 10.0, 0.0), Vec3::new(50.0, 10.0, 0.0), Vec3::new(50.0, 40.0, 0.0), Vec3::new(10.0, 40.0, 0.0)];
        for corner in corners {
            assert!(
                candidates.iter().any(|candidate| candidate.kind == SnapKind::Endpoint && candidate.at.dist(corner) < 1e-9),
                "the rectangle's corner {corner:?} is a snap"
            );
        }
        assert!(
            candidates.iter().any(|candidate| candidate.kind == SnapKind::Midpoint && candidate.at.dist(Vec3::new(30.0, 10.0, 0.0)) < 1e-9),
            "the bottom edge's midpoint is a snap"
        );
        assert!(!candidates.iter().any(|candidate| candidate.kind == SnapKind::Centre), "a rectangle's four separate edges have no centre");

        let mut document = CamDocument::default();
        document.draw(WireTool::Circle).expect("the circle draws on a new document");
        let candidates = snap_candidates(&document);
        let centre = Vec3::new(30.0, 25.0, 0.0);
        assert!(
            candidates.iter().any(|candidate| candidate.kind == SnapKind::Centre && candidate.at.dist(centre) < 1e-6),
            "the circle's centre is a snap"
        );
    }
}

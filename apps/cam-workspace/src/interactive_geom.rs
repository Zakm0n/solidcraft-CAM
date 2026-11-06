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
//! typed fields feed this machine from outside; this module owns only the state — when a click
//! anchors a point, when the entity is complete, and what the completed entity is. Drawing the
//! rubber-band preview and creating the geometry in the document are separate jobs.

use solidcore_record::{Plane, Vec3};
use solidcraft_geom::{PlaneGeom, Vec3Geom};

use crate::document::WireTool;

/// Distance below which two positions are the same point, in document units. A line or an arc
/// needs two distinct ends, and an arc's third point must leave the chord by more than this.
pub const POINT_TOLERANCE: f64 = 0.001;

/// The screen-space reach of a position pick, in logical pixels: a click within this distance of
/// a snap candidate takes the candidate instead of the point under the cursor.
pub const PICK_RADIUS_PIXELS: f32 = 8.0;

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
}

impl Default for InteractiveGeometry {
    /// Nothing is live, and positions land on the TOP plane.
    fn default() -> Self {
        Self { tool: WireframeTool::None, plane: Plane::XY, hover: None }
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

    /// How many points the live tool has anchored so far.
    pub const fn placed_points(&self) -> usize {
        self.tool.placed_points()
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
        self.tool.track(at);
    }

    /// Forgets the pointer position, once it leaves the viewport.
    pub fn clear_hover(&mut self) {
        self.hover = None;
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
}

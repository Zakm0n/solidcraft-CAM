//! The dynamic gnomon: a local coordinate frame the user places in the 3D view, with an origin
//! ball, one translation arm per axis (a shaft and a cone at its tip) and one rotation wheel per
//! axis. The arms keep a constant size on screen, as the Move triad's do, so the frame stays
//! usable at every zoom; the frame itself is kept square, so the three arms always name an
//! orthogonal machining frame, and the wheels lie in that frame's own planes.
//!
//! The widget owns only its own state: it draws over the viewport, hit-tests its handles against
//! the projected geometry, reports a drag's [`Change`] and remembers which of its click modes a
//! press opened ([`GnomonActiveState`]). The caller decides what the change means (the CAM
//! coordinate system dialog writes it into the frame's values), hands the frame the geometry a
//! live snap may take ([`SnapCandidate`]: the B-rep's vertices, edge midpoints, circle centres and
//! face centroids) and shows the floating value entry a click on a shaft or a wheel opens.
//!
//! While Origin Snap or Axis Orbit is on, the frame follows the pointer without a drag: the
//! nearest candidate within [`SNAP_RADIUS`] screen points is taken, and a pointer away from every
//! candidate is unprojected onto the view plane through where the mode began. The frame keeps its
//! axes square throughout (Gram-Schmidt), so a snap never tilts it.

use egui::{Align2, Color32, FontId, Painter, Pos2, Shape, Stroke};
use solidcore_record::Vec3;
use solidcraft_engine::geom::UnitVec3;
use solidcraft_engine::geom::prelude::*;

use crate::theme::Tokens;
use crate::viewport::Proj;

/// The arm length on screen, in points.
pub const ARM_PX: f32 = 96.0;
/// The rotation rings' radius as a share of the arm.
const RING_SHARE: f64 = 0.62;
/// The origin ball's radius, in points.
const ORIGIN_RADIUS: f32 = 6.0;
/// How near the origin ball a press still takes it, in points.
const ORIGIN_HIT: f32 = 10.0;
/// How near an arm a press still takes it, in points.
const AXIS_HIT: f32 = 6.0;
/// How near an arm's cone a press still takes the cone, in points.
const CONE_HIT: f32 = 9.0;
/// How near a rotation ring a press still takes it, in points.
const RING_HIT: f32 = 5.0;
/// How far the pointer may travel from a press and still count as a click, in points.
const CLICK_SLOP: f32 = 3.0;
/// The gap between a cone's tip and its axis letter, in points.
const LABEL_GAP: f32 = 10.0;
/// Points one rotation ring is drawn (and hit-tested) with.
const RING_POINTS: usize = 64;
/// Where an arm starts, as a share of its length, so it leaves the origin ball clear.
const ARM_START: f64 = 0.18;
/// How near (screen points) a live snap's cursor must come to a candidate to take it.
pub const SNAP_RADIUS: f32 = 20.0;
/// The radius of the mark a live snap draws on the candidate it takes, in points.
const SNAP_MARK: f32 = 5.0;
/// The most candidates a live snap looks through: a hostile design must not make a frame cost
/// grow without a bound.
const MAX_CANDIDATES: usize = 4096;

/// One of the frame's three axes, in the order the colours name them: X red, Y green, Z blue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    /// The three axes, in X, Y, Z order.
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    /// Its place in [`Axis::ALL`], and in [`Tokens::axes`].
    pub const fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }
}

/// The part of the gnomon a press can take.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GnomonHandle {
    /// No handle.
    #[default]
    None,
    /// The origin ball: a free drag in the view plane, or a click for Origin Snap Mode.
    Origin,
    /// An arm's shaft: a drag along that axis, or a click for a relative distance.
    Translate(Axis),
    /// An arm's cone: a click for Axis Orbit Mode, which points the axis at what is picked.
    Cone(Axis),
    /// A rotation wheel: a drag turning the frame about that axis, or a click for a relative
    /// angle.
    Rotate(Axis),
}

/// What a click on the gnomon opened, if anything: the mode its next geometry pick acts in, or
/// the value entry a click on a shaft or a wheel asked for. A drag leaves it [`Idle`]
/// ([`GnomonActiveState::Idle`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum GnomonActiveState {
    /// No click mode: a pick aligns the frame as the alignment mode says.
    #[default]
    Idle,
    /// The origin ball was clicked: the next pick puts the origin on the point it names
    /// (a vertex, a midpoint, a circle centre or a face's centroid).
    OriginSnap,
    /// A cone was clicked: the next pick turns that axis to point at it.
    AxisOrbit(Axis),
    /// A shaft or a wheel was clicked: a floating value entry sits at `pos`, for a distance along
    /// the shaft or an angle about the wheel.
    RelativeInput { handle: GnomonHandle, pos: Pos2 },
}

/// What a geometry pick aligns the frame to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GnomonAlignMode {
    /// The frame is moved by its own handles only.
    #[default]
    Free,
    /// A pick puts the origin on the picked point.
    SnapOrigin,
    /// A picked face (or plane) gives the frame its Z: the face's normal.
    AlignZFace,
    /// A picked edge gives the frame its X: the edge's direction.
    AlignXEdge,
}

impl GnomonAlignMode {
    /// The modes, in the order a mode row offers them.
    pub const ALL: [GnomonAlignMode; 4] =
        [GnomonAlignMode::Free, GnomonAlignMode::SnapOrigin, GnomonAlignMode::AlignZFace, GnomonAlignMode::AlignXEdge];
}

/// The frame change one frame of a drag made, or a live snap took.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    /// The origin moved by this offset, in mm.
    Translate(Vec3),
    /// The frame turned about one of its axes by this angle, in radians.
    Rotate { axis: Axis, angle: f64 },
    /// The frame was placed on the geometry under the pointer (Origin Snap or Axis Orbit): its
    /// own origin and axes are the change.
    Frame,
}

/// A point a live snap may take while Origin Snap or Axis Orbit follows the pointer: a B-rep
/// vertex, an edge's midpoint, a circle's centre or a face's centroid, and a sketch's point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapCandidate {
    /// The exact point, in model space (mm).
    pub point: Vec3,
    /// A planar face's outward normal, when the point is that face's centroid. Origin Snap turns
    /// the frame's Z to it.
    pub normal: Option<Vec3>,
}

/// Where a click mode began: the frame the fallback ray and the axes are measured from, so a
/// free-space move never drifts as the frame follows the pointer.
#[derive(Clone, Copy, Debug)]
struct SnapBase {
    origin: Vec3,
    /// +X, +Y and +Z as the mode opened.
    axes: [Vec3; 3],
}

/// The primary pointer's state, so [`DynamicGnomon::interact`] can be driven without an
/// [`egui::Ui`] (and checked without one): the caller reads it once per frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputState {
    /// Where the pointer is, in screen points.
    pub pointer: Option<Pos2>,
    /// The primary button went down this frame.
    pub primary_pressed: bool,
    /// The primary button is down.
    pub primary_down: bool,
    /// The primary button came up this frame.
    pub primary_released: bool,
    /// Escape was pressed with no field taking the keyboard: the click mode ends.
    pub escape: bool,
}

/// The keyboard the floating incremental value entry took for itself this frame: Enter applies
/// its value and Escape drops it, so the dialog that opened the gnomon never reads them as its
/// own OK or Cancel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IncrementalKeys {
    /// The entry's field held the keyboard, so Enter and Escape were the entry's own.
    pub active: bool,
    /// Enter was pressed: apply the entry's value.
    pub apply: bool,
    /// Escape was pressed: drop the entry without changing the frame.
    pub dismiss: bool,
}

/// Take this frame's Enter and Escape for the caller's floating incremental value entry while
/// its field holds the keyboard, so the dialog that opened the gnomon never reads them as its
/// own OK or Cancel. `open` says whether the entry is showing: a closed entry takes nothing.
/// Escape drops the keyboard at the start of a frame and Enter on a single-line field surrenders
/// it, so the field's focus now or last frame names the entry that owned the keys.
pub fn take_incremental_keys(ui: &egui::Ui, field: egui::Id, open: bool) -> IncrementalKeys {
    if !open || !ui.memory(|memory| memory.has_focus(field) || memory.had_focus_last_frame(field)) {
        return IncrementalKeys::default();
    }
    let (apply, dismiss) = ui
        .input_mut(|input| (input.consume_key(egui::Modifiers::NONE, egui::Key::Enter), input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)));
    IncrementalKeys { active: true, apply, dismiss }
}

impl InputState {
    /// This frame's pointer state from `ui`.
    pub fn read(ui: &egui::Ui) -> InputState {
        let (pointer, primary_pressed, primary_down, primary_released) =
            ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released()));
        // A value entry's field takes Escape for itself: the click mode stays until the field
        // lets the key go.
        let escape = ui.ctx().memory(|m| m.focused().is_none()) && ui.input(|i| i.key_pressed(egui::Key::Escape));
        InputState { pointer, primary_pressed, primary_down, primary_released, escape }
    }
}

/// What a drag began on, in the frame the drag started from.
#[derive(Clone, Copy, Debug)]
enum Grab {
    /// The view-plane point under the press (the origin ball).
    Point(Vec3),
    /// The parameter along the arm under the press.
    Along(f64),
    /// The angle on the rotation wheel under the press.
    Angle(f64),
}

/// A drag in progress. The frame it started from is kept, so every frame's change is measured
/// from the same place and rounding cannot accumulate over a long drag.
#[derive(Clone, Copy, Debug)]
struct DragState {
    handle: GnomonHandle,
    origin: Vec3,
    x_axis: UnitVec3,
    z_axis: UnitVec3,
    grab: Grab,
    /// Where the press was, so a press that never moved is a click.
    press: Pos2,
    /// Whether the pointer left the press by more than [`CLICK_SLOP`]: a click it is not.
    moved: bool,
    /// The view direction through the origin when the drag began: the plane the origin ball is
    /// dragged on, so a perspective camera's per-pixel rays cannot make the ball drift.
    view: Vec3,
}

impl DragState {
    /// The direction of `axis` in the frame the drag started from.
    fn axis(&self, axis: Axis) -> Vec3 {
        match axis {
            Axis::X => self.x_axis.vec(),
            Axis::Y => self.z_axis.vec().cross(self.x_axis.vec()).normalized().unwrap_or(Vec3::Y),
            Axis::Z => self.z_axis.vec(),
        }
    }

    /// The frame the drag started from: +X, +Y and +Z.
    fn frame(&self) -> [Vec3; 3] {
        [self.axis(Axis::X), self.axis(Axis::Y), self.axis(Axis::Z)]
    }

    /// What the press took, in this frame.
    fn grab(&self, proj: &Proj, pos: Pos2) -> Option<Grab> {
        match self.handle {
            GnomonHandle::Origin => Some(Grab::Point(crate::gizmo::on_plane(proj, pos, self.origin, self.view)?)),
            GnomonHandle::Translate(axis) => Some(Grab::Along(crate::canvas::along(proj, pos, self.origin, self.axis(axis))?)),
            GnomonHandle::Rotate(axis) => {
                let direction = self.axis(axis);
                let at = crate::gizmo::on_plane(proj, pos, self.origin, direction)?;
                let (e1, e2) = ring_basis(self.frame(), axis);
                let dart = at - self.origin;
                Some(Grab::Angle(dart.dot(e2).atan2(dart.dot(e1))))
            }
            // A cone is a click, not a drag.
            GnomonHandle::Cone(_) | GnomonHandle::None => None,
        }
    }
}

/// The gnomon's screen geometry: where the origin ball sits and where each handle runs.
struct Shapes {
    origin: Pos2,
    arrows: Vec<(Axis, Pos2, Pos2)>,
    rings: Vec<(Axis, Vec<Pos2>)>,
}

/// A frame the user places: where its origin is, the directions of its X and Z, and which handle
/// the pointer is over or dragging. Y is always Z × X, so the three are square by construction.
#[derive(Clone, Debug)]
pub struct DynamicGnomon {
    origin: Vec3,
    x_axis: UnitVec3,
    z_axis: UnitVec3,
    active_handle: GnomonHandle,
    hovered_handle: GnomonHandle,
    align_mode: GnomonAlignMode,
    drag: Option<DragState>,
    state: GnomonActiveState,
    /// The screen rectangle of the caller's floating value entry, while one is open. A press
    /// inside it belongs to the entry's own widgets, so it neither starts a drag nor ends the
    /// click mode the entry was opened by.
    entry_rect: Option<egui::Rect>,
    /// Whether the caller's floating incremental value entry took the keyboard this frame, so
    /// Enter and Escape were the entry's own and not the dialog's.
    incremental_input_active: bool,
    /// Where the click mode began, while Origin Snap or Axis Orbit follows the pointer.
    snap_base: Option<SnapBase>,
    /// The points a live snap may take.
    candidates: Vec<SnapCandidate>,
    /// Where the candidate a live snap took shows on screen, while there is one.
    snap_at: Option<Pos2>,
}

impl Default for DynamicGnomon {
    fn default() -> Self {
        DynamicGnomon {
            origin: Vec3::ZERO,
            x_axis: UnitVec3::X,
            z_axis: UnitVec3::Z,
            active_handle: GnomonHandle::None,
            hovered_handle: GnomonHandle::None,
            align_mode: GnomonAlignMode::Free,
            drag: None,
            state: GnomonActiveState::Idle,
            entry_rect: None,
            incremental_input_active: false,
            snap_base: None,
            candidates: Vec::new(),
            snap_at: None,
        }
    }
}

impl DynamicGnomon {
    /// A frame at `origin` with `x_axis` and `z_axis`. X is made square to Z, and a vector with no
    /// usable direction (zero, or not a finite number) is refused with `None`.
    pub fn new(origin: Vec3, x_axis: Vec3, z_axis: Vec3) -> Option<DynamicGnomon> {
        if !finite(origin) || !finite(x_axis) || !finite(z_axis) {
            return None;
        }
        let z_axis = UnitVec3::new(z_axis)?;
        Some(DynamicGnomon { origin, x_axis: square_to(x_axis, z_axis), z_axis, ..DynamicGnomon::default() })
    }

    /// Where the frame's zero sits, in model space (mm).
    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    /// The frame's +X direction.
    pub fn x_axis(&self) -> UnitVec3 {
        self.x_axis
    }

    /// The frame's +Y direction: Z × X, always square to both.
    pub fn y_axis(&self) -> UnitVec3 {
        self.z_axis.vec().cross(self.x_axis.vec()).normalized().and_then(UnitVec3::new).unwrap_or(UnitVec3::Y)
    }

    /// The frame's +Z direction.
    pub fn z_axis(&self) -> UnitVec3 {
        self.z_axis
    }

    /// The frame's +X, +Y and +Z directions, unit vectors in that order.
    pub fn axes(&self) -> [Vec3; 3] {
        [self.x_axis.vec(), self.y_axis().vec(), self.z_axis.vec()]
    }

    /// What a geometry pick aligns the frame to.
    pub fn align_mode(&self) -> GnomonAlignMode {
        self.align_mode
    }

    /// Set what a geometry pick aligns the frame to.
    pub fn set_align_mode(&mut self, mode: GnomonAlignMode) {
        self.align_mode = mode;
    }

    /// The handle being dragged, if any.
    pub fn active_handle(&self) -> GnomonHandle {
        self.active_handle
    }

    /// The handle under the pointer, from the last [`DynamicGnomon::interact`].
    pub fn hovered_handle(&self) -> GnomonHandle {
        self.hovered_handle
    }

    /// What a click on the gnomon opened: the mode its next pick acts in, or the value entry it
    /// asked for. [`GnomonActiveState::Idle`] after a drag.
    pub fn active_state(&self) -> GnomonActiveState {
        self.state
    }

    /// Set the click mode; the caller clears it to [`GnomonActiveState::Idle`] once its pick has
    /// been taken, so each click mode acts once. Opening Origin Snap or Axis Orbit remembers the
    /// frame it began from, which the fallback ray and the orbit's square frame are measured from.
    pub fn set_active_state(&mut self, state: GnomonActiveState) {
        self.enter_state(state);
    }

    /// The points a live snap may take, as the caller found them in the design. Only those near
    /// the pointer's path are used, and no more than a bounded number of them.
    pub fn set_snap_candidates(&mut self, candidates: Vec<SnapCandidate>) {
        self.candidates = candidates.into_iter().filter(|c| finite(c.point)).take(MAX_CANDIDATES).collect();
        if self.snap_base.is_none() {
            self.snap_at = None;
        }
    }

    /// Where the candidate a live snap took shows on screen, while one is within reach.
    pub fn snap_mark(&self) -> Option<Pos2> {
        self.snap_at
    }

    /// Whether a click mode is following the pointer, so the view must keep repainting while it
    /// is on: Origin Snap and Axis Orbit place the frame on every pointer move.
    pub fn tracks_pointer(&self) -> bool {
        matches!(self.state, GnomonActiveState::OriginSnap | GnomonActiveState::AxisOrbit(_))
    }

    /// Keep the view repainting while a click mode is on: live snapping needs a frame for every
    /// pointer move, even when nothing else would ask for one.
    pub fn request_repaint(&self, ctx: &egui::Context) {
        if self.tracks_pointer() {
            ctx.request_repaint();
        }
    }

    /// Take `state` as the click mode, remembering the frame Origin Snap and Axis Orbit follow the
    /// pointer from. Any other mode forgets it.
    fn enter_state(&mut self, state: GnomonActiveState) {
        self.state = state;
        self.snap_base = matches!(state, GnomonActiveState::OriginSnap | GnomonActiveState::AxisOrbit(_))
            .then(|| SnapBase { origin: self.origin, axes: self.axes() });
        if self.snap_base.is_none() {
            self.snap_at = None;
        }
    }

    /// Say where the caller's floating value entry sits, while one is open, so a press inside it
    /// stays with the entry instead of ending the click mode ([`GnomonActiveState::RelativeInput`]).
    /// `None` once the entry closes.
    pub fn set_entry_rect(&mut self, rect: Option<egui::Rect>) {
        self.entry_rect = rect;
    }

    /// Whether `p` lies on the floating value entry the caller draws, which owns its own presses.
    fn on_entry(&self, p: Pos2) -> bool {
        self.entry_rect.is_some_and(|r| r.contains(p))
    }

    /// Whether the caller's floating incremental value entry took the keyboard this frame
    /// ([`take_incremental_keys`]), so Enter and Escape were the entry's own and not the
    /// dialog's OK or Cancel.
    pub fn is_incremental_input_active(&self) -> bool {
        self.incremental_input_active
    }

    /// Say whether the incremental value entry took the keyboard this frame. The dialog that
    /// opened the gnomon asks this before it reads Enter or Escape as its own.
    pub fn set_incremental_input_active(&mut self, active: bool) {
        self.incremental_input_active = active;
    }

    /// Whether a drag is in progress.
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Whether the frame owns the pointer this frame, so the viewport must not start its marquee:
    /// a handle is hovered, a drag is in progress, or a click mode waits for its pick.
    pub fn blocks_viewport_marquee(&self) -> bool {
        self.drag.is_some()
            || self.hovered_handle != GnomonHandle::None
            || matches!(self.state, GnomonActiveState::OriginSnap | GnomonActiveState::AxisOrbit(_) | GnomonActiveState::RelativeInput { .. })
    }

    /// Put the origin at `p`. A point that is not a finite number is refused.
    pub fn set_origin(&mut self, p: Vec3) -> bool {
        if !finite(p) {
            return false;
        }
        self.origin = p;
        true
    }

    /// Point the frame's Z along `z`, keeping X as nearly as it was while square to the new Z.
    pub fn set_z_axis(&mut self, z: Vec3) -> bool {
        let Some(z) = UnitVec3::new(z) else { return false };
        self.x_axis = square_to(self.x_axis.vec(), z);
        self.z_axis = z;
        true
    }

    /// Point the frame's X along `x`, keeping Z as nearly as it was while square to the new X.
    pub fn set_x_axis(&mut self, x: Vec3) -> bool {
        let Some(x) = UnitVec3::new(x) else { return false };
        self.z_axis = square_to(self.z_axis.vec(), x);
        self.x_axis = x;
        true
    }

    /// Move the origin by `delta`.
    pub fn translate(&mut self, delta: Vec3) -> bool {
        if !finite(delta) {
            return false;
        }
        self.set_origin(self.origin + delta)
    }

    /// Turn the frame about one of its own axes by `angle` radians.
    pub fn rotate(&mut self, axis: Axis, angle: f64) -> bool {
        if !angle.is_finite() {
            return false;
        }
        let direction = match axis {
            Axis::X => self.x_axis.vec(),
            Axis::Y => self.y_axis().vec(),
            Axis::Z => self.z_axis.vec(),
        };
        let x = rotate_about(self.x_axis.vec(), direction, angle);
        let z = rotate_about(self.z_axis.vec(), direction, angle);
        self.set_frame(x, z)
    }

    /// Turn the frame about the shortest arc so `axis` points at `target`. False for a target the
    /// frame cannot use: a point that is not a number, or the frame's own origin.
    pub fn aim_axis(&mut self, axis: Axis, target: Vec3) -> bool {
        if !finite(target) {
            return false;
        }
        let Some(direction) = (target - self.origin).normalized() else { return false };
        let from = self.axes()[axis.index()];
        if !finite(from) {
            return false;
        }
        // The turn's axis and angle: the short way round, and a half turn about any direction
        // square to `from` when the two point opposite ways.
        let Some((turn, angle)) = from
            .cross(direction)
            .normalized()
            .map(|t| (t, from.dot(direction).clamp(-1.0, 1.0).acos()))
            .or_else(|| from.any_perp().normalized().map(|t| (t, std::f64::consts::PI)))
        else {
            return false;
        };
        if !turn.is_finite() || !angle.is_finite() {
            return false;
        }
        self.set_frame(rotate_about(self.x_axis.vec(), turn, angle), rotate_about(self.z_axis.vec(), turn, angle))
    }

    /// Whether `p` is over one of the handles, so a press there belongs to the gnomon rather
    /// than to the camera (the Move triad's [`crate::gizmo::contains_pointer`]).
    pub fn contains_pointer(&self, proj: &Proj, p: Pos2) -> bool {
        self.shapes(proj).is_some_and(|shapes| hit(&shapes, p) != GnomonHandle::None)
    }

    /// The handle under `p`, as [`DynamicGnomon::interact`] would take a press there.
    pub fn handle_at(&self, proj: &Proj, p: Pos2) -> GnomonHandle {
        self.hit(proj, p)
    }

    /// Draw the frame over the view. The hovered or dragged handle is drawn in the accent colour
    /// and thicker, so it reads as the one a press would take; each arm's letter sits past its
    /// cone, and the mode a click opened rings the handle it opened on.
    pub fn draw(&self, painter: &Painter, proj: &Proj, theme: &Tokens) {
        let Some(shapes) = self.shapes(proj) else { return };
        let colour = |handle: GnomonHandle, axis: Axis| {
            if self.lit(handle) { theme.accent } else { theme.axes()[axis.index()] }
        };
        // Wheels behind the arms, the origin ball in front of both: the part a press takes first
        // is drawn last, as a press takes the ball before an arm and an arm before a wheel.
        for (axis, points) in &shapes.rings {
            let handle = GnomonHandle::Rotate(*axis);
            painter.add(Shape::line(points.clone(), Stroke::new(if self.lit(handle) { 3.0 } else { 1.4 }, colour(handle, *axis))));
        }
        let font = FontId::proportional(theme.font_triad);
        for (axis, from, to) in &shapes.arrows {
            let handle = GnomonHandle::Translate(*axis);
            let cone = GnomonHandle::Cone(*axis);
            let lit = self.lit(handle) || self.lit(cone);
            let c = if lit { theme.accent } else { theme.axes()[axis.index()] };
            arrow(painter, *from, *to, c, lit);
            // The axis's letter, past its cone: never on it, so a press still takes the cone.
            let v = *to - *from;
            let beyond = if v.length() > 1.0 { *to + v.normalized() * LABEL_GAP } else { *to };
            painter.text(beyond, Align2::CENTER_CENTER, axis_name(*axis).to_uppercase(), font.clone(), c);
        }
        let origin_handle = GnomonHandle::Origin;
        let origin_colour = if self.lit(origin_handle) { theme.accent } else { theme.text };
        painter.circle(shapes.origin, ORIGIN_RADIUS, theme.panel, Stroke::new(1.5, origin_colour));
        // The click mode's handle: Origin Snap rings the ball, Axis Orbit its cone.
        match self.state {
            GnomonActiveState::OriginSnap => {
                painter.circle_stroke(shapes.origin, ORIGIN_HIT + 3.0, Stroke::new(2.0, theme.accent));
            }
            GnomonActiveState::AxisOrbit(axis) => {
                if let Some((_, _, to)) = shapes.arrows.iter().find(|(a, _, _)| *a == axis) {
                    painter.circle_stroke(*to, CONE_HIT, Stroke::new(2.0, theme.accent));
                }
            }
            GnomonActiveState::Idle | GnomonActiveState::RelativeInput { .. } => {}
        }
        // The point a live snap holds the frame on: a filled dot in the accent colour, ringed in
        // the panel colour so it reads over any geometry.
        if let Some(at) = self.snap_at {
            painter.circle(at, SNAP_MARK, theme.accent, Stroke::new(2.0, theme.panel));
        }
        publish_handles(&shapes);
        if let Some(at) = self.snap_at {
            crate::scenario::publish_handle("dynamic_gnomon:snap", at);
        }
    }

    /// Follow this frame's drag, or begin one when the press landed on a handle. Every frame a
    /// drag changes the frame, the change is applied here and returned for the caller's values.
    /// A press that is released without moving is a click: it opens the handle's mode
    /// ([`GnomonActiveState`]) rather than moving the frame.
    ///
    /// While Origin Snap or Axis Orbit is on, the frame follows the pointer without a drag: it is
    /// placed on the candidate within [`SNAP_RADIUS`], or on the view plane through where the mode
    /// began, and the change is returned the same way. A press then takes that place as it ends
    /// the mode.
    pub fn interact(&mut self, proj: &Proj, input: &InputState) -> Option<Change> {
        if input.escape {
            self.enter_state(GnomonActiveState::Idle);
        }
        if let Some(mut drag) = self.drag {
            let change = input.pointer.filter(|p| finite_screen(*p)).and_then(|p| {
                if p.distance(drag.press) > CLICK_SLOP {
                    drag.moved = true;
                }
                self.apply(proj, p)
            });
            if input.primary_released || !input.primary_down {
                self.active_handle = GnomonHandle::None;
                self.hovered_handle = GnomonHandle::None;
                if drag.moved {
                    // A drag places the frame; a click asks for a mode or a value instead.
                    self.enter_state(GnomonActiveState::Idle);
                } else if let Some(mode) = click_mode(drag.handle, drag.press) {
                    // A second click on the handle that opened a snap mode drops it.
                    self.enter_state(toggled(self.state, mode));
                }
                self.drag = None;
            } else {
                self.drag = Some(drag);
            }
            return change;
        }
        let pointer = input.pointer.filter(|p| finite_screen(*p) && proj.rect.contains(*p));
        // A press on the caller's value entry belongs to that entry: it must not end the mode the
        // entry was opened by, nor start a drag behind the entry.
        self.hovered_handle = pointer.filter(|p| !self.on_entry(*p)).map_or(GnomonHandle::None, |p| self.hit(proj, p));
        let mut pressed = false;
        if input.primary_pressed
            && let Some(p) = pointer
            && !self.on_entry(p)
        {
            pressed = true;
            // While a click mode follows the pointer, the origin ball sits under it by
            // construction: a press there is the commit the mode is waiting for, not a second
            // click on the handle that opened it.
            let mut handle = self.hit(proj, p);
            if handle == GnomonHandle::Origin && self.snap_base.is_some() {
                handle = GnomonHandle::None;
            }
            if handle != GnomonHandle::None {
                self.begin(proj, p, handle);
            } else {
                // A press on the model takes the place the live snap found and ends the click
                // mode: the pick the caller takes is this place, not one under a moving pointer.
                self.enter_state(GnomonActiveState::Idle);
            }
        }
        // A click mode follows the pointer with no drag and no button down: the frame is placed
        // where the cursor points every frame, so a snap is taken on hover alone.
        if !pressed && !input.primary_pressed && !input.primary_down && self.drag.is_none() {
            if let Some(p) = pointer.filter(|p| !self.on_entry(*p)) {
                return self.track(proj, p);
            }
            self.snap_at = None;
        }
        None
    }

    /// Place the frame where `pos` points, for the click mode the frame is in. A candidate within
    /// [`SNAP_RADIUS`] is taken exactly; away from every candidate the pointer is unprojected
    /// onto the view plane through where the mode began. The change is reported only when the
    /// frame actually moved, so a still pointer costs nothing.
    fn track(&mut self, proj: &Proj, pos: Pos2) -> Option<Change> {
        let base = self.snap_base?;
        let snap = self.nearest(proj, pos);
        self.snap_at = snap.map(|(_, at, _)| at);
        let before = (self.origin, self.axes());
        match self.state {
            GnomonActiveState::OriginSnap => {
                let point = match snap {
                    Some((candidate, ..)) => candidate.point,
                    // Free space: the point under the pointer on a plane facing the camera
                    // through where the mode began, so the frame follows the cursor.
                    None => crate::gizmo::on_plane(proj, pos, base.origin, view_dir(proj, base.origin))?,
                };
                if !finite(point) {
                    return None;
                }
                self.origin = point;
                match snap.and_then(|(candidate, ..)| candidate.normal) {
                    // A face's centre turns the frame's Z to the face's outward normal, X kept
                    // as square to it as it can be (Gram-Schmidt).
                    Some(normal) => {
                        let _ = self.set_z_axis(normal);
                    }
                    // Any other point moves the origin only, in the frame the mode began with.
                    None => {
                        let _ = self.set_frame(base.axes[0], base.axes[2]);
                    }
                }
            }
            GnomonActiveState::AxisOrbit(axis) => {
                // Axis Orbit keeps the frame's origin where it was: only the axes turn.
                self.origin = base.origin;
                let target = match snap {
                    Some((candidate, ..)) => candidate.point,
                    None => crate::gizmo::on_plane(proj, pos, base.origin, view_dir(proj, base.origin))?,
                };
                let direction = (target - base.origin).normalized().filter(|d| finite(*d))?;
                let _ = self.set_axis_square(base.axes, axis, direction);
            }
            GnomonActiveState::Idle | GnomonActiveState::RelativeInput { .. } => return None,
        }
        ((self.origin, self.axes()) != before).then_some(Change::Frame)
    }

    /// The candidate nearest `pos` on screen, within [`SNAP_RADIUS`], with where it shows.
    fn nearest(&self, proj: &Proj, pos: Pos2) -> Option<(SnapCandidate, Pos2, f32)> {
        let mut best: Option<(SnapCandidate, Pos2, f32)> = None;
        for candidate in &self.candidates {
            let Some(at) = proj.to_screen(candidate.point) else { continue };
            let distance = at.distance(pos);
            if distance <= SNAP_RADIUS && best.as_ref().is_none_or(|(_, _, d)| distance < *d) {
                best = Some((*candidate, at, distance));
            }
        }
        best
    }

    /// Point `axis` along `direction`, its other two axes taken from `base` as near as they can
    /// be kept while staying square to it (Gram-Schmidt), the three right-handed: `Z × X = Y`.
    fn set_axis_square(&mut self, base: [Vec3; 3], axis: Axis, direction: Vec3) -> bool {
        let Some(direction) = UnitVec3::new(direction) else { return false };
        let (x, z) = match axis {
            Axis::X => (direction, square_to(base[2], direction)),
            Axis::Z => (square_to(base[0], direction), direction),
            Axis::Y => {
                let x = square_to(base[0], direction);
                let Some(z) = x.vec().cross(direction.vec()).normalized().and_then(UnitVec3::new) else { return false };
                (x, z)
            }
        };
        self.x_axis = x;
        self.z_axis = z;
        true
    }

    /// Whether `handle` is the hovered or dragged one.
    fn lit(&self, handle: GnomonHandle) -> bool {
        self.hovered_handle == handle || self.active_handle == handle
    }

    /// Take the handle under `pos` and remember the frame it started from. Nothing moves until
    /// the pointer does.
    fn begin(&mut self, proj: &Proj, pos: Pos2, handle: GnomonHandle) {
        let started = DragState {
            handle,
            origin: self.origin,
            x_axis: self.x_axis,
            z_axis: self.z_axis,
            grab: Grab::Point(Vec3::ZERO),
            press: pos,
            moved: false,
            view: view_dir(proj, self.origin),
        };
        // A cone takes no drag: its press is only ever a click, so it needs no grab.
        let grab = match handle {
            GnomonHandle::Cone(_) => Grab::Point(Vec3::ZERO),
            _ => match started.grab(proj, pos) {
                Some(grab) => grab,
                None => return,
            },
        };
        self.active_handle = handle;
        self.drag = Some(DragState { grab, ..started });
    }

    /// Apply the drag as far as `pos` takes it, measured from where it started.
    fn apply(&mut self, proj: &Proj, pos: Pos2) -> Option<Change> {
        let drag = self.drag?;
        match drag.handle {
            GnomonHandle::Origin => {
                let Grab::Point(grab) = drag.grab else { return None };
                let at = crate::gizmo::on_plane(proj, pos, drag.origin, drag.view)?;
                let delta = at - grab;
                if !finite(delta) {
                    return None;
                }
                self.origin = drag.origin + delta;
                Some(Change::Translate(delta))
            }
            GnomonHandle::Translate(axis) => {
                let Grab::Along(t0) = drag.grab else { return None };
                let direction = drag.axis(axis);
                let t = crate::canvas::along(proj, pos, drag.origin, direction)?;
                let delta = direction * (t - t0);
                if !finite(delta) {
                    return None;
                }
                self.origin = drag.origin + delta;
                Some(Change::Translate(delta))
            }
            GnomonHandle::Rotate(axis) => {
                let Grab::Angle(angle0) = drag.grab else { return None };
                let direction = drag.axis(axis);
                let at = crate::gizmo::on_plane(proj, pos, drag.origin, direction)?;
                let (e1, e2) = ring_basis(drag.frame(), axis);
                let dart = at - drag.origin;
                let angle = wrap(dart.dot(e2).atan2(dart.dot(e1)) - angle0);
                if !angle.is_finite() {
                    return None;
                }
                self.set_frame(rotate_about(drag.x_axis.vec(), direction, angle), rotate_about(drag.z_axis.vec(), direction, angle));
                Some(Change::Rotate { axis, angle })
            }
            // A cone never starts a drag, so its press changes nothing.
            GnomonHandle::Cone(_) | GnomonHandle::None => None,
        }
    }

    /// The handle under `pos`, in the order a press takes them: origin ball, cone, shaft, wheel.
    fn hit(&self, proj: &Proj, pos: Pos2) -> GnomonHandle {
        self.shapes(proj).map_or(GnomonHandle::None, |shapes| hit(&shapes, pos))
    }

    /// Where the frame's handles are on screen. `None` when the origin or a usable scale is
    /// behind the camera.
    fn shapes(&self, proj: &Proj) -> Option<Shapes> {
        let origin = proj.to_screen(self.origin)?;
        let len = mm_per_px(proj, self.origin)? * f64::from(ARM_PX);
        if !len.is_finite() || len <= 0.0 {
            return None;
        }
        let frame = self.axes();
        let mut arrows = Vec::with_capacity(3);
        let mut rings = Vec::with_capacity(3);
        for axis in Axis::ALL {
            let direction = frame[axis.index()];
            if let (Some(from), Some(to)) =
                (proj.to_screen(self.origin + direction * (ARM_START * len)), proj.to_screen(self.origin + direction * len))
            {
                arrows.push((axis, from, to));
            }
            // The wheel lies in the frame's own plane for `axis`, so it turns with the frame.
            let (e1, e2) = ring_basis(frame, axis);
            let points: Vec<Pos2> = (0..=RING_POINTS)
                .filter_map(|i| {
                    let angle = std::f64::consts::TAU * i as f64 / RING_POINTS as f64;
                    proj.to_screen(self.origin + (e1 * angle.cos() + e2 * angle.sin()) * (RING_SHARE * len))
                })
                .collect();
            if points.len() > 2 {
                rings.push((axis, points));
            }
        }
        Some(Shapes { origin, arrows, rings })
    }

    /// Replace the frame with `x` and `z` made square, unless either has no usable direction.
    fn set_frame(&mut self, x: Vec3, z: Vec3) -> bool {
        let Some(z) = UnitVec3::new(z) else { return false };
        let x = square_to(x, z);
        self.x_axis = x;
        self.z_axis = z;
        true
    }
}

/// `v` made unit length and square to the unit vector `z`. A `v` parallel to `z` has no direction
/// left: any square direction will do, so the frame keeps a usable third axis.
fn square_to(v: Vec3, z: UnitVec3) -> UnitVec3 {
    let square = v - *z * v.dot(*z);
    square.unit().or_else(|| z.any_perp().unit()).unwrap_or(UnitVec3::X)
}

/// The in-plane directions of the wheel about `axis`, in the frame's own axes `frame` (+X, +Y,
/// +Z), right-handed: `e1 × e2 = axis`.
fn ring_basis(frame: [Vec3; 3], axis: Axis) -> (Vec3, Vec3) {
    match axis {
        Axis::X => (frame[1], frame[2]),
        Axis::Y => (frame[2], frame[0]),
        Axis::Z => (frame[0], frame[1]),
    }
}

/// What a click on `handle` at `press` opens: the origin ball its Origin Snap Mode, a cone Axis
/// Orbit, a shaft or a wheel its value entry.
fn click_mode(handle: GnomonHandle, press: Pos2) -> Option<GnomonActiveState> {
    match handle {
        GnomonHandle::Origin => Some(GnomonActiveState::OriginSnap),
        GnomonHandle::Cone(axis) => Some(GnomonActiveState::AxisOrbit(axis)),
        GnomonHandle::Translate(_) | GnomonHandle::Rotate(_) => Some(GnomonActiveState::RelativeInput { handle, pos: press }),
        GnomonHandle::None => None,
    }
}

/// The state a click leaves: `opened`, or [`GnomonActiveState::Idle`] when the click landed on
/// the handle that already opened that snap mode, so a snap mode toggles on and off.
fn toggled(current: GnomonActiveState, opened: GnomonActiveState) -> GnomonActiveState {
    match (current, opened) {
        (GnomonActiveState::OriginSnap, GnomonActiveState::OriginSnap) => GnomonActiveState::Idle,
        (GnomonActiveState::AxisOrbit(a), GnomonActiveState::AxisOrbit(b)) if a == b => GnomonActiveState::Idle,
        _ => opened,
    }
}

/// `v` turned by `angle` radians about the unit vector `direction` (Rodrigues).
fn rotate_about(v: Vec3, direction: Vec3, angle: f64) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    v * cos + direction.cross(v) * sin + direction * (direction.dot(v) * (1.0 - cos))
}

/// An angle in (−π, π].
fn wrap(angle: f64) -> f64 {
    let mut a = angle % std::f64::consts::TAU;
    if a > std::f64::consts::PI {
        a -= std::f64::consts::TAU;
    } else if a <= -std::f64::consts::PI {
        a += std::f64::consts::TAU;
    }
    a
}

/// The view direction through `at`, or through the view's centre when `at` is behind the camera.
fn view_dir(proj: &Proj, at: Vec3) -> Vec3 {
    let c = proj.to_screen(at).unwrap_or_else(|| proj.rect.center());
    let direction = proj.ray(c).1;
    if finite(direction) { direction } else { Vec3::new(0.0, 0.0, -1.0) }
}

/// Millimetres per screen point at `at`.
fn mm_per_px(proj: &Proj, at: Vec3) -> Option<f64> {
    let c = proj.to_screen(at)?;
    let u = proj.ray(c).1.any_perp().normalized()?;
    let pixels = f64::from(c.distance(proj.to_screen(at + u)?));
    (pixels.is_finite() && pixels > 1e-8).then(|| 1.0 / pixels)
}

/// Distance from `p` to the screen segment `a`–`b`.
fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// The handle nearest `p`, strictly separated so the parts of an arm never steal each other's
/// clicks: a cone (the arrowhead at the arm's tip) first, then the origin ball, then the shaft
/// (the line the cone sits on), then a wheel. The shaft's test stops short of the cone, so the
/// two never overlap.
fn hit(shapes: &Shapes, p: Pos2) -> GnomonHandle {
    if let Some((axis, ..)) = shapes.arrows.iter().find(|(_, _, to)| to.distance(p) <= CONE_HIT) {
        return GnomonHandle::Cone(*axis);
    }
    if shapes.origin.distance(p) <= ORIGIN_HIT {
        return GnomonHandle::Origin;
    }
    if let Some((axis, _, _)) = shapes.arrows.iter().find(|(_, from, to)| seg_dist(p, *from, shaft_end(*from, *to)) <= AXIS_HIT) {
        return GnomonHandle::Translate(*axis);
    }
    shapes
        .rings
        .iter()
        .find(|(_, points)| points.windows(2).any(|w| seg_dist(p, w[0], w[1]) <= RING_HIT))
        .map_or(GnomonHandle::None, |(axis, _)| GnomonHandle::Rotate(*axis))
}

/// Where the shaft's hit test ends: `to` backed off by the cone's radius, so a click on the
/// arrowhead is never taken as the line beneath it.
fn shaft_end(from: Pos2, to: Pos2) -> Pos2 {
    let v = to - from;
    let len = v.length();
    if len <= CONE_HIT { to } else { to - v / len * CONE_HIT }
}

/// Draw one arm: its line and an arrowhead at `to`.
fn arrow(painter: &Painter, from: Pos2, to: Pos2, colour: Color32, hot: bool) {
    let v = to - from;
    let len = v.length();
    if len < 1.0 {
        return;
    }
    let u = v / len;
    let n = egui::vec2(-u.y, u.x);
    painter.line_segment([from, to], Stroke::new(if hot { 3.0 } else { 2.0 }, colour));
    let head = 12.0f32.min(len * 0.5);
    painter.add(Shape::convex_polygon(vec![to + u * 2.0, to - u * head + n * head * 0.4, to - u * head - n * head * 0.4], colour, Stroke::NONE));
}

/// Publish the handles for scenarios and checks, every frame they show.
fn publish_handles(shapes: &Shapes) {
    crate::scenario::publish_handle("dynamic_gnomon:origin", shapes.origin);
    for (axis, _, to) in &shapes.arrows {
        crate::scenario::publish_handle(&format!("dynamic_gnomon:{}", axis_name(*axis)), *to);
        crate::scenario::publish_handle(&format!("dynamic_gnomon:cone_{}", axis_name(*axis)), *to);
    }
    for (axis, points) in &shapes.rings {
        if let Some(at) = points.first() {
            crate::scenario::publish_handle(&format!("dynamic_gnomon:r{}", axis_name(*axis)), *at);
        }
    }
}

/// The axis's name, as a published handle names it.
fn axis_name(axis: Axis) -> &'static str {
    match axis {
        Axis::X => "x",
        Axis::Y => "y",
        Axis::Z => "z",
    }
}

/// Whether every part of `v` is a finite number.
fn finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

/// Whether a screen point is a finite number.
fn finite_screen(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

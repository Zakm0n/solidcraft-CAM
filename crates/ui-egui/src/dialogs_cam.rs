//! The Cam dialog (`joint.cam`): a cam turning on a revolute joint drives a follower, a slider or
//! a revolute, along the cam's profile (a closed sketch profile, or edges of the cam's body). The
//! follower is a knife edge, a roller (its radius typed, or its round face picked, which is also
//! where it touches) or a flat face. Picking the profile on a part chooses the joint that part
//! turns on. Play turns the cam a full turn so the follower shows its motion before OK.

use crate::ui_kit::BackingTooltipExt as _;
use crate::ui_kit::{Container, Surface};
use egui::{Painter, Pos2, RichText};
use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_engine::Sel;
use solidcraft_engine::Session;
use solidcraft_engine::doc::joints::JointKind;
use solidcraft_engine::geom::prelude::*;
use solidcraft_engine::kernel::FaceSurface;

#[path = "dialogs_cam_drill.rs"]
pub mod drill;
#[path = "dialogs_cam_facing.rs"]
pub mod facing;

use super::{ROUND, joint_row, occurrence, pick, picked_radius, trim, turn};
use crate::SolveApp;
use crate::dialogs::Kind;
use crate::dialogs::{Dialog, combo, field, form_row};
use crate::dialogs_assembly::Asm;
use crate::dialogs_cam_pocket::PocketDialog;
use crate::dynamic_gnomon::{DynamicGnomon, GnomonActiveState, GnomonAlignMode, GnomonHandle, IncrementalKeys, InputState, SnapCandidate};
use crate::selection::{BODIES, EDGES, FACES, PROFILES, SelInput, SelectionExt as _, VERTICES};
use crate::theme::Tokens;
use crate::viewport::Proj;

/// The follower types, as `joint.cam` takes them, and their labels.
const FOLLOWERS: [&str; 3] = ["knife", "roller", "flat"];
const FOLLOWER_LABELS: [&str; 3] = ["Knife edge", "Roller", "Flat face"];

/// The inputs: the cam's profile, the roller's round face, the follower's tip.
const PROFILE: usize = 0;
const ROLLER: usize = 1;
const TIP: usize = 2;

#[derive(Clone, Debug)]
pub struct CamForm {
    /// The cam's joint and the follower's (ids).
    pub a: u64,
    pub b: u64,
    /// Index into `FOLLOWERS`.
    pub follower: usize,
    /// The roller's radius, used where no round face is picked.
    pub radius: String,
    /// Ride the cam from the other side.
    pub flip: bool,
    /// The body of the profile pick the cam's joint was last chosen from, so a pick chooses once.
    pub picked: Option<String>,
    pub animate: bool,
    /// Animation phase, 0…1.
    pub phase: f64,
}

/// Joints of a kind: (id, label).
fn joints_of(s: &Session, kinds: &[JointKind]) -> Vec<(u64, String)> {
    s.doc.assembly.joints.iter().filter(|j| kinds.contains(&j.kind)).map(|j| (j.id, crate::dialogs_assembly::joint_choice(&j.name, j.kind))).collect()
}

pub fn start(app: &SolveApp) -> (Asm, Vec<SelInput>) {
    let s = &app.session;
    let cams = joints_of(s, &[JointKind::Revolute]);
    let followers = joints_of(s, &[JointKind::Slider, JointKind::Revolute]);
    let a = cams.first().map(|j| j.0).unwrap_or(0);
    // A slider first, else another revolute than the cam's.
    let b = followers.iter().find(|j| j.0 != a).map(|j| j.0).unwrap_or(0);
    let f = CamForm {
        a,
        b,
        follower: 0,
        radius: crate::dialogs::length_default(&app.session, "5"),
        flip: false,
        picked: None,
        animate: false,
        phase: 0.0,
    };
    let inputs = vec![
        SelInput::new("Profile", PROFILES | EDGES, true),
        SelInput::new("Roller (optional)", ROUND, false),
        SelInput::new("Tip (optional)", FACES | EDGES | VERTICES, false),
    ];
    (Asm::Cam(Box::new(f)), inputs)
}

/// The occurrence the profile picked in `inputs` is on: an edge's body's, or a sketch's component's
/// first.
fn profile_occurrence(s: &Session, inputs: &[SelInput]) -> Option<(String, u64)> {
    match inputs.get(PROFILE)?.items.first()? {
        Sel::Edge { body, .. } => Some((body.clone(), occurrence(s, body)?)),
        Sel::Profile { sketch, .. } => {
            let comp = s.doc.feature(*sketch).map(|f| f.component)?;
            let occ = if comp == 0 { 0 } else { s.doc.occurrence_of(comp)?.id };
            Some((format!("sketch {sketch}"), occ))
        }
        _ => None,
    }
}

/// The dialog's rows; true when Enter was pressed in a value.
pub fn rows(app: &mut SolveApp, ui: &mut egui::Ui, f: &mut CamForm, inputs: &[SelInput]) -> bool {
    let t = Tokens::get();
    let s = &app.session;
    let (cams, followers) = (joints_of(s, &[JointKind::Revolute]), joints_of(s, &[JointKind::Slider, JointKind::Revolute]));
    if cams.is_empty() || followers.len() < 2 && followers.iter().all(|j| cams.iter().any(|c| c.0 == j.0)) {
        crate::ui_kit::status_line(ui, "needs a joint that turns and one that slides or swings", crate::ui_kit::StatusTone::Dim);
        return false;
    }
    // A profile picked on a part chooses the joint that part turns on, once.
    let at = profile_occurrence(s, inputs);
    if at.as_ref().map(|x| &x.0) != f.picked.as_ref() {
        if let Some(j) = at.as_ref().and_then(|(_, o)| {
            let turning = s.doc.assembly.joints.iter().filter(|j| j.kind == JointKind::Revolute);
            turning.clone().find(|j| j.b.occurrence == *o).or(turning.clone().find(|j| j.a.occurrence == *o))
        }) {
            f.a = j.id;
            if f.b == f.a {
                f.b = followers.iter().find(|x| x.0 != f.a).map(|x| x.0).unwrap_or(0);
            }
        }
        f.picked = at.map(|x| x.0);
    }
    let mut enter = false;
    joint_row(ui, "Cam", "cam_a", &cams, &mut f.a);
    joint_row(ui, "Follower", "cam_b", &followers, &mut f.b);
    form_row(ui, "Type", |ui| {
        let labels: Vec<&str> = FOLLOWER_LABELS.iter().map(|l| crate::i18n::tr_label(l)).collect();
        combo(ui, "cam_follower", &labels, &mut f.follower);
    });
    if FOLLOWERS.get(f.follower) == Some(&"roller") {
        form_row(ui, "Radius", |ui| match (pick(inputs, ROLLER), picked_radius(s, inputs, ROLLER)) {
            (_, Some(r)) => {
                ui.label(format!("{} mm  ({})", trim(r), crate::i18n::tr_label("picked")));
            }
            (Some(_), None) => {
                ui.label(RichText::new(crate::i18n::tr_label("not a round face")).color(t.warning));
            }
            (None, None) => enter |= field(ui, &mut f.radius),
        });
    }
    crate::ui_kit::options(ui, "cam_options", &mut [("Other side", &mut f.flip)]);
    form_row(ui, "Animate", |ui| {
        let label = crate::i18n::tr(if f.animate { "ui.joint_stop" } else { "ui.joint_play" });
        let r = crate::ui_kit::button(ui, label, crate::ui_kit::Button::Normal)
            .backing_on_hover_text(crate::i18n::tr_label("Play the joint's motion in the view"));
        crate::scenario::publish_handle("dialog:Animate", r.rect.center());
        if r.clicked() {
            f.animate = !f.animate;
        }
    });
    if f.animate {
        let now = ui.input(|i| i.time);
        // 72 steps over 6 s: each one a preview.
        f.phase = ((now / 6.0).fract() * 72.0).floor() / 72.0;
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
    }
    enter
}

/// The command OK runs.
pub fn commands(app: &SolveApp, f: &CamForm, inputs: &[SelInput]) -> Result<Vec<(String, Value)>, String> {
    let s = &app.session;
    let joint = |id: u64| {
        s.doc.assembly.joints.iter().find(|j| j.id == id).map(|j| j.id).ok_or_else(|| "choose the cam's and the follower's joints".to_string())
    };
    let (a, b) = (joint(f.a)?, joint(f.b)?);
    let items = inputs.get(PROFILE).map(|i| i.items.as_slice()).unwrap_or_default();
    let profile = match items.first() {
        Some(Sel::Profile { sketch, index }) => json!({"sketch": sketch, "index": index}),
        Some(Sel::Edge { body, .. }) => {
            let pts: Vec<Value> = items.edge_points().into_iter().map(crate::selection::point_value).collect();
            let mut p = json!({ "edges": pts });
            if let Some(o) = occurrence(s, body) {
                p["occurrence"] = json!(o);
            }
            p
        }
        _ => return Err("select the cam's profile (a sketch profile or the cam's edges)".into()),
    };
    let follower = FOLLOWERS.get(f.follower).copied().unwrap_or("knife");
    let mut p = json!({"a": a, "b": b, "profile": profile, "follower": follower, "flip": f.flip});
    if follower == "roller" {
        p["radius"] = match pick(inputs, ROLLER) {
            Some((body, at)) => {
                let mut v = json!({"circle": [at.x, at.y, at.z]});
                if let Some(o) = occurrence(s, body) {
                    v["occurrence"] = json!(o);
                }
                v
            }
            None => json!(f.radius.trim()),
        };
    }
    if let Some(snap) = inputs.get(TIP).and_then(|i| i.items.first()).and_then(|x| crate::dialogs_assembly::snap_of(s, x)) {
        p["tip"] = snap.param;
    }
    Ok(vec![("joint.cam".to_string(), p)])
}

/// While animating, the preview turns the cam a full turn.
pub fn animate(s: &Session, f: &CamForm, cmds: &mut Vec<(String, Value)>) {
    if f.animate {
        turn(s, f.a, f.phase, cmds);
    }
}

// The CAM workspace's own dialogs (`cam.setup.create`, `cam.tool.list`): the machining setup a
// part is cut in and the tool library it is cut with. The joint cam's dialog above is the
// assembly feature of the same abbreviation; these forms belong to the Toolpath tab.

/// The CAM workspace forms the Toolpath tab starts.
#[derive(Clone, Debug)]
pub enum Zcam {
    /// A machining setup: its coordinate system, stock and bodies (`cam.setup.create`).
    Setup(DialogCamSetup),
    /// A machining origin and axes placed in the view, aligned to a face or an edge
    /// (`cam.coordinate_system.create`). Boxed: its editable fields and the frame it drags are
    /// several times the size of the other forms, which the dialog enum holds inline.
    CoordinateSystem(Box<DialogCamCoordinateSystem>),
    /// The design's tool library: pick a tool to edit, or fill in a new one (`cam.tool.list`).
    ToolLibrary(DialogCamToolLibrary),
    /// Follow a part's outline with the side of the cutter (`cam.toolpath.contour`).
    Contour(DialogCamContour),
    /// Follow the picked edges with the side of the cutter, offset by its radius
    /// (`cam.toolpath.contour2d`).
    Contour2d(DialogCamContour2d),
    /// Clear the material inside a part's outline (`cam.toolpath.pocket`).
    Pocket(PocketDialog),
    /// Clear the region of the picked faces with concentric offset passes
    /// (`cam.toolpath.pocket_2d`).
    Pocket2d(DialogCamPocket2d),
    /// Plunge a tool at the picked holes' rims, in a cycle down to the chosen depth
    /// (`cam.toolpath.drill`).
    Drill(drill::DialogCamDrill),
    /// Sweep the cutter back and forth across the top of the stock, pass beside pass, down to a
    /// depth (`cam.toolpath.facing`).
    Facing(facing::DialogCamFacing),
}

impl Zcam {
    /// The command OK runs.
    pub fn command(&self) -> &'static str {
        match self {
            Zcam::Setup(_) => "cam.setup.create",
            Zcam::CoordinateSystem(f) => {
                if f.editing.is_some() {
                    "cam.coordinate_system.edit"
                } else {
                    "cam.coordinate_system.create"
                }
            }
            Zcam::ToolLibrary(_) => "cam.tool.list",
            Zcam::Contour(_) => "cam.toolpath.contour",
            Zcam::Contour2d(_) => "cam.toolpath.contour2d",
            Zcam::Pocket(_) => "cam.toolpath.pocket",
            Zcam::Pocket2d(_) => "cam.toolpath.pocket_2d",
            Zcam::Drill(_) => "cam.toolpath.drill",
            Zcam::Facing(_) => "cam.toolpath.facing",
        }
    }

    /// The dialog's heading.
    pub fn title(&self) -> &'static str {
        match self {
            Zcam::Setup(_) => "SETUP",
            Zcam::CoordinateSystem(_) => "COORDINATE SYSTEM",
            Zcam::ToolLibrary(_) => "TOOL LIBRARY",
            Zcam::Contour(_) => "CONTOUR",
            Zcam::Contour2d(_) => "2D CONTOUR",
            Zcam::Pocket(_) => "POCKET",
            Zcam::Pocket2d(_) => "2D POCKET",
            Zcam::Drill(_) => "DRILLING",
            Zcam::Facing(_) => "2D FACING",
        }
    }
}

/// The kinds a tool library entry may have, in the command's spelling, and their labels.
const TOOL_KINDS: [&str; 3] = ["flat_end_mill", "ball_end_mill", "bull_nose_end_mill"];
const TOOL_KIND_LABELS: [&str; 3] = ["Flat end mill", "Ball end mill", "Bull nose end mill"];

/// A machining setup's form: its name, coordinate system, raw stock and bodies.
#[derive(Clone, Debug)]
pub struct DialogCamSetup {
    /// Empty: the command names it "Setup" and the first free number.
    pub name: String,
    /// Index into the coordinate system choices: 0 is the model origin, n is
    /// `doc.coordinate_systems[n - 1]`.
    pub coordinate_system: usize,
    /// The stock's size along X, Y and Z; 0 along an axis takes the bodies' extent there.
    pub stock_x: String,
    pub stock_y: String,
    pub stock_z: String,
    /// Where the stock's corner nearest the machine zero sits.
    pub offset_x: String,
    pub offset_y: String,
    pub offset_z: String,
}

impl DialogCamSetup {
    fn new(app: &SolveApp) -> Self {
        let zero = || crate::length_input::default(&app.session.doc, "0");
        DialogCamSetup {
            name: String::new(),
            coordinate_system: 0,
            stock_x: zero(),
            stock_y: zero(),
            stock_z: zero(),
            offset_x: zero(),
            offset_y: zero(),
            offset_z: zero(),
        }
    }
}

/// A machining origin being placed (`cam.coordinate_system.create`): a dynamic gnomon the user
/// drags and snaps to B-rep geometry in the view, and its origin typed exactly.
#[derive(Clone, Debug)]
pub struct DialogCamCoordinateSystem {
    /// The frame the view draws and drags: its origin, +X and +Z (Y is Z × X).
    pub gnomon: DynamicGnomon,
    /// The machining setup this frame is made for, when the dialog was opened from one.
    pub target_setup: Option<u64>,
    /// Empty: the command names it "Coordinate System" and the first free number.
    pub name: String,
    /// The origin's X, Y and Z, as typed.
    pub origin_x: String,
    pub origin_y: String,
    pub origin_z: String,
    /// The frame moved in the view (a drag or an alignment): the fields show it again.
    refresh: bool,
    /// The coordinate system this dialog changes (`cam.coordinate_system.edit`); `None` creates
    /// a new one.
    editing: Option<u64>,
    /// The value a click on an arm the frame draws or on a wheel opened, and that handle. Boxed:
    /// the entry is open only while its field is, and the dialog is in a small enum.
    entry: Option<Box<RelativeEntry>>,
    /// The screen rectangle the entry was drawn in last frame, handed to the gnomon so a press
    /// inside the entry stays with it. `None` while no entry is open.
    entry_rect: Option<egui::Rect>,
    /// The click mode the gnomon showed last frame, so a new one opens its value entry once.
    seen: GnomonActiveState,
}

/// The floating value entry a click on an arm's shaft or a wheel opens: a relative distance
/// along the shaft, or a relative angle about the wheel.
#[derive(Clone, Debug)]
struct RelativeEntry {
    /// The handle clicked: a shaft ([`GnomonHandle::Translate`]) or a wheel
    /// ([`GnomonHandle::Rotate`]).
    handle: GnomonHandle,
    /// The value as typed, in mm or in degrees. Empty as the entry opens, so the field starts
    /// zeroed rather than showing a value the user did not ask for.
    text: String,
    /// The field still needs the keyboard: true as the entry opens, cleared once it has it.
    focus: bool,
}

impl RelativeEntry {
    /// The row's label: the quantity a value of this handle's entry is.
    fn label(&self) -> &'static str {
        match self.handle {
            GnomonHandle::Rotate(_) => "Angle",
            _ => "Distance",
        }
    }

    /// The unit a value of this entry is typed in.
    fn unit(&self) -> &'static str {
        match self.handle {
            GnomonHandle::Rotate(_) => "deg",
            _ => "mm",
        }
    }

    /// The expression kind this entry's value is read as.
    fn kind(&self) -> solidcraft_engine::doc::expr::Kind {
        match self.handle {
            GnomonHandle::Rotate(_) => solidcraft_engine::doc::expr::Kind::Angle,
            _ => solidcraft_engine::doc::expr::Kind::Length,
        }
    }
}

impl DialogCamCoordinateSystem {
    fn new(s: &Session) -> Self {
        let zero = || crate::length_input::shown(&s.doc, 0.0);
        DialogCamCoordinateSystem {
            gnomon: DynamicGnomon::default(),
            target_setup: None,
            name: String::new(),
            origin_x: zero(),
            origin_y: zero(),
            origin_z: zero(),
            refresh: false,
            editing: None,
            entry: None,
            entry_rect: None,
            seen: GnomonActiveState::Idle,
        }
    }

    /// Write the frame's origin into the fields, where it moved in the view.
    fn sync_fields(&mut self, doc: &solidcraft_engine::doc::Document) {
        let o = self.gnomon.origin();
        self.origin_x = crate::length_input::shown(doc, o.x);
        self.origin_y = crate::length_input::shown(doc, o.y);
        self.origin_z = crate::length_input::shown(doc, o.z);
    }

    /// The coordinate system this form changes, when it edits one rather than creating one.
    pub fn editing(&self) -> Option<u64> {
        self.editing
    }

    /// Fill the form with an existing coordinate system: its name, origin and frame, and the
    /// command OK runs becomes `cam.coordinate_system.edit`. False when the design has no such
    /// coordinate system, or its frame holds a direction the view cannot use.
    pub fn edit_existing(&mut self, doc: &solidcraft_engine::doc::Document, id: u64) -> bool {
        let Some(cs) = doc.coordinate_systems.iter().find(|c| c.id == id) else { return false };
        let (Some(origin), Some(x), Some(z)) = (finite_point(cs.origin), finite_point(cs.x_axis), finite_point(cs.z_axis)) else {
            return false;
        };
        let Some(gnomon) = DynamicGnomon::new(origin, x, z) else { return false };
        self.gnomon = gnomon;
        self.name = cs.name.clone();
        self.editing = Some(id);
        self.sync_fields(doc);
        true
    }

    /// Start from a construction plane: its origin, its own X and Y (so Z is the plane's
    /// normal), its name, and Origin Snap on so the first pick moves the frame onto a point.
    pub fn from_plane(&mut self, doc: &solidcraft_engine::doc::Document, plane: solidcore_record::Plane, name: String) {
        if let Some(gnomon) = DynamicGnomon::new(plane.origin, plane.x, plane.x.cross(plane.y)) {
            self.gnomon = gnomon;
        }
        self.name = name;
        self.gnomon.set_align_mode(GnomonAlignMode::SnapOrigin);
        self.sync_fields(doc);
    }
}

/// A stored point as a frame can use it: `None` when a part is not a finite number.
fn finite_point(p: [f64; 3]) -> Option<Vec3> {
    p.iter().all(|v| v.is_finite()).then(|| Vec3::new(p[0], p[1], p[2]))
}

/// Where a click on the frame snaps to: the exact geometry the B-rep holds for the pick, not the
/// point under the pointer, and the face normal that comes with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SnapTarget {
    /// The snap point: a vertex, an edge's midpoint, a circle's centre or a face's centroid.
    pub point: Vec3,
    /// A planar face's outward normal, which the frame's Z follows.
    pub normal: Option<Vec3>,
}

/// What `sel` snaps to. A vertex is itself; a face its centroid, with the outward normal of its
/// plane; an edge the middle of its straight line, or the centre of the circle it lies on.
pub(crate) fn snap_target(app: &SolveApp, sel: &Sel) -> Option<SnapTarget> {
    let st = crate::open_meshing::session_world(&app.session);
    match sel {
        Sel::Vertex { point, .. } => Some(SnapTarget { point: *point, normal: None }),
        Sel::Face { body, index, point } => {
            let b = st.body(body)?;
            let tol = (b.body.size() * 1e-3).max(1e-3);
            let centroid = b.body.faces(tol).ok().and_then(|f| f.iter().find(|f| f.index == *index).map(|f| f.centroid)).unwrap_or(*point);
            let normal = match solidcraft_engine::kernel::face_surface(&b.body, *index) {
                Some(FaceSurface::Plane { normal, .. }) => Some(normal.vec()),
                _ => None,
            };
            Some(SnapTarget { point: centroid, normal })
        }
        Sel::Edge { body, index, point } => {
            let b = st.body(body)?;
            Some(SnapTarget { point: edge_snap_point(&b.body, *index).unwrap_or(*point), normal: None })
        }
        _ => None,
    }
}

/// An edge's exact snap point: the middle of a straight edge, the centre of the circle a circular
/// edge lies on. `None` for any other curve.
fn edge_snap_point(body: &solidcraft_engine::kernel::Body, index: usize) -> Option<Vec3> {
    use solidcore_record::brep::Curve;
    match &body.record().edges.get(index)?.curve {
        Curve::Line { start, end } => {
            let mid = (Vec3::new(start[0], start[1], start[2]) + Vec3::new(end[0], end[1], end[2])) * 0.5;
            (mid.is_finite()).then_some(mid)
        }
        Curve::Circle { .. } => solidcraft_engine::kernel::edge_circle(body, index).map(|c| c.center).filter(|c| c.is_finite()),
        _ => None,
    }
}

/// Put the frame's origin on `target`, and its Z along the target's normal when the target came
/// from a planar face. True when the frame changed.
fn snap_origin(gnomon: &mut DynamicGnomon, target: SnapTarget) -> bool {
    let moved = gnomon.set_origin(target.point);
    let aligned = target.normal.is_some_and(|n| gnomon.set_z_axis(n));
    moved || aligned
}

/// The points a live Origin Snap or Axis Orbit may take while the frame is placed: every visible
/// body's vertices, straight edges' midpoints, circles' centres and faces' centroids (a planar
/// face's centroid carrying its outward normal), and every visible sketch's points. The B-rep is
/// read exactly, as a pick is; a point that is not a finite number is left out, since a design
/// file is input like any other.
pub(crate) fn snap_candidates(app: &SolveApp) -> Vec<SnapCandidate> {
    let st = crate::open_meshing::session_world(&app.session);
    let hidden = &app.session.visibility.hidden_bodies;
    let mut out = Vec::new();
    for b in &st.bodies {
        if hidden.iter().any(|name| name == &b.name) {
            continue;
        }
        let record = b.body.record();
        for vertex in &record.vertices {
            if let Some(point) = finite_point(vertex.point) {
                out.push(SnapCandidate { point, normal: None });
            }
        }
        for (index, edge) in record.edges.iter().enumerate() {
            let point = match &edge.curve {
                solidcore_record::brep::Curve::Line { start, end } => {
                    let (Some(start), Some(end)) = (finite_point(*start), finite_point(*end)) else { continue };
                    Some((start + end) * 0.5)
                }
                solidcore_record::brep::Curve::Circle { .. } => solidcraft_engine::kernel::edge_circle(&b.body, index).map(|c| c.center),
                _ => None,
            };
            if let Some(point) = point.filter(|p| p.is_finite()) {
                out.push(SnapCandidate { point, normal: None });
            }
        }
        let tol = (b.body.size() * 1e-3).max(1e-3);
        for face in b.body.faces(tol).unwrap_or_default() {
            if face.centroid.is_finite() {
                out.push(SnapCandidate { point: face.centroid, normal: face.plane_normal });
            }
        }
    }
    for sketch in &st.sketches {
        if app.session.visibility.hidden_sketches.contains(&sketch.feature) {
            continue;
        }
        for point in &sketch.sketch.points {
            let world = sketch.plane.to_world(point.pos);
            if world.is_finite() {
                out.push(SnapCandidate { point: world, normal: None });
            }
        }
    }
    out
}

impl DialogCamCoordinateSystem {
    /// Follow the click the gnomon took: the origin ball turns Origin Snap on, and a shaft or a
    /// wheel opens its value entry.
    fn follow_click(&mut self) {
        let state = self.gnomon.active_state();
        if state == self.seen {
            return;
        }
        match state {
            // The ball's click makes the next pick put the origin on it.
            GnomonActiveState::OriginSnap => {
                self.gnomon.set_align_mode(GnomonAlignMode::SnapOrigin);
                self.entry = None;
            }
            GnomonActiveState::RelativeInput { handle, .. } => {
                self.entry = Some(Box::new(RelativeEntry { handle, text: String::new(), focus: true }))
            }
            GnomonActiveState::AxisOrbit(_) => self.entry = None,
            GnomonActiveState::Idle => {
                // A snap mode dropped by a second click on its handle leaves no alignment behind.
                if self.seen == GnomonActiveState::OriginSnap {
                    self.gnomon.set_align_mode(GnomonAlignMode::Free);
                }
                self.entry = None;
            }
        }
        self.seen = state;
    }

    /// Take a pick for the click mode the gnomon is in, or align the frame as the alignment mode
    /// says. True when the frame changed.
    fn take_pick(&mut self, app: &SolveApp, sel: &Sel) -> bool {
        match self.gnomon.active_state() {
            GnomonActiveState::OriginSnap => {
                let Some(target) = snap_target(app, sel) else { return false };
                let changed = snap_origin(&mut self.gnomon, target);
                self.end_click();
                changed
            }
            GnomonActiveState::AxisOrbit(axis) => {
                let Some(target) = snap_target(app, sel) else { return false };
                let changed = self.gnomon.aim_axis(axis, target.point);
                self.end_click();
                changed
            }
            GnomonActiveState::Idle | GnomonActiveState::RelativeInput { .. } => align_to(app, self, sel),
        }
    }

    /// End the click mode once its pick has been taken, so each click acts once.
    fn end_click(&mut self) {
        self.gnomon.set_active_state(GnomonActiveState::Idle);
        self.seen = GnomonActiveState::Idle;
        self.entry = None;
    }

    /// Apply the relative value typed into the entry: move the frame along the arm's shaft, or
    /// turn it about the wheel. The entry stays open while its value is not a number.
    fn apply_entry(&mut self, app: &SolveApp) {
        let Some(entry) = self.entry.as_ref() else { return };
        let Ok(value) = crate::length_input::eval(&app.session.doc, entry.text.trim(), entry.kind()) else { return };
        if !value.is_finite() {
            return;
        }
        let handle = entry.handle;
        self.entry = None;
        let changed = match handle {
            GnomonHandle::Translate(axis) => {
                let direction = self.gnomon.axes()[axis.index()];
                self.gnomon.translate(direction * value)
            }
            GnomonHandle::Rotate(axis) => self.gnomon.rotate(axis, value),
            GnomonHandle::Origin | GnomonHandle::Cone(_) | GnomonHandle::None => false,
        };
        if changed {
            self.refresh = true;
        }
        self.end_click();
    }
}

/// The id of the floating value entry's field, whose keyboard ownership the gnomon and the
/// dialog both answer to while the entry is open.
fn entry_field_id() -> egui::Id {
    egui::Id::unique("zcam_gnomon_entry_value")
}

/// The floating value entry a click on a shaft or a wheel opens, at `at` where the click was: its
/// label and unit, and the keyboard in its field. Enter applies it, Escape drops it, and a click
/// anywhere outside it ends it; a click inside it belongs to its own field, so it stays open.
fn entry_box(app: &SolveApp, ui: &mut egui::Ui, f: &mut DialogCamCoordinateSystem, at: Pos2, keys: IncrementalKeys) {
    let Some(entry) = f.entry.as_mut() else { return };
    let t = Tokens::get();
    let area = egui::Id::unique("zcam_gnomon_entry");
    let id = entry_field_id();
    // The entry opens empty and takes the keyboard at once, so typing goes straight into it. An
    // area's first frame is its sizing pass, which takes no focus, so the field asks until it
    // really holds the keyboard.
    if entry.focus {
        if ui.memory(|memory| memory.has_focus(id) && memory.had_focus_last_frame(id)) {
            entry.focus = false;
        } else {
            ui.memory_mut(|memory| memory.request_focus(id));
        }
    }
    let shown = Container::new(ui.ctx(), Surface::Canvas)
        .area(area)
        .fixed_pos(at + egui::vec2(t.space_8, t.space_8))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            let frame = Container::popup_frame(ui.style()).inner_margin(egui::Margin::symmetric(6, 3));
            Container::show_backing(ui, frame, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(entry.label()).color(t.text_dim).size(t.font_caption));
                    crate::ui_kit::canvas_value_field(ui, id, &mut entry.text, false);
                    ui.label(RichText::new(entry.unit()).color(t.text_dim).size(t.font_caption));
                });
            });
        });
    // A press inside this rectangle stays with the entry (its field), as the gnomon knows.
    f.entry_rect = Some(shown.response.rect);
    // The global shortcuts run before the view, and egui drops a focused field's keyboard at the
    // start of a frame when Escape is pressed. Keep the keyboard on the entry's field so Escape
    // reaches this frame as the entry's own key instead of a dialog-cancelling shortcut.
    ui.memory_mut(|memory| memory.set_focus_lock_filter(id, egui::EventFilter { escape: true, ..Default::default() }));
    // The entry's field takes Enter and Escape for itself: they applied or dropped its value
    // above, so they must not also submit or cancel the dialog that opened the gnomon.
    if keys.dismiss {
        f.entry = None;
        f.entry_rect = None;
        f.end_click();
    } else if keys.apply {
        f.apply_entry(app);
    }
}

/// The dialog's rows: its name and the origin's three values. The frame is placed in the view —
/// dragged, snapped by a click — so the form offers no alignment choice.
fn coordinate_system_rows(app: &mut SolveApp, ui: &mut egui::Ui, f: &mut DialogCamCoordinateSystem) -> bool {
    // The gnomon's floating value entry owned Enter and Escape this frame while its field held
    // the keyboard: they applied or dropped the entry's value, so no row here — and neither the
    // dialog's OK nor its Cancel — may act on them again.
    let entry_owns_keys = f.gnomon.is_incremental_input_active();
    if entry_owns_keys {
        ui.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
        });
    }
    let mut enter = crate::ui_kit::text_row(ui, "label.name", &mut f.name, "").0;
    // A drag in the view writes its origin here; typing here moves the frame.
    if f.refresh {
        f.sync_fields(&app.session.doc);
        f.refresh = false;
    }
    enter |= crate::ui_kit::expr_row(ui, "label.origin_x", &mut f.origin_x);
    enter |= crate::ui_kit::expr_row(ui, "label.origin_y", &mut f.origin_y);
    enter |= crate::ui_kit::expr_row(ui, "label.origin_z", &mut f.origin_z);
    let mut o = f.gnomon.origin();
    let mut typed = false;
    for (text, slot) in [(&f.origin_x, 0usize), (&f.origin_y, 1), (&f.origin_z, 2)] {
        let Ok(v) = crate::length_input::eval(&app.session.doc, text.trim(), solidcraft_engine::doc::expr::Kind::Length) else { continue };
        if !v.is_finite() {
            continue;
        }
        let next = match slot {
            0 => Vec3::new(v, o.y, o.z),
            1 => Vec3::new(o.x, v, o.z),
            _ => Vec3::new(o.x, o.y, v),
        };
        if next != o {
            o = next;
            typed = true;
        }
    }
    if typed {
        let _ = f.gnomon.set_origin(o);
    }
    let axes = [f.gnomon.x_axis().vec(), f.gnomon.y_axis().vec(), f.gnomon.z_axis().vec()];
    let readout =
        ["X", "Y", "Z"].into_iter().zip(axes).map(|(name, v)| format!("{name} ({:.2}, {:.2}, {:.2})", v.x, v.y, v.z)).collect::<Vec<_>>().join("   ");
    crate::ui_kit::status_line(ui, &readout, crate::ui_kit::StatusTone::Dim);
    enter && !entry_owns_keys
}

/// The command OK runs: the exact frame the gnomon holds.
fn coordinate_system_commands(f: &DialogCamCoordinateSystem) -> Result<Vec<(String, Value)>, String> {
    let (x, y, z) = (f.gnomon.x_axis().vec(), f.gnomon.y_axis().vec(), f.gnomon.z_axis().vec());
    if [x, y, z].iter().any(|v| !v.is_finite()) {
        return Err("the frame's axes are not numbers".into());
    }
    let point = |v: Vec3| json!([v.x, v.y, v.z]);
    let origin = f.gnomon.origin();
    if !origin.is_finite() {
        return Err("the frame's origin is not a number".into());
    }
    let mut p = json!({
        "origin": point(origin),
        "x_axis": point(x),
        "y_axis": point(y),
        "z_axis": point(z),
    });
    let name = f.name.trim();
    if !name.is_empty() {
        p["name"] = json!(name);
    }
    match f.editing {
        Some(id) => {
            p["coordinate_system"] = json!(id);
            Ok(vec![("cam.coordinate_system.edit".to_string(), p)])
        }
        None => Ok(vec![("cam.coordinate_system.create".to_string(), p)]),
    }
}

/// Draw the gnomon over the view and take this frame's drag and clicks; a click opens a mode or a
/// value entry, and a pick in the view snaps or aligns the frame as the mode says. While Origin
/// Snap or Axis Orbit is on the frame follows the pointer on hover, so the design's snap points are
/// handed to it each such frame.
pub fn overlay(app: &mut SolveApp, ui: &mut egui::Ui, painter: &Painter, proj: &Proj, d: &mut Dialog) {
    let Kind::Cam(Zcam::CoordinateSystem(f)) = &mut d.kind else { return };
    // The floating value entry's field owns Enter and Escape while it holds the keyboard: take
    // them before the gnomon or the dialog can read them as a commit or a cancel.
    let entry_keys = crate::dynamic_gnomon::take_incremental_keys(ui, entry_field_id(), f.entry.is_some());
    f.gnomon.set_incremental_input_active(entry_keys.active);
    let mut input = InputState::read(ui);
    if entry_keys.active {
        // Escape drops the entry's value, never the click mode the entry was opened by.
        input.escape = false;
    }
    // Where the value entry sat last frame owns its own presses: the gnomon must not end the
    // click mode or start a drag behind it.
    f.gnomon.set_entry_rect(f.entry_rect);
    // The pick is this view's click on geometry, never the picks of an older mode or a drag. It
    // comes before this frame's press, so the click that takes a snap does not first end the mode
    // it snaps in.
    if let Some(sel) = d.inputs.first_mut().and_then(|i| i.items.drain(..).next()) {
        if f.take_pick(app, &sel) {
            f.refresh = true;
        }
        // That click is the pick's: a live snap holds the frame's origin under the pointer, so the
        // gnomon must not also read the press as a click on its ball, which would re-open the
        // mode the pick just ended.
        input.primary_pressed = false;
        input.primary_down = false;
        input.primary_released = false;
    }
    // The geometry a live snap may take: read only while a click mode follows the pointer, so a
    // drag or an idle frame costs nothing.
    let tracking = matches!(f.gnomon.active_state(), GnomonActiveState::OriginSnap | GnomonActiveState::AxisOrbit(_));
    f.gnomon.set_snap_candidates(if tracking { snap_candidates(app) } else { Vec::new() });
    if f.gnomon.interact(proj, &input).is_some() {
        f.refresh = true;
    }
    // A click mode needs a frame for every pointer move, even when nothing else asks for one.
    f.gnomon.request_repaint(ui.ctx());
    // What a click on the ball, a cone, a shaft or a wheel asked for: a snap mode, an orbit mode,
    // or the floating value entry it sits at.
    f.follow_click();
    f.entry_rect = None;
    if let GnomonActiveState::RelativeInput { pos, .. } = f.gnomon.active_state() {
        entry_box(app, ui, f, pos, entry_keys);
    }
    f.gnomon.draw(painter, proj, &Tokens::get());
}

/// Whether the open coordinate system dialog's gnomon has a handle under `p`: a press there
/// drags the frame, not the camera, and picks nothing in the model.
pub(crate) fn gnomon_owns(app: &SolveApp, proj: &Proj, p: Pos2) -> bool {
    gnomon(app).is_some_and(|g| g.contains_pointer(proj, p))
}

/// Whether the open coordinate system dialog's gnomon owns the pointer, so the viewport must not
/// start its drag-selection marquee: a handle is hovered, the frame is dragged, or a click mode
/// waits for its pick.
pub(crate) fn gnomon_blocks_marquee(app: &SolveApp) -> bool {
    gnomon(app).is_some_and(|g| g.blocks_viewport_marquee())
}

/// The gnomon of the open coordinate system dialog, if that is the open dialog.
fn gnomon(app: &SolveApp) -> Option<&DynamicGnomon> {
    match &app.dialog.as_ref()?.kind {
        Kind::Cam(Zcam::CoordinateSystem(f)) => Some(&f.gnomon),
        _ => None,
    }
}

/// Move or turn `f`'s frame to what `sel` is: a point for its origin, a planar face's normal for
/// its Z or a straight edge's direction for its X, as the alignment mode says. True when the
/// frame changed.
pub(crate) fn align_to(app: &SolveApp, f: &mut DialogCamCoordinateSystem, sel: &Sel) -> bool {
    match f.gnomon.align_mode() {
        GnomonAlignMode::Free => false,
        GnomonAlignMode::SnapOrigin => snap_target(app, sel).is_some_and(|target| snap_origin(&mut f.gnomon, target)),
        GnomonAlignMode::AlignZFace => match sel {
            Sel::Face { body, index, .. } => planar_normal(app, body, *index).is_some_and(|n| f.gnomon.set_z_axis(n)),
            _ => false,
        },
        GnomonAlignMode::AlignXEdge => match sel {
            Sel::Edge { body, index, .. } => straight_edge_direction(app, body, *index).is_some_and(|x| f.gnomon.set_x_axis(x)),
            _ => false,
        },
    }
}

/// The outward normal of the planar face `index` of `body`, from the B-rep's own surface.
fn planar_normal(app: &SolveApp, body: &str, index: usize) -> Option<Vec3> {
    let st = crate::open_meshing::session_world(&app.session);
    match solidcraft_engine::kernel::face_surface(&st.body(body)?.body, index)? {
        FaceSurface::Plane { normal, .. } => Some(normal.vec()),
        _ => None,
    }
}

/// The direction of the straight edge `index` of `body`, from the B-rep's own line. Curved edges
/// have no single direction, so they align nothing.
fn straight_edge_direction(app: &SolveApp, body: &str, index: usize) -> Option<Vec3> {
    let st = crate::open_meshing::session_world(&app.session);
    let b = st.body(body)?;
    let solidcore_record::brep::Curve::Line { start, end } = &b.body.record().edges.get(index)?.curve else { return None };
    let dir = (Vec3::new(end[0], end[1], end[2]) - Vec3::new(start[0], start[1], start[2])).normalized()?;
    // The largest part points positive, as a joint's axis is: whichever way the edge was
    // tessellated must not flip the frame.
    let leading = [dir.x, dir.y, dir.z].into_iter().max_by(|a, b| a.abs().total_cmp(&b.abs()))?;
    Some(if leading < 0.0 { -dir } else { dir })
}

/// One entry of the tool library, as `cam.tool.list` reports it.
#[derive(Clone, Debug)]
struct CamTool {
    id: u64,
    name: String,
    kind: usize,
    diameter: f64,
    corner_radius: f64,
    flute_length: f64,
    flutes: u32,
}

/// The design's tools, from `cam.tool.list`; empty when the library is empty.
fn cam_tools(app: &SolveApp) -> Vec<CamTool> {
    let mut session = app.session.scratch();
    let Ok(reply) = session.execute("cam.tool.list", &json!({})) else { return Vec::new() };
    let Some(tools) = reply.get("tools").and_then(Value::as_array) else { return Vec::new() };
    tools
        .iter()
        .filter_map(|tool| {
            let kind = tool.get("kind").and_then(Value::as_str).and_then(|kind| TOOL_KINDS.iter().position(|k| *k == kind)).unwrap_or(0);
            Some(CamTool {
                id: tool.get("id")?.as_u64()?,
                name: tool.get("name")?.as_str()?.to_string(),
                kind,
                diameter: tool.get("diameter").and_then(Value::as_f64).unwrap_or(0.0),
                corner_radius: tool.get("corner_radius").and_then(Value::as_f64).unwrap_or(0.0),
                flute_length: tool.get("flute_length").and_then(Value::as_f64).unwrap_or(0.0),
                flutes: tool.get("flutes").and_then(Value::as_u64).unwrap_or(0) as u32,
            })
        })
        .collect()
}

/// The tool library's form: the tool being edited (0 is a new one) and its cutting values.
#[derive(Clone, Debug)]
pub struct DialogCamToolLibrary {
    /// Index into the tool choices: 0 makes a new tool, n edits the nth tool.
    pub selected: usize,
    /// The selection `selected` last filled the fields from, so typing is never overwritten.
    shown: usize,
    /// The tool being edited; none makes a new one.
    pub editing: Option<u64>,
    pub name: String,
    /// Index into [`TOOL_KINDS`].
    pub kind: usize,
    pub diameter: String,
    pub corner_radius: String,
    pub flute_length: String,
    pub flutes: String,
}

impl DialogCamToolLibrary {
    fn new(app: &SolveApp) -> Self {
        let tools = cam_tools(app);
        let mut form = DialogCamToolLibrary {
            selected: 0,
            shown: 0,
            editing: None,
            name: String::new(),
            kind: 0,
            diameter: crate::length_input::default(&app.session.doc, "6"),
            corner_radius: crate::length_input::default(&app.session.doc, "0"),
            flute_length: crate::length_input::default(&app.session.doc, "0"),
            flutes: "0".into(),
        };
        // A library with tools opens on the first one, ready to edit.
        if !tools.is_empty() {
            form.select(1, &tools, &app.session.doc);
        }
        form
    }

    /// Fill the fields from the selection's tool, or the defaults for a new one.
    fn select(&mut self, selected: usize, tools: &[CamTool], doc: &solidcraft_engine::doc::Document) {
        self.selected = selected;
        self.shown = selected;
        match selected.checked_sub(1).and_then(|i| tools.get(i)) {
            Some(tool) => {
                self.editing = Some(tool.id);
                self.name = tool.name.clone();
                self.kind = tool.kind;
                self.diameter = crate::length_input::shown(doc, tool.diameter);
                self.corner_radius = crate::length_input::shown(doc, tool.corner_radius);
                self.flute_length = crate::length_input::shown(doc, tool.flute_length);
                self.flutes = tool.flutes.to_string();
            }
            None => {
                self.editing = None;
                self.name.clear();
                self.kind = 0;
                self.diameter = crate::length_input::default(doc, "6");
                self.corner_radius = crate::length_input::default(doc, "0");
                self.flute_length = crate::length_input::default(doc, "0");
                self.flutes = "0".into();
            }
        }
    }
}

/// The cutting values a contour or pocket operation runs at, and the setup and tool it uses.
///
/// A field left empty is not sent: the command then takes its own default (a stepover of half
/// the tool's diameter, a total depth through the machined bodies, no stated rate).
#[derive(Clone, Debug)]
pub struct DialogCamToolpath {
    /// Empty: the command names it "Contour" or "Pocket" and the first free number.
    pub name: String,
    /// Index into the setup choices: 0 is the model origin, n is `doc.cam.setups[n - 1]`.
    pub setup: usize,
    /// Index into the design's tools, as `cam.tool.list` reports them.
    pub tool: usize,
    pub feed_rate: String,
    pub plunge_rate: String,
    pub spindle_speed: String,
    pub stepover: String,
    pub depth_per_pass: String,
    pub total_depth: String,
    pub stock_to_leave: String,
}

impl DialogCamToolpath {
    fn new(app: &SolveApp) -> Self {
        DialogCamToolpath {
            name: String::new(),
            // A design with setups cuts in its first one; without, in the model origin.
            setup: usize::from(!app.session.doc.cam.setups.is_empty()),
            tool: 0,
            feed_rate: String::new(),
            plunge_rate: String::new(),
            spindle_speed: String::new(),
            stepover: String::new(),
            depth_per_pass: String::new(),
            total_depth: String::new(),
            stock_to_leave: String::new(),
        }
    }
}

/// Contour: follow the part's outline with the side of the cutter (`cam.toolpath.contour`).
#[derive(Clone, Debug)]
pub struct DialogCamContour {
    pub form: DialogCamToolpath,
}

impl DialogCamContour {
    fn new(app: &SolveApp) -> Self {
        DialogCamContour { form: DialogCamToolpath::new(app) }
    }
}

impl std::ops::Deref for DialogCamContour {
    type Target = DialogCamToolpath;
    fn deref(&self) -> &Self::Target {
        &self.form
    }
}

impl std::ops::DerefMut for DialogCamContour {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.form
    }
}

/// 2D Contour: follow the picked edges with the side of the cutter (`cam.toolpath.contour2d`).
///
/// The edges are picked in the view; the engine reads their exact B-rep curves, offsets them by
/// the tool's radius to the open side and repeats the path down to `bottom_depth`.
#[derive(Clone, Debug)]
pub struct DialogCamContour2d {
    /// Empty: the command names it "2D Contour" and the first free number.
    pub name: String,
    /// Index into the setup choices: 0 is the model origin, n is `doc.cam.setups[n - 1]`.
    pub setup: usize,
    /// Index into the design's tools, as `cam.tool.list` reports them.
    pub tool: usize,
    /// How high the cutter travels between passes, above the stock, in mm. Empty is the
    /// command's own default of 0.
    pub clearance_height: String,
    /// How deep it cuts below the top of the stock, in mm.
    pub bottom_depth: String,
    /// How deep one pass cuts, in mm.
    pub stepdown: String,
    /// The toolpath the view draws: the parameters it was made from, and its points. The command
    /// is run on a scratch session only when the parameters change, so a preview never changes
    /// the design and a still dialog costs nothing.
    preview: Option<(String, Vec<[f64; 3]>)>,
}

impl DialogCamContour2d {
    fn new(app: &SolveApp) -> Self {
        DialogCamContour2d {
            name: String::new(),
            // A design with setups cuts in its first one; without, in the model origin.
            setup: usize::from(!app.session.doc.cam.setups.is_empty()),
            tool: 0,
            clearance_height: String::new(),
            bottom_depth: crate::length_input::default(&app.session.doc, "10"),
            stepdown: crate::length_input::default(&app.session.doc, "2"),
            preview: None,
        }
    }
}

/// 2D Pocket: clear the region of the picked faces (`cam.toolpath.pocket_2d`).
///
/// The faces are picked in the view; the engine reads their exact planar loops and clears each
/// region with concentric offset passes, one stepover apart, down to `depth`.
#[derive(Clone, Debug)]
pub struct DialogCamPocket2d {
    /// Index into the setup choices: 0 is the model origin, n is `doc.cam.setups[n - 1]`.
    pub setup: usize,
    /// Index into the design's tools, as `cam.tool.list` reports them.
    pub tool: usize,
    /// How far the cutter moves over between neighbouring passes, in mm.
    pub stepover: String,
    /// How deep it cuts below the top of the stock, in mm.
    pub depth: String,
}

impl DialogCamPocket2d {
    fn new(app: &SolveApp) -> Self {
        let doc = &app.session.doc;
        // Half the first tool's diameter is the usual stepover; a design without tools leaves
        // the field empty until one is created.
        let stepover = doc.cam.tools.first().map(|t| crate::length_input::default(doc, &format!("{}", t.diameter / 2.0))).unwrap_or_default();
        DialogCamPocket2d {
            // A design with setups cuts in its first one; without, in the model origin.
            setup: usize::from(!doc.cam.setups.is_empty()),
            tool: 0,
            stepover,
            depth: crate::length_input::default(doc, "10"),
        }
    }
}

/// The 2D pocket's parameters: the tool, the picked faces, the setup it cuts in, the stepover
/// between its passes and its depth. A picked point stands for the face it lies on; the engine
/// finds that face's exact boundary loops.
fn pocket2d_params(app: &SolveApp, f: &DialogCamPocket2d, inputs: &[SelInput]) -> Result<Value, String> {
    let s = &app.session;
    let tool = s.doc.cam.tools.get(f.tool).ok_or_else(|| "choose a tool (the design's Tool Library is empty)".to_string())?;
    let faces: Vec<Value> =
        inputs.first().map(|i| i.items.face_points()).unwrap_or_default().into_iter().map(crate::selection::point_value).collect();
    if faces.is_empty() {
        return Err("select the faces whose region the cutter clears".into());
    }
    let length = |label: &str, text: &str| -> Result<f64, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err(format!("enter {}", crate::i18n::tr_label(label)));
        }
        crate::length_input::eval(&s.doc, text, solidcraft_engine::doc::expr::Kind::Length)
            .map_err(|e| format!("{}: {e}", crate::i18n::tr_label(label)))
    };
    let mut p =
        json!({ "tool": tool.id, "boundary_faces": faces, "stepover": length("Stepover", &f.stepover)?, "depth": length("Total Depth", &f.depth)? });
    if let Some(setup) = f.setup.checked_sub(1).and_then(|i| s.doc.cam.setups.get(i)) {
        p["setup"] = json!(setup.id);
    }
    Ok(p)
}

/// The 2D contour's parameters: the tool, the picked edges, the setup it cuts in and its depths.
/// A picked point stands for the edge it lies on; the engine finds that edge's exact curve.
fn contour2d_params(app: &SolveApp, f: &DialogCamContour2d, inputs: &[SelInput]) -> Result<Value, String> {
    let s = &app.session;
    let tool = s.doc.cam.tools.get(f.tool).ok_or_else(|| "choose a tool (the design's Tool Library is empty)".to_string())?;
    let edges: Vec<Value> =
        inputs.first().map(|i| i.items.edge_points()).unwrap_or_default().into_iter().map(crate::selection::point_value).collect();
    if edges.is_empty() {
        return Err("select the edges the cutter follows".into());
    }
    let length = |label: &str, text: &str| -> Result<f64, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err(format!("enter {}", crate::i18n::tr_label(label)));
        }
        crate::length_input::eval(&s.doc, text, solidcraft_engine::doc::expr::Kind::Length)
            .map_err(|e| format!("{}: {e}", crate::i18n::tr_label(label)))
    };
    let bottom_depth = length("Bottom depth", &f.bottom_depth)?;
    let stepdown = length("Step down", &f.stepdown)?;
    let mut p = json!({ "tool": tool.id, "target_edges": edges, "bottom_depth": bottom_depth, "stepdown": stepdown });
    if let Some(setup) = f.setup.checked_sub(1).and_then(|i| s.doc.cam.setups.get(i)) {
        p["setup"] = json!(setup.id);
    }
    if !f.clearance_height.trim().is_empty() {
        p["clearance_height"] = json!(length("Clearance height", &f.clearance_height)?);
    }
    let name = f.name.trim();
    if !name.is_empty() {
        p["name"] = json!(name);
    }
    Ok(p)
}

/// The toolpath the open 2D contour's parameters make now, for the view to draw. It comes from a
/// scratch session, so nothing is added to the design; an incomplete form has no path.
pub fn contour2d_path(app: &SolveApp, f: &mut DialogCamContour2d, inputs: &[SelInput]) -> Vec<[f64; 3]> {
    let Ok(params) = contour2d_params(app, f, inputs) else { return Vec::new() };
    let key = params.to_string();
    if let Some((shown, points)) = &f.preview
        && *shown == key
    {
        return points.clone();
    }
    let points: Vec<[f64; 3]> = app
        .session
        .scratch()
        .execute("cam.toolpath.contour2d", &params)
        .ok()
        .and_then(|reply| reply.get("waypoints").and_then(Value::as_array).cloned())
        .map(|list| list.iter().filter_map(waypoint).collect())
        .unwrap_or_default();
    f.preview = Some((key, points.clone()));
    points
}

/// A waypoint's `[x, y, z]`, unless the reply holds a number the view cannot place.
fn waypoint(point: &Value) -> Option<[f64; 3]> {
    let list = point.as_array()?;
    let p = [list.first()?.as_f64()?, list.get(1)?.as_f64()?, list.get(2)?.as_f64()?];
    p.iter().all(|v| v.is_finite()).then_some(p)
}

/// Draw the open 2D contour's toolpath over the model: its cutting moves in the cut colour, and
/// the links between passes and between the edges it follows in the link colour. An incomplete
/// form, or one the command refuses, draws nothing.
pub fn contour2d_overlay(app: &mut SolveApp, painter: &Painter, proj: &Proj, d: &mut Dialog) {
    let Dialog { kind, inputs, .. } = d;
    let Kind::Cam(Zcam::Contour2d(f)) = kind else { return };
    let points = contour2d_path(app, f, inputs);
    let t = Tokens::get();
    let mut segments = 0;
    for pair in points.windows(2) {
        let at = |p: [f64; 3]| proj.to_screen(Vec3::new(p[0], p[1], p[2]));
        let (Some(from), Some(to)) = (at(pair[0]), at(pair[1])) else { continue };
        // A level change plunges: that move is a link, not a cut.
        let cutting = (pair[0][2] - pair[1][2]).abs() <= 1e-9;
        let (colour, width) = if cutting { (t.cam_path_cut, 2.0) } else { (t.cam_path_link, 1.0) };
        painter.line_segment([from, to], egui::Stroke::new(width, colour));
        segments += 1;
    }
    crate::scenario::publish_count("cam:contour2d:segments", segments as f64);
}

/// Start the CAM workspace dialog `id` opens; none for another command.
pub fn start_workspace(app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    match id {
        "cam.coordinate_system.create" => {
            let f = DialogCamCoordinateSystem::new(&app.session);
            Some((Kind::Cam(Zcam::CoordinateSystem(Box::new(f))), vec![SelInput::new("Alignment", FACES | EDGES | VERTICES, false)]))
        }
        "cam.setup.create" => Some((Kind::Cam(Zcam::Setup(DialogCamSetup::new(app))), vec![SelInput::new("Bodies", BODIES, true)])),
        "cam.tool.list" => Some((Kind::Cam(Zcam::ToolLibrary(DialogCamToolLibrary::new(app))), Vec::new())),
        "cam.toolpath.contour" => Some((Kind::Cam(Zcam::Contour(DialogCamContour::new(app))), Vec::new())),
        "cam.toolpath.contour2d" => Some((Kind::Cam(Zcam::Contour2d(DialogCamContour2d::new(app))), vec![SelInput::new("Edges", EDGES, true)])),
        "cam.toolpath.pocket" => Some((Kind::Cam(Zcam::Pocket(PocketDialog::new(app))), Vec::new())),
        "cam.toolpath.pocket_2d" => Some((Kind::Cam(Zcam::Pocket2d(DialogCamPocket2d::new(app))), vec![SelInput::new("Faces", FACES, true)])),
        "cam.toolpath.drill" => {
            let (f, inputs) = drill::start(app);
            Some((Kind::Cam(Zcam::Drill(f)), inputs))
        }
        "cam.toolpath.facing" => Some((Kind::Cam(Zcam::Facing(facing::DialogCamFacing::new(app))), Vec::new())),
        _ => None,
    }
}

/// The CAM workspace dialogs' rows; true when Enter was pressed in a value.
pub fn zcam_rows(app: &mut SolveApp, ui: &mut egui::Ui, form: &mut Zcam) -> bool {
    match form {
        Zcam::Setup(f) => setup_rows(app, ui, f),
        Zcam::CoordinateSystem(f) => coordinate_system_rows(app, ui, f),
        Zcam::ToolLibrary(f) => tool_rows(app, ui, f),
        Zcam::Contour(f) => toolpath_rows(app, ui, &mut f.form),
        Zcam::Contour2d(f) => contour2d_rows(app, ui, f),
        Zcam::Pocket(f) => crate::dialogs_cam_pocket::draw_pocket_dialog(app, ui, f),
        Zcam::Pocket2d(f) => pocket2d_rows(app, ui, f),
        Zcam::Drill(f) => drill::rows(app, ui, f),
        Zcam::Facing(f) => facing::draw_facing_dialog(app, ui, f),
    }
}

fn setup_rows(app: &mut SolveApp, ui: &mut egui::Ui, f: &mut DialogCamSetup) -> bool {
    let mut enter = false;
    enter |= crate::ui_kit::text_row(ui, "Name", &mut f.name, "").0;
    let names: Vec<&str> = app.session.doc.coordinate_systems.iter().map(|c| c.name.as_str()).collect();
    let mut choices = Vec::with_capacity(names.len() + 1);
    choices.push(crate::i18n::tr_label("Model origin"));
    choices.extend(names.iter().copied());
    f.coordinate_system = f.coordinate_system.min(names.len());
    crate::ui_kit::choice_row(ui, "Coordinate system", "cam_setup_cs", &mut f.coordinate_system, &choices);
    enter |= crate::ui_kit::expr_row(ui, "Stock X", &mut f.stock_x);
    enter |= crate::ui_kit::expr_row(ui, "Stock Y", &mut f.stock_y);
    enter |= crate::ui_kit::expr_row(ui, "Stock Z", &mut f.stock_z);
    enter |= crate::ui_kit::expr_row(ui, "Offset X", &mut f.offset_x);
    enter |= crate::ui_kit::expr_row(ui, "Offset Y", &mut f.offset_y);
    enter |= crate::ui_kit::expr_row(ui, "Offset Z", &mut f.offset_z);
    enter
}

fn tool_rows(app: &mut SolveApp, ui: &mut egui::Ui, f: &mut DialogCamToolLibrary) -> bool {
    let mut enter = false;
    let tools = cam_tools(app);
    let mut choices: Vec<&str> = Vec::with_capacity(tools.len() + 1);
    choices.push(crate::i18n::tr_label("New tool"));
    choices.extend(tools.iter().map(|t| t.name.as_str()));
    f.selected = f.selected.min(tools.len());
    if crate::ui_kit::choice_row(ui, "Tool", "cam_tool_pick", &mut f.selected, &choices).changed() {
        let doc = app.session.doc.clone();
        f.select(f.selected, &tools, &doc);
    }
    enter |= crate::ui_kit::text_row(ui, "Name", &mut f.name, "").0;
    crate::ui_kit::choice_row(ui, "Type", "cam_tool_kind", &mut f.kind, &TOOL_KIND_LABELS);
    enter |= crate::ui_kit::expr_row(ui, "Diameter", &mut f.diameter);
    enter |= crate::ui_kit::expr_row(ui, "Corner radius", &mut f.corner_radius);
    enter |= crate::ui_kit::expr_row(ui, "Flute length", &mut f.flute_length);
    enter |= crate::ui_kit::expr_row(ui, "Flutes", &mut f.flutes);
    enter
}

/// A contour or pocket form: its name, setup, tool and cutting values.
fn toolpath_rows(app: &mut SolveApp, ui: &mut egui::Ui, f: &mut DialogCamToolpath) -> bool {
    let tools = cam_tools(app);
    if tools.is_empty() {
        crate::ui_kit::status_line(ui, "Create a tool in the Tool Library first", crate::ui_kit::StatusTone::Dim);
        return false;
    }
    let mut enter = crate::ui_kit::text_row(ui, "Name", &mut f.name, "").0;
    // The model origin first, then each setup of the design.
    let mut setups: Vec<&str> = Vec::with_capacity(app.session.doc.cam.setups.len() + 1);
    setups.push(crate::i18n::tr_label("Model origin"));
    setups.extend(app.session.doc.cam.setups.iter().map(|s| s.name.as_str()));
    f.setup = f.setup.min(setups.len() - 1);
    crate::ui_kit::choice_row(ui, "Setup", "cam_toolpath_setup", &mut f.setup, &setups);
    let choices: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    f.tool = f.tool.min(choices.len() - 1);
    crate::ui_kit::choice_row(ui, crate::i18n::tr("ui.cam.select_tool"), "cam_toolpath_tool", &mut f.tool, &choices);
    enter |= crate::ui_kit::expr_row(ui, "Feed rate", &mut f.feed_rate);
    enter |= crate::ui_kit::expr_row(ui, "Plunge rate", &mut f.plunge_rate);
    enter |= crate::ui_kit::expr_row(ui, "Spindle speed", &mut f.spindle_speed);
    enter |= crate::ui_kit::expr_row(ui, crate::i18n::tr("ui.cam.stepover"), &mut f.stepover);
    enter |= crate::ui_kit::expr_row(ui, crate::i18n::tr("ui.cam.depth_per_pass"), &mut f.depth_per_pass);
    enter |= crate::ui_kit::expr_row(ui, "Total depth", &mut f.total_depth);
    enter |= crate::ui_kit::expr_row(ui, "Stock to leave", &mut f.stock_to_leave);
    enter
}

/// A 2D contour form: its name, the setup it cuts in, its tool and the depths it cuts to. The
/// edges it follows are picked in the view, in the form's edge input.
fn contour2d_rows(app: &mut SolveApp, ui: &mut egui::Ui, f: &mut DialogCamContour2d) -> bool {
    let tools = cam_tools(app);
    let mut enter = crate::ui_kit::text_row(ui, "Name", &mut f.name, "").0;
    // The model origin first, then each setup of the design.
    let mut setups: Vec<&str> = Vec::with_capacity(app.session.doc.cam.setups.len() + 1);
    setups.push(crate::i18n::tr_label("Model origin"));
    setups.extend(app.session.doc.cam.setups.iter().map(|s| s.name.as_str()));
    f.setup = f.setup.min(setups.len() - 1);
    crate::ui_kit::choice_row(ui, "Setup", "cam_contour2d_setup", &mut f.setup, &setups);
    if tools.is_empty() {
        crate::ui_kit::status_line(ui, "Create a tool in the Tool Library first", crate::ui_kit::StatusTone::Dim);
        return enter;
    }
    let choices: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    f.tool = f.tool.min(choices.len() - 1);
    crate::ui_kit::choice_row(ui, crate::i18n::tr("ui.cam.select_tool"), "cam_contour2d_tool", &mut f.tool, &choices);
    enter |= crate::ui_kit::expr_row(ui, "Clearance height", &mut f.clearance_height);
    enter |= crate::ui_kit::expr_row(ui, "Bottom depth", &mut f.bottom_depth);
    enter |= crate::ui_kit::expr_row(ui, "Step down", &mut f.stepdown);
    enter
}

/// A 2D pocket form: the setup it cuts in, its tool, the stepover between its passes and its
/// depth. The faces whose region it clears are picked in the view, in the form's face input.
fn pocket2d_rows(app: &mut SolveApp, ui: &mut egui::Ui, f: &mut DialogCamPocket2d) -> bool {
    let tools = cam_tools(app);
    // The model origin first, then each setup of the design.
    let mut setups: Vec<&str> = Vec::with_capacity(app.session.doc.cam.setups.len() + 1);
    setups.push(crate::i18n::tr_label("Model origin"));
    setups.extend(app.session.doc.cam.setups.iter().map(|s| s.name.as_str()));
    f.setup = f.setup.min(setups.len() - 1);
    crate::ui_kit::choice_row(ui, "Setup", "cam_pocket2d_setup", &mut f.setup, &setups);
    if tools.is_empty() {
        crate::ui_kit::status_line(ui, "Create a tool in the Tool Library first", crate::ui_kit::StatusTone::Dim);
        return false;
    }
    let choices: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    f.tool = f.tool.min(choices.len() - 1);
    crate::ui_kit::choice_row(ui, crate::i18n::tr("ui.cam.select_tool"), "cam_pocket2d_tool", &mut f.tool, &choices);
    let mut enter = crate::ui_kit::expr_row(ui, crate::i18n::tr("dialog.cam.pocket.stepover"), &mut f.stepover);
    enter |= crate::ui_kit::expr_row(ui, crate::i18n::tr("dialog.cam.pocket.depth"), &mut f.depth);
    enter
}

/// The commands OK runs for a CAM workspace dialog.
pub fn zcam_commands(app: &SolveApp, form: &Zcam, inputs: &[SelInput]) -> Result<Vec<(String, Value)>, String> {
    match form {
        Zcam::Setup(f) => setup_commands(app, f, inputs),
        Zcam::CoordinateSystem(f) => coordinate_system_commands(f),
        Zcam::ToolLibrary(f) => tool_commands(app, f),
        Zcam::Contour(f) => toolpath_commands(app, &f.form, "cam.toolpath.contour"),
        Zcam::Contour2d(f) => contour2d_params(app, f, inputs).map(|p| vec![("cam.toolpath.contour2d".to_string(), p)]),
        Zcam::Pocket(f) => crate::dialogs_cam_pocket::params(app, f).map(|p| vec![("cam.toolpath.pocket".to_string(), p)]),
        Zcam::Pocket2d(f) => pocket2d_params(app, f, inputs).map(|p| vec![("cam.toolpath.pocket_2d".to_string(), p)]),
        Zcam::Drill(f) => drill::params(app, f, inputs).map(|p| vec![("cam.toolpath.drill".to_string(), p)]),
        Zcam::Facing(f) => facing::params(app, f).map(|p| vec![("cam.toolpath.facing".to_string(), p)]),
    }
}

fn setup_commands(app: &SolveApp, f: &DialogCamSetup, inputs: &[SelInput]) -> Result<Vec<(String, Value)>, String> {
    let s = &app.session;
    let length = |label: &str, text: &str| setup_length(s, label, text);
    let mut p = json!({
        "stock_size": [length("Stock X", &f.stock_x)?, length("Stock Y", &f.stock_y)?, length("Stock Z", &f.stock_z)?],
        "stock_offset": [length("Offset X", &f.offset_x)?, length("Offset Y", &f.offset_y)?, length("Offset Z", &f.offset_z)?],
    });
    if let Some(cs) = f.coordinate_system.checked_sub(1).and_then(|i| s.doc.coordinate_systems.get(i)) {
        p["coordinate_system"] = json!(cs.id);
    }
    let name = f.name.trim();
    if !name.is_empty() {
        p["name"] = json!(name);
    }
    let bodies: Vec<String> = inputs.first().map(|i| i.items.body_names()).unwrap_or_default();
    if !bodies.is_empty() {
        p["bodies"] = json!(bodies);
    }
    Ok(vec![("cam.setup.create".to_string(), p)])
}

/// A setup's length field: an empty field is 0, the command's "read the bodies" default.
fn setup_length(s: &Session, label: &str, text: &str) -> Result<f64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(0.0);
    }
    crate::length_input::eval(&s.doc, text, solidcraft_engine::doc::expr::Kind::Length).map_err(|e| format!("{}: {e}", crate::i18n::tr_label(label)))
}

fn tool_commands(app: &SolveApp, f: &DialogCamToolLibrary) -> Result<Vec<(String, Value)>, String> {
    let s = &app.session;
    if f.diameter.trim().is_empty() {
        return Err("enter the tool's diameter".into());
    }
    let diameter = crate::length_input::eval(&s.doc, &f.diameter, solidcraft_engine::doc::expr::Kind::Length)
        .map_err(|e| format!("{}: {e}", crate::i18n::tr_label("Diameter")))?;
    let mut p = json!({
        "kind": TOOL_KINDS.get(f.kind).copied().unwrap_or("flat_end_mill"),
        "diameter": diameter,
    });
    let optional = |key: &str, text: &str| -> Result<Option<f64>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        crate::length_input::eval(&s.doc, text, solidcraft_engine::doc::expr::Kind::Length)
            .map(Some)
            .map_err(|e| format!("{}: {e}", crate::i18n::tr_label(key)))
    };
    if let Some(radius) = optional("Corner radius", &f.corner_radius)? {
        p["corner_radius"] = json!(radius);
    }
    if let Some(length) = optional("Flute length", &f.flute_length)? {
        p["flute_length"] = json!(length);
    }
    let flutes = f.flutes.trim();
    if !flutes.is_empty() {
        let count: u32 = flutes.parse().map_err(|_| format!("{}: enter a whole number", crate::i18n::tr_label("Flutes")))?;
        p["flutes"] = json!(count);
    }
    let name = f.name.trim();
    if !name.is_empty() {
        p["name"] = json!(name);
    }
    let command = match f.editing {
        Some(tool) => {
            p["tool"] = json!(tool);
            "cam.tool.edit"
        }
        None => "cam.tool.create",
    };
    Ok(vec![(command.to_string(), p)])
}

/// A contour or pocket form's command: the setup and tool it names, and every cutting value it
/// states. An empty field is left out, so the command takes its own default.
fn toolpath_commands(app: &SolveApp, f: &DialogCamToolpath, command: &str) -> Result<Vec<(String, Value)>, String> {
    let s = &app.session;
    let tool = s.doc.cam.tools.get(f.tool).ok_or_else(|| "choose a tool (the design's Tool Library is empty)".to_string())?;
    let mut p = json!({ "tool": tool.id });
    if let Some(setup) = f.setup.checked_sub(1).and_then(|i| s.doc.cam.setups.get(i)) {
        p["setup"] = json!(setup.id);
    }
    let name = f.name.trim();
    if !name.is_empty() {
        p["name"] = json!(name);
    }
    let optional = |label: &str, text: &str, kind: solidcraft_engine::doc::expr::Kind| -> Result<Option<f64>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        crate::length_input::eval(&s.doc, text, kind).map(Some).map_err(|e| format!("{}: {e}", crate::i18n::tr_label(label)))
    };
    use solidcraft_engine::doc::expr::Kind;
    for (key, label, text, kind) in [
        ("feed_rate", "Feed rate", f.feed_rate.as_str(), Kind::Unitless),
        ("plunge_rate", "Plunge rate", f.plunge_rate.as_str(), Kind::Unitless),
        ("spindle_speed", "Spindle speed", f.spindle_speed.as_str(), Kind::Unitless),
        ("stepover", "Stepover", f.stepover.as_str(), Kind::Length),
        ("depth_per_pass", "Depth per pass", f.depth_per_pass.as_str(), Kind::Length),
        ("total_depth", "Total depth", f.total_depth.as_str(), Kind::Length),
        ("stock_to_leave", "Stock to leave", f.stock_to_leave.as_str(), Kind::Length),
    ] {
        if let Some(value) = optional(label, text, kind)? {
            p[key] = json!(value);
        }
    }
    Ok(vec![(command.to_string(), p)])
}

/// Start the CAM workspace's G-code export (`cam.export.gcode`) from the ribbon: ask where to
/// save the program, then write it. With no toolpath there is nothing to name, so the command
/// reports its own error without opening a picker.
pub fn start_export(app: &mut SolveApp, id: &str) -> bool {
    if id != "cam.export.gcode" {
        return false;
    }
    if app.session.doc.cam.toolpaths.is_empty() {
        if let Err(why) = app.run(id, json!({})) {
            app.set_status(why, true);
        }
    } else {
        export_gcode(app, None);
    }
    true
}

/// Ask where to save the G-code program and write it there. `toolpath` limits the program to
/// that one operation; none exports every toolpath of the design.
pub fn export_gcode(app: &mut SolveApp, toolpath: Option<u64>) {
    let design = app.session.doc.name.trim();
    let name = if design.is_empty() { "program.nc".to_string() } else { format!("{design}.nc") };
    app.pick_save(&name, &["nc", "gcode", "tap"], move |app, path| {
        let Some(path) = path else { return };
        let params = match toolpath {
            Some(id) => json!({ "path": path, "toolpaths": [id] }),
            None => json!({ "path": path }),
        };
        match app.run("cam.export.gcode", params) {
            Ok(_) => app.set_status(format!("Exported {path}"), false),
            Err(why) => app.set_status(why, true),
        }
    });
}

#[cfg(test)]
#[path = "dialogs_cam_tests.rs"]
mod tests;

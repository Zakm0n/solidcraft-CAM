//! The Cam dialog (`joint.cam`): a cam turning on a revolute joint drives a follower, a slider or
//! a revolute, along the cam's profile (a closed sketch profile, or edges of the cam's body). The
//! follower is a knife edge, a roller (its radius typed, or its round face picked, which is also
//! where it touches) or a flat face. Picking the profile on a part chooses the joint that part
//! turns on. Play turns the cam a full turn so the follower shows its motion before OK.

use crate::ui_kit::BackingTooltipExt as _;
use egui::{Painter, Pos2, RichText};
use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_engine::Sel;
use solidcraft_engine::Session;
use solidcraft_engine::doc::joints::JointKind;
use solidcraft_engine::geom::prelude::*;
use solidcraft_engine::kernel::FaceSurface;

use super::{ROUND, joint_row, occurrence, pick, picked_radius, trim, turn};
use crate::SolveApp;
use crate::dialogs::Kind;
use crate::dialogs::{Dialog, combo, field, form_row};
use crate::dialogs_assembly::Asm;
use crate::dynamic_gnomon::{DynamicGnomon, GnomonAlignMode, InputState};
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
    /// (`cam.coordinate_system.create`).
    CoordinateSystem(DialogCamCoordinateSystem),
    /// The design's tool library: pick a tool to edit, or fill in a new one (`cam.tool.list`).
    ToolLibrary(DialogCamToolLibrary),
    /// Follow a part's outline with the side of the cutter (`cam.toolpath.contour`).
    Contour(DialogCamContour),
    /// Clear the material inside a part's outline (`cam.toolpath.pocket`).
    Pocket(DialogCamPocket),
}

impl Zcam {
    /// The command OK runs.
    pub fn command(&self) -> &'static str {
        match self {
            Zcam::Setup(_) => "cam.setup.create",
            Zcam::CoordinateSystem(_) => "cam.coordinate_system.create",
            Zcam::ToolLibrary(_) => "cam.tool.list",
            Zcam::Contour(_) => "cam.toolpath.contour",
            Zcam::Pocket(_) => "cam.toolpath.pocket",
        }
    }

    /// The dialog's heading.
    pub fn title(&self) -> &'static str {
        match self {
            Zcam::Setup(_) => "SETUP",
            Zcam::CoordinateSystem(_) => "COORDINATE SYSTEM",
            Zcam::ToolLibrary(_) => "TOOL LIBRARY",
            Zcam::Contour(_) => "CONTOUR",
            Zcam::Pocket(_) => "POCKET",
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

/// The alignment modes' labels, in [`GnomonAlignMode::ALL`] order.
const ALIGN_LABELS: [&str; 4] = ["label.align_free", "label.snap_origin", "label.align_z_to_face", "label.align_x_edge"];

/// A machining origin being placed (`cam.coordinate_system.create`): a dynamic gnomon the user
/// drags and aligns to B-rep geometry in the view, and its origin typed exactly.
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
        }
    }

    /// Write the frame's origin into the fields, where it moved in the view.
    fn sync_fields(&mut self, doc: &solidcraft_engine::doc::Document) {
        let o = self.gnomon.origin();
        self.origin_x = crate::length_input::shown(doc, o.x);
        self.origin_y = crate::length_input::shown(doc, o.y);
        self.origin_z = crate::length_input::shown(doc, o.z);
    }
}

/// The dialog's rows: its name, the alignment mode, and the origin's three values.
fn coordinate_system_rows(app: &mut SolveApp, ui: &mut egui::Ui, f: &mut DialogCamCoordinateSystem) -> bool {
    let mut enter = crate::ui_kit::text_row(ui, "label.name", &mut f.name, "").0;
    let labels: Vec<&str> = ALIGN_LABELS.iter().map(|l| crate::i18n::tr_label(l)).collect();
    let mut mode = GnomonAlignMode::ALL.iter().position(|m| *m == f.gnomon.align_mode()).unwrap_or(0);
    if crate::ui_kit::choice_row(ui, "label.alignment", "cam_cs_align", &mut mode, &labels).changed()
        && let Some(m) = GnomonAlignMode::ALL.get(mode)
    {
        f.gnomon.set_align_mode(*m);
    }
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
    let readout = ["X", "Y", "Z"]
        .into_iter()
        .zip(axes)
        .map(|(name, v)| format!("{name} ({:.2}, {:.2}, {:.2})", v.x, v.y, v.z))
        .collect::<Vec<_>>()
        .join("   ");
    crate::ui_kit::status_line(ui, &readout, crate::ui_kit::StatusTone::Dim);
    enter
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
    Ok(vec![("cam.coordinate_system.create".to_string(), p)])
}

/// Draw the gnomon over the view and take this frame's drag; a pick in the view aligns the frame
/// to what was picked, as the mode says.
pub fn overlay(app: &mut SolveApp, ui: &mut egui::Ui, painter: &Painter, proj: &Proj, d: &mut Dialog) {
    let Kind::Cam(Zcam::CoordinateSystem(f)) = &mut d.kind else { return };
    let input = InputState::read(ui);
    if f.gnomon.interact(proj, &input).is_some() {
        f.refresh = true;
    }
    // The pick is this view's click on geometry, never the picks of an older mode or a drag.
    if let Some(sel) = d.inputs.first_mut().and_then(|i| i.items.drain(..).next())
        && align_to(app, f, &sel)
    {
        f.refresh = true;
    }
    f.gnomon.draw(painter, proj, &Tokens::get());
}

/// Whether the open coordinate system dialog's gnomon has a handle under `p`: a press there
/// drags the frame, not the camera, and picks nothing in the model.
pub(crate) fn gnomon_owns(app: &SolveApp, proj: &Proj, p: Pos2) -> bool {
    let gnomon = app.dialog.as_ref().and_then(|d| match &d.kind {
        Kind::Cam(Zcam::CoordinateSystem(f)) => Some(&f.gnomon),
        _ => None,
    });
    gnomon.is_some_and(|g| g.contains_pointer(proj, p))
}

/// Move or turn `f`'s frame to what `sel` is: a point for its origin, a planar face's normal for
/// its Z or a straight edge's direction for its X, as the alignment mode says. True when the
/// frame changed.
pub(crate) fn align_to(app: &SolveApp, f: &mut DialogCamCoordinateSystem, sel: &Sel) -> bool {
    match f.gnomon.align_mode() {
        GnomonAlignMode::Free => false,
        GnomonAlignMode::SnapOrigin => match sel {
            Sel::Face { point, .. } | Sel::Edge { point, .. } | Sel::Vertex { point, .. } => f.gnomon.set_origin(*point),
            _ => false,
        },
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

/// Pocket: clear the material inside the part's outline (`cam.toolpath.pocket`).
#[derive(Clone, Debug)]
pub struct DialogCamPocket {
    pub form: DialogCamToolpath,
}

impl DialogCamPocket {
    fn new(app: &SolveApp) -> Self {
        DialogCamPocket { form: DialogCamToolpath::new(app) }
    }
}

impl std::ops::Deref for DialogCamPocket {
    type Target = DialogCamToolpath;
    fn deref(&self) -> &Self::Target {
        &self.form
    }
}

impl std::ops::DerefMut for DialogCamPocket {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.form
    }
}

/// Start the CAM workspace dialog `id` opens; none for another command.
pub fn start_workspace(app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    match id {
        "cam.coordinate_system.create" => {
            let f = DialogCamCoordinateSystem::new(&app.session);
            Some((Kind::Cam(Zcam::CoordinateSystem(f)), vec![SelInput::new("Alignment", FACES | EDGES | VERTICES, false)]))
        }
        "cam.setup.create" => Some((Kind::Cam(Zcam::Setup(DialogCamSetup::new(app))), vec![SelInput::new("Bodies", BODIES, true)])),
        "cam.tool.list" => Some((Kind::Cam(Zcam::ToolLibrary(DialogCamToolLibrary::new(app))), Vec::new())),
        "cam.toolpath.contour" => Some((Kind::Cam(Zcam::Contour(DialogCamContour::new(app))), Vec::new())),
        "cam.toolpath.pocket" => Some((Kind::Cam(Zcam::Pocket(DialogCamPocket::new(app))), Vec::new())),
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
        Zcam::Pocket(f) => toolpath_rows(app, ui, &mut f.form),
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

/// The commands OK runs for a CAM workspace dialog.
pub fn zcam_commands(app: &SolveApp, form: &Zcam, inputs: &[SelInput]) -> Result<Vec<(String, Value)>, String> {
    match form {
        Zcam::Setup(f) => setup_commands(app, f, inputs),
        Zcam::CoordinateSystem(f) => coordinate_system_commands(f),
        Zcam::ToolLibrary(f) => tool_commands(app, f),
        Zcam::Contour(f) => toolpath_commands(app, &f.form, "cam.toolpath.contour"),
        Zcam::Pocket(f) => toolpath_commands(app, &f.form, "cam.toolpath.pocket"),
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
    crate::length_input::eval(&s.doc, text, solidcraft_engine::doc::expr::Kind::Length)
        .map_err(|e| format!("{}: {e}", crate::i18n::tr_label(label)))
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

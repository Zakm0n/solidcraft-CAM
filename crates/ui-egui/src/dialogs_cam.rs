//! The Cam dialog (`joint.cam`): a cam turning on a revolute joint drives a follower, a slider or
//! a revolute, along the cam's profile (a closed sketch profile, or edges of the cam's body). The
//! follower is a knife edge, a roller (its radius typed, or its round face picked, which is also
//! where it touches) or a flat face. Picking the profile on a part chooses the joint that part
//! turns on. Play turns the cam a full turn so the follower shows its motion before OK.

use crate::ui_kit::BackingTooltipExt as _;
use egui::RichText;
use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::Session;
use solidcraft_engine::doc::joints::JointKind;

use super::{ROUND, joint_row, occurrence, pick, picked_radius, trim, turn};
use crate::SolveApp;
use crate::dialogs::Kind;
use crate::dialogs::{combo, field, form_row};
use crate::dialogs_assembly::Asm;
use crate::selection::{BODIES, EDGES, FACES, PROFILES, SelInput, SelectionExt as _, VERTICES};
use crate::theme::Tokens;

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
            Zcam::ToolLibrary(_) => "cam.tool.list",
            Zcam::Contour(_) => "cam.toolpath.contour",
            Zcam::Pocket(_) => "cam.toolpath.pocket",
        }
    }

    /// The dialog's heading.
    pub fn title(&self) -> &'static str {
        match self {
            Zcam::Setup(_) => "SETUP",
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

#[cfg(test)]
#[path = "dialogs_cam_tests.rs"]
mod tests;

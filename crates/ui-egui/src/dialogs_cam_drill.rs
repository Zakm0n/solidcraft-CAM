//! CAM ▸ TOOLPATH: the drilling operation's form (`cam.toolpath.drill`).
//!
//! The holes are picked in the view: each pick is the top rim of a hole, and the engine reads
//! that rim's exact B-rep curve for the hole's center, axis and depth. The form names the setup
//! and tool the operation runs with, its cycle, where it takes its depth from and how high the
//! cutter travels between holes; OK runs `cam.toolpath.drill` and adds the operation to the
//! design.
//!
//! The form is drawn in the shared `ui_kit` dialog window (the Toolpath tab's host), with the
//! kit's rows and theme tokens, so it follows the rest of the CAM workspace.

use serde_json::{Value, json};
use solidcraft_engine::Sel;

use super::cam_tools;
use crate::SolveApp;
use crate::selection::{EDGES, SelInput};

/// The cycles, as `cam.toolpath.drill` takes them, and their labels.
const CYCLES: [&str; 3] = ["standard", "peck", "deep_hole"];
const CYCLE_LABELS: [&str; 3] = ["Standard", "Peck", "Deep hole"];

/// The depth references, as the command takes them, and their labels.
const REFERENCES: [&str; 3] = ["hole_bottom", "model_bottom", "absolute"];
const REFERENCE_LABELS: [&str; 3] = ["Hole bottom", "Model bottom", "Absolute"];

/// The index into [`CYCLES`] and [`REFERENCES`] of `hole_bottom` and `standard`.
const STANDARD: usize = 0;
const HOLE_BOTTOM: usize = 0;
const ABSOLUTE: usize = 2;

/// A drilling operation's form: its name, setup, tool, cycle, depth reference and cutting values.
#[derive(Clone, Debug)]
pub struct DialogCamDrill {
    /// Empty: the command names it "Drilling" and the first free number.
    pub name: String,
    /// Index into the setup choices: 0 is the model origin, n is `doc.cam.setups[n - 1]`.
    pub setup: usize,
    /// Index into the design's tools, as `cam.tool.list` reports them.
    pub tool: usize,
    /// Index into [`CYCLES`].
    pub cycle_type: usize,
    /// Index into [`REFERENCES`].
    pub depth_reference: usize,
    /// How high the cutter travels between holes, above the top of the stock, in mm. Empty is
    /// the command's own default of 0.
    pub clearance_height: String,
    /// How deep one peck cuts, in mm. Empty takes the tool's flute length.
    pub peck_depth: String,
    /// How deep it cuts below the top of the stock, in mm, when the reference is `Absolute`.
    pub depth: String,
}

impl DialogCamDrill {
    pub(super) fn new(app: &SolveApp) -> Self {
        DialogCamDrill {
            name: String::new(),
            // A design with setups cuts in its first one; without, in the model origin.
            setup: usize::from(!app.session.doc.cam.setups.is_empty()),
            tool: 0,
            cycle_type: STANDARD,
            depth_reference: HOLE_BOTTOM,
            clearance_height: String::new(),
            peck_depth: String::new(),
            depth: String::new(),
        }
    }
}

/// The form's rows; true when Enter was pressed in a value.
pub(super) fn rows(app: &mut SolveApp, ui: &mut egui::Ui, f: &mut DialogCamDrill) -> bool {
    let tools = cam_tools(app);
    let mut enter = crate::ui_kit::text_row(ui, "Name", &mut f.name, "").0;
    // The model origin first, then each setup of the design.
    let mut setups: Vec<&str> = Vec::with_capacity(app.session.doc.cam.setups.len() + 1);
    setups.push(crate::i18n::tr_label("Model origin"));
    setups.extend(app.session.doc.cam.setups.iter().map(|s| s.name.as_str()));
    f.setup = f.setup.min(setups.len() - 1);
    crate::ui_kit::choice_row(ui, "Setup", "cam_drill_setup", &mut f.setup, &setups);
    if tools.is_empty() {
        crate::ui_kit::status_line(ui, "Create a tool in the Tool Library first", crate::ui_kit::StatusTone::Dim);
        return enter;
    }
    let choices: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    f.tool = f.tool.min(choices.len() - 1);
    crate::ui_kit::choice_row(ui, crate::i18n::tr("ui.cam.select_tool"), "cam_drill_tool", &mut f.tool, &choices);
    f.cycle_type = f.cycle_type.min(CYCLES.len() - 1);
    crate::ui_kit::choice_row(ui, crate::i18n::tr("dialog.cam.drill.cycle_type"), "cam_drill_cycle", &mut f.cycle_type, &CYCLE_LABELS);
    enter |= crate::ui_kit::expr_row(ui, "Clearance height", &mut f.clearance_height);
    if f.cycle_type != STANDARD {
        enter |= crate::ui_kit::expr_row(ui, crate::i18n::tr("dialog.cam.drill.peck_depth"), &mut f.peck_depth);
        crate::ui_kit::status_line(ui, "Empty takes the tool's flute length", crate::ui_kit::StatusTone::Dim);
    }
    f.depth_reference = f.depth_reference.min(REFERENCES.len() - 1);
    crate::ui_kit::choice_row(ui, "Depth reference", "cam_drill_reference", &mut f.depth_reference, &REFERENCE_LABELS);
    if f.depth_reference == ABSOLUTE {
        enter |= crate::ui_kit::expr_row(ui, "Depth", &mut f.depth);
        if f.depth.trim().is_empty() {
            crate::ui_kit::status_line(ui, "Absolute needs a depth below the top of the stock", crate::ui_kit::StatusTone::Warning);
        }
    }
    enter
}

/// The persistent names of the rims the picks in `inputs` name, in the order they were picked:
/// each pick is a point on an exact edge, and the edge's name is what `cam.toolpath.drill` reads
/// for the hole's exact B-rep curve. A pick on no named edge is left out; the command refuses an
/// empty list with its own message.
fn hole_centers(app: &SolveApp, inputs: &[SelInput]) -> Vec<String> {
    let Ok(state) = app.session.world_state() else { return Vec::new() };
    let mut names: Vec<String> = Vec::new();
    for sel in inputs.first().map(|i| i.items.as_slice()).unwrap_or_default() {
        let Sel::Edge { body, point, .. } = sel else { continue };
        let Some(b) = state.body(body) else { continue };
        let Some(name) = solidcraft_engine::doc::naming::edge_names_at(b, &[*point]).into_iter().next().filter(|n| !n.is_empty()) else {
            continue;
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// The command's parameters: the tool, the picked hole rims, the setup, cycle, depth reference
/// and the values the form states. An empty optional field is left out, so the command takes
/// its own default.
pub(super) fn params(app: &SolveApp, f: &DialogCamDrill, inputs: &[SelInput]) -> Result<Value, String> {
    let s = &app.session;
    let tool = s.doc.cam.tools.get(f.tool).ok_or_else(|| "choose a tool (the design's Tool Library is empty)".to_string())?;
    let centers = hole_centers(app, inputs);
    if centers.is_empty() {
        return Err("select the holes' top rims (their circular edges) in the view".into());
    }
    let length = |label: &str, text: &str| -> Result<f64, String> {
        crate::length_input::eval(&s.doc, text.trim(), solidcraft_engine::doc::expr::Kind::Length)
            .map_err(|e| format!("{}: {e}", crate::i18n::tr_label(label)))
    };
    let mut p = json!({
        "tool": tool.id,
        "hole_centers": centers,
        "cycle_type": CYCLES.get(f.cycle_type).copied().unwrap_or("standard"),
        "depth_reference": REFERENCES.get(f.depth_reference).copied().unwrap_or("hole_bottom"),
    });
    if let Some(setup) = f.setup.checked_sub(1).and_then(|i| s.doc.cam.setups.get(i)) {
        p["setup"] = json!(setup.id);
    }
    if !f.clearance_height.trim().is_empty() {
        p["clearance_height"] = json!(length("Clearance height", &f.clearance_height)?);
    }
    if f.cycle_type != STANDARD && !f.peck_depth.trim().is_empty() {
        p["peck_depth"] = json!(length("Peck Depth", &f.peck_depth)?);
    }
    if f.depth_reference == ABSOLUTE {
        if f.depth.trim().is_empty() {
            return Err("enter the depth the operation cuts to".into());
        }
        p["depth"] = json!(length("Depth", &f.depth)?);
    }
    let name = f.name.trim();
    if !name.is_empty() {
        p["name"] = json!(name);
    }
    Ok(p)
}

/// The command that opens the drilling form, and the edges it picks.
pub(super) fn start(app: &SolveApp) -> (DialogCamDrill, Vec<SelInput>) {
    (DialogCamDrill::new(app), vec![SelInput::new("Holes", EDGES, true)])
}

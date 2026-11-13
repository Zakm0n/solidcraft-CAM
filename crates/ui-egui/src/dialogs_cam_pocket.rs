//! CAM ▸ TOOLPATH: the pocket operation's form (`cam.toolpath.pocket`).
//!
//! A pocket clears the material inside the machined bodies' outlines, pass beside pass, down to
//! a bottom height. The form names the operation, the setup whose machine zero, axes and stock
//! it cuts in, the tool whose diameter spaces the passes, and the stepover, pass depth, bottom
//! height, stock to leave and rates it cuts at; OK runs `cam.toolpath.pocket` and adds the
//! operation to the design.
//!
//! The values are expressions: `expr_row` accepts a length or hostile arithmetic such as
//! `"10mm + 2"` and the engine evaluates it against the design's parameters. The form is drawn
//! in the shared `ui_kit` dialog window (the Toolpath tab's host), so it uses the kit's rows and
//! theme tokens like the rest of the CAM workspace, and never a raw `egui::Window` or
//! `egui::Frame` of its own.

use serde_json::{Value, json};
use solidcraft_engine::doc::expr::Kind;

use crate::SolveApp;

/// A pocket operation's form: its name, setup, tool and cutting values.
#[derive(Clone, Debug)]
pub struct PocketDialog {
    /// Empty: the command names it "Pocket" and the first free number.
    pub name: String,
    /// Index into the setup choices: 0 is the model origin, n is `doc.cam.setups[n - 1]`.
    pub setup: usize,
    /// Index into the design's tools, as `cam.tool.list` reports them.
    pub tool: usize,
    /// How far the cutter moves over between neighbouring passes, in mm. Empty takes the
    /// command's own default of half the tool's diameter.
    pub stepover: String,
    /// How deep one pass cuts, in mm. Empty takes the command's default (the whole total depth,
    /// one pass).
    pub depth_per_pass: String,
    /// How deep the operation cuts below the top of the stock, in mm: the bottom height. Empty
    /// takes the command's default (through the machined bodies).
    pub total_depth: String,
    /// How much material the operation leaves for a finishing pass, in mm. Empty is the
    /// command's default of 0 (none).
    pub stock_to_leave: String,
    /// How fast the cutter moves through material, in mm per minute. Empty is not stated.
    pub feed_rate: String,
    /// How fast the cutter enters the material, in mm per minute. Empty is not stated.
    pub plunge_rate: String,
    /// How fast the spindle turns, in revolutions per minute. Empty is not stated.
    pub spindle_speed: String,
}

impl PocketDialog {
    pub(crate) fn new(app: &SolveApp) -> Self {
        PocketDialog {
            name: String::new(),
            // A design with setups cuts in its first one; without, in the model origin.
            setup: usize::from(!app.session.doc.cam.setups.is_empty()),
            tool: 0,
            stepover: String::new(),
            depth_per_pass: String::new(),
            total_depth: String::new(),
            stock_to_leave: String::new(),
            feed_rate: String::new(),
            plunge_rate: String::new(),
            spindle_speed: String::new(),
        }
    }
}

/// The form's rows; true when Enter was pressed in a value.
pub(crate) fn draw_pocket_dialog(app: &mut SolveApp, ui: &mut egui::Ui, f: &mut PocketDialog) -> bool {
    let mut enter = crate::ui_kit::text_row(ui, "Name", &mut f.name, "").0;
    // The model origin first, then each setup of the design.
    let mut setups: Vec<&str> = Vec::with_capacity(app.session.doc.cam.setups.len() + 1);
    setups.push(crate::i18n::tr_label("Model origin"));
    setups.extend(app.session.doc.cam.setups.iter().map(|s| s.name.as_str()));
    f.setup = f.setup.min(setups.len() - 1);
    crate::ui_kit::choice_row(ui, "Setup", "cam_pocket_setup", &mut f.setup, &setups);
    if app.session.doc.cam.tools.is_empty() {
        crate::ui_kit::status_line(ui, "Create a tool in the Tool Library first", crate::ui_kit::StatusTone::Dim);
        return enter;
    }
    let choices: Vec<&str> = app.session.doc.cam.tools.iter().map(|t| t.name.as_str()).collect();
    f.tool = f.tool.min(choices.len() - 1);
    crate::ui_kit::choice_row(ui, crate::i18n::tr("ui.cam.select_tool"), "cam_pocket_tool", &mut f.tool, &choices);
    enter |= crate::ui_kit::expr_row(ui, crate::i18n::tr("dialog.cam.pocket.stepover"), &mut f.stepover);
    enter |= crate::ui_kit::expr_row(ui, crate::i18n::tr("ui.cam.depth_per_pass"), &mut f.depth_per_pass);
    enter |= crate::ui_kit::expr_row(ui, crate::i18n::tr("dialog.cam.pocket.depth"), &mut f.total_depth);
    enter |= crate::ui_kit::expr_row(ui, "Stock to leave", &mut f.stock_to_leave);
    enter |= crate::ui_kit::expr_row(ui, "Feed rate", &mut f.feed_rate);
    enter |= crate::ui_kit::expr_row(ui, "Plunge rate", &mut f.plunge_rate);
    enter |= crate::ui_kit::expr_row(ui, "Spindle speed", &mut f.spindle_speed);
    enter
}

/// The command's parameters: the tool, the setup and every cutting value the form states. An
/// empty optional field is left out, so the command takes its own default. The host runs the
/// returned `cam.toolpath.pocket` call; nothing is executed while the form is edited.
pub(crate) fn params(app: &SolveApp, f: &PocketDialog) -> Result<Value, String> {
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
    let optional = |label: &str, text: &str, kind: Kind| -> Result<Option<f64>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        crate::length_input::eval(&s.doc, text, kind).map(Some).map_err(|e| format!("{}: {e}", crate::i18n::tr_label(label)))
    };
    for (key, label, text, kind) in [
        ("stepover", "Stepover", f.stepover.as_str(), Kind::Length),
        ("depth_per_pass", "Depth per pass", f.depth_per_pass.as_str(), Kind::Length),
        ("total_depth", "Total depth", f.total_depth.as_str(), Kind::Length),
        ("stock_to_leave", "Stock to leave", f.stock_to_leave.as_str(), Kind::Length),
        ("feed_rate", "Feed rate", f.feed_rate.as_str(), Kind::Unitless),
        ("plunge_rate", "Plunge rate", f.plunge_rate.as_str(), Kind::Unitless),
        ("spindle_speed", "Spindle speed", f.spindle_speed.as_str(), Kind::Unitless),
    ] {
        if let Some(value) = optional(label, text, kind)? {
            p[key] = json!(value);
        }
    }
    Ok(p)
}

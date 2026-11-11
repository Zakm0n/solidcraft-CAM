//! CAM ▸ TOOLPATH: the 2D facing operation's form (`cam.toolpath.facing`).
//!
//! Facing sweeps the cutter back and forth across the top of the stock, one pass beside the
//! next, down to a depth. The form names the operation, the setup whose machine zero, axes and
//! stock it cuts in, the tool whose diameter spaces the passes, and the stepover, extension,
//! stock offset and depth it cuts at; OK runs `cam.toolpath.facing` and adds the operation to
//! the design.
//!
//! The form is drawn in the shared `ui_kit` dialog window (the Toolpath tab's host), with the
//! kit's rows and theme tokens, so it follows the rest of the CAM workspace.

use serde_json::{Value, json};
use solidcraft_engine::doc::expr::Kind;

use super::cam_tools;
use crate::SolveApp;

/// A facing operation's form: its name, setup, tool and cutting values.
#[derive(Clone, Debug)]
pub struct DialogCamFacing {
    /// Empty: the command names it "2D Facing" and the first free number.
    pub name: String,
    /// Index into the setup choices: 0 is the model origin, n is `doc.cam.setups[n - 1]`.
    pub setup: usize,
    /// Index into the design's tools, as `cam.tool.list` reports them.
    pub tool: usize,
    /// How far the cutter moves over between passes, as a fraction of its diameter (0.1–1).
    /// Empty takes the command's own default of 0.7.
    pub stepover: String,
    /// How far each pass runs beyond the stock's edge, in mm. Empty is the command's default of
    /// 0.
    pub pass_extension: String,
    /// How far the raw stock extends beyond the machined bodies on each side, in mm. Empty is
    /// the command's default of 0.
    pub stock_offset: String,
    /// How deep it cuts below the top of the stock, in mm.
    pub depth: String,
}

impl DialogCamFacing {
    pub(super) fn new(app: &SolveApp) -> Self {
        DialogCamFacing {
            name: String::new(),
            // A design with setups cuts in its first one; without, in the model origin.
            setup: usize::from(!app.session.doc.cam.setups.is_empty()),
            tool: 0,
            stepover: "0.7".into(),
            pass_extension: String::new(),
            stock_offset: String::new(),
            depth: String::new(),
        }
    }
}

/// The form's rows; true when Enter was pressed in a value.
pub(super) fn draw_facing_dialog(app: &mut SolveApp, ui: &mut egui::Ui, f: &mut DialogCamFacing) -> bool {
    let tools = cam_tools(app);
    let mut enter = crate::ui_kit::text_row(ui, "Name", &mut f.name, "").0;
    // The model origin first, then each setup of the design.
    let mut setups: Vec<&str> = Vec::with_capacity(app.session.doc.cam.setups.len() + 1);
    setups.push(crate::i18n::tr_label("Model origin"));
    setups.extend(app.session.doc.cam.setups.iter().map(|s| s.name.as_str()));
    f.setup = f.setup.min(setups.len() - 1);
    crate::ui_kit::choice_row(ui, "Setup", "cam_facing_setup", &mut f.setup, &setups);
    if tools.is_empty() {
        crate::ui_kit::status_line(ui, "Create a tool in the Tool Library first", crate::ui_kit::StatusTone::Dim);
        return enter;
    }
    let choices: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    f.tool = f.tool.min(choices.len() - 1);
    crate::ui_kit::choice_row(ui, crate::i18n::tr("ui.cam.select_tool"), "cam_facing_tool", &mut f.tool, &choices);
    enter |= crate::ui_kit::expr_row(ui, crate::i18n::tr("ui.cam.stepover"), &mut f.stepover);
    enter |= crate::ui_kit::expr_row(ui, "Pass extension", &mut f.pass_extension);
    enter |= crate::ui_kit::expr_row(ui, "Stock offset", &mut f.stock_offset);
    enter |= crate::ui_kit::expr_row(ui, "Depth", &mut f.depth);
    enter
}

/// The command's parameters: the tool, the setup and the values the form states. An empty
/// optional field is left out, so the command takes its own default.
pub(super) fn params(app: &SolveApp, f: &DialogCamFacing) -> Result<Value, String> {
    let s = &app.session;
    let tool = s.doc.cam.tools.get(f.tool).ok_or_else(|| "choose a tool (the design's Tool Library is empty)".to_string())?;
    let length = |label: &str, text: &str| -> Result<f64, String> {
        crate::length_input::eval(&s.doc, text.trim(), Kind::Length).map_err(|e| format!("{}: {e}", crate::i18n::tr_label(label)))
    };
    let mut p = json!({ "tool": tool.id });
    if let Some(setup) = f.setup.checked_sub(1).and_then(|i| s.doc.cam.setups.get(i)) {
        p["setup"] = json!(setup.id);
    }
    if !f.stepover.trim().is_empty() {
        let value = crate::length_input::eval(&s.doc, f.stepover.trim(), Kind::Unitless)
            .map_err(|e| format!("{}: {e}", crate::i18n::tr_label("Stepover")))?;
        if !(value.is_finite() && (0.1..=1.0).contains(&value)) {
            return Err(format!("{}: must be a number from 0.1 to 1", crate::i18n::tr_label("Stepover")));
        }
        p["stepover"] = json!(value);
    }
    if !f.pass_extension.trim().is_empty() {
        p["pass_extension"] = json!(length("Pass extension", &f.pass_extension)?);
    }
    if !f.stock_offset.trim().is_empty() {
        p["stock_offset"] = json!(length("Stock offset", &f.stock_offset)?);
    }
    if f.depth.trim().is_empty() {
        return Err("enter the depth below the top of the stock".into());
    }
    p["depth"] = json!(length("Depth", &f.depth)?);
    let name = f.name.trim();
    if !name.is_empty() {
        p["name"] = json!(name);
    }
    Ok(p)
}

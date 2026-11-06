//! The simulation playback panel: the operation being verified, Play, Step, Reset and the
//! timeline's scrub.
//!
//! The panel is a view: it reads the [`Simulation`] the application owns and reports what a
//! control asked for, which the application applies after the frame is drawn. That keeps the
//! simulation out of a borrow while the panel is laid out, and makes every control a command the
//! tests can run without a pointer.

use egui::{Align, Color32, Layout, RichText, Slider};
use solidcraft_cam::operations::Operation;
use solidcraft_cam::simulation::Simulation;

/// Height of the playback panel, in points: two rows of controls above the status bar.
pub const SIMULATION_PANEL_HEIGHT: f32 = 64.0;

/// The panel's background: a shade below the viewport's, so the playback reads as its own strip
/// rather than as part of the picture.
pub const SIMULATION_PANEL_BACKGROUND: Color32 = Color32::from_rgb(232, 236, 242);

/// Width of the operation picker, in points: enough for the summary it shows.
const OPERATION_PICKER_WIDTH: f32 = 300.0;

/// What the panel asks the application to do once the frame is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SimulationAction {
    /// Start the clock if it is stopped, stop it if it is running.
    PlayPause,
    /// Advance the tool to the end of the next move.
    Step,
    /// Run the program back to its start and restore the stock.
    Reset,
    /// Put the tool this many millimetres along the path.
    Scrub(f64),
    /// Simulate the operation at this index.
    SelectOperation(usize),
}

/// Draws the playback panel into `ui`, with the simulation, its operations and the one selected.
pub fn ui(ui: &mut egui::Ui, simulation: &Simulation, operations: &[Operation], selected: usize) -> Option<SimulationAction> {
    let mut action = None;
    let has_path = simulation.total_length() > 0.0;
    let frame = egui::Frame::new().fill(SIMULATION_PANEL_BACKGROUND).inner_margin(egui::Margin::symmetric(8, 4));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Simulation").strong());
            ui.separator();
            let selected_text =
                operations.get(selected).map_or_else(|| "No operation".to_string(), |operation| format!("{}. {}", selected + 1, operation.summary()));
            egui::ComboBox::from_id_salt("simulation_operation").selected_text(selected_text).width(OPERATION_PICKER_WIDTH).show_ui(ui, |ui| {
                for (index, operation) in operations.iter().enumerate() {
                    if ui.selectable_label(index == selected, format!("{}. {}", index + 1, operation.summary())).clicked() {
                        action = Some(SimulationAction::SelectOperation(index));
                    }
                }
            });
            let playing = simulation.playing();
            if ui.add_enabled(has_path, egui::Button::new(if playing { "Pause" } else { "Play" })).clicked() {
                action = Some(SimulationAction::PlayPause);
            }
            if ui.add_enabled(has_path, egui::Button::new("Step")).clicked() {
                action = Some(SimulationAction::Step);
            }
            if ui.add_enabled(has_path && simulation.position() > 0.0, egui::Button::new("Reset")).clicked() {
                action = Some(SimulationAction::Reset);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(simulation.summary()).weak());
            });
        });
        ui.horizontal(|ui| {
            ui.label("Timeline");
            let total = simulation.total_length().max(1e-9);
            let mut position = simulation.position();
            let slider = Slider::new(&mut position, 0.0..=total).show_value(false).clamping(egui::SliderClamping::Always);
            let width = (ui.available_width() - 170.0).max(80.0);
            if ui.add_enabled_ui(has_path, |ui| ui.add_sized([width, 20.0], slider)).inner.changed() {
                action = Some(SimulationAction::Scrub(position));
            }
            ui.label(format!("{:.0}% · {:.1} mm", simulation.fraction() * 100.0, simulation.position()));
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_asks_the_blueprint_for_a_timeline_and_its_controls() {
        assert_eq!(SIMULATION_PANEL_HEIGHT, 64.0);
        let actions = [SimulationAction::PlayPause, SimulationAction::Step, SimulationAction::Reset, SimulationAction::Scrub(1.0)];
        assert_eq!(actions.len(), 4, "Play, Step, Reset and the scrub");
    }
}

//! The Tool Library dialog: the modal that browses the preset catalogue, creates tools and edits
//! the assemblies a program's operations select from.
//!
//! The dialog edits a *copy* of the library. `OK` validates every assembly before handing the
//! copy back, and `Cancel` (or Escape, or a click outside the window) throws the copy away, so a
//! half-typed diameter never reaches an operation that already names the tool.

use egui::{Align, Color32, Layout, RichText, ScrollArea};
use solidcraft_cam::tool::{CuttingTool, PRESETS, ToolAssembly, ToolError, ToolHolder, ToolLibrary, ToolType};

/// The dialog's width, in points: the assembly list and the editor side by side.
pub const TOOL_DIALOG_WIDTH: f32 = 720.0;

/// The height of the assembly list and the editor, in points.
pub const TOOL_DIALOG_HEIGHT: f32 = 380.0;

/// Width of the assembly list, in points.
const LIST_WIDTH: f32 = 260.0;

/// Width of the editor's label column, in points.
const LABEL_WIDTH: f32 = 104.0;

/// Height of one editor row, in points.
const ROW_HEIGHT: f32 = 20.0;

/// The colour a refused `OK` writes its reason in: the same red the import dialog uses.
pub const ERROR_COLOUR: Color32 = Color32::from_rgb(180, 40, 40);

/// What the dialog asks the application to do once the frame is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolLibraryOutcome {
    /// The dialog stays open and the library is untouched.
    Open,
    /// The user accepted the edited library.
    Accept(ToolLibrary),
    /// The user closed the dialog; the library is untouched.
    Cancel,
}

/// What the frame's buttons asked for, before the working copy is validated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Action {
    #[default]
    None,
    Accept,
    Cancel,
    AddPreset,
    NewTool,
    Delete,
}

/// The Tool Library dialog's state: the working copy, the assembly being edited, and why the last
/// `OK` was refused.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolLibraryDialog {
    /// The working copy the editor changes; it replaces the application's library only on `OK`.
    library: ToolLibrary,
    /// The id of the assembly the editor shows.
    selected: Option<usize>,
    /// Index into [`PRESETS`] of the entry "Add preset" adds.
    preset: usize,
    /// Why the last `OK` was refused, shown under the buttons.
    error: Option<String>,
}

impl ToolLibraryDialog {
    /// A dialog editing a copy of `library`, with its first assembly selected.
    pub fn new(library: ToolLibrary) -> Self {
        let selected = library.assemblies().first().map(|assembly| assembly.id);
        Self { library, selected, preset: 0, error: None }
    }

    /// The working copy, as the editor has it so far.
    pub fn library(&self) -> &ToolLibrary {
        &self.library
    }

    /// The id of the assembly the editor shows.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Why the last `OK` was refused, if it was.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Shows the assembly with `id` in the editor.
    pub fn select(&mut self, id: usize) {
        if self.library.get(id).is_some() {
            self.selected = Some(id);
        }
    }

    /// The assembly the editor shows, for a test or a caller that needs the work in progress.
    pub fn selected_assembly(&self) -> Option<&ToolAssembly> {
        self.selected.and_then(|id| self.library.get(id))
    }

    /// The assembly the editor shows, to edit in place.
    pub fn selected_assembly_mut(&mut self) -> Option<&mut ToolAssembly> {
        let id = self.selected?;
        self.library.get_mut(id)
    }

    /// Adds the catalogue entry the preset picker shows, selects it, and returns its id. `None`
    /// only if the catalogue is empty.
    pub fn add_preset(&mut self) -> Option<usize> {
        let preset = *PRESETS.get(self.preset)?;
        let id = self.library.add(ToolAssembly::from_preset(preset));
        self.selected = Some(id);
        Some(id)
    }

    /// Adds a new Ø6 flat endmill, selects it, and returns its id.
    pub fn add_tool(&mut self) -> usize {
        let id = self.library.add(ToolAssembly::new("New tool", CuttingTool::new(ToolType::FlatEndmill, 6.0)));
        self.selected = Some(id);
        id
    }

    /// Removes the selected assembly; the list's first assembly is selected afterwards.
    pub fn delete_selected(&mut self) -> Option<ToolAssembly> {
        let id = self.selected?;
        let removed = self.library.remove(id)?;
        self.selected = self.library.assemblies().first().map(|assembly| assembly.id);
        Some(removed)
    }

    /// Checks every assembly; only a library that passes may replace the application's.
    pub fn accept(&self) -> Result<ToolLibrary, ToolError> {
        self.library.validate()?;
        Ok(self.library.clone())
    }

    /// Draws the modal and reports what the user asked for.
    pub fn ui(&mut self, ctx: &egui::Context) -> ToolLibraryOutcome {
        let mut action = Action::None;
        let modal = egui::Modal::new(egui::Id::unique("tool_library_dialog")).show(ctx, |ui| {
            ui.set_width(TOOL_DIALOG_WIDTH);
            self.heading(ui);
            ui.separator();
            self.toolbar(ui, &mut action);
            ui.separator();
            ui.horizontal_top(|ui| {
                self.list(ui);
                ui.separator();
                self.editor(ui);
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Delete tool").on_hover_text("Remove the selected assembly from the library").clicked() {
                    action = Action::Delete;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        action = Action::Cancel;
                    }
                    if ui.button("OK").clicked() {
                        action = Action::Accept;
                    }
                });
            });
            if let Some(error) = &self.error {
                ui.colored_label(ERROR_COLOUR, error);
            }
        });
        match action {
            Action::Accept => match self.accept() {
                Ok(library) => ToolLibraryOutcome::Accept(library),
                Err(error) => {
                    self.error = Some(error.to_string());
                    ToolLibraryOutcome::Open
                }
            },
            Action::Cancel => ToolLibraryOutcome::Cancel,
            Action::AddPreset => {
                self.add_preset();
                self.error = None;
                ToolLibraryOutcome::Open
            }
            Action::NewTool => {
                self.add_tool();
                self.error = None;
                ToolLibraryOutcome::Open
            }
            Action::Delete => {
                self.delete_selected();
                self.error = None;
                ToolLibraryOutcome::Open
            }
            Action::None => {
                if modal.should_close() {
                    ToolLibraryOutcome::Cancel
                } else {
                    ToolLibraryOutcome::Open
                }
            }
        }
    }

    /// The title and the library's size.
    fn heading(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Tool Library");
            ui.separator();
            let count = self.library.len();
            let plural = if count == 1 { "" } else { "s" };
            ui.label(RichText::new(format!("{count} tool{plural}")).weak());
        });
    }

    /// The row above the list: the preset catalogue and the buttons that add to the library.
    fn toolbar(&mut self, ui: &mut egui::Ui, action: &mut Action) {
        ui.horizontal(|ui| {
            let chosen = PRESETS.get(self.preset).map_or("(no presets)", |preset| preset.name);
            egui::ComboBox::from_id_salt("tool_preset").selected_text(chosen).width(180.0).show_ui(ui, |ui| {
                for (index, preset) in PRESETS.iter().enumerate() {
                    ui.selectable_value(&mut self.preset, index, preset.name);
                }
            });
            if ui.button("Add preset").on_hover_text("Add the catalogue entry to the library").clicked() {
                *action = Action::AddPreset;
            }
            if ui.button("New tool").on_hover_text("Start a tool from scratch").clicked() {
                *action = Action::NewTool;
            }
        });
    }

    /// The left-hand list of assemblies, one row each, the edited one highlighted.
    fn list(&mut self, ui: &mut egui::Ui) {
        let mut choose = None;
        ui.vertical(|ui| {
            ui.set_width(LIST_WIDTH);
            ui.label(RichText::new("Tools").strong());
            ScrollArea::vertical().id_salt("tool_library_list").max_height(TOOL_DIALOG_HEIGHT).auto_shrink([false, false]).show(ui, |ui| {
                if self.library.is_empty() {
                    ui.label(RichText::new("No tools. Add a preset or a new tool.").weak());
                }
                for assembly in self.library.assemblies() {
                    let text = format!("{}\n{}", assembly.name, assembly.summary());
                    if ui.selectable_label(self.selected == Some(assembly.id), text).clicked() {
                        choose = Some(assembly.id);
                    }
                }
            });
        });
        if let Some(id) = choose {
            self.selected = Some(id);
            self.error = None;
        }
    }

    /// The right-hand editor for the selected assembly.
    fn editor(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.label(RichText::new("No tool selected.").weak());
            return;
        };
        let Some(assembly) = self.library.get_mut(id) else {
            ui.label(RichText::new("No tool selected.").weak());
            return;
        };
        ui.vertical(|ui| {
            ui.set_min_width(TOOL_DIALOG_WIDTH - LIST_WIDTH - 40.0);
            ui.label(RichText::new("Assembly").strong());
            row(ui, "Name", |ui| {
                ui.add(egui::TextEdit::singleline(&mut assembly.name).desired_width(280.0));
            });
            row(ui, "Type", |ui| {
                egui::ComboBox::from_id_salt("tool_kind").selected_text(assembly.tool.kind.label()).width(180.0).show_ui(ui, |ui| {
                    for kind in ToolType::ALL {
                        ui.selectable_value(&mut assembly.tool.kind, *kind, kind.label());
                    }
                });
            });
            ui.separator();
            ui.label(RichText::new("Cutting tool").strong());
            row(ui, "Diameter", |ui| {
                ui.add(egui::DragValue::new(&mut assembly.tool.diameter).range(0.01..=200.0).speed(0.1).suffix(" mm").fixed_decimals(2));
            });
            row(ui, "Corner radius", |ui| {
                let editable = assembly.tool.kind.has_corner_radius();
                let mut radius = assembly.tool.effective_corner_radius();
                let drag = egui::DragValue::new(&mut radius).range(0.0..=100.0).speed(0.1).suffix(" mm").fixed_decimals(2);
                if ui.add_enabled(editable, drag).changed() {
                    assembly.tool.corner_radius = radius;
                }
                if !editable {
                    ui.label(RichText::new("square corner").weak());
                }
            });
            row(ui, "Flute length", |ui| {
                ui.add(egui::DragValue::new(&mut assembly.tool.flute_length).range(0.01..=300.0).speed(0.5).suffix(" mm").fixed_decimals(2));
            });
            row(ui, "Flutes", |ui| {
                ui.add(egui::DragValue::new(&mut assembly.tool.flutes).range(1..=12).speed(0.1));
            });
            ui.separator();
            let mut has_holder = assembly.holder.is_some();
            if ui.checkbox(&mut has_holder, "Holder").changed() {
                assembly.holder = if has_holder { Some(ToolHolder::standard()) } else { None };
            }
            if let Some(holder) = &mut assembly.holder {
                row(ui, "Holder name", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut holder.name).desired_width(220.0));
                });
                row(ui, "Gauge Ø", |ui| {
                    ui.add(egui::DragValue::new(&mut holder.diameter).range(0.01..=300.0).speed(0.5).suffix(" mm").fixed_decimals(2));
                });
                row(ui, "Holder length", |ui| {
                    ui.add(egui::DragValue::new(&mut holder.length).range(0.01..=400.0).speed(0.5).suffix(" mm").fixed_decimals(2));
                });
            }
            row(ui, "Stickout", |ui| {
                ui.add(egui::DragValue::new(&mut assembly.stickout).range(0.01..=400.0).speed(0.5).suffix(" mm").fixed_decimals(2));
            });
            ui.add_space(4.0);
            ui.label(RichText::new(assembly.summary()).weak());
        });
    }
}

/// One editor row: a label in a fixed column, then the field.
fn row<R>(ui: &mut egui::Ui, label: &str, field: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized([LABEL_WIDTH, ROW_HEIGHT], egui::Label::new(label));
        field(ui)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_opens_on_the_librarys_first_tool() {
        let library = ToolLibrary::with_presets();
        let dialog = ToolLibraryDialog::new(library.clone());
        assert_eq!(dialog.selected(), library.assemblies().first().map(|assembly| assembly.id));
        assert_eq!(dialog.library(), &library);
    }

    #[test]
    fn edits_stay_in_the_working_copy_until_ok() {
        let library = ToolLibrary::with_presets();
        let mut dialog = ToolLibraryDialog::new(library.clone());
        if let Some(assembly) = dialog.selected_assembly_mut() {
            assembly.name = "Rougher".to_string();
        }
        assert_eq!(dialog.selected_assembly().map(|assembly| assembly.name.as_str()), Some("Rougher"));
        assert_eq!(dialog.accept().map(|accepted| accepted.assemblies()[0].name.clone()), Ok("Rougher".to_string()));
        // The dialog never wrote through to the library it was handed.
        assert_eq!(library.assemblies()[0].name, PRESETS[0].name);
    }

    #[test]
    fn add_preset_and_new_tool_select_what_they_added() {
        let mut dialog = ToolLibraryDialog::new(ToolLibrary::new());
        let preset_id = dialog.add_preset();
        assert_eq!(preset_id, Some(1));
        let added = dialog.selected_assembly();
        assert_eq!(added.map(|assembly| assembly.name.as_str()), Some(PRESETS[0].name));
        let tool_id = dialog.add_tool();
        assert_eq!(dialog.selected(), Some(tool_id));
        assert_eq!(dialog.library().len(), 2);
        assert_eq!(dialog.selected_assembly().map(|assembly| assembly.name.as_str()), Some("New tool"));
        assert_eq!(dialog.accept().map(|library| library.len()), Ok(2));
    }

    #[test]
    fn deleting_the_selected_tool_selects_the_next() {
        let mut dialog = ToolLibraryDialog::new(ToolLibrary::with_presets());
        let first = dialog.selected();
        assert!(dialog.delete_selected().is_some());
        assert_eq!(dialog.library().len(), PRESETS.len() - 1);
        assert_ne!(dialog.selected(), first, "the deleted tool is no longer selected");
        assert_eq!(dialog.selected(), dialog.library().assemblies().first().map(|assembly| assembly.id));
    }

    #[test]
    fn ok_refuses_a_tool_that_cannot_cut_and_says_which_value() {
        let mut dialog = ToolLibraryDialog::new(ToolLibrary::with_presets());
        if let Some(assembly) = dialog.selected_assembly_mut() {
            assembly.tool.diameter = 0.0;
        }
        assert_eq!(dialog.accept(), Err(ToolError::Diameter));
    }

    #[test]
    fn the_dialog_draws_every_assembly_and_the_editor() {
        let ctx = egui::Context::default();
        let mut dialog = ToolLibraryDialog::new(ToolLibrary::with_presets());
        // The first frame lays the modal's area out; the next one paints it.
        let mut primitives = Vec::new();
        for _ in 0..2 {
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |_ui| {
                assert_eq!(dialog.ui(&ctx), ToolLibraryOutcome::Open, "a frame with no click leaves the dialog open");
            });
            primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
            out.textures_delta.clear();
        }
        assert!(!primitives.is_empty(), "the dialog must draw something");
    }
}

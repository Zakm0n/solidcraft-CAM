//! The Tool Library dialog: the modal that browses the preset catalogue, creates tools and edits
//! the assemblies a program's operations select from.
//!
//! The dialog edits a *copy* of the library. `OK` validates every assembly before handing the
//! copy back, and `Cancel` (or Escape, or a click outside the window) throws the copy away, so a
//! half-typed diameter never reaches an operation that already names the tool.

use egui::{Align, Color32, Layout, RichText, ScrollArea};
use solidcraft_cam::operations::contour2d::{
    Compensation, Contour2DError, Contour2DParams, CutDirection, DepthOrder, LeadKind, LeadMove, LinkingMode,
};
use solidcraft_cam::stock::{StockDef, StockError, StockExcess};
use solidcraft_cam::tool::{CuttingTool, PRESETS, ToolAssembly, ToolError, ToolHolder, ToolLibrary, ToolType};

use crate::document::CamDocument;

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

/// The Stock Setup dialog's width, in points.
pub const STOCK_DIALOG_WIDTH: f32 = 460.0;

/// How the Stock Setup dialog defines the material.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StockMode {
    /// A box around the part geometry, grown by the excess on each side.
    #[default]
    BoundingBox,
    /// An existing solid body used as the raw material.
    Solid,
}

/// What the Stock Setup frame's buttons asked for, before the definition is validated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum StockAction {
    #[default]
    None,
    Accept,
    Remove,
    Cancel,
}

/// What the Stock Setup dialog asks the application to do once the frame is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum StockOutcome {
    /// The dialog stays open and the document is untouched.
    Open,
    /// The user accepted a new stock definition (`None`: remove the stock).
    Apply(Option<StockDef>),
    /// The user closed the dialog; the document is untouched.
    Cancel,
}

/// The Stock Setup dialog's state: the mode, the excess offsets and the body a solid stock names.
///
/// The dialog edits nothing until `OK`: it computes the definition from the document when the
/// user accepts, so a half-typed excess never reaches the viewport.
#[derive(Clone, Debug, PartialEq)]
pub struct StockSetupDialog {
    /// Whether the stock is a box around the part or an existing body.
    mode: StockMode,
    /// The material left around the part on each side, in millimetres.
    excess: StockExcess,
    /// The id of the body a solid stock names.
    body: Option<usize>,
    /// Why the last `OK` was refused, shown under the buttons.
    error: Option<String>,
}

impl StockSetupDialog {
    /// A dialog over `document`, starting on the stock it already has: a solid stock reopens in
    /// solid mode, everything else starts from a bounding box with no excess.
    pub fn new(document: &CamDocument) -> Self {
        let (mode, body) = match document.stock() {
            Some(StockDef::Solid(id)) => (StockMode::Solid, Some(id)),
            _ => (StockMode::BoundingBox, document.solids().first().map(|solid| solid.id)),
        };
        Self { mode, excess: StockExcess::default(), body, error: None }
    }

    /// Whether the dialog defines a bounding box or a solid.
    pub fn mode(&self) -> StockMode {
        self.mode
    }

    /// The excess offsets the dialog would apply to a bounding box.
    pub fn excess(&self) -> StockExcess {
        self.excess
    }

    /// Sets the excess offsets: what a test or a caller types into the dialog's fields.
    pub fn set_excess(&mut self, excess: StockExcess) {
        self.excess = excess;
        self.error = None;
    }

    /// The id of the body a solid stock names.
    pub fn body(&self) -> Option<usize> {
        self.body
    }

    /// Shows `id` as the solid stock, if the document holds that body.
    pub fn select_body(&mut self, document: &CamDocument, id: usize) {
        if document.solid_by_id(id).is_some() {
            self.body = Some(id);
            self.error = None;
        }
    }

    /// Why the last `OK` was refused, if it was.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The stock definition the dialog's settings describe, checked against `document`.
    pub fn preview(&self, document: &CamDocument) -> Result<StockDef, StockError> {
        match self.mode {
            StockMode::BoundingBox => {
                let (min, max) = document.part_bounds().ok_or(StockError::NoGeometry)?;
                let stock = StockDef::from_bounds(min, max, self.excess);
                stock.validate()?;
                Ok(stock)
            }
            StockMode::Solid => {
                let id = self.body.ok_or(StockError::Body)?;
                if document.solid_by_id(id).is_none() {
                    return Err(StockError::Body);
                }
                let stock = StockDef::Solid(id);
                stock.validate()?;
                Ok(stock)
            }
        }
    }

    /// The definition `OK` would store, or why it cannot.
    pub fn accept(&self, document: &CamDocument) -> Result<Option<StockDef>, StockError> {
        self.preview(document).map(Some)
    }

    /// Draws the modal and reports what the user asked for.
    pub fn ui(&mut self, ctx: &egui::Context, document: &CamDocument) -> StockOutcome {
        let mut action = StockAction::None;
        let modal = egui::Modal::new(egui::Id::unique("stock_setup_dialog")).show(ctx, |ui| {
            ui.set_width(STOCK_DIALOG_WIDTH);
            self.heading(ui, document);
            ui.separator();
            ui.radio_value(&mut self.mode, StockMode::BoundingBox, "Bounding box around part geometry");
            if self.mode == StockMode::BoundingBox {
                self.bounding_box_ui(ui, document);
            }
            ui.radio_value(&mut self.mode, StockMode::Solid, "Use an existing solid body");
            if self.mode == StockMode::Solid {
                self.solid_ui(ui, document);
            }
            ui.separator();
            ui.horizontal(|ui| {
                let can_remove = document.stock().is_some();
                if ui.add_enabled(can_remove, egui::Button::new("Remove stock")).on_hover_text("Cut with no stock defined").clicked() {
                    action = StockAction::Remove;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        action = StockAction::Cancel;
                    }
                    if ui.button("OK").clicked() {
                        action = StockAction::Accept;
                    }
                });
            });
            if let Some(error) = &self.error {
                ui.colored_label(ERROR_COLOUR, error);
            }
        });
        match action {
            StockAction::Accept => match self.accept(document) {
                Ok(stock) => StockOutcome::Apply(stock),
                Err(error) => {
                    self.error = Some(error.to_string());
                    StockOutcome::Open
                }
            },
            StockAction::Remove => StockOutcome::Apply(None),
            StockAction::Cancel => StockOutcome::Cancel,
            StockAction::None => {
                if modal.should_close() {
                    StockOutcome::Cancel
                } else {
                    StockOutcome::Open
                }
            }
        }
    }

    /// The title and the stock the document currently has.
    fn heading(&self, ui: &mut egui::Ui, document: &CamDocument) {
        ui.horizontal(|ui| {
            ui.heading("Stock Setup");
            ui.separator();
            let current = document.stock().map_or_else(|| "no stock".to_string(), StockDef::summary);
            ui.label(RichText::new(current).weak());
        });
    }

    /// The bounding-box mode: the excess offsets and the box they produce.
    fn bounding_box_ui(&mut self, ui: &mut egui::Ui, document: &CamDocument) {
        if document.part_bounds().is_none() {
            ui.label(RichText::new("No solid geometry to size the stock from. Create or import a body first.").weak());
            return;
        }
        ui.add_space(2.0);
        row(ui, "Excess X", |ui| {
            ui.add(egui::DragValue::new(&mut self.excess.x).range(-1000.0..=1000.0).speed(0.5).suffix(" mm").fixed_decimals(2));
        });
        row(ui, "Excess Y", |ui| {
            ui.add(egui::DragValue::new(&mut self.excess.y).range(-1000.0..=1000.0).speed(0.5).suffix(" mm").fixed_decimals(2));
        });
        row(ui, "Excess Z", |ui| {
            ui.add(egui::DragValue::new(&mut self.excess.z).range(-1000.0..=1000.0).speed(0.5).suffix(" mm").fixed_decimals(2));
        });
        match self.preview(document) {
            Ok(stock) => {
                if let Some([x, y, z]) = stock.size() {
                    ui.label(RichText::new(format!("Stock size: {x:.2} × {y:.2} × {z:.2} mm")).weak());
                }
                if let Some(extents) = stock.extents() {
                    ui.label(
                        RichText::new(format!(
                            "X {:.2} … {:.2}   Y {:.2} … {:.2}   Z {:.2} … {:.2} mm",
                            extents[0], extents[3], extents[1], extents[4], extents[2], extents[5]
                        ))
                        .weak(),
                    );
                }
            }
            Err(error) => {
                ui.colored_label(ERROR_COLOUR, error.to_string());
            }
        }
    }

    /// The solid mode: the body picker.
    fn solid_ui(&mut self, ui: &mut egui::Ui, document: &CamDocument) {
        if document.solids().is_empty() {
            ui.label(RichText::new("No solid bodies. Create or import one first.").weak());
            return;
        }
        let chosen = self.body.and_then(|id| document.solid_by_id(id)).map_or("(no body)", |solid| solid.name.as_str());
        ui.horizontal(|ui| {
            ui.add_sized([LABEL_WIDTH, ROW_HEIGHT], egui::Label::new("Body"));
            egui::ComboBox::from_id_salt("stock_solid_body").selected_text(chosen).width(240.0).show_ui(ui, |ui| {
                for solid in document.solids() {
                    ui.selectable_value(&mut self.body, Some(solid.id), solid.name.as_str());
                }
            });
        });
        if self.body.is_none() {
            ui.label(RichText::new("Select the body the raw material is.").weak());
        }
    }
}

/// The 2D Contour dialog's width, in points: the page list and the active page side by side.
pub const CONTOUR_DIALOG_WIDTH: f32 = 760.0;

/// Width of the page list on the dialog's left, in points.
const CONTOUR_PAGES_WIDTH: f32 = 156.0;

/// Height of the active page, in points: enough for the longest page without scrolling the modal.
const CONTOUR_PAGE_HEIGHT: f32 = 330.0;

/// The parameter pages of the 2D Contour dialog, in Mastercam's order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContourPage {
    /// The wireframe entities the toolpath follows.
    #[default]
    Geometry,
    /// The tool assembly that cuts it.
    Tool,
    /// Compensation, cutter side and stock to leave.
    CutParameters,
    /// How the total depth is divided into cuts.
    DepthCuts,
    /// Extra passes across the wall at each depth.
    MultiPasses,
    /// The entry and exit moves.
    LeadInOut,
    /// Clearance, retract, feed plane, top of stock and depth.
    Linking,
}

impl ContourPage {
    /// The pages in the order the dialog lists them.
    pub const ALL: [Self; 7] = [Self::Geometry, Self::Tool, Self::CutParameters, Self::DepthCuts, Self::MultiPasses, Self::LeadInOut, Self::Linking];

    /// The page's text in the page list.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Geometry => "Geometry",
            Self::Tool => "Tool",
            Self::CutParameters => "Cut Parameters",
            Self::DepthCuts => "Depth Cuts",
            Self::MultiPasses => "Multi Passes",
            Self::LeadInOut => "Lead In/Out",
            Self::Linking => "Linking",
        }
    }
}

/// What the 2D Contour dialog asks the application to do once the frame is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum Contour2DOutcome {
    /// The dialog stays open and the document is untouched.
    Open,
    /// The user accepted a valid operation.
    Accept(Box<Contour2DParams>),
    /// The user closed the dialog; nothing is stored.
    Cancel,
}

/// What the footer's buttons asked for, before the parameters are validated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ContourAction {
    #[default]
    None,
    Accept,
    Cancel,
}

/// Which lead move a lead section edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeadSide {
    /// The move onto the chain.
    Entry,
    /// The move off the chain.
    Exit,
}

/// The 2D Contour dialog: the parameter pages of one operation, as a modal.
///
/// The dialog edits a copy of the operation's parameters; `OK` validates the result before
/// handing it back, and `Cancel` throws it away. A chain left empty or a depth above the stock is
/// reported beside the buttons instead of being stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Contour2DDialog {
    /// The operation's parameters, as the pages have them so far.
    params: Contour2DParams,
    /// The page the dialog shows.
    page: ContourPage,
    /// Why the last `OK` was refused, shown under the buttons.
    error: Option<String>,
}

impl Contour2DDialog {
    /// A dialog over `params`, starting on the geometry page.
    pub fn new(params: Contour2DParams) -> Self {
        Self { params, page: ContourPage::Geometry, error: None }
    }

    /// The parameters as the pages have them so far.
    pub fn params(&self) -> &Contour2DParams {
        &self.params
    }

    /// The parameters, to drive the pages from a test.
    pub fn params_mut(&mut self) -> &mut Contour2DParams {
        &mut self.params
    }

    /// The page the dialog shows.
    pub fn page(&self) -> ContourPage {
        self.page
    }

    /// Shows `page`.
    pub fn set_page(&mut self, page: ContourPage) {
        self.page = page;
    }

    /// Why the last `OK` was refused, if it was.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The operation `OK` would store, or why it cannot cut.
    pub fn accept(&self) -> Result<Contour2DParams, Contour2DError> {
        self.params.validate()?;
        Ok(self.params.clone())
    }

    /// Draws the modal and reports what the user asked for.
    pub fn ui(&mut self, ctx: &egui::Context, document: &CamDocument, tools: &ToolLibrary) -> Contour2DOutcome {
        let mut action = ContourAction::None;
        let modal = egui::Modal::new(egui::Id::unique("contour2d_dialog")).show(ctx, |ui| {
            ui.set_width(CONTOUR_DIALOG_WIDTH);
            self.heading(ui, tools);
            ui.separator();
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(CONTOUR_PAGES_WIDTH);
                    for page in ContourPage::ALL {
                        ui.selectable_value(&mut self.page, page, page.label());
                    }
                });
                ui.separator();
                ScrollArea::vertical().id_salt("contour2d_page").max_height(CONTOUR_PAGE_HEIGHT).auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_min_width(CONTOUR_DIALOG_WIDTH - CONTOUR_PAGES_WIDTH - 48.0);
                    ui.set_min_height(CONTOUR_PAGE_HEIGHT);
                    // The page sits beside the page list, so it needs a layout of its own; the
                    // rows would otherwise run to the right of one another.
                    ui.vertical(|ui| match self.page {
                        ContourPage::Geometry => self.geometry_page(ui, document),
                        ContourPage::Tool => self.tool_page(ui, tools),
                        ContourPage::CutParameters => self.cut_page(ui),
                        ContourPage::DepthCuts => self.depth_page(ui),
                        ContourPage::MultiPasses => self.multipass_page(ui),
                        ContourPage::LeadInOut => self.lead_page(ui),
                        ContourPage::Linking => self.linking_page(ui),
                    });
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                let tool = tools.get(self.params.tool_id).map_or("no tool", |assembly| assembly.name.as_str());
                ui.label(RichText::new(format!("{} · {tool}", self.params.summary())).weak());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        action = ContourAction::Cancel;
                    }
                    if ui.button("OK").clicked() {
                        action = ContourAction::Accept;
                    }
                });
            });
            if let Some(error) = &self.error {
                ui.colored_label(ERROR_COLOUR, error);
            }
        });
        match action {
            ContourAction::Accept => match self.accept() {
                Ok(params) => Contour2DOutcome::Accept(Box::new(params)),
                Err(error) => {
                    self.error = Some(error.to_string());
                    Contour2DOutcome::Open
                }
            },
            ContourAction::Cancel => Contour2DOutcome::Cancel,
            ContourAction::None => {
                if modal.should_close() {
                    Contour2DOutcome::Cancel
                } else {
                    Contour2DOutcome::Open
                }
            }
        }
    }

    /// The title, and the entity and tool counts the operation has so far.
    fn heading(&self, ui: &mut egui::Ui, tools: &ToolLibrary) {
        ui.horizontal(|ui| {
            ui.heading("2D Contour");
            ui.separator();
            let count = self.params.chain_len();
            let noun = if count == 1 { "entity" } else { "entities" };
            ui.label(RichText::new(format!("{count} chained {noun}")).weak());
            ui.separator();
            let tool = tools.get(self.params.tool_id).map_or("no tool", |assembly| assembly.name.as_str());
            ui.label(RichText::new(tool).weak());
        });
    }

    /// The `Geometry` page: every chainable curve of the document, ticked when chained.
    fn geometry_page(&mut self, ui: &mut egui::Ui, document: &CamDocument) {
        let entities = document.chain_entities();
        ui.label(RichText::new("Geometry chaining").strong());
        ui.label(RichText::new("Tick the curves the toolpath follows, in cutting order.").weak());
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            if ui.button("Select all").clicked() {
                self.params.chain = entities.iter().map(|entity| entity.id).collect();
            }
            if ui.button("Clear").clicked() {
                self.params.chain.clear();
            }
            let chained = self.params.chain_len();
            ui.label(RichText::new(format!("{chained} of {} chained", entities.len())).weak());
        });
        ui.separator();
        if entities.is_empty() {
            ui.label(RichText::new("No wireframe on the visible, selectable levels. Draw or import geometry first.").weak());
            return;
        }
        ScrollArea::vertical().id_salt("contour2d_chain").max_height(CONTOUR_PAGE_HEIGHT - 90.0).auto_shrink([false, false]).show(ui, |ui| {
            for entity in &entities {
                let mut chained = self.params.chain.contains(&entity.id);
                let shape = if entity.is_closed() { "closed" } else { "open" };
                let text = format!("E{} · level {} · {:.1} mm · {shape}", entity.id, entity.level, entity.length());
                if ui.checkbox(&mut chained, text).changed() {
                    if chained {
                        self.params.chain.push(entity.id);
                    } else {
                        self.params.chain.retain(|id| *id != entity.id);
                    }
                    self.error = None;
                }
            }
        });
    }

    /// The `Tool` page: the assembly the operation selects from the library.
    fn tool_page(&mut self, ui: &mut egui::Ui, tools: &ToolLibrary) {
        ui.label(RichText::new("Tool selection").strong());
        ui.label(RichText::new("The operation cuts with this assembly from the tool library.").weak());
        ui.add_space(2.0);
        if tools.is_empty() {
            ui.label(RichText::new("The tool library is empty. Add a tool in the Tool Library first.").weak());
            return;
        }
        let chosen = tools.get(self.params.tool_id).map_or("(no tool)", |assembly| assembly.name.as_str());
        row(ui, "Tool", |ui| {
            egui::ComboBox::from_id_salt("contour_tool").selected_text(chosen).width(330.0).show_ui(ui, |ui| {
                for assembly in tools.assemblies() {
                    ui.selectable_value(&mut self.params.tool_id, assembly.id, format!("{} — {}", assembly.name, assembly.summary()));
                }
            });
        });
        ui.separator();
        match tools.get(self.params.tool_id) {
            Some(assembly) => {
                let tool = &assembly.tool;
                row(ui, "Type", |ui| {
                    ui.label(tool.kind.label());
                });
                row(ui, "Diameter", |ui| {
                    ui.label(format!("{:.2} mm", tool.diameter));
                });
                row(ui, "Corner radius", |ui| {
                    ui.label(format!("{:.2} mm", tool.effective_corner_radius()));
                });
                row(ui, "Flute length", |ui| {
                    ui.label(format!("{:.2} mm", tool.flute_length));
                });
                row(ui, "Flutes", |ui| {
                    ui.label(tool.flutes.to_string());
                });
                let holder = assembly.holder.as_ref().map_or("none".to_string(), |holder| format!("{} · Ø{:.1} mm", holder.name, holder.diameter));
                row(ui, "Holder", |ui| {
                    ui.label(holder);
                });
                row(ui, "Stickout", |ui| {
                    ui.label(format!("{:.2} mm", assembly.stickout));
                });
            }
            None => {
                ui.colored_label(ERROR_COLOUR, "No tool selected. Pick an assembly to cut with.");
            }
        }
    }

    /// The `Cut Parameters` page: compensation, cutter side and stock to leave.
    fn cut_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Cut parameters").strong());
        ui.add_space(2.0);
        row(ui, "Compensation", |ui| {
            if let Some(value) = choice(ui, "contour_comp", self.params.cut.compensation, &Compensation::ALL, Compensation::label) {
                self.params.cut.compensation = value;
            }
        });
        ui.label(
            RichText::new(match self.params.cut.compensation {
                Compensation::Computer => "Computer: the toolpath is offset in the program, with no compensation codes.",
                Compensation::Control => "Control: the program names the chain and the machine applies the offset.",
                Compensation::Off => "Off: the tool centre follows the chain.",
            })
            .weak(),
        );
        row(ui, "Direction", |ui| {
            if let Some(value) = choice(ui, "contour_direction", self.params.cut.direction, &CutDirection::ALL, CutDirection::label) {
                self.params.cut.direction = value;
            }
        });
        ui.separator();
        ui.label(RichText::new("Stock to leave").strong());
        row(ui, "On walls", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.cut.stock_to_leave_walls).range(-100.0..=100.0).speed(0.05).suffix(" mm").fixed_decimals(2));
        });
        row(ui, "On floors", |ui| {
            ui.add(
                egui::DragValue::new(&mut self.params.cut.stock_to_leave_floors).range(-100.0..=100.0).speed(0.05).suffix(" mm").fixed_decimals(2),
            );
        });
        ui.label(RichText::new("A positive value leaves material; a negative value removes more.").weak());
    }

    /// The `Depth Cuts` page: the rough step, the finish cuts and whether the tool stays down.
    fn depth_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Depth cuts").strong());
        ui.add_space(2.0);
        row(ui, "Max rough step", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.depth_cuts.max_rough_step).range(0.01..=1000.0).speed(0.25).suffix(" mm").fixed_decimals(2));
        });
        ui.label(RichText::new("Equal rough cuts no larger than this divide the total depth.").weak());
        ui.separator();
        row(ui, "Finish cuts", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.depth_cuts.finish_cuts).range(0..=50).speed(0.2));
        });
        row(ui, "Finish step", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.depth_cuts.finish_step).range(0.0..=100.0).speed(0.05).suffix(" mm").fixed_decimals(2));
        });
        ui.label(RichText::new("Zero finish cuts leaves the rough step to reach the depth.").weak());
        ui.separator();
        ui.checkbox(&mut self.params.depth_cuts.keep_tool_down, "Keep tool down between depth cuts");
        row(ui, "Order", |ui| {
            if let Some(value) = choice(ui, "contour_depth_order", self.params.depth_cuts.order, &DepthOrder::ALL, DepthOrder::label) {
                self.params.depth_cuts.order = value;
            }
        });
    }

    /// The `Multi Passes` page: the rough and finish passes across the wall.
    fn multipass_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Multiple passes").strong());
        ui.label(RichText::new("Step out from the wall and repeat the finish, all at the same depth.").weak());
        ui.add_space(2.0);
        row(ui, "Rough passes", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.multipasses.rough_passes).range(0..=100).speed(0.2));
        });
        row(ui, "Rough spacing", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.multipasses.rough_spacing).range(0.0..=100.0).speed(0.05).suffix(" mm").fixed_decimals(2));
        });
        ui.separator();
        row(ui, "Finish passes", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.multipasses.finish_passes).range(0..=100).speed(0.2));
        });
        row(ui, "Finish spacing", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.multipasses.finish_spacing).range(0.0..=100.0).speed(0.05).suffix(" mm").fixed_decimals(2));
        });
        ui.separator();
        row(ui, "Spring passes", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.multipasses.spring_passes).range(0..=100).speed(0.2));
        });
        ui.checkbox(&mut self.params.multipasses.keep_tool_down, "Keep tool down between passes");
        ui.checkbox(&mut self.params.multipasses.finish_after_roughing, "Machine finish passes after roughing all contours");
    }

    /// The `Lead In/Out` page: how the tool enters and leaves the chain.
    fn lead_page(&mut self, ui: &mut egui::Ui) {
        self.lead_section(ui, "Entry", LeadSide::Entry);
        ui.separator();
        self.lead_section(ui, "Exit", LeadSide::Exit);
        ui.separator();
        ui.checkbox(&mut self.params.lead_in_out.entry_at_midpoint, "Enter/exit at midpoint in closed contours");
        ui.checkbox(&mut self.params.lead_in_out.exit_on_last_depth_cut_only, "Exit on last depth cut only");
    }

    /// One lead move's fields: the kind, then the size the kind uses.
    fn lead_section(&mut self, ui: &mut egui::Ui, title: &str, side: LeadSide) {
        let lead: &mut LeadMove = match side {
            LeadSide::Entry => &mut self.params.lead_in_out.entry,
            LeadSide::Exit => &mut self.params.lead_in_out.exit,
        };
        ui.label(RichText::new(title).strong());
        let id = match side {
            LeadSide::Entry => "contour_lead_entry",
            LeadSide::Exit => "contour_lead_exit",
        };
        row(ui, "Move", |ui| {
            if let Some(kind) = choice(ui, id, lead.kind, &LeadKind::ALL, LeadKind::label) {
                lead.kind = kind;
            }
        });
        match lead.kind {
            LeadKind::None => {
                ui.label(RichText::new("The tool plunges on the chain itself.").weak());
            }
            LeadKind::Line => {
                row(ui, "Line length", |ui| {
                    ui.add(egui::DragValue::new(&mut lead.line_length).range(0.0..=100.0).speed(0.1).suffix(" mm").fixed_decimals(2));
                });
            }
            LeadKind::Arc => {
                row(ui, "Arc radius", |ui| {
                    ui.add(egui::DragValue::new(&mut lead.arc_radius).range(0.0..=100.0).speed(0.1).suffix(" mm").fixed_decimals(2));
                });
                row(ui, "Sweep", |ui| {
                    ui.add(egui::DragValue::new(&mut lead.arc_sweep_degrees).range(1.0..=180.0).speed(0.5).suffix(" °").fixed_decimals(1));
                });
            }
        }
    }

    /// The `Linking Parameters` page: the heights the tool moves between.
    fn linking_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Linking parameters").strong());
        ui.add_space(2.0);
        row(ui, "Mode", |ui| {
            if let Some(value) = choice(ui, "contour_linking_mode", self.params.linking.mode, &LinkingMode::ALL, LinkingMode::label) {
                self.params.linking.mode = value;
            }
        });
        ui.label(
            RichText::new(match self.params.linking.mode {
                LinkingMode::Absolute => "Every height is an absolute Z coordinate.",
                LinkingMode::Incremental => "Every height is measured from the top of stock.",
            })
            .weak(),
        );
        ui.separator();
        row(ui, "Clearance", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.linking.clearance).range(-10_000.0..=10_000.0).speed(0.5).suffix(" mm").fixed_decimals(2));
        });
        row(ui, "Retract", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.linking.retract).range(-10_000.0..=10_000.0).speed(0.5).suffix(" mm").fixed_decimals(2));
        });
        row(ui, "Feed plane", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.linking.feed_plane).range(-10_000.0..=10_000.0).speed(0.5).suffix(" mm").fixed_decimals(2));
        });
        row(ui, "Top of stock", |ui| {
            ui.add(
                egui::DragValue::new(&mut self.params.linking.top_of_stock).range(-10_000.0..=10_000.0).speed(0.5).suffix(" mm").fixed_decimals(2),
            );
        });
        row(ui, "Depth", |ui| {
            ui.add(egui::DragValue::new(&mut self.params.linking.depth).range(-10_000.0..=10_000.0).speed(0.5).suffix(" mm").fixed_decimals(2));
        });
        ui.label(RichText::new("Clearance, retract, feed plane and top of stock stack in that order; the depth is below the stock.").weak());
    }
}

/// One row of the modal's lists: the current choice, the options and whether the user changed it.
fn choice<T: Copy + PartialEq>(ui: &mut egui::Ui, id: &str, current: T, all: &[T], label: impl Fn(T) -> &'static str) -> Option<T> {
    let mut chosen = current;
    egui::ComboBox::from_id_salt(id).selected_text(label(current)).width(220.0).show_ui(ui, |ui| {
        for option in all {
            ui.selectable_value(&mut chosen, *option, label(*option));
        }
    });
    if chosen == current { None } else { Some(chosen) }
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

    /// A document holding one extruded plate, the geometry a stock box is sized from.
    fn plate_document() -> CamDocument {
        use crate::document::WireTool;

        let mut document = CamDocument::default();
        document.draw(WireTool::Rectangle).unwrap_or_default();
        document.extrude(10.0).unwrap_or_default();
        document
    }

    #[test]
    fn the_stock_dialog_sizes_a_box_from_the_part_and_its_excess() {
        let document = plate_document();
        let mut dialog = StockSetupDialog::new(&document);
        assert_eq!(dialog.mode(), StockMode::BoundingBox);
        dialog.set_excess(StockExcess::new(5.0, 2.0, 1.0));
        let stock = dialog.accept(&document).unwrap_or_default();
        let extents = stock.and_then(StockDef::extents).unwrap_or_default();
        assert!((extents[0] - 5.0).abs() < 1e-4, "the box starts 5 mm before the part, got {extents:?}");
        assert!((extents[3] - 55.0).abs() < 1e-4, "and ends 5 mm past it");
        assert!((extents[4] - 42.0).abs() < 1e-4, "Y gets 2 mm on each side");
        assert!((extents[5] - 11.0).abs() < 1e-4, "Z gets 1 mm on each side");
    }

    #[test]
    fn a_box_that_has_no_material_left_is_refused_with_the_reason() {
        let document = plate_document();
        let mut dialog = StockSetupDialog::new(&document);
        dialog.set_excess(StockExcess::new(0.0, 0.0, -10.0));
        assert_eq!(dialog.accept(&document), Err(StockError::Extents));
    }

    #[test]
    fn a_document_without_bodies_cannot_size_a_box() {
        let document = CamDocument::default();
        let dialog = StockSetupDialog::new(&document);
        assert_eq!(dialog.accept(&document), Err(StockError::NoGeometry));
    }

    #[test]
    fn solid_mode_names_an_existing_body() {
        let mut document = plate_document();
        let id = document.solids()[0].id;
        let mut dialog = StockSetupDialog::new(&document);
        dialog.mode = StockMode::Solid;
        assert_eq!(dialog.body(), Some(id), "the first body is offered by default");
        assert_eq!(dialog.accept(&document), Ok(Some(StockDef::Solid(id))));
        dialog.body = None;
        assert_eq!(dialog.accept(&document), Err(StockError::Body), "a solid stock needs a body");
        dialog.select_body(&document, id);
        dialog.select_body(&document, 999);
        assert_eq!(dialog.body(), Some(id), "an unknown body is not selected");
        document.set_stock(Some(StockDef::Solid(id)));
        assert_eq!(StockSetupDialog::new(&document).mode(), StockMode::Solid, "a solid stock reopens in solid mode");
    }

    #[test]
    fn the_stock_dialog_draws_its_modes() {
        let document = plate_document();
        let ctx = egui::Context::default();
        let mut dialog = StockSetupDialog::new(&document);
        let mut primitives = Vec::new();
        for _ in 0..2 {
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |_ui| {
                assert_eq!(dialog.ui(&ctx, &document), StockOutcome::Open, "a frame with no click leaves the dialog open");
            });
            primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
            out.textures_delta.clear();
        }
        assert!(!primitives.is_empty(), "the dialog must draw something");
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

    #[test]
    fn the_contour_dialog_opens_on_geometry_and_refuses_an_unchained_operation() {
        let dialog = Contour2DDialog::new(Contour2DParams::new(Vec::new(), 1));
        assert_eq!(dialog.page(), ContourPage::Geometry);
        assert_eq!(dialog.accept(), Err(Contour2DError::EmptyChain));
        assert!(dialog.error().is_none(), "opening the dialog reports nothing until OK is pressed");
    }

    #[test]
    fn the_contour_dialog_accepts_the_pages_it_edited() {
        let mut dialog = Contour2DDialog::new(Contour2DParams::new(vec![2, 3], 4));
        dialog.params_mut().depth_cuts.max_rough_step = 2.5;
        dialog.params_mut().multipasses.rough_passes = 3;
        dialog.params_mut().lead_in_out.entry = LeadMove { kind: LeadKind::Arc, arc_radius: 4.0, arc_sweep_degrees: 90.0, ..LeadMove::default() };
        dialog.set_page(ContourPage::Linking);
        let accepted = dialog.accept().unwrap_or_default();
        assert_eq!(accepted.chain, [2, 3]);
        assert_eq!(accepted.tool_id, 4);
        assert!((accepted.depth_cuts.max_rough_step - 2.5).abs() < 1e-9);
        assert_eq!(accepted.multipasses.rough_passes, 3);
        assert_eq!(accepted.lead_in_out.entry.kind, LeadKind::Arc);
    }

    #[test]
    fn the_contour_dialog_lists_all_seven_parameter_pages() {
        assert_eq!(
            ContourPage::ALL.map(ContourPage::label),
            ["Geometry", "Tool", "Cut Parameters", "Depth Cuts", "Multi Passes", "Lead In/Out", "Linking"]
        );
    }

    #[test]
    fn the_contour_dialog_draws_every_page() {
        let document = plate_document();
        let tools = ToolLibrary::with_presets();
        let ctx = egui::Context::default();
        let chain: Vec<usize> = document.chain_entities().iter().map(|entity| entity.id).collect();
        let mut dialog = Contour2DDialog::new(Contour2DParams::new(chain, 1));
        for page in ContourPage::ALL {
            dialog.set_page(page);
            // The first frame lays the modal's area out; the next one paints it.
            let mut primitives = Vec::new();
            for _ in 0..2 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |_ui| {
                    assert_eq!(
                        dialog.ui(&ctx, &document, &tools),
                        Contour2DOutcome::Open,
                        "a frame with no click leaves the {} page open",
                        page.label()
                    );
                });
                primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
                out.textures_delta.clear();
            }
            assert!(!primitives.is_empty(), "the {} page must draw something", page.label());
        }
    }
}

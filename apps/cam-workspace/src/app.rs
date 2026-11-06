//! The CAM workspace: application state and the frame loop.
//!
//! The window is a workspace rather than a black screen: the ribbon and the status bar claim
//! their space first, then the unified left manager panels the model, and the central panel keeps
//! whatever remains for the 3D viewport. The document under all of them is the same one the
//! ribbon's tools edit and the manager lists, so a shape drawn on the active level shows up in
//! the viewport and its solid shows up in the manager at once.

use egui::{Align, Color32, Layout};
use solidcraft_cam::operations::{Contour2DError, Contour2DParams, Operation};
use solidcraft_cam::simulation::Simulation;
use solidcraft_cam::stock::StockDef;
use solidcraft_cam::tool::ToolLibrary;
use solidcraft_cam::toolpath::Toolpath;

use crate::dialogs_cam::{Contour2DDialog, Contour2DOutcome, StockOutcome, StockSetupDialog, ToolLibraryDialog, ToolLibraryOutcome};
use crate::document::{CamDocument, ImportSummary, SolidTool, WireTool, standard_plane};
use crate::files::{self, Picked};
use crate::interactive_geom::PendingWire;
use crate::left_manager::{LeftManager, ManagerAction};
use crate::ribbon::{Command, ImportKind, Ribbon, RibbonTab, SimulationCommand, ViewCommand};
use crate::simulation_panel::{self, SimulationAction};
use crate::viewport::Viewport;

pub use crate::ribbon::{RIBBON_HEIGHT, RIBBON_TAB_HEIGHT};

/// Background of the 3D viewport (light grey-blue), so the region reserved for graphics reads as
/// workspace instead of unrendered black.
pub const VIEWPORT_BACKGROUND: Color32 = Color32::from_rgb(216, 220, 227);

/// Height of the status bar, in points: one line of coordinates.
pub const STATUS_BAR_HEIGHT: f32 = 24.0;

/// Background of the status bar: the deep blue that separates it from the workspace.
pub const STATUS_BAR_BACKGROUND: Color32 = Color32::from_rgb(45, 80, 140);

/// Text of the status bar: white, for contrast on [`STATUS_BAR_BACKGROUND`].
pub const STATUS_BAR_TEXT: Color32 = Color32::WHITE;

/// Padding and spacing used throughout the workspace, in points: tight, so the manager and the
/// viewport get the screen real estate.
pub const PADDING: f32 = 4.0;

/// Width of the left manager panel, in points, until the user drags its edge.
pub const LEFT_PANEL_WIDTH: f32 = 320.0;

/// Narrowest the left manager panel may be dragged, in points: any less and the level columns
/// stop being readable.
pub const LEFT_PANEL_MIN_WIDTH: f32 = 250.0;

/// Widest the left manager panel may be dragged, in points: beyond this it would crowd out the
/// viewport rather than serve it.
pub const LEFT_PANEL_MAX_WIDTH: f32 = 500.0;

/// What the status bar says before the first command.
pub const READY_STATUS: &str = "Ready";

/// The CAM workspace application.
#[derive(Debug, Default)]
pub struct CamWorkspaceApp {
    /// The ribbon, and the tab whose tools the command area shows.
    pub ribbon: Ribbon,
    /// The unified left manager: levels, toolpaths, planes and solids.
    pub manager: LeftManager,
    /// The document the ribbon edits and the manager lists.
    pub document: CamDocument,
    /// The 3D viewport: the camera, the scene it draws and its overlays.
    pub viewport: Viewport,
    /// The tool library the operations select from, starting with the preset catalogue.
    pub tools: ToolLibrary,
    /// The last command's result, shown in the status bar.
    status: String,
    /// The import dialog, while one is open.
    import: Option<ImportDialog>,
    /// The Tool Library dialog, while one is open.
    tool_library: Option<ToolLibraryDialog>,
    /// The Stock Setup dialog, while one is open.
    stock_setup: Option<StockSetupDialog>,
    /// The 2D Contour dialog, while one is open.
    contour2d: Option<Contour2DDialog>,
    /// The operations the workspace has defined, each with its calculated toolpath.
    operations: Vec<Operation>,
    /// The simulation of one of those operations: the playback clock and the stock it has cut.
    pub simulation: Simulation,
    /// The index in `operations` the simulation runs.
    simulation_operation: usize,
}

/// The import dialog: a path field and the button that reads the file into the active level.
#[derive(Clone, Debug)]
struct ImportDialog {
    /// The format being imported, which fixes the title and the expected extension.
    kind: ImportKind,
    /// The file path as typed (a browser has no path to type: it shows its own dialog).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    path: String,
    /// Why the last attempt failed, if it did.
    error: Option<String>,
}

impl ImportDialog {
    /// A dialog for `kind`, with an empty path.
    fn new(kind: ImportKind) -> Self {
        Self { kind, path: String::new(), error: None }
    }

    /// Reads the file named in the path field and imports it into the document's active level.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn run(&self, document: &mut CamDocument) -> Result<String, String> {
        let path = self.path.trim();
        if path.is_empty() {
            return Err(format!("Type the path to a .{} file", self.kind.extension()));
        }
        let bytes = std::fs::read(path).map_err(|error| format!("{path}: {error}"))?;
        let name = std::path::Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path);
        self.import_bytes(document, name, &bytes)
    }

    /// Imports bytes a file dialog read, under the name it reports, into the active level.
    fn import_bytes(&self, document: &mut CamDocument, name: &str, bytes: &[u8]) -> Result<String, String> {
        let text = String::from_utf8_lossy(bytes);
        let summary = match self.kind {
            ImportKind::Step => document.import_step(&text, name),
            ImportKind::Dxf => document.import_dxf(&text, name),
        }
        .map_err(|error| error.to_string())?;
        Ok(import_message(name, summary))
    }
}

/// The status line an import leaves behind.
fn import_message(name: &str, summary: ImportSummary) -> String {
    let noun = if summary.added == 1 { "entity" } else { "entities" };
    let skipped = if summary.skipped == 0 { String::new() } else { format!(", {} skipped", summary.skipped) };
    format!("Imported {} {noun} from {name}{skipped}", summary.added)
}

impl CamWorkspaceApp {
    /// Creates the app from an eframe creation context.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::with_context(&cc.egui_ctx)
    }

    /// Creates the app against any egui context: the window's, or a headless one in a test.
    pub fn with_context(ctx: &egui::Context) -> Self {
        install_style(ctx);
        Self { status: READY_STATUS.to_string(), tools: ToolLibrary::with_presets(), ..Self::default() }
    }

    /// The last command's result, as the status bar shows it.
    pub fn status(&self) -> &str {
        self.status.as_str()
    }

    /// Whether the import dialog is open.
    pub fn import_open(&self) -> bool {
        self.import.is_some()
    }

    /// Whether the Tool Library dialog is open.
    pub fn tool_library_open(&self) -> bool {
        self.tool_library.is_some()
    }

    /// The Tool Library dialog, while one is open.
    pub fn tool_library_dialog(&self) -> Option<&ToolLibraryDialog> {
        self.tool_library.as_ref()
    }

    /// The Tool Library dialog, to drive its working copy from a test.
    pub fn tool_library_dialog_mut(&mut self) -> Option<&mut ToolLibraryDialog> {
        self.tool_library.as_mut()
    }

    /// Stores a library the dialog accepted, closes it and reports the new size.
    pub fn accept_tool_library(&mut self, library: ToolLibrary) {
        let count = library.len();
        let noun = if count == 1 { "assembly" } else { "assemblies" };
        self.status = format!("Tool Library: {count} tool {noun}");
        self.tools = library;
        self.tool_library = None;
    }

    /// Closes the Tool Library dialog without changing the library.
    pub fn cancel_tool_library(&mut self) {
        self.tool_library = None;
    }

    /// Whether the Stock Setup dialog is open.
    pub fn stock_setup_open(&self) -> bool {
        self.stock_setup.is_some()
    }

    /// The Stock Setup dialog, while one is open.
    pub fn stock_setup_dialog(&self) -> Option<&StockSetupDialog> {
        self.stock_setup.as_ref()
    }

    /// The Stock Setup dialog, to drive its settings from a test.
    pub fn stock_setup_dialog_mut(&mut self) -> Option<&mut StockSetupDialog> {
        self.stock_setup.as_mut()
    }

    /// Stores a stock definition the dialog accepted (`None`: no stock), closes it and reports
    /// what the viewport now draws.
    pub fn accept_stock(&mut self, stock: Option<StockDef>) {
        self.document.set_stock(stock);
        self.status = match stock {
            None => "Stock: removed".to_string(),
            Some(StockDef::Solid(id)) => {
                let name = self.document.solid_by_id(id).map_or_else(|| format!("solid {id}"), |solid| solid.name.clone());
                format!("Stock: {name}")
            }
            Some(definition) => format!("Stock: {}", definition.summary()),
        };
        self.stock_setup = None;
        // The simulation's height map is sampled over the stock box, so it follows the change.
        self.rebuild_simulation();
    }

    /// Closes the Stock Setup dialog without changing the stock.
    pub fn cancel_stock_setup(&mut self) {
        self.stock_setup = None;
    }

    /// Whether the 2D Contour dialog is open.
    pub fn contour2d_open(&self) -> bool {
        self.contour2d.is_some()
    }

    /// The 2D Contour dialog, while one is open.
    pub fn contour2d_dialog(&self) -> Option<&Contour2DDialog> {
        self.contour2d.as_ref()
    }

    /// The 2D Contour dialog, to drive its pages from a test.
    pub fn contour2d_dialog_mut(&mut self) -> Option<&mut Contour2DDialog> {
        self.contour2d.as_mut()
    }

    /// The operations the workspace has defined.
    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    /// Stores a 2D Contour operation the dialog accepted, calculates its toolpath, closes the
    /// dialog and reports the result in the status bar.
    ///
    /// A path that cannot be calculated is still stored, with no moves, so the parameters the
    /// user typed are not thrown away; the status bar names the value that stopped it.
    pub fn accept_contour2d(&mut self, params: Contour2DParams) {
        let tool = self.tools.get(params.tool_id).cloned();
        let chain = self.document.chain_curves(&params.chain);
        let mut operation = Operation::new(params);
        let result = match &tool {
            Some(assembly) => operation.generate(&chain, &assembly.tool),
            None => Err(Contour2DError::NoTool),
        };
        let name = tool.as_ref().map_or_else(|| format!("tool {}", operation.params.tool_id), |assembly| assembly.name.clone());
        self.status = match result {
            Ok(()) => format!("{name} · {} · {}", operation.summary(), operation.toolpath.summary()),
            Err(error) => format!("2D Contour stored, not generated: {error}"),
        };
        self.operations.push(operation);
        self.contour2d = None;
        // The new operation is what the Simulation tab verifies.
        self.simulation_operation = self.operations.len().saturating_sub(1);
        self.rebuild_simulation();
    }

    /// Recalculates the toolpath of the operation at `index` from the document's geometry and
    /// the tool library, and reports what it cut.
    pub fn regenerate_operation(&mut self, index: usize) -> bool {
        let Some(operation) = self.operations.get(index) else {
            self.status = "No such operation to regenerate".to_string();
            return false;
        };
        let chain = self.document.chain_curves(&operation.params.chain);
        let tool = self.tools.get(operation.params.tool_id).map(|assembly| assembly.tool.clone());
        let summary = operation.summary();
        let result = match tool {
            Some(tool) => self.operations.get_mut(index).map(|operation| operation.generate(&chain, &tool)),
            None => Some(Err(Contour2DError::NoTool)),
        };
        let regenerated = match result {
            Some(Ok(())) => {
                let path = self.operations.get(index).map(|operation| operation.toolpath.summary()).unwrap_or_default();
                self.status = format!("{summary}: regenerated · {path}");
                true
            }
            Some(Err(error)) => {
                self.status = format!("{summary}: not regenerated · {error}");
                false
            }
            None => false,
        };
        // The simulation plays the moves the operation now holds, from the start.
        self.rebuild_simulation();
        regenerated
    }

    /// Shows or hides the toolpath of the operation at `index`.
    pub fn toggle_operation_visibility(&mut self, index: usize) {
        let Some(operation) = self.operations.get_mut(index) else {
            return;
        };
        let visible = !operation.visible;
        operation.set_visible(visible);
        let state = if visible { "shown" } else { "hidden" };
        self.status = format!("{}: toolpath {state}", operation.summary());
    }

    /// Closes the 2D Contour dialog without storing an operation.
    pub fn cancel_contour2d(&mut self) {
        self.contour2d = None;
    }

    /// The simulation of the selected operation: its playback clock and the stock it has cut.
    pub fn simulation(&self) -> &Simulation {
        &self.simulation
    }

    /// The index in [`Self::operations`] the simulation runs.
    pub fn simulation_operation(&self) -> usize {
        self.simulation_operation
    }

    /// Whether the playback panel is showing: on the `Simulation` tab, or while the clock runs.
    pub fn simulation_panel_visible(&self) -> bool {
        self.ribbon.tab == RibbonTab::Simulation || self.simulation.playing()
    }

    /// Starts the playback clock, replaying a program that had already finished.
    pub fn play_simulation(&mut self) {
        if self.operations.is_empty() {
            self.status = "Simulation: no toolpath to run".to_string();
            return;
        }
        self.simulation.play();
        self.status = format!("Simulation: playing · {}", self.simulation.summary());
    }

    /// Stops the playback clock where it is.
    pub fn pause_simulation(&mut self) {
        self.simulation.pause();
        self.status = format!("Simulation: paused · {}", self.simulation.summary());
    }

    /// Starts the clock if it is stopped, stops it if it is running.
    pub fn toggle_simulation(&mut self) {
        if self.simulation.playing() {
            self.pause_simulation();
        } else {
            self.play_simulation();
        }
    }

    /// Advances the tool to the end of the next move and stops there.
    pub fn step_simulation(&mut self) {
        if self.operations.is_empty() {
            self.status = "Simulation: no toolpath to run".to_string();
            return;
        }
        self.simulation.step();
        self.status = format!("Simulation: step · {}", self.simulation.summary());
    }

    /// Runs the program back to its start and restores the stock to raw material.
    pub fn reset_simulation(&mut self) {
        self.simulation.reset();
        self.status = format!("Simulation: reset · {}", self.simulation.summary());
    }

    /// Puts the tool `position` millimetres along the path: what the timeline's scrub does.
    pub fn scrub_simulation(&mut self, position: f64) {
        self.simulation.set_position(position);
        self.status = format!("Simulation: {}", self.simulation.summary());
    }

    /// Moves a playing simulation on by `seconds` of real time at its playback speed.
    pub fn advance_simulation(&mut self, seconds: f64) {
        self.simulation.advance(seconds);
    }

    /// Picks the operation the simulation runs and rebuilds it at the program's start.
    pub fn select_simulation_operation(&mut self, index: usize) {
        let Some(operation) = self.operations.get(index) else {
            self.status = "No such operation to simulate".to_string();
            return;
        };
        self.simulation_operation = index;
        let summary = operation.summary();
        self.rebuild_simulation();
        self.status = format!("Simulation: {summary}");
    }

    /// Rebuilds the simulation from the selected operation, its tool and the stock box, stopped at
    /// the start: what a new operation, a regenerated path or a changed stock asks for.
    pub fn rebuild_simulation(&mut self) {
        self.simulation = self.build_simulation(self.simulation_operation);
    }

    /// The simulation of the operation at `index`, cut by the tool the library holds for it out of
    /// the document's stock.
    fn build_simulation(&self, index: usize) -> Simulation {
        let (toolpath, tool) = match self.operations.get(index) {
            Some(operation) => {
                let tool = self.tools.get(operation.params.tool_id).map(|assembly| assembly.tool.clone());
                (operation.toolpath.clone(), tool)
            }
            None => (Toolpath::new(), None),
        };
        Simulation::new(&toolpath, tool.as_ref(), self.document.stock_extents())
    }

    /// Runs a `Simulation` tab button's command.
    fn run_simulation_command(&mut self, command: SimulationCommand) {
        match command {
            SimulationCommand::Play => self.play_simulation(),
            SimulationCommand::Pause => self.pause_simulation(),
            SimulationCommand::Step => self.step_simulation(),
            SimulationCommand::Reset => self.reset_simulation(),
        }
    }

    /// Runs what the playback panel asked for, after the panel has been drawn.
    fn run_simulation_action(&mut self, action: SimulationAction) {
        match action {
            SimulationAction::PlayPause => self.toggle_simulation(),
            SimulationAction::Step => self.step_simulation(),
            SimulationAction::Reset => self.reset_simulation(),
            SimulationAction::Scrub(position) => self.scrub_simulation(position),
            SimulationAction::SelectOperation(index) => self.select_simulation_operation(index),
        }
    }

    /// Advances a playing simulation by the frame's elapsed time, and asks for the next frame
    /// while it runs: the tool crosses the part at the rates the simulation times its moves at.
    fn tick_simulation(&mut self, ctx: &egui::Context) {
        if !self.simulation.playing() {
            return;
        }
        let dt = f64::from(ctx.input(|input| input.stable_dt).clamp(0.0, 0.1));
        self.simulation.advance(dt);
        if self.simulation.playing() {
            ctx.request_repaint();
        } else {
            self.status = format!("Simulation: finished · {}", self.simulation.summary());
        }
    }

    /// Runs the command a ribbon button asked for, and reports the result in the status bar.
    pub fn run(&mut self, command: Command) {
        match command {
            Command::Draw(tool) => self.run_draw(tool),
            Command::Solid(tool) => self.run_solid(tool),
            Command::Import(kind) => self.import = Some(ImportDialog::new(kind)),
            Command::ToolLibrary => self.open_tool_library(),
            Command::Contour2D => self.open_contour2d(),
            Command::StockSetup => self.open_stock_setup(),
            Command::Simulation(action) => self.run_simulation_command(action),
            Command::View(view) => {
                self.apply_view(view);
                self.status = format!("{} view", view.label());
            }
        }
    }

    /// Opens the 2D Contour dialog on a new operation: the first tool in the library, and no
    /// geometry chained until the user picks it.
    fn open_contour2d(&mut self) {
        let tool_id = self.tools.assemblies().first().map_or(0, |assembly| assembly.id);
        self.status = "2D Contour: chain geometry and choose a tool".to_string();
        self.contour2d = Some(Contour2DDialog::new(Contour2DParams::new(Vec::new(), tool_id)));
    }

    /// Runs what a left manager view asked for, after the panel has been drawn.
    fn run_manager(&mut self, action: ManagerAction) {
        match action {
            ManagerAction::NewContour2D => self.open_contour2d(),
            ManagerAction::RegenerateOperation(index) => {
                self.regenerate_operation(index);
            }
            ManagerAction::ToggleOperationVisibility(index) => self.toggle_operation_visibility(index),
            ManagerAction::CommitWire => match self.viewport.commit_cursor() {
                Some(wire) => self.commit_wire(wire),
                None => self.status = "Create: point at a position in the viewport first".to_string(),
            },
            ManagerAction::CancelWireTool => {
                self.viewport.cancel_wireframe_tool();
                self.status = "Tool cancelled".to_string();
            }
        }
    }

    /// Opens the Tool Library dialog on a copy of the library.
    fn open_tool_library(&mut self) {
        let count = self.tools.len();
        let noun = if count == 1 { "assembly" } else { "assemblies" };
        self.status = format!("Tool Library: {count} tool {noun}");
        self.tool_library = Some(ToolLibraryDialog::new(self.tools.clone()));
    }

    /// Opens the Stock Setup dialog over the document's bodies.
    fn open_stock_setup(&mut self) {
        let stock = self.document.stock().map_or_else(|| "no stock".to_string(), StockDef::summary);
        self.status = format!("Stock Setup: {stock}");
        self.stock_setup = Some(StockSetupDialog::new(&self.document));
    }

    /// Starts a clicking wireframe tool on the active construction plane, and reports that the
    /// next clicks place it; the fixed-size tools draw at once.
    fn run_draw(&mut self, tool: WireTool) {
        let plane = standard_plane(self.document.active_plane());
        if self.viewport.begin_wireframe_tool(tool, plane) {
            self.status = format!("{}: click points on the {} plane, or type them in the tool panel", tool.label(), self.document.active_plane_name());
            return;
        }
        let level = self.document.active_level();
        match self.document.draw(tool) {
            Ok(added) => {
                let noun = if added == 1 { "entity" } else { "entities" };
                self.status = format!("{}: added {added} {noun} to level {level}", tool.label());
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    /// Creates a wireframe entity the interactive tool completed on the active level, and
    /// reports what landed.
    fn commit_wire(&mut self, wire: PendingWire) {
        let level = self.document.active_level();
        match self.document.create_wire(wire) {
            Ok(added) => {
                let noun = if added == 1 { "entity" } else { "entities" };
                self.status = format!("{}: added {added} {noun} to level {level}", wire.label());
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    /// Models a solid from the active level's wireframe and reports what landed.
    fn run_solid(&mut self, tool: SolidTool) {
        let level = self.document.active_level();
        match self.document.solid(tool) {
            Ok(count) => {
                let noun = if count == 1 { "body" } else { "bodies" };
                self.status = format!("{}: {count} {noun} on level {level}", tool.label());
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    /// Moves the camera to a standard view.
    fn apply_view(&mut self, view: ViewCommand) {
        self.viewport.set_view(view);
    }

    /// The status bar: the active level, the last command and the construction plane.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let level = self.document.active_level_name().to_string();
        let plane = self.document.active_plane_name();
        let solids = self.document.solids().len();
        let tools = self.tools.len();
        let operations = self.operations.len();
        let status = self.status.clone();
        let frame = egui::Frame::new().fill(STATUS_BAR_BACKGROUND).inner_margin(egui::Margin::symmetric(8, 2));
        egui::Panel::bottom("status_bar").exact_size(STATUS_BAR_HEIGHT).frame(frame).show(ui, |ui| {
            ui.visuals_mut().override_text_color = Some(STATUS_BAR_TEXT);
            ui.horizontal_centered(|ui| {
                // Left: the creation level and what the document holds.
                ui.label(format!("Level: {level}"));
                ui.separator();
                ui.label(format!("{solids} solid(s)"));
                ui.separator();
                ui.label(format!("{tools} tool(s)"));
                ui.separator();
                ui.label(format!("{operations} toolpath(s)"));
                ui.separator();
                ui.label(status);
                // Right: the construction plane new geometry is drawn on.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(format!("CPLANE: {plane}"));
                });
            });
        });
    }

    /// The left manager panel: one tab strip over the four views, resizable within limits that
    /// keep both the level columns and the viewport usable.
    fn manager_panel(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        egui::Panel::left("left_manager_panel")
            .resizable(true)
            .default_size(LEFT_PANEL_WIDTH)
            .min_size(LEFT_PANEL_MIN_WIDTH)
            .max_size(LEFT_PANEL_MAX_WIDTH)
            .show(ui, |ui| {
                action = self.manager.ui(ui, &mut self.document, &self.operations, &self.tools, &mut self.viewport.interactive);
            });
        if let Some(action) = action {
            self.run_manager(action);
        }
    }

    /// The playback panel above the status bar, while the `Simulation` tab is open or the clock
    /// runs: the operation being verified, Play/Step/Reset and the timeline's scrub.
    fn simulation_panel(&mut self, ui: &mut egui::Ui) {
        if !self.simulation_panel_visible() {
            return;
        }
        let mut action = None;
        egui::Panel::bottom("simulation_playback").exact_size(simulation_panel::SIMULATION_PANEL_HEIGHT).show(ui, |ui| {
            action = simulation_panel::ui(ui, &self.simulation, &self.operations, self.simulation_operation);
        });
        if let Some(action) = action {
            self.run_simulation_action(action);
        }
    }

    /// The import dialog, while one is open: it is a modal window over the viewport.
    fn import_dialog(&mut self, ui: &mut egui::Ui) {
        // The browser answers its file dialog asynchronously: apply the answer on the frame it
        // arrives, before the dialog draws the result.
        if let Some(picked) = files::take() {
            self.answer_import(picked);
        }
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let Some(mut dialog) = self.import.take() else { return };
        let mut close = false;
        #[cfg(not(target_arch = "wasm32"))]
        let mut run = false;
        #[cfg(target_arch = "wasm32")]
        let mut browse = false;
        let mut still_open = true;
        egui::Window::new(dialog.kind.title()).collapsible(false).resizable(false).open(&mut still_open).show(ui.ctx(), |ui| {
            #[cfg(not(target_arch = "wasm32"))]
            {
                ui.label(format!("Path to a .{} file:", dialog.kind.extension()));
                let hint = format!("/path/to/part.{}", dialog.kind.extension());
                ui.add(egui::TextEdit::singleline(&mut dialog.path).desired_width(360.0).hint_text(hint));
            }
            #[cfg(target_arch = "wasm32")]
            {
                // A browser has no paths to type: it shows its own file dialog instead.
                ui.label(format!("Choose a .{} file from this computer:", dialog.kind.extension()));
            }
            ui.horizontal(|ui| {
                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Import").clicked() {
                    run = true;
                }
                #[cfg(target_arch = "wasm32")]
                if ui.button("Import…").clicked() {
                    browse = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
            if let Some(error) = &dialog.error {
                ui.colored_label(Color32::from_rgb(180, 40, 40), error);
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        if run {
            match dialog.run(&mut self.document) {
                Ok(message) => {
                    self.status = message;
                    close = true;
                }
                Err(error) => dialog.error = Some(error),
            }
        }
        #[cfg(target_arch = "wasm32")]
        if browse {
            // Only the dialog's own answer belongs to it: drop one left by a dismissed dialog.
            files::discard();
            files::pick(dialog.kind, ui.ctx());
        }
        if !still_open {
            close = true;
        }
        if !close {
            self.import = Some(dialog);
        }
    }

    /// Applies the answer of a file dialog to the import dialog that asked for it: the file is
    /// imported and the dialog closes, or the dialog stays open and shows why it could not be.
    fn answer_import(&mut self, picked: Picked) {
        let Some(mut dialog) = self.import.take() else { return };
        match picked {
            Picked::File(file) => match dialog.import_bytes(&mut self.document, &file.name, &file.bytes) {
                Ok(message) => self.status = message,
                Err(error) => {
                    dialog.error = Some(error);
                    self.import = Some(dialog);
                }
            },
            Picked::Cancelled => self.import = Some(dialog),
            Picked::Failed(error) => {
                dialog.error = Some(error);
                self.import = Some(dialog);
            }
        }
    }

    /// The Stock Setup dialog, while one is open: it is a modal window over the viewport.
    fn show_stock_setup(&mut self, ui: &mut egui::Ui) {
        let Some(mut dialog) = self.stock_setup.take() else { return };
        match dialog.ui(ui.ctx(), &self.document) {
            StockOutcome::Open => self.stock_setup = Some(dialog),
            StockOutcome::Apply(stock) => self.accept_stock(stock),
            StockOutcome::Cancel => (),
        }
    }

    /// The Tool Library dialog, while one is open: it is a modal window over the viewport.
    fn show_tool_library(&mut self, ui: &mut egui::Ui) {
        let Some(mut dialog) = self.tool_library.take() else { return };
        match dialog.ui(ui.ctx()) {
            ToolLibraryOutcome::Open => self.tool_library = Some(dialog),
            ToolLibraryOutcome::Accept(library) => self.accept_tool_library(library),
            ToolLibraryOutcome::Cancel => (),
        }
    }

    /// The 2D Contour dialog, while one is open: it is a modal window over the viewport.
    fn show_contour2d(&mut self, ui: &mut egui::Ui) {
        let Some(mut dialog) = self.contour2d.take() else { return };
        match dialog.ui(ui.ctx(), &self.document, &self.tools) {
            Contour2DOutcome::Open => self.contour2d = Some(dialog),
            Contour2DOutcome::Accept(params) => self.accept_contour2d(*params),
            Contour2DOutcome::Cancel => (),
        }
    }

    /// Draws one frame of the workspace into `ui`: the panels first, the viewport last.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        // A playing simulation moves with the frame clock before anything is drawn, so the panels
        // and the viewport show the same moment.
        self.tick_simulation(ui.ctx());
        // The ribbon runs its button's command after it has drawn, so the panel is not borrowed
        // while the document changes.
        if let Some(command) = self.ribbon.ui(ui) {
            self.run(command);
        }
        self.status_bar(ui);
        self.simulation_panel(ui);
        self.manager_panel(ui);

        // Last, so that it fills what the panels leave over: the live 3D viewport.
        let mut completed = Vec::new();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(VIEWPORT_BACKGROUND)).show(ui, |ui| {
            completed = self.viewport.ui(ui, &self.document, &self.operations, Some(&self.simulation));
        });
        // A click that finished a wireframe entity creates it after the frame is drawn, so the
        // document is not borrowed while the viewport paints it.
        for wire in completed {
            self.commit_wire(wire);
        }
        self.import_dialog(ui);
        self.show_tool_library(ui);
        self.show_stock_setup(ui);
        self.show_contour2d(ui);
    }
}

/// Installs the window's look: the light theme, tight spacing and the manager's active-row
/// highlight. Split from [`CamWorkspaceApp::new`] so a headless renderer can use the same look.
pub fn install_style(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Light);
    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(PADDING, PADDING);
        style.spacing.button_padding = egui::vec2(PADDING, PADDING);
        style.visuals.selection.bg_fill = crate::left_manager::ACTIVE_HIGHLIGHT_LIGHT;
        style.visuals.selection.stroke = egui::Stroke::new(1.0, crate::left_manager::ACTIVE_HIGHLIGHT_TEXT);
    });
}

impl ViewCommand {
    /// The status-bar name of a standard view.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Top => "Top",
            Self::Front => "Front",
            Self::Right => "Right",
            Self::Iso => "Isometric",
        }
    }
}

impl eframe::App for CamWorkspaceApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::left_manager::ManagerTab;

    #[test]
    fn a_click_that_finishes_a_line_creates_it_in_the_document() {
        use solidcore_record::Vec3;

        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.run(Command::Draw(WireTool::Line));
        assert!(app.viewport.interactive.is_active(), "the Line tool waits for clicks instead of drawing at once");
        assert!(app.status().starts_with("Line: click points"), "status: {}", app.status());
        app.viewport.interactive.track_point(Vec3::ZERO);
        assert!(app.viewport.place_point().is_none(), "the first click anchors the line");
        app.viewport.interactive.track_point(Vec3::new(30.0, 0.0, 0.0));
        let wire = app.viewport.place_point().expect("the second click completes the line");
        app.commit_wire(wire);
        assert_eq!(app.status(), "Line: added 1 entity to level 1");
        assert_eq!(app.document.chain_entities().len(), 1, "the line is real document geometry");
        assert!(app.viewport.interactive.is_active(), "the tool stays live for the next line");
    }

    #[test]
    fn a_command_reports_its_result_in_the_status_bar() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        assert_eq!(app.status(), READY_STATUS);
        app.run(Command::Draw(WireTool::Rectangle));
        assert_eq!(app.status(), "Rectangle: added 4 entities to level 1");
        assert_eq!(app.document.levels()[0].entity_count, 4);
        app.run(Command::Solid(SolidTool::Extrude));
        assert_eq!(app.status(), "Extrude: 1 body on level 1");
        assert_eq!(app.document.solids().len(), 1);
    }

    #[test]
    fn a_failing_command_explains_itself_instead_of_doing_nothing() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.run(Command::Solid(SolidTool::Extrude));
        assert!(app.status().contains("no wireframe"), "status: {}", app.status());
        assert!(app.document.solids().is_empty());
    }

    #[test]
    fn the_import_command_opens_the_dialog() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.run(Command::Import(ImportKind::Step));
        assert!(app.import_open());
    }

    /// One DXF line and one DXF circle, the file a browser's dialog would hand back as bytes.
    const DXF_BYTES: &[u8] = b"0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n40\n21\n0\n0\nCIRCLE\n10\n20\n20\n15\n40\n10\n0\nENDSEC\n0\nEOF\n";

    #[test]
    fn a_file_the_dialog_read_is_imported_into_the_active_level() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.run(Command::Import(ImportKind::Dxf));
        app.answer_import(Picked::File(files::PickedFile { name: "plate.dxf".into(), bytes: DXF_BYTES.to_vec() }));
        assert!(!app.import_open(), "the dialog closes once its file is imported");
        assert_eq!(app.status(), "Imported 2 entities from plate.dxf");
        assert_eq!(app.document.level(1).map(|level| level.entity_count), Some(2), "the entities land on the active level");
    }

    #[test]
    fn a_file_the_dialog_could_not_read_keeps_the_dialog_open_with_the_reason() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.run(Command::Import(ImportKind::Dxf));
        app.answer_import(Picked::Failed("plate.dxf: the browser could not read the file".into()));
        assert!(app.import_open(), "the dialog stays open so the reason is visible");
        assert_eq!(app.document.level(1).map(|level| level.entity_count), Some(0));
        // Dismissing the browser's dialog changes nothing either.
        app.answer_import(Picked::Cancelled);
        assert!(app.import_open());
    }

    #[test]
    fn a_new_workspace_starts_with_the_preset_tool_library() {
        let ctx = egui::Context::default();
        let app = CamWorkspaceApp::with_context(&ctx);
        assert!(!app.tools.is_empty(), "the tool library starts on the preset catalogue");
        assert_eq!(app.tools.validate(), Ok(()));
    }

    #[test]
    fn the_tool_library_command_opens_the_dialog_on_a_copy() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        let before = app.tools.clone();
        app.run(Command::ToolLibrary);
        assert!(app.tool_library_open());
        assert_eq!(app.tools, before, "opening the dialog changes nothing");
        assert!(app.status().starts_with("Tool Library:"), "status: {}", app.status());
        app.cancel_tool_library();
        assert!(!app.tool_library_open());
        assert_eq!(app.tools, before, "cancelling keeps the library");
    }

    #[test]
    fn accepting_the_dialog_stores_its_tools_and_reports_the_count() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        let before = app.tools.len();
        app.run(Command::ToolLibrary);
        let added = app.tool_library_dialog_mut().map(crate::dialogs_cam::ToolLibraryDialog::add_tool);
        assert_eq!(added, Some(before + 1), "a new tool takes the next id");
        let accepted = app.tool_library_dialog().and_then(|dialog| dialog.accept().ok());
        assert_eq!(accepted.as_ref().map(ToolLibrary::len), Some(before + 1), "the edited library is valid");
        if let Some(library) = accepted {
            app.accept_tool_library(library);
        }
        assert_eq!(app.tools.len(), before + 1);
        assert!(!app.tool_library_open(), "OK closes the dialog");
        assert_eq!(app.status(), format!("Tool Library: {} tool assemblies", before + 1));
    }

    #[test]
    fn the_stock_setup_command_opens_the_dialog() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.run(Command::StockSetup);
        assert!(app.stock_setup_open());
        assert_eq!(app.status(), "Stock Setup: no stock");
        app.cancel_stock_setup();
        assert!(!app.stock_setup_open());
        assert_eq!(app.document.stock(), None, "cancelling keeps the document");
    }

    #[test]
    fn accepting_the_stock_dialog_stores_the_material() {
        use solidcraft_cam::stock::StockExcess;

        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.document.draw(WireTool::Rectangle).unwrap_or_default();
        app.document.extrude(10.0).unwrap_or_default();
        app.run(Command::StockSetup);
        if let Some(dialog) = app.stock_setup_dialog_mut() {
            dialog.set_excess(StockExcess::new(5.0, 2.0, 1.0));
        }
        let stock = app.stock_setup_dialog().and_then(|dialog| dialog.accept(&app.document).ok()).flatten();
        app.accept_stock(stock);
        assert!(!app.stock_setup_open());
        assert!(matches!(app.document.stock(), Some(StockDef::BoundingBox { .. })), "the viewport draws the box");
        assert_eq!(app.status(), "Stock: 50.0 × 34.0 × 12.0 mm");
    }

    #[test]
    fn removing_the_stock_clears_the_document() {
        use solidcraft_cam::stock::StockExcess;

        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.document.draw(WireTool::Rectangle).unwrap_or_default();
        app.document.extrude(10.0).unwrap_or_default();
        let (min, max) = app.document.part_bounds().unwrap_or_default();
        app.accept_stock(Some(StockDef::from_bounds(min, max, StockExcess::default())));
        assert!(app.document.stock().is_some());
        app.accept_stock(None);
        assert_eq!(app.document.stock(), None);
        assert_eq!(app.status(), "Stock: removed");
    }

    #[test]
    fn the_view_commands_move_the_camera() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.viewport.camera.orbit(30.0, 15.0);
        app.run(Command::View(ViewCommand::Top));
        assert!((app.viewport.camera.pitch - std::f64::consts::FRAC_PI_2).abs() < 1e-5, "the Top view looks down the Z axis");
        app.run(Command::View(ViewCommand::Home));
        assert_eq!(app.viewport.camera, crate::viewport::home_camera());
        assert_eq!(app.status(), "Home view");
    }

    #[test]
    fn the_contour_command_opens_the_dialog_on_the_libraries_first_tool() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.run(Command::Contour2D);
        assert!(app.contour2d_open());
        let first_tool = app.tools.assemblies().first().map(|assembly| assembly.id);
        assert_eq!(app.contour2d_dialog().map(|dialog| dialog.params().tool_id), first_tool);
        assert!(app.operations().is_empty(), "opening the dialog stores nothing");
        app.cancel_contour2d();
        assert!(!app.contour2d_open());
        assert!(app.operations().is_empty(), "cancelling stores nothing");
    }

    #[test]
    fn the_toolpaths_tab_defines_and_stores_a_contour_operation() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.document.draw(WireTool::Rectangle).unwrap_or_default();
        app.manager.tab = ManagerTab::Toolpaths;
        app.run_manager(ManagerAction::NewContour2D);
        assert!(app.contour2d_open(), "the Toolpaths tab's button opens the parameter dialog");

        let chain: Vec<usize> = app.document.chain_entities().iter().map(|entity| entity.id).collect();
        assert_eq!(chain.len(), 4);
        if let Some(dialog) = app.contour2d_dialog_mut() {
            dialog.params_mut().chain = chain;
        }
        let accepted = app.contour2d_dialog().and_then(|dialog| dialog.accept().ok()).unwrap_or_default();
        app.accept_contour2d(accepted);

        assert!(!app.contour2d_open());
        assert_eq!(app.operations().len(), 1);
        assert_eq!(app.operations()[0].params.chain_len(), 4);
        assert!(app.operations()[0].is_generated(), "accepting calculates the toolpath");
        assert!(app.operations()[0].toolpath.feed_length() > 0.0, "the path cuts");
        assert!(app.status().contains("2D Contour"), "status: {}", app.status());
        assert!(app.status().starts_with("Ø6 flat endmill"), "status names the tool, got {}", app.status());
        assert!(app.status().contains("moves"), "status sums the toolpath up, got {}", app.status());
    }

    #[test]
    fn the_toolpaths_tab_toggles_and_regenerates_an_operation() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.document.draw(WireTool::Rectangle).unwrap_or_default();
        app.manager.tab = ManagerTab::Toolpaths;
        let chain: Vec<usize> = app.document.chain_entities().iter().map(|entity| entity.id).collect();
        let tool = app.tools.assemblies().first().map_or(0, |assembly| assembly.id);
        app.accept_contour2d(Contour2DParams::new(chain, tool));
        let revision = app.operations()[0].revision;

        app.run_manager(ManagerAction::ToggleOperationVisibility(0));
        assert!(!app.operations()[0].visible, "the visibility toggle hides the path");
        assert!(app.operations()[0].revision > revision);
        assert_eq!(app.status(), "2D Contour · 4 entities · depth -10.0 mm: toolpath hidden");
        app.run_manager(ManagerAction::ToggleOperationVisibility(0));
        assert!(app.operations()[0].visible);

        app.run_manager(ManagerAction::RegenerateOperation(0));
        assert!(app.status().contains("regenerated"), "status: {}", app.status());
        assert!(app.operations()[0].is_generated());
        app.run_manager(ManagerAction::RegenerateOperation(9));
        assert!(app.status().contains("No such operation"), "an absent index is reported, got {}", app.status());
    }

    #[test]
    fn an_operation_whose_tool_is_gone_is_stored_without_a_path() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.document.draw(WireTool::Rectangle).unwrap_or_default();
        let chain: Vec<usize> = app.document.chain_entities().iter().map(|entity| entity.id).collect();
        app.accept_contour2d(Contour2DParams::new(chain, 999));
        assert_eq!(app.operations().len(), 1, "the parameters are kept");
        assert!(!app.operations()[0].is_generated());
        assert!(app.status().contains("not generated"), "status: {}", app.status());
        assert!(!app.regenerate_operation(0), "regenerating cannot find the tool either");
        assert!(app.status().contains("not regenerated"), "status: {}", app.status());
    }

    /// A plate with stock around it and a 2D Contour that cuts inside that stock, so the
    /// simulation has material to remove.
    fn machinable_app() -> CamWorkspaceApp {
        use solidcraft_cam::stock::StockExcess;

        let mut app = CamWorkspaceApp::with_context(&egui::Context::default());
        app.document.draw(WireTool::Rectangle).unwrap_or_default();
        app.document.extrude(crate::document::DEFAULT_EXTRUDE_DEPTH).unwrap_or_default();
        let (min, max) = app.document.part_bounds().unwrap_or_default();
        app.accept_stock(Some(StockDef::from_bounds(min, max, StockExcess::new(5.0, 5.0, 2.0))));
        let chain = app.document.chain_entities().iter().map(|entity| entity.id).collect();
        let tool = app.tools.assemblies().first().map_or(1, |assembly| assembly.id);
        let mut params = Contour2DParams::new(chain, tool);
        // The default depth is below the part's origin; put the cuts inside the stock instead.
        let top = max[2] + 2.0;
        params.linking.top_of_stock = top;
        params.linking.feed_plane = top + 2.0;
        params.linking.retract = top + 4.0;
        params.linking.clearance = top + 6.0;
        params.linking.depth = max[2] - 4.0;
        params.depth_cuts.max_rough_step = 4.0;
        app.accept_contour2d(params);
        app
    }

    #[test]
    fn a_new_operation_becomes_what_the_simulation_runs() {
        let app = CamWorkspaceApp::with_context(&egui::Context::default());
        assert!(app.simulation().is_empty(), "a new workspace has nothing to run");
        let app = machinable_app();
        assert_eq!(app.simulation_operation(), 0);
        assert!(!app.simulation().is_empty(), "the operation's moves are what the clock runs");
        assert!(app.simulation().grid().is_some(), "the stock box is what the height map samples");
        assert!(app.simulation().tool().is_some(), "the operation's tool reaches the simulation");
    }

    #[test]
    fn the_playback_runs_the_toolpath_and_cuts_the_stock() {
        use solidcraft_cam::simulation::StockGrid;

        let mut app = machinable_app();
        let total = app.simulation().total_length();
        assert!(total > 0.0);
        assert!(app.simulation().grid().is_some_and(StockGrid::is_untouched));

        app.play_simulation();
        assert!(app.simulation().playing());
        assert!(app.status().starts_with("Simulation: playing"));
        app.advance_simulation(0.05);
        assert!(app.simulation().position() > 0.0, "the clock moves the tool");
        app.pause_simulation();
        assert!(!app.simulation().playing());

        app.scrub_simulation(total);
        assert!((app.simulation().position() - total).abs() < 1e-6);
        assert!(app.simulation().grid().is_some_and(|grid| !grid.is_untouched()), "the cut lowers the surface");

        app.reset_simulation();
        assert_eq!(app.simulation().position(), 0.0);
        assert!(app.simulation().grid().is_some_and(StockGrid::is_untouched), "a reset restores the raw stock");
        assert!(app.status().starts_with("Simulation: reset"));
    }

    #[test]
    fn the_simulation_tab_commands_drive_the_playback() {
        let mut app = machinable_app();
        app.run(Command::Simulation(SimulationCommand::Play));
        assert!(app.simulation().playing());
        app.run(Command::Simulation(SimulationCommand::Pause));
        assert!(!app.simulation().playing());
        app.run(Command::Simulation(SimulationCommand::Step));
        assert!(app.simulation().position() > 0.0, "a step advances one move");
        assert!(!app.simulation().playing());
        app.run(Command::Simulation(SimulationCommand::Reset));
        assert_eq!(app.simulation().position(), 0.0);
    }

    #[test]
    fn the_playback_panel_shows_on_the_simulation_tab() {
        let mut app = CamWorkspaceApp::with_context(&egui::Context::default());
        assert!(!app.simulation_panel_visible());
        app.ribbon.tab = RibbonTab::Simulation;
        assert!(app.simulation_panel_visible());
        app.ribbon.tab = RibbonTab::Home;
        assert!(!app.simulation_panel_visible());
        app.play_simulation();
        assert!(!app.simulation_panel_visible(), "there is no toolpath to play, so the panel stays shut");
    }

    #[test]
    fn the_panel_can_pick_between_operations_to_simulate() {
        let mut app = machinable_app();
        let chain = app.document.chain_entities().iter().map(|entity| entity.id).collect();
        let tool = app.tools.assemblies().first().map_or(1, |assembly| assembly.id);
        app.accept_contour2d(Contour2DParams::new(chain, tool));
        assert_eq!(app.operations().len(), 2);
        assert_eq!(app.simulation_operation(), 1, "a new operation is the one verified");
        app.select_simulation_operation(0);
        assert_eq!(app.simulation_operation(), 0);
        app.select_simulation_operation(9);
        assert_eq!(app.simulation_operation(), 0, "an absent operation changes nothing");
        assert!(app.status().contains("No such operation"));
    }

    #[test]
    fn a_contour_without_a_chain_is_refused_by_the_dialog() {
        let ctx = egui::Context::default();
        let mut app = CamWorkspaceApp::with_context(&ctx);
        app.run(Command::Contour2D);
        assert!(app.contour2d_dialog().and_then(|dialog| dialog.accept().ok()).is_none());
        assert!(app.operations().is_empty());
    }
}

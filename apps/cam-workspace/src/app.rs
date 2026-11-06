//! The CAM workspace: application state and the frame loop.
//!
//! The window is a workspace rather than a black screen: the status bar and the ribbon claim
//! their space first, then the unified left manager panels the model, and the central panel keeps
//! whatever remains for the 3D viewport.

use egui::{Align, Color32, Layout, RichText};

use crate::left_manager::{ACTIVE_HIGHLIGHT_LIGHT, ACTIVE_HIGHLIGHT_TEXT, LeftManager};

/// Background of the 3D viewport (light grey-blue), so the region reserved for graphics reads as
/// workspace instead of unrendered black.
pub const VIEWPORT_BACKGROUND: Color32 = Color32::from_rgb(216, 220, 227);

/// Height of the ribbon, in points: the tab strip and the command area under it.
pub const RIBBON_HEIGHT: f32 = 120.0;

/// Height of the ribbon's tab strip, in points: a compact CAD tab row.
pub const RIBBON_TAB_HEIGHT: f32 = 24.0;

/// Height of the ribbon's command area, in points: the home of the toolbar's buttons.
pub const RIBBON_COMMAND_HEIGHT: f32 = 60.0;

/// Height of the status bar, in points: one line of coordinates.
pub const STATUS_BAR_HEIGHT: f32 = 24.0;

/// Background of the status bar: the deep blue that separates it from the workspace.
pub const STATUS_BAR_BACKGROUND: Color32 = Color32::from_rgb(45, 80, 140);

/// Text of the status bar: white, for contrast on [`STATUS_BAR_BACKGROUND`].
pub const STATUS_BAR_TEXT: Color32 = Color32::WHITE;

/// Padding and spacing used throughout the workspace, in points: tight, so the manager and the
/// viewport get the screen real estate.
pub const PADDING: f32 = 4.0;

/// The ribbon's tabs, left to right, each a domain of tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RibbonTab {
    /// The modelling and file commands shared by every workflow.
    #[default]
    Home,
    /// Points, curves and construction geometry.
    Wireframe,
    /// Surfaces built from curves.
    Surfaces,
    /// Solid primitives and features.
    Solids,
    /// Machining operations.
    Toolpaths,
}

impl RibbonTab {
    /// The tabs in the order they appear, left to right.
    pub const ALL: [Self; 5] = [Self::Home, Self::Wireframe, Self::Surfaces, Self::Solids, Self::Toolpaths];

    /// The tab's text in the tab strip.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Wireframe => "Wireframe",
            Self::Surfaces => "Surfaces",
            Self::Solids => "Solids",
            Self::Toolpaths => "Toolpaths",
        }
    }

    /// Placeholder commands for the tab's command area, until the real toolbars exist.
    pub const fn placeholder_commands(self) -> &'static [&'static str] {
        match self {
            Self::Home => &["New", "Open", "Save", "Undo", "Redo"],
            Self::Wireframe => &["Points", "Lines", "Arcs", "Splines"],
            Self::Surfaces => &["Loft", "Revolve", "Sweep", "Trim"],
            Self::Solids => &["Extrude", "Revolve", "Loft", "Boolean", "Fillet"],
            Self::Toolpaths => &["Rough", "Finish", "Drill", "Simulate"],
        }
    }
}

/// Width of the left manager panel, in points, until the user drags its edge.
pub const LEFT_PANEL_WIDTH: f32 = 320.0;

/// Narrowest the left manager panel may be dragged, in points: any less and the level columns
/// stop being readable.
pub const LEFT_PANEL_MIN_WIDTH: f32 = 250.0;

/// Widest the left manager panel may be dragged, in points: beyond this it would crowd out the
/// viewport rather than serve it.
pub const LEFT_PANEL_MAX_WIDTH: f32 = 500.0;

/// The CAM workspace application.
#[derive(Debug, Default)]
pub struct CamWorkspaceApp {
    /// The ribbon tab whose tools the command area shows.
    active_ribbon_tab: RibbonTab,
    /// The unified left manager: levels, toolpaths, planes and solids.
    pub manager: LeftManager,
}

impl CamWorkspaceApp {
    /// Creates the app from an eframe creation context.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::with_context(&cc.egui_ctx)
    }

    /// Creates the app against any egui context: the window's, or a headless one in a test.
    pub fn with_context(ctx: &egui::Context) -> Self {
        install_style(ctx);
        Self::default()
    }

    /// The ribbon: the tab strip and, under it, the command area of the active tab.
    fn ribbon(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("ribbon_panel").exact_size(RIBBON_HEIGHT).show(ui, |ui| {
            // Compact tabs: a CAD ribbon spends its height on commands, not on tab labels.
            ui.spacing_mut().interact_size.y = RIBBON_TAB_HEIGHT;
            ui.horizontal_wrapped(|ui| {
                for tab in RibbonTab::ALL {
                    ui.selectable_value(&mut self.active_ribbon_tab, tab, tab.label());
                }
            });
            ui.separator();
            // The active tab's tools go here. Until they exist, disabled buttons hold their
            // places, so the command area reads as a toolbar rather than as a gap.
            let command_area = egui::vec2(ui.available_width(), ui.available_height().max(RIBBON_COMMAND_HEIGHT));
            ui.allocate_ui_with_layout(command_area, Layout::left_to_right(Align::Center), |ui| {
                for command in self.active_ribbon_tab.placeholder_commands() {
                    ui.add_enabled(false, egui::Button::new(*command));
                }
            });
        });
    }

    /// The status bar: the application state a user checks while modelling.
    fn status_bar(ui: &mut egui::Ui) {
        let frame = egui::Frame::new().fill(STATUS_BAR_BACKGROUND).inner_margin(egui::Margin::symmetric(8, 2));
        egui::Panel::bottom("status_bar").exact_size(STATUS_BAR_HEIGHT).frame(frame).show(ui, |ui| {
            ui.visuals_mut().override_text_color = Some(STATUS_BAR_TEXT);
            ui.horizontal_centered(|ui| {
                // Left: the sketch and selection state.
                ui.label("Section View: Off");
                ui.separator();
                ui.label("X 0.000   Y 0.000   Z 0.000");
                // Right: the construction plane the coordinates are in.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label("CPLANE: Top");
                });
            });
        });
    }

    /// The left manager panel: one tab strip over the four views, resizable within limits that
    /// keep both the level columns and the viewport usable.
    fn manager_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("left_manager_panel")
            .resizable(true)
            .default_size(LEFT_PANEL_WIDTH)
            .min_size(LEFT_PANEL_MIN_WIDTH)
            .max_size(LEFT_PANEL_MAX_WIDTH)
            .show(ui, |ui| self.manager.ui(ui));
    }

    /// Draws one frame of the workspace into `ui`: the panels first, the viewport last.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        // The peripheral panels claim their space first, in the blueprint's order: ribbon, status
        // bar, manager panel.
        self.ribbon(ui);
        Self::status_bar(ui);
        self.manager_panel(ui);

        // Last, so that it fills what the panels leave over.
        egui::CentralPanel::default().frame(egui::Frame::new().fill(VIEWPORT_BACKGROUND)).show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("3D viewport").weak());
            });
        });
    }
}

/// Installs the window's look: the light theme, tight spacing and the manager's active-row
/// highlight. Split from [`CamWorkspaceApp::new`] so a headless renderer can use the same look.
pub fn install_style(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Light);
    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(PADDING, PADDING);
        style.spacing.button_padding = egui::vec2(PADDING, PADDING);
        style.visuals.selection.bg_fill = ACTIVE_HIGHLIGHT_LIGHT;
        style.visuals.selection.stroke = egui::Stroke::new(1.0, ACTIVE_HIGHLIGHT_TEXT);
    });
}

impl eframe::App for CamWorkspaceApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

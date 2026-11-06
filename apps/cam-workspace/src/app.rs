//! The CAM workspace: application state and the frame loop.
//!
//! The window is a workspace rather than a black screen: the status bar and the ribbon claim
//! their space first, then the left manager panel splits what is left vertically between the CAD
//! and CAM trees, and the central panel keeps whatever remains for the 3D viewport.

use egui::{Align, Color32, Layout, RichText, ScrollArea};

/// Background of the 3D viewport (light grey-blue), so the region reserved for graphics reads as
/// workspace instead of unrendered black.
pub const VIEWPORT_BACKGROUND: Color32 = Color32::from_rgb(216, 220, 227);

/// Height of the ribbon, in points: the tab strip and the command area under it.
pub const RIBBON_HEIGHT: f32 = 104.0;

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

/// Padding and spacing used throughout the workspace, in points: tight, so the trees and the
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

/// Narrowest the left manager panel may be dragged, in points: any less and the tree columns
/// stop being readable.
pub const LEFT_PANEL_MIN_WIDTH: f32 = 250.0;

/// Widest the left manager panel may be dragged, in points: beyond this it would crowd out the
/// viewport rather than serve it.
pub const LEFT_PANEL_MAX_WIDTH: f32 = 500.0;

/// A manager tree's tabs: a small fixed set of views over one tree.
trait ManagerTab: Copy + PartialEq + 'static {
    /// The tabs in the order they appear, left to right.
    const ALL: &'static [Self];

    /// The tab's text in the tab strip.
    fn label(self) -> &'static str;
}

/// The upper tree's tabs: the CAD side of the manager.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpperTreeTab {
    /// The levels (layers) that group the model's geometry.
    #[default]
    Levels,
    /// The construction planes that sketches are drawn on.
    Planes,
}

impl ManagerTab for UpperTreeTab {
    const ALL: &'static [Self] = &[Self::Levels, Self::Planes];

    fn label(self) -> &'static str {
        match self {
            Self::Levels => "Levels",
            Self::Planes => "Planes",
        }
    }
}

/// The lower tree's tabs: the CAM side of the manager.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LowerTreeTab {
    /// The setups and the operations each one machines.
    #[default]
    Toolpaths,
    /// The solid bodies the toolpaths machine.
    Solids,
}

impl ManagerTab for LowerTreeTab {
    const ALL: &'static [Self] = &[Self::Toolpaths, Self::Solids];

    fn label(self) -> &'static str {
        match self {
            Self::Toolpaths => "Toolpaths",
            Self::Solids => "Solids",
        }
    }
}

/// Mocked levels (`Number`, `Name`, `Visible`), until levels come from a document.
const MOCK_LEVELS: [(u32, &str, bool); 4] = [(1, "Default", true), (2, "Construction", false), (3, "Fixture", true), (4, "Hidden", false)];

/// Mocked construction planes, until planes come from a document.
const MOCK_PLANES: [&str; 3] = ["Top (XY)", "Front (XZ)", "Right (YZ)"];

/// Mocked setups and their operations, until toolpaths come from a document.
const MOCK_TOOLPATHS: [(&str, &[&str]); 2] = [("Setup 1", &["Roughing", "Finishing"]), ("Setup 2", &["Drilling"])];

/// Mocked solid bodies, until solids come from a document.
const MOCK_SOLIDS: [&str; 3] = ["Body 1", "Body 2", "Stock"];

/// The CAM workspace application.
#[derive(Debug, Default)]
pub struct CamWorkspaceApp {
    /// The ribbon tab whose tools the command area shows.
    active_ribbon_tab: RibbonTab,
    /// The upper tree's tab: levels or planes.
    active_upper_tree_tab: UpperTreeTab,
    /// The lower tree's tab: toolpaths or solids.
    active_lower_tree_tab: LowerTreeTab,
}

impl CamWorkspaceApp {
    /// Creates the app and forces the light theme and tight spacing: industrial CAD readability,
    /// and as much room for the trees and the viewport as possible.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.set_theme(egui::ThemePreference::Light);
        ctx.global_style_mut(|style| {
            style.spacing.item_spacing = egui::vec2(PADDING, PADDING);
            style.spacing.button_padding = egui::vec2(PADDING, PADDING);
        });
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

    /// One manager section's tab strip.
    fn tree_tabs<T: ManagerTab>(ui: &mut egui::Ui, active: &mut T) {
        ui.horizontal_wrapped(|ui| {
            for tab in T::ALL {
                ui.selectable_value(active, *tab, tab.label());
            }
        });
    }

    /// The CAD tree's `Levels` view: a property grid of the model's levels.
    fn levels_grid(ui: &mut egui::Ui) {
        egui::Grid::new("levels_grid").num_columns(3).striped(true).show(ui, |ui| {
            ui.label(RichText::new("Number").strong());
            ui.label(RichText::new("Name").strong());
            ui.label(RichText::new("Visible").strong());
            ui.end_row();
            for (number, name, visible) in MOCK_LEVELS {
                ui.monospace(number.to_string());
                ui.label(name);
                ui.label(if visible { "Yes" } else { "No" });
                ui.end_row();
            }
        });
    }

    /// The CAD tree's `Planes` view: the construction planes, as collapsible nodes.
    fn planes_tree(ui: &mut egui::Ui) {
        for plane in MOCK_PLANES {
            egui::CollapsingHeader::new(plane).id_salt(plane).show(ui, |ui| {
                ui.label(RichText::new("Construction plane").weak());
            });
        }
    }

    /// The CAM tree's `Toolpaths` view: each setup, with the operations it machines.
    fn toolpaths_tree(ui: &mut egui::Ui) {
        for (setup, operations) in MOCK_TOOLPATHS {
            egui::CollapsingHeader::new(setup).id_salt(setup).default_open(true).show(ui, |ui| {
                for operation in operations {
                    ui.label(*operation);
                }
            });
        }
    }

    /// The CAM tree's `Solids` view: the bodies the toolpaths machine.
    fn solids_tree(ui: &mut egui::Ui) {
        for solid in MOCK_SOLIDS {
            egui::CollapsingHeader::new(solid).id_salt(solid).show(ui, |ui| {
                ui.label(RichText::new("Solid body").weak());
            });
        }
    }

    /// The upper half of the manager: the CAD tree and the tabs that select its view.
    fn upper_tree_section(&mut self, ui: &mut egui::Ui) {
        Self::tree_tabs(ui, &mut self.active_upper_tree_tab);
        ui.separator();
        // `auto_shrink` off: the tree keeps its section's height, which is also what makes the
        // manager panel resizable.
        let active = self.active_upper_tree_tab;
        ScrollArea::vertical().id_salt("cad_tree_scroll").auto_shrink([false, false]).show(ui, |ui| match active {
            UpperTreeTab::Levels => Self::levels_grid(ui),
            UpperTreeTab::Planes => Self::planes_tree(ui),
        });
    }

    /// The lower half of the manager: the CAM tree and the tabs that select its view.
    fn lower_tree_section(&mut self, ui: &mut egui::Ui) {
        Self::tree_tabs(ui, &mut self.active_lower_tree_tab);
        ui.separator();
        let active = self.active_lower_tree_tab;
        ScrollArea::vertical().id_salt("cam_tree_scroll").auto_shrink([false, false]).show(ui, |ui| match active {
            LowerTreeTab::Toolpaths => Self::toolpaths_tree(ui),
            LowerTreeTab::Solids => Self::solids_tree(ui),
        });
    }

    /// The left manager panel: the CAD feature tree in the upper half, the CAM toolpath tree in
    /// the lower half, split by a separator.
    fn manager_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("left_tree_panel")
            .resizable(true)
            .default_size(LEFT_PANEL_WIDTH)
            .min_size(LEFT_PANEL_MIN_WIDTH)
            .max_size(LEFT_PANEL_MAX_WIDTH)
            .show(ui, |ui| {
                // Both halves get the same height; the separator and the spacing around it come
                // out of what is split.
                let gap = 3.0 * ui.spacing().item_spacing.y;
                let upper_height = ((ui.available_height() - gap) * 0.5).max(0.0);
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), upper_height), Layout::top_down(Align::Min), |ui| {
                    ui.set_min_height(upper_height);
                    self.upper_tree_section(ui);
                });
                ui.separator();
                self.lower_tree_section(ui);
            });
    }
}

impl eframe::App for CamWorkspaceApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The peripheral panels claim their space first, in the blueprint's order: ribbon, status
        // bar, manager panel.
        self.ribbon(ui);
        Self::status_bar(ui);
        self.manager_panel(ui);

        // Last, so that it fills what the panels leave over.
        egui::CentralPanel::default().frame(egui::Frame::new().fill(VIEWPORT_BACKGROUND)).show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("3D viewport");
            });
        });
    }
}

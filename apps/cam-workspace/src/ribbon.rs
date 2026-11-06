//! The ribbon: the tab strip over the workspace's domains and the command area of the active tab.
//!
//! Z-CAM's ribbon follows the same twelve domains a Mastercam user knows. A tab whose tools this
//! phase does not implement yet keeps its buttons in place, disabled, so the ribbon reads as the
//! finished layout rather than as a gap. The `Wireframe`, `Solids`, `File` and `View` tabs carry
//! live commands: their buttons draw geometry on the active level, model solids from it, import
//! files and move the camera.

use egui::{Align, Layout};

use crate::document::{SolidTool, WireTool};

/// Height of the ribbon, in points: the tab strip and the command area under it.
pub const RIBBON_HEIGHT: f32 = 120.0;

/// Height of the ribbon's tab strip, in points: a compact CAD tab row.
pub const RIBBON_TAB_HEIGHT: f32 = 24.0;

/// Height of the ribbon's command area, in points: the home of the toolbar's buttons.
pub const RIBBON_COMMAND_HEIGHT: f32 = 60.0;

/// The ribbon's tabs, left to right, each a domain of tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RibbonTab {
    /// Files: new, open, save, import.
    File,
    /// The commands shared by every workflow.
    #[default]
    Home,
    /// Points, curves and construction geometry.
    Wireframe,
    /// Surfaces built from curves.
    Surfaces,
    /// Solid primitives and features.
    Solids,
    /// Cleanup of imported bodies before machining.
    ModelPrep,
    /// Triangle meshes.
    Mesh,
    /// Dimensions, notes and drawing sheets.
    Drafting,
    /// Move, rotate, mirror and scale.
    Transform,
    /// Free-form shaping.
    Art,
    /// Machines, groups and toolpaths.
    Machine,
    /// Camera and display.
    View,
}

impl RibbonTab {
    /// The tabs in the order they appear, left to right.
    pub const ALL: &'static [Self] = &[
        Self::File,
        Self::Home,
        Self::Wireframe,
        Self::Surfaces,
        Self::Solids,
        Self::ModelPrep,
        Self::Mesh,
        Self::Drafting,
        Self::Transform,
        Self::Art,
        Self::Machine,
        Self::View,
    ];

    /// The tab's text in the tab strip.
    pub const fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Home => "Home",
            Self::Wireframe => "Wireframe",
            Self::Surfaces => "Surfaces",
            Self::Solids => "Solids",
            Self::ModelPrep => "Model Prep",
            Self::Mesh => "Mesh",
            Self::Drafting => "Drafting",
            Self::Transform => "Transform",
            Self::Art => "Art",
            Self::Machine => "Machine",
            Self::View => "View",
        }
    }

    /// The tab's tools, in the order they appear. A tool without a command is not implemented
    /// yet and is drawn disabled.
    pub const fn tools(self) -> &'static [Tool] {
        match self {
            Self::File => &FILE_TOOLS,
            Self::Home => &HOME_TOOLS,
            Self::Wireframe => &WIREFRAME_TOOLS,
            Self::Surfaces => &SURFACE_TOOLS,
            Self::Solids => &SOLID_TOOLS,
            Self::ModelPrep => &MODEL_PREP_TOOLS,
            Self::Mesh => &MESH_TOOLS,
            Self::Drafting => &DRAFTING_TOOLS,
            Self::Transform => &TRANSFORM_TOOLS,
            Self::Art => &ART_TOOLS,
            Self::Machine => &MACHINE_TOOLS,
            Self::View => &VIEW_TOOLS,
        }
    }
}

/// A standard camera the `View` tab, and the viewport's own controls, move to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewCommand {
    /// The isometric home view of the whole workspace.
    Home,
    /// Looking down the Z axis.
    Top,
    /// Looking along the Y axis.
    Front,
    /// Looking along the X axis.
    Right,
    /// The isometric view at the current scale.
    Iso,
}

/// The file formats the `File` tab imports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportKind {
    /// STEP solids, read by the kernel and added to the `Solids` view.
    Step,
    /// DXF wireframe, read as sketch entities on the active level.
    Dxf,
}

impl ImportKind {
    /// The dialog's title.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Step => "Import STEP",
            Self::Dxf => "Import DXF",
        }
    }

    /// The file extension the dialog expects.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Dxf => "dxf",
        }
    }
}

/// What a ribbon button asks the application to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Draw wireframe on the active level.
    Draw(WireTool),
    /// Model a solid from the active level's wireframe.
    Solid(SolidTool),
    /// Open the import dialog for a format.
    Import(ImportKind),
    /// Move the camera to a standard view.
    View(ViewCommand),
}

/// One button in a ribbon tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tool {
    /// The button's text.
    pub label: &'static str,
    /// What a click does; `None` until the tool is implemented.
    pub command: Option<Command>,
}

/// A tool that is not implemented yet: drawn, disabled, with a note explaining why.
const fn pending(label: &'static str) -> Tool {
    Tool { label, command: None }
}

/// A tool with a live command.
const fn live(label: &'static str, command: Command) -> Tool {
    Tool { label, command: Some(command) }
}

/// The `File` tab: the four import paths Z-CAM reads today, and the placeholders for the rest.
const FILE_TOOLS: [Tool; 6] = [
    pending("New"),
    pending("Open"),
    pending("Save"),
    live("Import STEP", Command::Import(ImportKind::Step)),
    live("Import DXF", Command::Import(ImportKind::Dxf)),
    pending("Export"),
];

/// The `Home` tab: the editing commands and the way back to the home view.
const HOME_TOOLS: [Tool; 6] =
    [pending("Undo"), pending("Redo"), pending("Copy"), pending("Delete"), pending("Sketch"), live("Home", Command::View(ViewCommand::Home))];

/// The `Wireframe` tab: the sketch primitives, all live.
const WIREFRAME_TOOLS: [Tool; 6] = [
    live(WireTool::Point.label(), Command::Draw(WireTool::Point)),
    live(WireTool::Line.label(), Command::Draw(WireTool::Line)),
    live(WireTool::Rectangle.label(), Command::Draw(WireTool::Rectangle)),
    live(WireTool::Circle.label(), Command::Draw(WireTool::Circle)),
    live(WireTool::Arc.label(), Command::Draw(WireTool::Arc)),
    live(WireTool::Spline.label(), Command::Draw(WireTool::Spline)),
];

/// The `Surfaces` tab: surface modeling, none of it wired yet.
const SURFACE_TOOLS: [Tool; 5] = [pending("Loft"), pending("Revolve"), pending("Sweep"), pending("Trim"), pending("Offset")];

/// The `Solids` tab: the kernel's solid features, all live.
const SOLID_TOOLS: [Tool; 6] = [
    live(SolidTool::Extrude.label(), Command::Solid(SolidTool::Extrude)),
    live(SolidTool::Revolve.label(), Command::Solid(SolidTool::Revolve)),
    live(SolidTool::BooleanCut.label(), Command::Solid(SolidTool::BooleanCut)),
    live(SolidTool::BooleanUnion.label(), Command::Solid(SolidTool::BooleanUnion)),
    live(SolidTool::Fillet.label(), Command::Solid(SolidTool::Fillet)),
    live(SolidTool::Chamfer.label(), Command::Solid(SolidTool::Chamfer)),
];

/// The `Model Prep` tab: cleanup of imported bodies, none of it wired yet.
const MODEL_PREP_TOOLS: [Tool; 4] = [pending("Remove Faces"), pending("Delete Hole"), pending("Simplify"), pending("Fit Arcs")];

/// The `Mesh` tab: triangle meshes, none of it wired yet.
const MESH_TOOLS: [Tool; 4] = [pending("Mesh Body"), pending("Reduce"), pending("Repair"), pending("Compare")];

/// The `Drafting` tab: dimensions and sheets, none of it wired yet.
const DRAFTING_TOOLS: [Tool; 4] = [pending("Dimension"), pending("Note"), pending("View"), pending("Title Block")];

/// The `Transform` tab: moving geometry, none of it wired yet.
const TRANSFORM_TOOLS: [Tool; 5] = [pending("Translate"), pending("Rotate"), pending("Mirror"), pending("Scale"), pending("Array")];

/// The `Art` tab: free-form shaping, none of it wired yet.
const ART_TOOLS: [Tool; 4] = [pending("Loft"), pending("Sculpt"), pending("Stitch"), pending("Smooth")];

/// The `Machine` tab: machines and toolpaths, none of it wired yet.
const MACHINE_TOOLS: [Tool; 4] = [pending("Machine Group"), pending("Toolpaths"), pending("Verify"), pending("Post")];

/// The `View` tab: the standard cameras, all live.
const VIEW_TOOLS: [Tool; 5] = [
    live("Home", Command::View(ViewCommand::Home)),
    live("Top", Command::View(ViewCommand::Top)),
    live("Front", Command::View(ViewCommand::Front)),
    live("Right", Command::View(ViewCommand::Right)),
    live("Iso", Command::View(ViewCommand::Iso)),
];

/// The ribbon's state: which tab's tools the command area shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ribbon {
    /// The tab whose tools the command area shows.
    pub tab: RibbonTab,
}

impl Ribbon {
    /// Draws the ribbon into `ui` and returns the command a button asked for, if any.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut command = None;
        egui::Panel::top("ribbon_panel").exact_size(RIBBON_HEIGHT).show(ui, |ui| {
            // Compact tabs: a CAD ribbon spends its height on commands, not on tab labels.
            ui.spacing_mut().interact_size.y = RIBBON_TAB_HEIGHT;
            ui.horizontal_wrapped(|ui| {
                for &tab in RibbonTab::ALL {
                    ui.selectable_value(&mut self.tab, tab, tab.label());
                }
            });
            ui.separator();
            let command_area = egui::vec2(ui.available_width(), ui.available_height().max(RIBBON_COMMAND_HEIGHT));
            ui.allocate_ui_with_layout(command_area, Layout::left_to_right(Align::Center), |ui| {
                for tool in self.tab.tools() {
                    let button = egui::Button::new(tool.label).min_size(egui::vec2(0.0, 28.0));
                    match tool.command {
                        Some(asked) => {
                            if ui.add(button).clicked() {
                                command = Some(asked);
                            }
                        }
                        None => {
                            ui.add_enabled(false, button).on_hover_text("Available in a later phase");
                        }
                    }
                }
            });
        });
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ribbon_lists_the_twelve_z_cam_tabs_in_order() {
        assert_eq!(
            RibbonTab::ALL.iter().copied().map(RibbonTab::label).collect::<Vec<_>>(),
            ["File", "Home", "Wireframe", "Surfaces", "Solids", "Model Prep", "Mesh", "Drafting", "Transform", "Art", "Machine", "View"]
        );
        assert_eq!(RibbonTab::default(), RibbonTab::Home);
    }

    #[test]
    fn the_wireframe_tab_exposes_the_sketching_tools() {
        let commands: Vec<_> = WIREFRAME_TOOLS.iter().filter_map(|tool| tool.command).collect();
        assert_eq!(commands, WireTool::ALL.map(Command::Draw));
    }

    #[test]
    fn the_solids_tab_exposes_the_kernel_tools() {
        let commands: Vec<_> = SOLID_TOOLS.iter().filter_map(|tool| tool.command).collect();
        assert_eq!(commands, SolidTool::ALL.map(Command::Solid));
    }

    #[test]
    fn the_file_tab_imports_step_and_dxf() {
        let imports: Vec<_> = FILE_TOOLS.iter().filter_map(|tool| tool.command).collect();
        assert_eq!(imports, [Command::Import(ImportKind::Step), Command::Import(ImportKind::Dxf)]);
    }

    #[test]
    fn the_view_tab_moves_the_camera() {
        let views: Vec<_> = VIEW_TOOLS.iter().filter_map(|tool| tool.command).collect();
        assert_eq!(views.len(), 5);
        assert!(views.iter().all(|command| matches!(command, Command::View(_))), "every View tool moves the camera");
    }

    #[test]
    fn an_unimplemented_tool_keeps_its_button() {
        assert!(SURFACE_TOOLS.iter().all(|tool| tool.command.is_none()));
        assert!(SURFACE_TOOLS.iter().all(|tool| !tool.label.is_empty()));
    }
}

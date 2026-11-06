//! The left manager: one panel, four views over the CAM document.
//!
//! Mastercam-style managers split the model into levels, toolpaths, construction planes and
//! solids. Z-CAM keeps them in a single panel with a tab strip, so the viewport keeps the width
//! the old pair of stacked trees spent on two tab rows and two separators.
//!
//! The manager is a view: the levels, their visibility and the solids all live in the
//! [`CamDocument`], which the ribbon's tools edit. The manager only owns which tab is open.

use egui::{Color32, RichText, ScrollArea};

use crate::document::CamDocument;

pub use crate::document::{Level, STANDARD_PLANES};

/// Height of a level row, in points: one dense manager line.
const ROW_HEIGHT: f32 = 20.0;

/// Width of the active-level marker column, in points: the checkmark that marks the level new
/// geometry is created on.
const MARKER_WIDTH: f32 = 20.0;

/// Width of the level number column, in points.
const NUMBER_WIDTH: f32 = 28.0;

/// Width of the visibility column, in points: a checkbox under a three-letter heading.
const VISIBLE_WIDTH: f32 = 30.0;

/// Width of the selectability column, in points.
const SELECTABLE_WIDTH: f32 = 30.0;

/// Width of the entity count column, in points.
const ENTITY_WIDTH: f32 = 52.0;

/// The highlight behind an active tab and the row it selects, in the light theme: a subtle blue
/// that reads as "current" without competing with the model.
pub const ACTIVE_HIGHLIGHT_LIGHT: Color32 = Color32::from_rgb(220, 235, 255);

/// The same highlight in the dark theme.
pub const ACTIVE_HIGHLIGHT_DARK: Color32 = Color32::from_rgb(40, 60, 90);

/// Text on [`ACTIVE_HIGHLIGHT_LIGHT`]: the deep blue the rest of the shell uses for emphasis.
pub const ACTIVE_HIGHLIGHT_TEXT: Color32 = Color32::from_rgb(20, 45, 85);

/// Width of the name column, in points: whatever the other columns leave of the row.
fn name_width(ui: &egui::Ui) -> f32 {
    let fixed = MARKER_WIDTH + NUMBER_WIDTH + VISIBLE_WIDTH + SELECTABLE_WIDTH + ENTITY_WIDTH;
    // One gap between each pair of the six columns.
    let gaps = 5.0 * ui.spacing().item_spacing.x;
    (ui.available_width() - fixed - gaps).max(40.0)
}

/// Width of a solid's name column: the table has three columns, not six.
fn solid_name_width(ui: &egui::Ui) -> f32 {
    let fixed = VISIBLE_WIDTH + NUMBER_WIDTH;
    let gaps = 2.0 * ui.spacing().item_spacing.x;
    (ui.available_width() - fixed - gaps).max(40.0)
}

/// Paints the checkmark that marks the active creation level, into the marker's cell: a glyph
/// would depend on a font that ships a check mark, a path always draws.
fn paint_checkmark(ui: &egui::Ui, rect: egui::Rect) {
    let rect = rect.shrink2(egui::vec2(4.0, 5.0));
    let stroke = egui::Stroke::new(2.0, ACTIVE_HIGHLIGHT_TEXT);
    let points = vec![
        egui::pos2(rect.left(), rect.center().y),
        egui::pos2(rect.left() + rect.width() * 0.35, rect.bottom()),
        egui::pos2(rect.right(), rect.top()),
    ];
    ui.painter().add(egui::Shape::line(points, stroke));
}

/// The row background of the selected item: the blueprint's active highlight.
fn selected_fill(selected: bool) -> Color32 {
    if selected { ACTIVE_HIGHLIGHT_LIGHT } else { Color32::TRANSPARENT }
}

/// The manager's tabs, left to right, each a view over the CAM document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ManagerTab {
    /// The levels that group the model's geometry.
    #[default]
    Levels,
    /// The setups and operations that machine the part.
    Toolpaths,
    /// The construction planes sketches are drawn on.
    Planes,
    /// The solid bodies the toolpaths machine.
    Solids,
}

impl ManagerTab {
    /// The tabs in the order they appear, left to right.
    pub const ALL: [Self; 4] = [Self::Levels, Self::Toolpaths, Self::Planes, Self::Solids];

    /// The tab's text in the tab strip.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Levels => "Levels",
            Self::Toolpaths => "Toolpaths",
            Self::Planes => "Planes",
            Self::Solids => "Solids",
        }
    }
}

/// The unified left manager: its tab strip and the view over the document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeftManager {
    /// The tab whose view the panel shows.
    pub tab: ManagerTab,
}

impl LeftManager {
    /// The manager panel's contents: the tab strip and the active view under it.
    pub fn ui(&mut self, ui: &mut egui::Ui, document: &mut CamDocument) {
        ui.horizontal_wrapped(|ui| {
            for tab in ManagerTab::ALL {
                ui.selectable_value(&mut self.tab, tab, tab.label());
            }
        });
        ui.separator();
        // `auto_shrink` off: the view keeps the panel's height, so the list scrolls rather than
        // the panel collapsing around a short list.
        ScrollArea::vertical().id_salt("left_manager_scroll").auto_shrink([false, false]).show(ui, |ui| match self.tab {
            ManagerTab::Levels => Self::levels_view(ui, document),
            ManagerTab::Toolpaths => empty_state(ui, "No toolpaths created"),
            ManagerTab::Planes => Self::planes_view(ui, document),
            ManagerTab::Solids => Self::solids_view(ui, document),
        });
    }

    /// The `Levels` view: the level table, a new-level button and the level count.
    fn levels_view(ui: &mut egui::Ui, document: &mut CamDocument) {
        ui.horizontal(|ui| {
            if ui.button("+ New level").clicked() {
                document.add_level();
            }
            let count = document.levels().len();
            let plural = if count == 1 { "" } else { "s" };
            ui.label(RichText::new(format!("{count} level{plural}")).weak());
        });
        ui.separator();
        Self::levels_header(ui);

        let mut activate = None;
        let mut visibility = None;
        let mut selectability = None;
        for level in document.levels() {
            let active = level.id == document.active_level();
            let row = egui::Frame::new().fill(selected_fill(active)).inner_margin(egui::Margin::symmetric(2, 1));
            row.show(ui, |ui| {
                let name_width = name_width(ui);
                ui.horizontal(|ui| {
                    let marker = ui.allocate_response(egui::vec2(MARKER_WIDTH, ROW_HEIGHT), egui::Sense::click());
                    if marker.clicked() {
                        activate = Some(level.id);
                    }
                    if active {
                        paint_checkmark(ui, marker.rect);
                    }
                    marker.on_hover_text("Create new geometry on this level");
                    ui.add_sized([NUMBER_WIDTH, ROW_HEIGHT], egui::Label::new(RichText::new(level.id.to_string()).monospace()));
                    let name = if active { RichText::new(&level.name).strong() } else { RichText::new(&level.name) };
                    ui.add_sized([name_width, ROW_HEIGHT], egui::Label::new(name));
                    let mut visible = level.visible;
                    if ui.add_sized([VISIBLE_WIDTH, ROW_HEIGHT], egui::Checkbox::without_text(&mut visible)).on_hover_text("Visible").changed() {
                        visibility = Some((level.id, visible));
                    }
                    let mut selectable = level.selectable;
                    if ui
                        .add_sized([SELECTABLE_WIDTH, ROW_HEIGHT], egui::Checkbox::without_text(&mut selectable))
                        .on_hover_text("Selectable")
                        .changed()
                    {
                        selectability = Some((level.id, selectable));
                    }
                    ui.add_sized([ENTITY_WIDTH, ROW_HEIGHT], egui::Label::new(RichText::new(level.entity_count.to_string()).monospace()));
                });
            });
        }
        if let Some(id) = activate {
            document.set_active_level(id);
        }
        if let Some((id, visible)) = visibility {
            document.set_level_visible(id, visible);
        }
        if let Some((id, selectable)) = selectability {
            document.set_level_selectable(id, selectable);
        }
    }

    /// The column headings of the level table, laid out over the columns [`Self::levels_view`]
    /// draws.
    fn levels_header(ui: &mut egui::Ui) {
        let name_width = name_width(ui);
        let cells = [
            (MARKER_WIDTH, ""),
            (NUMBER_WIDTH, "No."),
            (name_width, "Name"),
            (VISIBLE_WIDTH, "Vis"),
            (SELECTABLE_WIDTH, "Sel"),
            (ENTITY_WIDTH, "Entities"),
        ];
        ui.horizontal(|ui| {
            for (width, text) in cells {
                ui.add_sized([width, ROW_HEIGHT], egui::Label::new(RichText::new(text).strong()));
            }
        });
        ui.separator();
    }

    /// The `Planes` view: the standard construction planes, one of them selected.
    fn planes_view(ui: &mut egui::Ui, document: &mut CamDocument) {
        let mut choose = None;
        let row_width = ui.available_width();
        for (index, plane) in STANDARD_PLANES.iter().enumerate() {
            let selected = index == document.active_plane();
            let row = egui::Frame::new().fill(selected_fill(selected)).inner_margin(egui::Margin::symmetric(2, 1));
            row.show(ui, |ui| {
                ui.set_min_width(row_width);
                let text = if selected { RichText::new(*plane).strong() } else { RichText::new(*plane) };
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    if ui.add(egui::Label::new(text).sense(egui::Sense::click())).clicked() {
                        choose = Some(index);
                    }
                });
            });
        }
        if let Some(index) = choose {
            document.set_active_plane(index);
        }
    }

    /// The `Solids` view: the bodies created, imported or modeled into the document, with the
    /// level each one belongs to.
    fn solids_view(ui: &mut egui::Ui, document: &mut CamDocument) {
        if document.solids().is_empty() {
            empty_state(ui, "No solids created");
            return;
        }
        let count = document.solids().len();
        let plural = if count == 1 { "" } else { "s" };
        ui.label(RichText::new(format!("{count} solid{plural}")).weak());
        ui.separator();
        Self::solids_header(ui);

        let mut visibility = None;
        for solid in document.solids() {
            let row = egui::Frame::new().inner_margin(egui::Margin::symmetric(2, 1));
            row.show(ui, |ui| {
                let name_width = solid_name_width(ui);
                ui.horizontal(|ui| {
                    let mut visible = solid.visible;
                    if ui.add_sized([VISIBLE_WIDTH, ROW_HEIGHT], egui::Checkbox::without_text(&mut visible)).on_hover_text("Visible").changed() {
                        visibility = Some((solid.id, visible));
                    }
                    ui.add_sized([name_width, ROW_HEIGHT], egui::Label::new(&solid.name));
                    ui.add_sized([NUMBER_WIDTH, ROW_HEIGHT], egui::Label::new(RichText::new(solid.level.to_string()).monospace()));
                });
            });
        }
        if let Some((id, visible)) = visibility {
            document.set_solid_visible(id, visible);
        }
    }

    /// The column headings of the solids table.
    fn solids_header(ui: &mut egui::Ui) {
        let name_width = solid_name_width(ui);
        ui.horizontal(|ui| {
            for (width, text) in [(VISIBLE_WIDTH, "Vis"), (name_width, "Name"), (NUMBER_WIDTH, "Level")] {
                ui.add_sized([width, ROW_HEIGHT], egui::Label::new(RichText::new(text).strong()));
            }
        });
        ui.separator();
    }
}

/// A view's empty state: what it will hold, centred in the space it has.
fn empty_state(ui: &mut egui::Ui, text: &str) {
    let height = ui.available_height();
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), height), egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
        ui.label(RichText::new(text).weak());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::WireTool;

    #[test]
    fn the_tab_strip_lists_the_four_manager_views_in_order() {
        assert_eq!(ManagerTab::ALL.map(ManagerTab::label), ["Levels", "Toolpaths", "Planes", "Solids"]);
        assert_eq!(LeftManager::default().tab, ManagerTab::Levels);
    }

    #[test]
    fn the_planes_view_lists_the_standard_construction_planes() {
        assert_eq!(STANDARD_PLANES, ["TOP", "FRONT", "RIGHT", "BACK", "LEFT", "BOTTOM"]);
        assert_eq!(CamDocument::default().active_plane(), 0);
    }

    #[test]
    fn the_solids_view_reads_the_documents_bodies() {
        let mut document = CamDocument::default();
        document.draw(WireTool::Rectangle).unwrap_or_default();
        document.extrude(10.0).unwrap_or_default();
        assert_eq!(document.solids().len(), 1);
        assert_eq!(document.solids()[0].level, document.active_level());
    }
}

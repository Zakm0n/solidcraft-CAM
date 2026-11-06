//! The left manager: one panel, four views over the CAM document.
//!
//! Mastercam-style managers split the model into levels, toolpaths, construction planes and
//! solids. Z-CAM keeps them in a single panel with a tab strip, so the viewport keeps the width
//! the old pair of stacked trees spent on two tab rows and two separators.
//!
//! The manager owns its own view state (which tab is open, which level is being created into).
//! The document itself arrives in a later phase; until then the levels are real enough to add
//! to, and the toolpath, plane and solid views state what they will hold.

use egui::{Color32, RichText, ScrollArea};

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

/// The standard construction planes every CAM part starts with, in the order the manager lists
/// them: the three principal planes first, then the three opposing ones.
pub const STANDARD_PLANES: [&str; 6] = ["TOP", "FRONT", "RIGHT", "BACK", "LEFT", "BOTTOM"];

/// One level (layer): a named group of geometry with its own visibility and selectability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    /// The level's number, also its identity: numbers are never reused.
    pub id: usize,
    /// The level's name as the manager shows it.
    pub name: String,
    /// Whether the level's geometry is drawn.
    pub visible: bool,
    /// Whether the level's geometry can be picked.
    pub selectable: bool,
    /// How much geometry the level holds.
    pub entity_count: usize,
}

impl Level {
    /// A level that starts out visible, selectable and empty.
    pub fn new(id: usize, name: impl Into<String>) -> Self {
        Self { id, name: name.into(), visible: true, selectable: true, entity_count: 0 }
    }
}

/// The unified left manager: its tab strip, its views and the state those views edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftManager {
    /// The tab whose view the panel shows.
    pub tab: ManagerTab,
    /// The levels, in number order.
    levels: Vec<Level>,
    /// The id of the level new geometry is created on, marked with a checkmark.
    active_level: usize,
    /// The construction plane selectable in the `Planes` view.
    active_plane: usize,
}

impl Default for LeftManager {
    /// A new part starts on level 1, named `X`, with TOP as the construction plane: the same
    /// names the status bar shows.
    fn default() -> Self {
        Self { tab: ManagerTab::default(), levels: vec![Level::new(1, "X")], active_level: 1, active_plane: 0 }
    }
}

impl LeftManager {
    /// The levels, in number order.
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// The id of the level new geometry is created on.
    pub fn active_level(&self) -> usize {
        self.active_level
    }

    /// The index into [`STANDARD_PLANES`] of the selected construction plane.
    pub fn active_plane(&self) -> usize {
        self.active_plane
    }

    /// Adds a level and makes it the active creation level, so the next thing drawn lands on it.
    /// Returns the new level's number.
    pub fn add_level(&mut self) -> usize {
        let id = self.levels.iter().map(|level| level.id).max().unwrap_or(0) + 1;
        self.levels.push(Level::new(id, format!("Level {id}")));
        self.active_level = id;
        id
    }

    /// The manager panel's contents: the tab strip and the active view under it.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for tab in ManagerTab::ALL {
                ui.selectable_value(&mut self.tab, tab, tab.label());
            }
        });
        ui.separator();
        // `auto_shrink` off: the view keeps the panel's height, so the list scrolls rather than
        // the panel collapsing around a short list.
        ScrollArea::vertical().id_salt("left_manager_scroll").auto_shrink([false, false]).show(ui, |ui| match self.tab {
            ManagerTab::Levels => self.levels_view(ui),
            ManagerTab::Toolpaths => empty_state(ui, "No toolpaths created"),
            ManagerTab::Planes => self.planes_view(ui),
            ManagerTab::Solids => empty_state(ui, "No solids created"),
        });
    }

    /// The `Levels` view: the level table, a new-level button and the level count.
    fn levels_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("+ New level").clicked() {
                self.add_level();
            }
            let count = self.levels.len();
            let plural = if count == 1 { "" } else { "s" };
            ui.label(RichText::new(format!("{count} level{plural}")).weak());
        });
        ui.separator();
        Self::levels_header(ui);

        let mut activate = None;
        let mut levels = std::mem::take(&mut self.levels);
        for level in &mut levels {
            let active = level.id == self.active_level;
            let row = egui::Frame::new()
                .fill(if active { ACTIVE_HIGHLIGHT_LIGHT } else { Color32::TRANSPARENT })
                .inner_margin(egui::Margin::symmetric(2, 1));
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
                    ui.add_sized([VISIBLE_WIDTH, ROW_HEIGHT], egui::Checkbox::without_text(&mut level.visible)).on_hover_text("Visible");
                    ui.add_sized([SELECTABLE_WIDTH, ROW_HEIGHT], egui::Checkbox::without_text(&mut level.selectable)).on_hover_text("Selectable");
                    ui.add_sized([ENTITY_WIDTH, ROW_HEIGHT], egui::Label::new(RichText::new(level.entity_count.to_string()).monospace()));
                });
            });
        }
        self.levels = levels;
        if let Some(id) = activate {
            self.active_level = id;
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
    fn planes_view(&mut self, ui: &mut egui::Ui) {
        let mut choose = None;
        let row_width = ui.available_width();
        for (index, plane) in STANDARD_PLANES.iter().enumerate() {
            let selected = index == self.active_plane;
            let row = egui::Frame::new()
                .fill(if selected { ACTIVE_HIGHLIGHT_LIGHT } else { Color32::TRANSPARENT })
                .inner_margin(egui::Margin::symmetric(2, 1));
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
            self.active_plane = index;
        }
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

    #[test]
    fn a_new_part_starts_on_level_one_named_x() {
        let manager = LeftManager::default();
        assert_eq!(manager.levels(), &[Level::new(1, "X")]);
        assert_eq!(manager.active_level(), 1);
        assert!(manager.levels()[0].visible && manager.levels()[0].selectable);
        assert_eq!(manager.tab, ManagerTab::Levels);
    }

    #[test]
    fn a_new_level_is_numbered_next_and_becomes_the_creation_level() {
        let mut manager = LeftManager::default();
        assert_eq!(manager.add_level(), 2);
        assert_eq!(manager.add_level(), 3);
        assert_eq!(manager.levels().iter().map(|l| l.id).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(manager.levels()[2].name, "Level 3");
        assert_eq!(manager.active_level(), 3);
    }

    #[test]
    fn the_tab_strip_lists_the_four_manager_views_in_order() {
        assert_eq!(ManagerTab::ALL.map(ManagerTab::label), ["Levels", "Toolpaths", "Planes", "Solids"]);
    }

    #[test]
    fn the_planes_view_lists_the_standard_construction_planes() {
        assert_eq!(STANDARD_PLANES, ["TOP", "FRONT", "RIGHT", "BACK", "LEFT", "BOTTOM"]);
        assert_eq!(LeftManager::default().active_plane(), 0);
    }
}

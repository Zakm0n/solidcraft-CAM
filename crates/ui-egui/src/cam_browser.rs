//! The CAM workspace's Toolpaths card: the lower half of the left column, which will list the
//! toolpath setups a coordinate system and its operations make (`ui.cam.toolpaths`).
//!
//! The card is a workspace surface like the Browser above it, so it uses the shared container
//! and theme tokens and draws nothing while the space is too small to hold a header.

use egui::{Rect, Sense, pos2, vec2};

use crate::theme::Tokens;
use crate::ui_kit::{Container, DockEdges, Surface};
use crate::SolveApp;

/// The card's inner margin, matching the Browser card's inset.
const CARD_INSET: f32 = 4.0;
/// The header band's height, matching the Browser card's heading.
const HEADER_H: f32 = 26.0;

/// Draw the Toolpaths card in `bounds`, the Browser column's lower half, and return where it
/// was drawn. No card is drawn when the space cannot hold one.
pub(crate) fn toolpaths_panel(_app: &mut SolveApp, ui: &mut egui::Ui, bounds: Rect) -> Option<Rect> {
    let t = Tokens::get();
    if !bounds.is_finite() || bounds.width() < t.control_height || bounds.height() < t.form_header_height {
        return None;
    }
    let container = Container::new(ui.ctx(), Surface::Panel).joined(DockEdges { left: true, bottom: true, ..Default::default() });
    let frame = container.frame(ui.ctx(), t.panel, crate::frame::Side::Left).inner_margin(CARD_INSET as i8);
    let inner = (bounds.size() - frame.total_margin().sum()).max(egui::Vec2::ZERO);
    let card = Container::new(ui.ctx(), Surface::Panel).area(egui::Id::unique("sc_cam_toolpaths_card")).fixed_pos(bounds.min).show(
        ui.ctx(),
        |ui| {
            container.show_frame(ui, frame, bounds, |ui| {
                ui.set_min_size(inner);
                ui.set_max_size(inner);
                ui.set_clip_rect(ui.clip_rect().intersect(Rect::from_min_size(ui.max_rect().min, inner)));
                header(ui);
                empty_state(ui);
            })
        },
    );
    let rect = card.response.rect;
    crate::scenario::publish_rect("cam:toolpaths:card", rect);
    Some(rect)
}

/// The card's heading, in the Browser's title style.
fn header(ui: &mut egui::Ui) {
    let t = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    crate::scenario::publish_target("cam:toolpaths:header", rect);
    let title = crate::i18n::tr("ui.cam.toolpaths").to_string();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &title));
    let galley = ui.painter().layout_no_wrap(title, crate::fonts::semibold(t.font_heading), t.text);
    ui.painter().galley(pos2(rect.left() + t.space_6, rect.center().y - galley.size().y / 2.0), galley, t.text);
}

/// What the card holds until toolpath setups exist: a dim line that says so, in its own
/// scrollable body so a later list slots in without moving the header.
fn empty_state(ui: &mut egui::Ui) {
    let t = Tokens::get();
    egui::ScrollArea::vertical()
        .id_salt("sc_cam_toolpaths_body")
        .min_scrolled_height(0.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(t.space_4);
            ui.label(egui::RichText::new(crate::i18n::tr("ui.cam.toolpaths.empty")).color(t.text_dim).size(t.font_body));
        });
}

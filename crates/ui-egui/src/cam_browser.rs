//! The CAM workspace's Toolpaths card: the lower half of the left column, which lists the
//! machining setups a coordinate system and its operations make (`ui.cam.toolpaths`), each with
//! the toolpaths that run in it below the setup's row.
//!
//! The card is a workspace surface like the Browser above it, so it uses the shared container
//! and theme tokens and draws nothing while the space is too small to hold a header. The rows
//! are read from the design itself: a toolpath names its setup, so it is never listed under
//! another one.

use egui::{Rect, Sense, pos2, vec2};
use solidcraft_engine::doc::cam::{Toolpath, ToolpathKind};

use crate::SolveApp;
use crate::context_menu::{Item, Target};
use crate::theme::Tokens;
use crate::ui_kit::{Container, DockEdges, Surface};

/// The card's inner margin, matching the Browser card's inset.
const CARD_INSET: f32 = 4.0;
/// The header band's height, matching the Browser card's heading.
const HEADER_H: f32 = 26.0;
/// How far a toolpath row is set in from the setup that owns it.
const CHILD_INDENT: f32 = 14.0;

/// Draw the Toolpaths card in `bounds`, the Browser column's lower half, and return where it
/// was drawn. No card is drawn when the space cannot hold one.
pub(crate) fn toolpaths_panel(app: &mut SolveApp, ui: &mut egui::Ui, bounds: Rect) -> Option<Rect> {
    let t = Tokens::get();
    if !bounds.is_finite() || bounds.width() < t.control_height || bounds.height() < t.form_header_height {
        return None;
    }
    let container = Container::new(ui.ctx(), Surface::Panel).joined(DockEdges { left: true, bottom: true, ..Default::default() });
    let frame = container.frame(ui.ctx(), t.panel, crate::frame::Side::Left).inner_margin(CARD_INSET as i8);
    let inner = (bounds.size() - frame.total_margin().sum()).max(egui::Vec2::ZERO);
    let card = Container::new(ui.ctx(), Surface::Panel).area(egui::Id::unique("sc_cam_toolpaths_card")).fixed_pos(bounds.min).show(ui.ctx(), |ui| {
        container.show_frame(ui, frame, bounds, |ui| {
            ui.set_min_size(inner);
            ui.set_max_size(inner);
            ui.set_clip_rect(ui.clip_rect().intersect(Rect::from_min_size(ui.max_rect().min, inner)));
            header(ui);
            body(app, ui);
        })
    });
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

/// The setups and, below each, the toolpaths it runs. A toolpath whose setup is the model origin
/// (or a setup the design no longer has) sits under a "Model origin" row. Until the design has
/// any of them the card says so in its own scrollable body.
fn body(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    let cam = app.session.doc.cam.clone();
    if cam.setups.is_empty() && cam.toolpaths.is_empty() {
        empty_state(ui);
        return;
    }
    egui::ScrollArea::vertical().id_salt("sc_cam_toolpaths_body").min_scrolled_height(0.0).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(t.space_4);
        for setup in &cam.setups {
            let target = format!("cam:toolpaths:setup:{}", setup.id);
            group_row(ui, &setup.name, cam.toolpaths_in(setup.id).count(), &target);
            for toolpath in cam.toolpaths_in(setup.id) {
                toolpath_row(app, ui, toolpath);
            }
        }
        let unassigned: Vec<&Toolpath> = cam.toolpaths.iter().filter(|p| cam.setup(p.setup).is_none()).collect();
        if !unassigned.is_empty() {
            group_row(ui, crate::i18n::tr_label("Model origin"), unassigned.len(), "cam:toolpaths:setup:0");
            for toolpath in unassigned {
                toolpath_row(app, ui, toolpath);
            }
        }
        ui.add_space(t.space_4);
    });
}

/// A setup's own row: its name in the Browser's group style, and how many toolpaths it holds.
fn group_row(ui: &mut egui::Ui, name: &str, count: usize, target: &str) {
    let t = Tokens::get();
    let response = ui
        .horizontal(|ui| {
            ui.add_space(t.space_4);
            ui.label(egui::RichText::new(crate::i18n::tr_name(name)).font(crate::fonts::semibold(t.font_body)).color(t.text));
            if count > 0 {
                ui.label(egui::RichText::new(count.to_string()).size(t.font_small).color(t.text_dim));
            }
        })
        .response;
    crate::scenario::publish_rect(target, response.rect);
}

/// One toolpath under its setup: its name, and the operation and tool it cuts with. A right-click
/// opens its menu, where its G-code program can be exported.
fn toolpath_row(app: &mut SolveApp, ui: &mut egui::Ui, toolpath: &Toolpath) {
    let t = Tokens::get();
    let kind = match toolpath.kind {
        ToolpathKind::Facing => crate::i18n::tr_label("Facing"),
        ToolpathKind::Contour => crate::i18n::tr_label("Contour"),
        ToolpathKind::Pocket => crate::i18n::tr_label("Pocket"),
    };
    let tool = app.session.doc.cam.tool(toolpath.tool).map(|tool| tool.name.as_str()).unwrap_or("");
    let secondary = if tool.is_empty() { kind.to_string() } else { format!("{kind} · {tool}") };
    let response = ui
        .horizontal(|ui| {
            ui.add_space(t.space_4 + CHILD_INDENT);
            ui.label(egui::RichText::new(crate::i18n::tr_name(&toolpath.name)).size(t.font_body).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(t.space_4);
                ui.label(egui::RichText::new(secondary).size(t.font_small).color(t.text_dim));
            });
        })
        .response
        .interact(Sense::click());
    let target = format!("cam:toolpaths:toolpath:{}", toolpath.id);
    crate::scenario::publish_rect(&target, response.rect);
    crate::scenario::publish_handle(&target, response.rect.center());
    if response.secondary_clicked()
        && let Some(at) = response.interact_pointer_pos()
    {
        crate::context_menu::open_for(app, at, Target::CamToolpath { id: toolpath.id });
    }
}

/// One toolpath's menu: exporting it as a G-code program, once it has a feed rate the machine
/// can run at.
pub fn toolpath_items(app: &SolveApp, id: u64) -> Vec<Item> {
    let Some(toolpath) = app.session.doc.cam.toolpaths.iter().find(|p| p.id == id) else { return Vec::new() };
    let ready = toolpath.feed_rate > 0.0;
    vec![
        Item::action("ui.cam.export_gcode", "Export G-code…", "export")
            .with(serde_json::json!({ "toolpath": id }))
            .off_because((!ready).then(|| "Set a feed rate before exporting".to_string())),
    ]
}

/// What the card holds until a setup exists: a dim line that says so, in its own scrollable body
/// so a later list slots in without moving the header.
fn empty_state(ui: &mut egui::Ui) {
    let t = Tokens::get();
    egui::ScrollArea::vertical().id_salt("sc_cam_toolpaths_body").min_scrolled_height(0.0).auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(t.space_4);
        ui.label(egui::RichText::new(crate::i18n::tr("ui.cam.toolpaths.empty")).color(t.text_dim).size(t.font_body));
    });
}

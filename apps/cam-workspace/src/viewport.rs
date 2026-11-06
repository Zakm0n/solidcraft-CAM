//! The 3D viewport: an orbit camera over a scene, the controls that move it, and the overlays
//! that tell the user where they are.
//!
//! The picture is drawn by SolidCraft's render crate ([`solidcraft_render`]): the same
//! [`Camera`], scene description and rasteriser the headless snapshots use, so the window and
//! the screenshot harness show the same image. The scene is rebuilt whenever the camera or the
//! viewport changes and uploaded to egui as one texture; a still view costs nothing.
//!
//! The scene is the document: the workspace floor and origin gnomon, the wireframe of every
//! visible level, and the display mesh of every visible solid. Toolpath moves and stock join the
//! same scene in the phases that create them.

use std::sync::Arc;

use egui::{Align2, Color32, FontId, PointerButton, Rect, Sense, Stroke, TextureHandle, Ui, Vec2};
use solidcore_record::Vec3;
use solidcraft_cam::stock::StockDef;
use solidcraft_geom::Mesh;
use solidcraft_render::{Camera, Rgb, Scene, SceneLine, SceneMesh, StandardView, render};

use crate::document::CamDocument;
use crate::ribbon::{ViewCommand, ViewCommand::*};

/// Background gradient of the viewport, top and bottom: the light workspace the shell uses.
pub const BACKGROUND_TOP: Rgb = Rgb(240, 243, 247);
pub const BACKGROUND_BOTTOM: Rgb = Rgb(203, 210, 221);

/// The ground grid's minor and major line colours.
pub const GRID_MINOR: Rgb = Rgb(196, 202, 212);
pub const GRID_MAJOR: Rgb = Rgb(170, 178, 190);

/// The wireframe of the levels: a deep blue that reads on the light ground grid.
pub const WIREFRAME: Rgb = Rgb(30, 70, 140);

/// The solid bodies' fill: a cool steel that separates the part from the background.
pub const SOLID_FILL: Rgb = Rgb(138, 152, 174);

/// The stock's fill: the blueprint's translucent red, `rgba(200, 50, 50, 60)`.
pub const STOCK_FILL: Rgb = Rgb(200, 50, 50);

/// The stock box's edges: a darker red that stays readable over the see-through fill.
pub const STOCK_EDGE: Rgb = Rgb(170, 45, 45);

/// How opaque the stock is drawn: the blueprint's alpha of 60 out of 255, so the part shows
/// through it.
pub const STOCK_OPACITY: f32 = 60.0 / 255.0;

/// How opaque the stock box's edges are: nearly solid, so the box still reads as a box.
pub const STOCK_EDGE_ALPHA: f32 = 0.75;

/// Width of the stock box's edges, in output pixels.
pub const STOCK_EDGE_WIDTH: f64 = 1.4;

/// The origin axes: X, Y and Z, the same colours every SolidCraft view draws them in.
pub const AXIS_X: Rgb = Rgb(220, 60, 60);
pub const AXIS_Y: Rgb = Rgb(60, 170, 60);
pub const AXIS_Z: Rgb = Rgb(60, 100, 230);

/// How far the ground grid reaches from the origin, in mm, on each axis.
pub const GRID_EXTENT: f64 = 250.0;

/// Spacing of the grid's fine lines, in mm.
pub const GRID_STEP: f64 = 10.0;

/// Every fifth grid line is a major one, in mm.
pub const GRID_MAJOR_STEP: f64 = GRID_STEP * 5.0;

/// Length of the gnomon's three arms, in mm.
pub const GNOMON_LENGTH: f64 = 60.0;

/// Radius the camera clips against: the grid's own bounds with room to spare, so orbiting never
/// clips the floor away.
pub const SCENE_RADIUS: f64 = GRID_EXTENT * 1.5;

/// The camera's distance in the home view, in mm: the whole ground grid fits across the window.
pub const HOME_DISTANCE: f64 = 600.0;

/// How fast the wheel zooms: the exponential rate per point scrolled.
pub const ZOOM_SPEED: f64 = 0.002;

/// The scale bar aims for about this many points of length; the nearest 1-2-5 value wins.
pub const SCALE_BAR_TARGET_WIDTH: f32 = 120.0;

/// Largest side the viewport is rasterised at, in pixels: a picture bigger than this costs more
/// than it shows (the rasteriser already supersamples 2×).
pub const MAX_RENDER_SIDE: u32 = 1600;

/// One frame's worth of camera movement, in points (orbit and pan) and wheel points (zoom).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewInput {
    /// Drag delta that orbits the camera, in points.
    pub orbit: Vec2,
    /// Drag delta that pans the camera, in points.
    pub pan: Vec2,
    /// Wheel delta: positive (the wheel turned away) zooms in.
    pub zoom: f32,
}

impl ViewInput {
    /// Whether the input asks the camera to move at all.
    pub fn is_idle(self) -> bool {
        self.orbit == Vec2::ZERO && self.pan == Vec2::ZERO && self.zoom == 0.0
    }
}

/// The 3D viewport: camera, scene and overlays.
pub struct Viewport {
    /// The orbit camera the scene is drawn through.
    pub camera: Camera,
    /// The last picture uploaded to egui, with what it was rendered from.
    picture: Option<Rendered>,
}

/// An uploaded picture and the state it shows: re-rendering is skipped while this matches.
#[derive(Clone)]
struct Rendered {
    key: RenderKey,
    texture: TextureHandle,
}

/// What a picture depends on: the camera, the pixel size and the document revision. The scene is
/// deterministic, so those three decide whether the picture must be drawn again.
#[derive(Clone, Copy, PartialEq)]
struct RenderKey {
    camera: Camera,
    width: u32,
    height: u32,
    revision: u64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self { camera: home_camera(), picture: None }
    }
}

impl std::fmt::Debug for Viewport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Viewport").field("camera", &self.camera).field("rendered", &self.picture.is_some()).finish()
    }
}

/// The camera a new workspace, or Home, starts from: the isometric view of the origin from far
/// enough out that the whole ground grid is in view.
pub fn home_camera() -> Camera {
    let mut camera = Camera::default();
    camera.set_view(StandardView::Iso);
    camera.target = Vec3::ZERO;
    camera.distance = HOME_DISTANCE;
    camera
}

/// The ground grid of the XY plane, centred on the origin: fine lines every [`GRID_STEP`] and a
/// major line every [`GRID_MAJOR_STEP`]. The axes' own lines are the gnomon's, so the zeros are
/// left out.
pub fn grid_lines() -> Vec<SceneLine> {
    let mut lines = Vec::new();
    let steps = (GRID_EXTENT / GRID_STEP).floor() as i64;
    for i in -steps..=steps {
        if i == 0 {
            continue;
        }
        let at = i as f64 * GRID_STEP;
        let major = (at / GRID_MAJOR_STEP).fract().abs() < 1e-9;
        let (color, width) = if major { (GRID_MAJOR, 1.4) } else { (GRID_MINOR, 1.0) };
        // Parallel to X, then parallel to Y.
        lines.push(SceneLine {
            points: vec![Vec3::new(-GRID_EXTENT, at, 0.0), Vec3::new(GRID_EXTENT, at, 0.0)],
            color,
            alpha: 1.0,
            width,
            on_top: false,
        });
        lines.push(SceneLine {
            points: vec![Vec3::new(at, -GRID_EXTENT, 0.0), Vec3::new(at, GRID_EXTENT, 0.0)],
            color,
            alpha: 1.0,
            width,
            on_top: false,
        });
    }
    lines
}

/// The origin gnomon: the three positive axes as solid lines, X red, Y green, Z blue. Their
/// labels are painted in screen space by the viewport, so they stay readable at any distance.
pub fn gnomon_lines() -> [SceneLine; 3] {
    let arm = |color: Rgb, axis: Vec3| SceneLine { points: vec![Vec3::ZERO, axis * GNOMON_LENGTH], color, alpha: 1.0, width: 2.5, on_top: true };
    [arm(AXIS_X, Vec3::new(1.0, 0.0, 0.0)), arm(AXIS_Y, Vec3::new(0.0, 1.0, 0.0)), arm(AXIS_Z, Vec3::new(0.0, 0.0, 1.0))]
}

/// Whether `extents` describe a box with a positive, finite size.
pub fn valid_extents(extents: [f64; 6]) -> bool {
    extents.iter().all(|value| value.is_finite()) && (0..3).all(|axis| extents[axis + 3] > extents[axis])
}

/// A box's display mesh: six flat-shaded faces, so the renderer lights it like any other body.
pub fn stock_box_mesh(extents: [f64; 6]) -> Mesh {
    let [x0, y0, z0, x1, y1, z1] = extents;
    let corners = |points: [[f64; 3]; 4]| points.map(|[x, y, z]| Vec3::new(x, y, z));
    // One quad per face, counter-clockwise seen from outside, with its outward normal.
    let faces: [([[f64; 3]; 4], [f64; 3]); 6] = [
        ([[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]], [0.0, 0.0, -1.0]),
        ([[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], [0.0, 0.0, 1.0]),
        ([[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]], [0.0, -1.0, 0.0]),
        ([[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]], [0.0, 1.0, 0.0]),
        ([[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]], [-1.0, 0.0, 0.0]),
        ([[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]], [1.0, 0.0, 0.0]),
    ];
    let mut mesh = Mesh::default();
    for (index, (quad, normal)) in faces.into_iter().enumerate() {
        let base = (index * 4) as u32;
        mesh.positions.extend(corners(quad));
        mesh.normals.extend([Vec3::new(normal[0], normal[1], normal[2]); 4]);
        mesh.triangles.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
        mesh.tri_face.extend([index as u32, index as u32]);
    }
    mesh
}

/// The twelve edges of a box, in the stock's edge colour.
pub fn stock_box_lines(extents: [f64; 6]) -> Vec<SceneLine> {
    let [x0, y0, z0, x1, y1, z1] = extents;
    let corner = |x: f64, y: f64, z: f64| Vec3::new(x, y, z);
    let mut bounds = Vec::with_capacity(12);
    for z in [z0, z1] {
        bounds.push(([x0, y0, x1, y0], z));
        bounds.push(([x1, y0, x1, y1], z));
        bounds.push(([x1, y1, x0, y1], z));
        bounds.push(([x0, y1, x0, y0], z));
    }
    for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
        bounds.push(([x, y, x, y], 0.0));
    }
    let mut lines = Vec::with_capacity(12);
    for (index, (edge, z)) in bounds.into_iter().enumerate() {
        let points = if index < 8 {
            vec![corner(edge[0], edge[1], z), corner(edge[2], edge[3], z)]
        } else {
            vec![corner(edge[0], edge[1], z0), corner(edge[2], edge[3], z1)]
        };
        lines.push(SceneLine { points, color: STOCK_EDGE, alpha: STOCK_EDGE_ALPHA, width: STOCK_EDGE_WIDTH, on_top: false });
    }
    lines
}

/// The scene the viewport draws: the background, the ground grid, the gnomon, the visible
/// levels' wireframe, the visible solids and the stock.
pub fn scene(document: &CamDocument) -> Scene {
    let mut scene = Scene { background: Some((BACKGROUND_TOP, BACKGROUND_BOTTOM)), radius: SCENE_RADIUS, ..Scene::default() };
    scene.grid = grid_lines();
    let mut lines: Vec<SceneLine> = gnomon_lines().into_iter().collect();
    for wire in document.wireframe_lines() {
        lines.push(SceneLine { points: wire.points, color: WIREFRAME, alpha: 1.0, width: 1.6, on_top: true });
    }
    let mut meshes: Vec<SceneMesh> = document
        .solids()
        .iter()
        .filter(|solid| solid.visible && document.level_visible(solid.level))
        .map(|solid| SceneMesh::solid(solid.mesh.clone(), SOLID_FILL))
        .collect();
    // The stock is drawn over the part it holds: translucent, so the part still reads through
    // it, and in the blueprint's red rather than the bodies' steel.
    match document.stock() {
        Some(StockDef::Solid(id)) => {
            if let Some(solid) = document.solid_by_id(id) {
                let mut stock = SceneMesh::solid(solid.mesh.clone(), STOCK_FILL);
                stock.opacity = STOCK_OPACITY;
                meshes.push(stock);
            }
        }
        Some(StockDef::BoundingBox { .. }) => {
            if let Some(extents) = document.stock_extents().filter(|extents| valid_extents(*extents)) {
                let mut stock = SceneMesh::solid(Arc::new(stock_box_mesh(extents)), STOCK_FILL);
                stock.opacity = STOCK_OPACITY;
                meshes.push(stock);
                lines.extend(stock_box_lines(extents));
            }
        }
        None => {}
    }
    scene.lines = lines;
    scene.meshes = meshes;
    scene
}

/// The largest 1, 2 or 5 times a power of ten that is no bigger than `raw`: the length a scale
/// bar is drawn at, so its label is always a number a person reads at a glance.
pub fn nice_length(raw: f64) -> f64 {
    if !raw.is_finite() || !(raw > 0.0) {
        return 1.0;
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    for factor in [5.0, 2.0, 1.0] {
        let candidate = factor * magnitude;
        if candidate <= raw + 1e-12 {
            return candidate;
        }
    }
    magnitude
}

/// A length in millimetres as the scale bar labels it: whole numbers above 1 mm, one decimal
/// below.
pub fn format_length(length: f64) -> String {
    if length >= 1.0 { format!("{length:.0} mm") } else { format!("{length:.1} mm") }
}

/// A colour of the render crate as the shell paints it.
fn colour(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.0, rgb.1, rgb.2)
}

impl Viewport {
    /// Applies one frame's camera input: orbit, pan and zoom, each only when it moved.
    pub fn apply(&mut self, input: ViewInput, viewport_height: f32) {
        let (orbit, pan, zoom) = (input.orbit, input.pan, input.zoom);
        if orbit != Vec2::ZERO {
            self.camera.orbit(f64::from(orbit.x), f64::from(orbit.y));
        }
        if pan != Vec2::ZERO {
            self.camera.pan(f64::from(pan.x), f64::from(pan.y), f64::from(viewport_height));
        }
        if zoom != 0.0 && zoom.is_finite() {
            self.camera.zoom_at((-f64::from(zoom) * ZOOM_SPEED).exp(), None);
        }
    }

    /// The home view: the whole grid, iso, centred on the origin.
    pub fn home(&mut self) {
        self.camera = home_camera();
    }

    /// Moves the camera to a standard view, keeping its target and distance.
    pub fn set_view(&mut self, view: ViewCommand) {
        match view {
            Home => self.home(),
            Top => self.camera.set_view(StandardView::Top),
            Front => self.camera.set_view(StandardView::Front),
            Right => self.camera.set_view(StandardView::Right),
            Iso => self.camera.set_view(StandardView::Iso),
        }
    }

    /// World length one screen point covers at the camera's target: the scale bar's unit.
    pub fn world_per_point(&self, viewport_height: f32) -> f64 {
        let height = f64::from(viewport_height).max(1.0);
        2.0 * self.camera.half_height() / height
    }

    /// The camera input the pointer and wheel ask for in `rect`.
    fn read_input(ui: &Ui, response: &egui::Response) -> ViewInput {
        let mut input = ViewInput::default();
        if response.dragged() {
            // Any drag with Shift, and a right drag, pans; every other button orbits (the CAD
            // convention: drag to tumble, Shift-drag to slide the part).
            let pan = response.dragged_by(PointerButton::Secondary) || ui.input(|i| i.modifiers.shift);
            if pan {
                input.pan = response.drag_delta();
            } else {
                input.orbit = response.drag_delta();
            }
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                input.zoom = scroll;
            }
        }
        input
    }

    /// Draws the viewport into the space `ui` has left: the picture, the gnomon labels, the
    /// view controls and the scale bar.
    pub fn ui(&mut self, ui: &mut Ui, document: &CamDocument) {
        let rect = ui.available_rect_before_wrap();
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return;
        }
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let input = Self::read_input(ui, &response);
        self.apply(input, rect.height());
        if response.double_clicked() {
            self.home();
        }
        self.refresh(ui, rect, document);

        if let Some(picture) = &self.picture {
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(picture.texture.id(), rect, uv, Color32::WHITE);
        }
        self.paint_gnomon_labels(ui, rect);
        self.paint_scale_bar(ui, rect);
        // Last, so the controls sit over the picture rather than under it.
        self.view_controls(ui, rect);
    }

    /// The view controls overlaid in the viewport's top-left corner: Home and the axis views.
    fn view_controls(&mut self, ui: &mut Ui, rect: Rect) {
        let bar = Rect::from_min_size(rect.left_top() + Vec2::new(8.0, 8.0), Vec2::new((rect.width() - 16.0).max(1.0), 22.0));
        ui.scope_builder(egui::UiBuilder::new().id_salt("viewport_view_controls").max_rect(bar), |ui| {
            ui.horizontal(|ui| {
                if ui.small_button("Home").on_hover_text("Home view (double-click the viewport)").clicked() {
                    self.home();
                }
                for (label, view, hint) in [
                    ("Top", StandardView::Top, "Look down the Z axis"),
                    ("Front", StandardView::Front, "Look along the Y axis"),
                    ("Right", StandardView::Right, "Look along the X axis"),
                ] {
                    if ui.small_button(label).on_hover_text(hint).clicked() {
                        self.camera.set_view(view);
                    }
                }
            });
        });
    }

    /// Re-renders the picture when the camera or the viewport changed, and uploads it. The
    /// picture is drawn at the screen's pixel density, up to [`MAX_RENDER_SIDE`].
    fn refresh(&mut self, ui: &Ui, rect: Rect, document: &CamDocument) {
        let ppp = ui.ctx().pixels_per_point().clamp(0.5, 4.0);
        let width = ((rect.width() * ppp).round() as u32).clamp(1, MAX_RENDER_SIDE);
        let height = ((rect.height() * ppp).round() as u32).clamp(1, MAX_RENDER_SIDE);
        let key = RenderKey { camera: self.camera, width, height, revision: document.revision() };
        if self.picture.as_ref().is_some_and(|picture| picture.key == key) {
            return;
        }
        let canvas = render(&scene(document), &self.camera, width as usize, height as usize);
        let image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &canvas.rgba);
        let texture = ui.ctx().load_texture("cam_viewport", image, egui::TextureOptions::LINEAR);
        self.picture = Some(Rendered { key, texture });
    }

    /// The X, Y and Z labels at the gnomon's tips, projected through the camera.
    fn paint_gnomon_labels(&self, ui: &Ui, rect: Rect) {
        let painter = ui.painter();
        let (width, height) = (f64::from(rect.width()), f64::from(rect.height()));
        for (label, axis, rgb) in
            [("X", Vec3::new(1.0, 0.0, 0.0), AXIS_X), ("Y", Vec3::new(0.0, 1.0, 0.0), AXIS_Y), ("Z", Vec3::new(0.0, 0.0, 1.0), AXIS_Z)]
        {
            let tip = axis * (GNOMON_LENGTH * 1.1);
            let Some((x, y, _)) = self.camera.to_screen(tip, width, height, SCENE_RADIUS) else { continue };
            let at = rect.min + Vec2::new(x as f32, y as f32);
            painter.text(at, Align2::CENTER_CENTER, label, FontId::proportional(13.0), colour(rgb));
        }
    }

    /// The scale bar in the bottom-right corner: a 1-2-5 length that is close to
    /// [`SCALE_BAR_TARGET_WIDTH`] points wide, labelled in millimetres.
    fn paint_scale_bar(&self, ui: &Ui, rect: Rect) {
        let world_per_point = self.world_per_point(rect.height());
        if !world_per_point.is_finite() || !(world_per_point > 0.0) {
            return;
        }
        let length = nice_length(world_per_point * f64::from(SCALE_BAR_TARGET_WIDTH));
        let width = (length / world_per_point) as f32;
        if !width.is_finite() || width < 1.0 {
            return;
        }
        let painter = ui.painter();
        let inset = 16.0;
        let right = rect.right() - inset;
        let left = (right - width).max(rect.left() + inset);
        let base = rect.bottom() - inset;
        let stroke = Stroke::new(1.5, Color32::from_rgb(60, 70, 85));
        painter.line_segment([egui::pos2(left, base), egui::pos2(right, base)], stroke);
        for x in [left, right] {
            painter.line_segment([egui::pos2(x, base), egui::pos2(x, base - 6.0)], stroke);
        }
        painter.text(
            egui::pos2(right, base - 8.0),
            Align2::RIGHT_BOTTOM,
            format_length(length),
            FontId::proportional(12.0),
            Color32::from_rgb(45, 55, 70),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_view_is_isometric_and_frames_the_whole_grid() {
        let viewport = Viewport::default();
        assert_eq!(viewport.camera, home_camera());
        assert!((viewport.camera.distance - HOME_DISTANCE).abs() < 1e-9);
        assert_eq!(viewport.camera.target, Vec3::ZERO);
        assert_eq!(viewport.camera.fov, 0.0, "the workspace starts orthographic");
        // The visible height at the home distance covers the grid with room to spare.
        assert!(2.0 * viewport.camera.half_height() > GRID_EXTENT);
    }

    #[test]
    fn orbiting_turns_the_camera_about_its_target() {
        let mut viewport = Viewport::default();
        let (yaw, pitch) = (viewport.camera.yaw, viewport.camera.pitch);
        viewport.apply(ViewInput { orbit: Vec2::new(40.0, 12.0), ..ViewInput::default() }, 600.0);
        assert!((viewport.camera.yaw - yaw).abs() > 1e-6, "a horizontal drag turns the view");
        assert!((viewport.camera.pitch - pitch).abs() > 1e-6, "a vertical drag tilts the view");
        assert_eq!(viewport.camera.target, Vec3::ZERO, "an orbit keeps the target");
    }

    #[test]
    fn panning_moves_the_target_without_turning_the_view() {
        let mut viewport = Viewport::default();
        let (yaw, pitch, distance) = (viewport.camera.yaw, viewport.camera.pitch, viewport.camera.distance);
        viewport.apply(ViewInput { pan: Vec2::new(30.0, -20.0), ..ViewInput::default() }, 600.0);
        assert_ne!(viewport.camera.target, Vec3::ZERO);
        assert_eq!((viewport.camera.yaw, viewport.camera.pitch, viewport.camera.distance), (yaw, pitch, distance));
    }

    #[test]
    fn the_wheel_zooms_in_and_out_around_the_target() {
        let mut viewport = Viewport::default();
        let distance = viewport.camera.distance;
        viewport.apply(ViewInput { zoom: 120.0, ..ViewInput::default() }, 600.0);
        assert!(viewport.camera.distance < distance, "the wheel turned away zooms in");
        let close = viewport.camera.distance;
        viewport.apply(ViewInput { zoom: -120.0, ..ViewInput::default() }, 600.0);
        assert!(viewport.camera.distance > close, "the wheel turned toward the user zooms out");
        assert_eq!(viewport.camera.target, Vec3::ZERO);
    }

    #[test]
    fn idle_input_leaves_the_camera_alone() {
        let mut viewport = Viewport::default();
        let camera = viewport.camera;
        viewport.apply(ViewInput::default(), 600.0);
        assert_eq!(viewport.camera, camera);
        assert!(ViewInput::default().is_idle());
    }

    #[test]
    fn the_ground_grid_covers_the_origin_with_major_lines_every_fifth() {
        let lines = grid_lines();
        assert_eq!(lines.len(), 100, "ten steps each way, both directions, without the axes' zeros");
        let majors = lines.iter().filter(|line| line.color == GRID_MAJOR).count();
        assert_eq!(majors, 20, "one line in five is major, in both directions");
        assert!(lines.iter().all(|line| line.points.iter().all(|p| p.z == 0.0)));
    }

    #[test]
    fn the_gnomon_draws_the_three_positive_axes_at_the_origin() {
        let lines = gnomon_lines();
        assert_eq!(lines.len(), 3);
        for (line, (axis, rgb)) in lines.iter().zip([
            (Vec3::new(GNOMON_LENGTH, 0.0, 0.0), AXIS_X),
            (Vec3::new(0.0, GNOMON_LENGTH, 0.0), AXIS_Y),
            (Vec3::new(0.0, 0.0, GNOMON_LENGTH), AXIS_Z),
        ]) {
            assert_eq!(line.points, [Vec3::ZERO, axis]);
            assert_eq!(line.color, rgb);
            assert!(line.on_top, "the gnomon reads over the grid");
        }
    }

    #[test]
    fn the_scene_draws_the_grid_and_the_gnomon() {
        let document = CamDocument::default();
        let scene = scene(&document);
        assert_eq!(scene.grid.len(), 100);
        assert_eq!(scene.lines.len(), 3);
        assert!(scene.meshes.is_empty(), "an empty document draws no bodies");
        assert_eq!(scene.background, Some((BACKGROUND_TOP, BACKGROUND_BOTTOM)));
        assert_eq!(scene.radius, SCENE_RADIUS);
    }

    #[test]
    fn the_scene_draws_the_documents_wireframe_and_solids() {
        use crate::document::WireTool;

        let mut document = CamDocument::default();
        document.draw(WireTool::Rectangle).unwrap_or_default();
        document.extrude(10.0).unwrap_or_default();
        let drawn = scene(&document);
        assert_eq!(drawn.lines.len(), 3 + 4, "the gnomon and the rectangle's four lines");
        assert!(drawn.lines[3..].iter().all(|line| line.color == WIREFRAME && line.on_top));
        assert_eq!(drawn.meshes.len(), 1);
        assert_eq!(drawn.meshes[0].color, SOLID_FILL);
        assert!(drawn.meshes[0].closed);

        document.set_solid_visible(document.solids()[0].id, false);
        assert!(scene(&document).meshes.is_empty(), "a hidden body is not drawn");
        document.set_solid_visible(document.solids()[0].id, true);
        document.set_level_visible(1, false);
        let hidden = scene(&document);
        assert!(hidden.meshes.is_empty(), "a body on a hidden level is not drawn");
        assert_eq!(hidden.lines.len(), 3, "and neither is its wireframe");
    }

    #[test]
    fn a_standard_view_moves_the_camera() {
        let mut viewport = Viewport::default();
        viewport.set_view(ViewCommand::Top);
        assert!((viewport.camera.pitch - std::f64::consts::FRAC_PI_2).abs() < 1e-5, "the Top view looks down the Z axis");
        viewport.set_view(ViewCommand::Home);
        assert_eq!(viewport.camera, home_camera());
        viewport.set_view(ViewCommand::Iso);
        assert_eq!(viewport.camera.distance, HOME_DISTANCE, "a view keeps the home distance");
    }

    #[test]
    fn scale_bar_lengths_are_one_two_or_five() {
        for (raw, expected) in [(9.9, 5.0), (12.0, 10.0), (20.0, 20.0), (480.0, 200.0), (0.7, 0.5)] {
            assert_eq!(nice_length(raw), expected, "raw {raw}");
        }
        assert_eq!(nice_length(f64::NAN), 1.0);
        assert_eq!(nice_length(0.0), 1.0);
    }

    #[test]
    fn the_scale_bar_width_stays_near_its_target() {
        let viewport = Viewport::default();
        let height = 600.0;
        let world_per_point = viewport.world_per_point(height);
        let width = nice_length(world_per_point * f64::from(SCALE_BAR_TARGET_WIDTH)) / world_per_point;
        assert!(width > 24.0 && width <= f64::from(SCALE_BAR_TARGET_WIDTH), "bar {width} points wide");
    }

    #[test]
    fn lengths_are_labelled_in_millimetres() {
        assert_eq!(format_length(50.0), "50 mm");
        assert_eq!(format_length(0.5), "0.5 mm");
    }

    /// A document with one extruded plate and a box of stock around it.
    fn stocked_document() -> CamDocument {
        use solidcraft_cam::stock::StockExcess;

        let mut document = CamDocument::default();
        document.draw(crate::document::WireTool::Rectangle).unwrap_or_default();
        document.extrude(10.0).unwrap_or_default();
        let (min, max) = document.part_bounds().unwrap_or_default();
        document.set_stock(Some(StockDef::from_bounds(min, max, StockExcess::new(5.0, 5.0, 2.0))));
        document
    }

    #[test]
    fn the_scene_draws_the_stock_box_translucently() {
        let document = stocked_document();
        let scene = scene(&document);
        assert_eq!(scene.lines.len(), 3 + 4 + 12, "the gnomon, the plate's wireframe and the box's edges");
        assert_eq!(scene.meshes.len(), 2, "the plate and the stock");
        let stock = &scene.meshes[1];
        assert_eq!(stock.color, STOCK_FILL);
        assert!(stock.glass(), "the stock is drawn see-through");
        assert!((stock.alpha() - f64::from(STOCK_OPACITY)).abs() < 1e-6);
        assert!(scene.lines[7..].iter().all(|line| line.color == STOCK_EDGE && line.alpha == STOCK_EDGE_ALPHA));
    }

    #[test]
    fn the_stock_box_mesh_is_six_quads_around_its_extents() {
        let mesh = stock_box_mesh([0.0, 0.0, 0.0, 10.0, 20.0, 30.0]);
        assert_eq!(mesh.positions.len(), 24, "four vertices per face");
        assert_eq!(mesh.normals.len(), 24);
        assert_eq!(mesh.triangles.len(), 12);
        assert_eq!(mesh.positions.iter().map(|p| p.x).fold(f64::INFINITY, f64::min), 0.0);
        assert_eq!(mesh.positions.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max), 30.0);
        assert!(mesh.normals.iter().all(|normal| (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z - 1.0).abs() < 1e-9));
    }

    #[test]
    fn a_solid_stock_draws_that_body_in_the_stock_colour() {
        let mut document = stocked_document();
        let id = document.solids()[0].id;
        document.set_stock(Some(StockDef::Solid(id)));
        let scene = scene(&document);
        assert_eq!(scene.meshes.len(), 2, "the part and the stock body");
        assert_eq!(scene.meshes[1].color, STOCK_FILL);
        assert!(scene.meshes[1].glass());
        assert_eq!(scene.lines.len(), 3 + 4, "a body stock has no box edges of its own");
    }

    #[test]
    fn no_stock_and_an_empty_box_add_nothing() {
        let mut document = CamDocument::default();
        assert!(scene(&document).meshes.is_empty());
        document.set_stock(Some(StockDef::BoundingBox { extents: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0] }));
        let scene = scene(&document);
        assert!(scene.meshes.is_empty(), "a box with no size is not drawn");
        assert_eq!(scene.lines.len(), 3);
        assert!(!valid_extents([0.0, 0.0, 0.0, 0.0, 1.0, 1.0]));
        assert!(valid_extents([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]));
    }
}

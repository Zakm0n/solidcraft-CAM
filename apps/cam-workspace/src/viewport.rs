//! The 3D viewport: an orbit camera over a scene, the controls that move it, and the overlays
//! that tell the user where they are.
//!
//! The picture is drawn by SolidCraft's render crate ([`solidcraft_render`]): the same
//! [`Camera`], scene description and rasteriser the headless snapshots use, so the window and
//! the screenshot harness show the same image. The scene is rebuilt whenever the camera or the
//! viewport changes and uploaded to egui as one texture; a still view costs nothing.
//!
//! The scene is the document: the workspace floor and origin gnomon, the wireframe of every
//! visible level, and the display mesh of every visible solid. The toolpaths join it in yellow
//! and cyan, and a simulation joins it again: the stock as the surface its height map has been
//! cut to and the tool as a cylinder at the point the clock has reached.

use std::sync::Arc;

use egui::{Align2, Color32, FontId, PointerButton, Rect, Sense, Stroke, TextureHandle, Ui, Vec2};
use solidcore_record::{Plane, Vec3};
use solidcraft_cam::operations::Operation;
use solidcraft_cam::simulation::{Simulation, StockGrid};
use solidcraft_cam::stock::StockDef;
use solidcraft_cam::tool::CuttingTool;
use solidcraft_cam::toolpath::MoveKind;
use solidcraft_geom::Mesh;
use solidcraft_render::{Camera, Rgb, Scene, SceneLine, SceneMesh, StandardView, render};

use crate::document::{CamDocument, WireTool, standard_plane};
use crate::interactive_geom::{
    ARC_PREVIEW_SEGMENTS, ClickOutcome, InteractiveGeometry, PendingWire, SNAP_RADIUS_PIXELS, SnapCandidate, SnapKind, WireframeTool, snap_candidates,
};
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

/// The toolpath's rapid moves: the blueprint's yellow, so G0 reads at a glance against the light
/// workspace.
pub const TOOLPATH_RAPID: Rgb = Rgb(255, 255, 0);

/// The toolpath's feed moves: the blueprint's cyan, for G1, G2 and G3 alike.
pub const TOOLPATH_FEED: Rgb = Rgb(0, 255, 255);

/// Width of a toolpath line, in output pixels: a cutting move is drawn on top of the model, so
/// it reads over the wireframe and the stock without hiding them.
pub const TOOLPATH_WIDTH: f64 = 1.8;

/// The tool's fill during a simulation: a steel blue that separates the cutter from the part,
/// the stock and both toolpath colours.
pub const TOOL_FILL: Rgb = Rgb(70, 120, 190);

/// How opaque the simulated tool is: solid metal, not glass.
pub const TOOL_OPACITY: f32 = 1.0;

/// How many facets the simulated tool's cylinder is drawn with: round enough at the zoom the
/// verification uses, few enough to keep the picture cheap.
pub const TOOL_FACETS: usize = 24;

/// The origin axes: X, Y and Z, the same colours every SolidCraft view draws them in.
pub const AXIS_X: Rgb = Rgb(220, 60, 60);
pub const AXIS_Y: Rgb = Rgb(60, 170, 60);
pub const AXIS_Z: Rgb = Rgb(60, 100, 230);

/// The live wireframe preview's stroke: the cyan the blueprint gives the entity being placed.
pub const PREVIEW_COLOR: Color32 = Color32::from_rgb(0, 255, 255);

/// Width of the live preview's stroke, in points: thin enough to read the geometry under it.
pub const PREVIEW_WIDTH: f32 = 1.5;

/// The AutoCursor marker's colour: green, so it separates from the cyan preview.
pub const SNAP_COLOR: Color32 = Color32::GREEN;

/// The AutoCursor marker's radius, in points: the square, triangle and circle all reach this far
/// from the snapped point.
pub const SNAP_RADIUS: f32 = 4.0;

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
    /// The clicking wireframe tool, and where the pointer is on the construction plane.
    pub interactive: InteractiveGeometry,
    /// The document's snap candidates, gathered at `snap_revision` and reused while it holds.
    snap_candidates: Vec<SnapCandidate>,
    /// The document revision `snap_candidates` describes; `u64::MAX` until they are gathered.
    snap_revision: u64,
    /// The last picture uploaded to egui, with what it was rendered from.
    picture: Option<Rendered>,
}

/// An uploaded picture and the state it shows: re-rendering is skipped while this matches.
#[derive(Clone)]
struct Rendered {
    key: RenderKey,
    texture: TextureHandle,
}

/// What a picture depends on: the camera, the pixel size, the document revision and the
/// toolpaths' revisions. The scene is deterministic, so those decide whether the picture must be
/// drawn again.
#[derive(Clone, Copy, PartialEq)]
struct RenderKey {
    camera: Camera,
    width: u32,
    height: u32,
    revision: u64,
    toolpaths: u64,
    simulation: u64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            camera: home_camera(),
            interactive: InteractiveGeometry::default(),
            snap_candidates: Vec::new(),
            snap_revision: u64::MAX,
            picture: None,
        }
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
/// levels' wireframe, the visible operations' toolpaths, the visible solids, the stock, and — once
/// a simulation has started — the stock's machined surface and the tool.
pub fn scene(document: &CamDocument, operations: &[Operation], simulation: Option<&Simulation>) -> Scene {
    let mut scene = Scene { background: Some((BACKGROUND_TOP, BACKGROUND_BOTTOM)), radius: SCENE_RADIUS, ..Scene::default() };
    scene.grid = grid_lines();
    let mut lines: Vec<SceneLine> = gnomon_lines().into_iter().collect();
    for wire in document.wireframe_lines() {
        lines.push(SceneLine { points: wire.points, color: WIREFRAME, alpha: 1.0, width: 1.6, on_top: true });
    }
    lines.extend(toolpath_lines(operations));
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
            // Once the simulation has cut, its height map is the stock: the surface shows the
            // channel the tool has cleared instead of the untouched box.
            let machined = simulation.and_then(|simulation| simulation.grid().filter(|_| simulation.machined() > 0.0));
            if let Some(grid) = machined {
                let mut stock = SceneMesh::solid(Arc::new(stock_grid_mesh(grid)), STOCK_FILL);
                stock.opacity = STOCK_OPACITY;
                meshes.push(stock);
            } else if let Some(extents) = document.stock_extents().filter(|extents| valid_extents(*extents)) {
                let mut stock = SceneMesh::solid(Arc::new(stock_box_mesh(extents)), STOCK_FILL);
                stock.opacity = STOCK_OPACITY;
                meshes.push(stock);
                lines.extend(stock_box_lines(extents));
            }
        }
        None => {}
    }
    // The cutter itself, at the point the clock has reached: a cylinder whose tip is the move's
    // end, so the verification shows what is taking the material off.
    if let Some(simulation) = simulation
        && simulation.machined() > 0.0
        && let Some(sample) = simulation.sample()
        && let Some(tool) = simulation.tool()
        && let Some(mesh) = tool_cylinder_mesh(sample.position, tool)
    {
        let mut cutter = SceneMesh::solid(Arc::new(mesh), TOOL_FILL);
        cutter.opacity = TOOL_OPACITY;
        meshes.push(cutter);
    }
    scene.lines = lines;
    scene.meshes = meshes;
    scene
}

/// A quad of a display mesh in `mesh`, flat-shaded with its own outward normal and filed under
/// `face`, so the renderer lights it like a body face.
fn push_quad(mesh: &mut Mesh, quad: [Vec3; 4], face: u32) {
    let normal = quad_normal(quad);
    let base = mesh.positions.len() as u32;
    mesh.positions.extend(quad);
    mesh.normals.extend([normal; 4]);
    mesh.triangles.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
    mesh.tri_face.extend([face, face]);
}

/// The outward normal of a quad whose corners run counter-clockwise seen from outside; +Z when
/// the quad is degenerate (a flat, zero-area face lights as if it faced up).
fn quad_normal([a, b, c, _]: [Vec3; 4]) -> Vec3 {
    let (ux, uy, uz) = (b.x - a.x, b.y - a.y, b.z - a.z);
    let (vx, vy, vz) = (c.x - a.x, c.y - a.y, c.z - a.z);
    let (nx, ny, nz) = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
    let length = (nx * nx + ny * ny + nz * nz).sqrt();
    if length > 1e-12 { Vec3::new(nx / length, ny / length, nz / length) } else { Vec3::new(0.0, 0.0, 1.0) }
}

/// The stock's machined surface as a display mesh: its top from the height map, a wall down to
/// the stock's bottom on each edge, and the bottom, so the see-through stock still reads as a
/// block with a channel cut into it.
pub fn stock_grid_mesh(grid: &StockGrid) -> Mesh {
    let mut mesh = Mesh::default();
    let (columns, rows) = (grid.columns(), grid.rows());
    let [x0, y0, z0, x1, y1, _] = grid.extents();
    // The top: one flat-shaded quad per height-map cell.
    for row in 0..rows.saturating_sub(1) {
        for column in 0..columns.saturating_sub(1) {
            let (ax, bx) = (grid.x_of(column), grid.x_of(column + 1));
            let (ay, by) = (grid.y_of(row), grid.y_of(row + 1));
            let quad = [
                Vec3::new(ax, ay, grid.height(column, row)),
                Vec3::new(bx, ay, grid.height(column + 1, row)),
                Vec3::new(bx, by, grid.height(column + 1, row + 1)),
                Vec3::new(ax, by, grid.height(column, row + 1)),
            ];
            push_quad(&mut mesh, quad, 0);
        }
    }
    // The four walls, from the surface's edges down to the stock's bottom.
    for column in 0..columns.saturating_sub(1) {
        let (ax, bx) = (grid.x_of(column), grid.x_of(column + 1));
        let (ha, hb) = (grid.height(column, 0), grid.height(column + 1, 0));
        push_quad(&mut mesh, [Vec3::new(ax, y0, ha), Vec3::new(bx, y0, hb), Vec3::new(bx, y0, z0), Vec3::new(ax, y0, z0)], 1);
        let (ha, hb) = (grid.height(column, rows.saturating_sub(1)), grid.height(column + 1, rows.saturating_sub(1)));
        push_quad(&mut mesh, [Vec3::new(bx, y1, hb), Vec3::new(ax, y1, ha), Vec3::new(ax, y1, z0), Vec3::new(bx, y1, z0)], 1);
    }
    for row in 0..rows.saturating_sub(1) {
        let (ay, by) = (grid.y_of(row), grid.y_of(row + 1));
        let (ha, hb) = (grid.height(0, row), grid.height(0, row + 1));
        push_quad(&mut mesh, [Vec3::new(x0, by, hb), Vec3::new(x0, ay, ha), Vec3::new(x0, ay, z0), Vec3::new(x0, by, z0)], 1);
        let (ha, hb) = (grid.height(columns.saturating_sub(1), row), grid.height(columns.saturating_sub(1), row + 1));
        push_quad(&mut mesh, [Vec3::new(x1, ay, ha), Vec3::new(x1, by, hb), Vec3::new(x1, by, z0), Vec3::new(x1, ay, z0)], 1);
    }
    push_quad(&mut mesh, [Vec3::new(x0, y0, z0), Vec3::new(x0, y1, z0), Vec3::new(x1, y1, z0), Vec3::new(x1, y0, z0)], 2);
    mesh
}

/// The simulated tool as a display mesh: a cylinder of the tool's diameter and flute length whose
/// bottom face is at `tip`, so the cutter sits exactly where the toolpath clock has it.
pub fn tool_cylinder_mesh(tip: [f64; 3], tool: &CuttingTool) -> Option<Mesh> {
    let radius = tool.diameter / 2.0;
    let height = tool.flute_length;
    if !tip.iter().all(|value| value.is_finite()) || !(radius > 0.0) || !radius.is_finite() || !(height > 0.0) || !height.is_finite() {
        return None;
    }
    let (cx, cy, bottom) = (tip[0], tip[1], tip[2]);
    let top = bottom + height;
    let mut mesh = Mesh::default();
    let step = std::f64::consts::TAU / TOOL_FACETS as f64;
    for facet in 0..TOOL_FACETS {
        let (a, b) = (facet as f64 * step, (facet + 1) as f64 * step);
        let (ax, ay) = (cx + radius * a.cos(), cy + radius * a.sin());
        let (bx, by) = (cx + radius * b.cos(), cy + radius * b.sin());
        let normal = |x: f64, y: f64| Vec3::new((x - cx) / radius, (y - cy) / radius, 0.0);
        let base = mesh.positions.len() as u32;
        mesh.positions.extend([Vec3::new(ax, ay, bottom), Vec3::new(bx, by, bottom), Vec3::new(bx, by, top), Vec3::new(ax, ay, top)]);
        mesh.normals.extend([normal(ax, ay), normal(bx, by), normal(bx, by), normal(ax, ay)]);
        mesh.triangles.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
        mesh.tri_face.extend([0, 0]);
        // The caps, as fans from the centre: the bottom faces down, the top up.
        let centre = mesh.positions.len() as u32;
        mesh.positions.extend([Vec3::new(cx, cy, bottom), Vec3::new(ax, ay, bottom), Vec3::new(bx, by, bottom)]);
        mesh.normals.extend([Vec3::new(0.0, 0.0, -1.0); 3]);
        mesh.triangles.push([centre, centre + 2, centre + 1]);
        mesh.tri_face.push(1);
        let centre = mesh.positions.len() as u32;
        mesh.positions.extend([Vec3::new(cx, cy, top), Vec3::new(ax, ay, top), Vec3::new(bx, by, top)]);
        mesh.normals.extend([Vec3::new(0.0, 0.0, 1.0); 3]);
        mesh.triangles.push([centre, centre + 1, centre + 2]);
        mesh.tri_face.push(2);
    }
    Some(mesh)
}

/// The visible operations' moves as scene lines: every move its own polyline, in the rapid
/// yellow or the feed cyan, drawn over the model.
pub fn toolpath_lines(operations: &[Operation]) -> Vec<SceneLine> {
    let mut lines = Vec::new();
    for operation in operations.iter().filter(|operation| operation.visible) {
        for (kind, points) in operation.toolpath.segments() {
            if points.len() < 2 {
                continue;
            }
            let colour = match kind {
                MoveKind::Rapid => TOOLPATH_RAPID,
                MoveKind::Feed => TOOLPATH_FEED,
            };
            lines.push(SceneLine {
                points: points.into_iter().map(|point| Vec3::new(point[0], point[1], point[2])).collect(),
                color: colour,
                alpha: 1.0,
                width: TOOLPATH_WIDTH,
                on_top: true,
            });
        }
    }
    lines
}

/// The revision of what the toolpaths contribute to the picture: each operation's own revision
/// and its visibility decide whether the viewport must draw the scene again.
pub fn toolpaths_key(operations: &[Operation]) -> u64 {
    operations.iter().fold(0, |key, operation| {
        let visible = u64::from(operation.visible);
        key.rotate_left(9) ^ operation.revision.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ visible
    })
}

/// What the simulation contributes to the picture: its position, the path it runs, the tool and
/// the stock the height map is sampled over. The scene is deterministic, so this decides whether
/// a playing or scrubbed simulation must draw the picture again.
pub fn simulation_key(simulation: Option<&Simulation>) -> u64 {
    let Some(simulation) = simulation else { return 0 };
    let mut key = simulation.position().to_bits().rotate_left(7) ^ simulation.total_length().to_bits();
    if let Some(tool) = simulation.tool() {
        key = key.rotate_left(7) ^ tool.diameter.to_bits() ^ tool.effective_corner_radius().to_bits().rotate_left(3);
    }
    if let Some(grid) = simulation.grid() {
        for extent in grid.extents() {
            key = key.rotate_left(11) ^ extent.to_bits();
        }
    }
    key
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

/// The snap under `pointer`: the candidate whose projection is nearest, within
/// [`SNAP_RADIUS_PIXELS`] of the pointer. A candidate exactly as near as the winner loses to the
/// higher-priority kind, so a vertex beats the midpoint that happens to project onto it.
fn pick_snap(candidates: &[SnapCandidate], rect: Rect, pointer: egui::Pos2, camera: &Camera) -> Option<SnapCandidate> {
    let (width, height) = (f64::from(rect.width()), f64::from(rect.height()));
    let pointer = pointer - rect.min;
    let mut best: Option<(f32, u8, SnapCandidate)> = None;
    for candidate in candidates {
        let Some((x, y, _)) = camera.to_screen(candidate.at, width, height, SCENE_RADIUS) else { continue };
        let distance = (x as f32 - pointer.x).hypot(y as f32 - pointer.y);
        if !distance.is_finite() || distance > SNAP_RADIUS_PIXELS {
            continue;
        }
        let priority = candidate.kind.priority();
        let wins = best.is_none_or(|(best_distance, best_priority, _)| {
            distance < best_distance - 1e-6 || ((distance - best_distance).abs() <= 1e-6 && priority < best_priority)
        });
        if wins {
            best = Some((distance, priority, *candidate));
        }
    }
    best.map(|(_, _, candidate)| candidate)
}

/// Draws the AutoCursor marker for a snap `kind` at `at`: a square for an endpoint, a triangle
/// for a midpoint, and a circle for a centre, each [`SNAP_RADIUS`] across.
fn paint_snap_marker(painter: &egui::Painter, kind: SnapKind, at: egui::Pos2) {
    let stroke = Stroke::new(PREVIEW_WIDTH, SNAP_COLOR);
    match kind {
        SnapKind::Endpoint => {
            let corners = [at + Vec2::new(-SNAP_RADIUS, -SNAP_RADIUS), at + Vec2::new(SNAP_RADIUS, -SNAP_RADIUS), at + Vec2::new(SNAP_RADIUS, SNAP_RADIUS), at + Vec2::new(-SNAP_RADIUS, SNAP_RADIUS)];
            painter.add(egui::Shape::closed_line(corners.to_vec(), stroke));
        }
        SnapKind::Midpoint => {
            let half_base = SNAP_RADIUS * 0.866;
            let corners = [at + Vec2::new(0.0, -SNAP_RADIUS), at + Vec2::new(half_base, SNAP_RADIUS * 0.5), at + Vec2::new(-half_base, SNAP_RADIUS * 0.5)];
            painter.add(egui::Shape::closed_line(corners.to_vec(), stroke));
        }
        SnapKind::Centre => {
            painter.circle_stroke(at, SNAP_RADIUS, stroke);
        }
    }
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

    /// Starts a clicking wireframe tool on `plane`: the pointer's position on that plane follows
    /// the cursor, and clicks place the tool's points.
    pub fn begin_wireframe_tool(&mut self, tool: WireTool, plane: Plane) -> bool {
        self.interactive.begin(tool, plane)
    }

    /// Places the live tool's next point where the pointer is — over the projection, the snap,
    /// or as the typed dimensions moved it — and hands back the wireframe entity when that
    /// completed one. `None` while the tool is still collecting points, or when no tool is live.
    pub fn place_point(&mut self) -> Option<PendingWire> {
        let at = self.interactive.cursor()?;
        match self.interactive.click(at) {
            ClickOutcome::Complete(wire) => Some(wire),
            ClickOutcome::Ignored | ClickOutcome::Progress => None,
        }
    }

    /// Ends the live clicking tool, if any, dropping the points it had anchored.
    pub fn cancel_wireframe_tool(&mut self) {
        self.interactive.cancel();
    }

    /// Commits the point under the pointer, as the tool panel's Create button does, and hands
    /// back the wireframe entity when that completed one.
    pub fn commit_cursor(&mut self) -> Option<PendingWire> {
        match self.interactive.commit() {
            ClickOutcome::Complete(wire) => Some(wire),
            ClickOutcome::Ignored | ClickOutcome::Progress => None,
        }
    }

    /// Follows the pointer across the active construction plane: the world point a click would
    /// land on, which is what the wireframe tools anchor and what the coordinate readout shows.
    /// A pointer that is not over the viewport, or a view where the plane is edge-on, leaves no
    /// point at all rather than a stale one.
    ///
    /// While a clicking tool is live the pointer snaps to a document feature within
    /// [`SNAP_RADIUS_PIXELS`], and the rubber band ends on that feature instead.
    fn track_pointer(&mut self, rect: Rect, pointer: Option<egui::Pos2>, document: &CamDocument) {
        self.interactive.set_plane(standard_plane(document.active_plane()));
        let Some(position) = pointer else {
            // A live tool keeps the last point it followed: the user may be typing its
            // dimensions into the tool panel, and the rubber band shows what Create commits.
            if !self.interactive.is_active() {
                self.interactive.clear_hover();
            }
            return;
        };
        let x = f64::from(position.x - rect.min.x);
        let y = f64::from(position.y - rect.min.y);
        let (origin, direction) = self.camera.ray(x, y, f64::from(rect.width()), f64::from(rect.height()));
        if self.interactive.track_ray(origin, direction).is_none() {
            self.interactive.clear_hover();
            return;
        }
        if !self.interactive.is_active() {
            return;
        }
        let revision = document.revision();
        if revision != self.snap_revision {
            self.snap_candidates = snap_candidates(document);
            self.snap_revision = revision;
        }
        let snap = pick_snap(&self.snap_candidates, rect, position, &self.camera);
        self.interactive.set_snap(snap);
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
    ///
    /// A primary click over the viewport places the live tool's next point; every wireframe
    /// entity that click completed is returned, for the application to create in the document.
    /// Escape ends the live tool.
    pub fn ui(&mut self, ui: &mut Ui, document: &CamDocument, operations: &[Operation], simulation: Option<&Simulation>) -> Vec<PendingWire> {
        let rect = ui.available_rect_before_wrap();
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return Vec::new();
        }
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let input = Self::read_input(ui, &response);
        self.apply(input, rect.height());
        if response.double_clicked() {
            self.home();
        }
        self.track_pointer(rect, response.hover_pos(), document);
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel_wireframe_tool();
        }
        let completed = if self.interactive.is_active() && response.clicked_by(PointerButton::Primary) {
            self.place_point().into_iter().collect()
        } else {
            Vec::new()
        };
        self.refresh(ui, rect, document, operations, simulation);

        if let Some(picture) = &self.picture {
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(picture.texture.id(), rect, uv, Color32::WHITE);
        }
        self.paint_wireframe(ui, rect);
        self.paint_gnomon_labels(ui, rect);
        self.paint_scale_bar(ui, rect);
        // Last, so the controls sit over the picture rather than under it.
        self.view_controls(ui, rect);
        completed
    }

    /// Paints the live wireframe tool over the picture: the cyan rubber band from the anchored
    /// points to the pointer, a marker on each anchor, and the green AutoCursor marker of the
    /// document feature the pointer has snapped to.
    fn paint_wireframe(&self, ui: &Ui, rect: Rect) {
        if !self.interactive.is_active() {
            return;
        }
        let painter = ui.painter();
        let project = |point: Vec3| self.project(point, rect);
        let stroke = Stroke::new(PREVIEW_WIDTH, PREVIEW_COLOR);

        for polyline in self.interactive.preview(ARC_PREVIEW_SEGMENTS) {
            let points: Vec<egui::Pos2> = polyline.iter().filter_map(|point| project(*point)).collect();
            if points.len() == 2 {
                painter.line_segment([points[0], points[1]], stroke);
            } else if points.len() > 2 {
                painter.add(egui::Shape::line(points, stroke));
            }
        }

        // The anchored points, as small crosses: where the entity already starts.
        for anchor in self.interactive.anchors() {
            if let Some(at) = project(anchor) {
                let arm = Vec2::splat(SNAP_RADIUS);
                painter.line_segment([at - arm, at + arm], stroke);
            }
        }

        // The point tool has no band: its cursor is where the point would land.
        if matches!(self.interactive.tool(), WireframeTool::Point(_))
            && let Some(at) = self.interactive.hover().and_then(project)
        {
            let arm = Vec2::splat(SNAP_RADIUS);
            painter.line_segment([at - arm, at + arm], stroke);
        }

        if let Some(snap) = self.interactive.snap().and_then(|snap| project(snap.at).map(|at| (snap.kind, at))) {
            paint_snap_marker(painter, snap.0, snap.1);
        }
    }

    /// The screen point `point` projects to in `rect`, if the camera can see it.
    fn project(&self, point: Vec3, rect: Rect) -> Option<egui::Pos2> {
        let (x, y, _) = self.camera.to_screen(point, f64::from(rect.width()), f64::from(rect.height()), SCENE_RADIUS)?;
        Some(rect.min + Vec2::new(x as f32, y as f32))
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
    fn refresh(&mut self, ui: &Ui, rect: Rect, document: &CamDocument, operations: &[Operation], simulation: Option<&Simulation>) {
        let ppp = ui.ctx().pixels_per_point().clamp(0.5, 4.0);
        let width = ((rect.width() * ppp).round() as u32).clamp(1, MAX_RENDER_SIDE);
        let height = ((rect.height() * ppp).round() as u32).clamp(1, MAX_RENDER_SIDE);
        let key = RenderKey {
            camera: self.camera,
            width,
            height,
            revision: document.revision(),
            toolpaths: toolpaths_key(operations),
            simulation: simulation_key(simulation),
        };
        if self.picture.as_ref().is_some_and(|picture| picture.key == key) {
            return;
        }
        let canvas = render(&scene(document, operations, simulation), &self.camera, width as usize, height as usize);
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
    use solidcraft_geom::Vec3Geom;

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
    fn the_pointer_is_tracked_on_the_active_construction_plane() {
        use solidcraft_geom::Vec3Geom;

        let document = CamDocument::default();
        let mut viewport = Viewport::default();
        let rect = Rect::from_min_size(egui::pos2(10.0, 20.0), Vec2::new(600.0, 400.0));
        viewport.track_pointer(rect, Some(rect.center()), &document);
        let hover = viewport.interactive.hover().expect("the centre of the view projects onto the TOP plane");
        assert!(hover.dist(Vec3::ZERO) < 1e-9, "the home view centres the origin, got {hover:?}");
        viewport.track_pointer(rect, None, &document);
        assert_eq!(viewport.interactive.hover(), None, "a pointer outside the viewport has no position");
    }

    #[test]
    fn a_wireframe_tool_follows_the_pointer() {
        let document = CamDocument::default();
        let mut viewport = Viewport::default();
        assert!(viewport.begin_wireframe_tool(WireTool::Line, Plane::XY));
        let rect = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 400.0));
        viewport.track_pointer(rect, Some(rect.center()), &document);
        assert!(viewport.interactive.is_active());
        assert!(viewport.interactive.tool().current_pos().is_some(), "the line tool follows the cursor");
    }

    #[test]
    fn a_click_that_finishes_a_line_hands_it_back_for_creation() {
        let document = CamDocument::default();
        let mut viewport = Viewport::default();
        assert!(viewport.begin_wireframe_tool(WireTool::Line, Plane::XY));
        let rect = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 400.0));
        viewport.track_pointer(rect, Some(rect.center()), &document);
        assert_eq!(viewport.place_point(), None, "the first click only anchors the line");
        assert_eq!(viewport.interactive.placed_points(), 1);
        let end = rect.center() + Vec2::new(40.0, 0.0);
        viewport.track_pointer(rect, Some(end), &document);
        let wire = viewport.place_point().expect("the second click completes the line");
        let PendingWire::Line { start: from, end: to } = wire else { panic!("a line tool completes a line, got {wire:?}") };
        assert!(from.dist(to) > 1.0, "the line spans the two clicks, got {from:?} to {to:?}");
        assert_eq!(viewport.interactive.placed_points(), 1, "the tool stays live at the line's end");
        viewport.cancel_wireframe_tool();
        assert!(!viewport.interactive.is_active());
        assert_eq!(viewport.place_point(), None, "no tool places nothing");
    }

    #[test]
    fn a_live_tool_keeps_its_point_when_the_pointer_leaves_the_viewport() {
        let document = CamDocument::default();
        let mut viewport = Viewport::default();
        let rect = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 400.0));
        viewport.track_pointer(rect, Some(rect.center()), &document);
        viewport.track_pointer(rect, None, &document);
        assert_eq!(viewport.interactive.hover(), None, "with no tool the pointer leaves no point");
        assert!(viewport.begin_wireframe_tool(WireTool::Point, Plane::XY));
        viewport.track_pointer(rect, Some(rect.center()), &document);
        let hover = viewport.interactive.hover();
        viewport.track_pointer(rect, None, &document);
        assert_eq!(viewport.interactive.hover(), hover, "the tool keeps the point the panel's fields apply to");
    }

    #[test]
    fn a_typed_length_in_the_tool_panel_moves_the_rubber_band() {
        let document = CamDocument::default();
        let mut viewport = Viewport::default();
        assert!(viewport.begin_wireframe_tool(WireTool::Line, Plane::XY));
        let rect = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 400.0));
        viewport.track_pointer(rect, Some(rect.center()), &document);
        viewport.interactive.click(Vec3::ZERO);
        viewport.track_pointer(rect, Some(rect.center() + Vec2::new(50.0, 0.0)), &document);
        viewport.interactive.set_dimension(crate::interactive_geom::DimensionField::Length, Some(12.0));
        let cursor = viewport.interactive.cursor().expect("the tool follows the pointer");
        assert!((cursor.dist(Vec3::ZERO) - 12.0).abs() < 1e-9, "the typed length locks the band, got {cursor:?}");
    }

    #[test]
    fn snapping_takes_a_feature_within_eight_pixels() {
        let viewport = Viewport::default();
        let rect = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 400.0));
        let at = Vec3::new(10.0, 10.0, 0.0);
        let on_screen = viewport.project(at, rect).expect("the TOP plane is in view");
        let candidates = [SnapCandidate { kind: SnapKind::Endpoint, at }];
        assert_eq!(pick_snap(&candidates, rect, on_screen, &viewport.camera), Some(candidates[0]), "the pointer on the vertex snaps to it");
        let inside = on_screen + Vec2::new(SNAP_RADIUS_PIXELS - 1.0, 0.0);
        assert_eq!(pick_snap(&candidates, rect, inside, &viewport.camera), Some(candidates[0]), "a pixel inside the reach snaps");
        let outside = on_screen + Vec2::new(SNAP_RADIUS_PIXELS + 1.0, 0.0);
        assert_eq!(pick_snap(&candidates, rect, outside, &viewport.camera), None, "a pixel outside the reach does not");
    }

    #[test]
    fn an_endpoint_beats_a_midpoint_at_the_same_distance() {
        let viewport = Viewport::default();
        let rect = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 400.0));
        let at = Vec3::new(10.0, 10.0, 0.0);
        let candidates = [
            SnapCandidate { kind: SnapKind::Midpoint, at },
            SnapCandidate { kind: SnapKind::Centre, at },
            SnapCandidate { kind: SnapKind::Endpoint, at },
        ];
        let on_screen = viewport.project(at, rect).expect("the TOP plane is in view");
        let snap = pick_snap(&candidates, rect, on_screen, &viewport.camera).expect("the stacked candidates snap");
        assert_eq!(snap.kind, SnapKind::Endpoint, "the vertex wins the tie");
    }

    #[test]
    fn a_pointer_over_a_document_vertex_snaps_while_a_tool_is_live() {
        let mut document = CamDocument::default();
        document.draw(WireTool::Rectangle).expect("the rectangle draws on a new document");
        let mut viewport = Viewport::default();
        let rect = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 400.0));
        // With no tool live the pointer stays where it projects.
        let corner = Vec3::new(50.0, 40.0, 0.0);
        let pointer = viewport.project(corner, rect).expect("the rectangle is in view");
        viewport.track_pointer(rect, Some(pointer), &document);
        assert_eq!(viewport.interactive.snap(), None);

        assert!(viewport.begin_wireframe_tool(WireTool::Line, Plane::XY));
        viewport.track_pointer(rect, Some(pointer), &document);
        let snap = viewport.interactive.snap().expect("the pointer is on the rectangle's corner");
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert!(snap.at.dist(corner) < 1e-9, "the snap is the corner itself, got {:?}", snap.at);
        assert_eq!(viewport.interactive.hover(), Some(snap.at), "the pointer is the snap, not the projection beside it");
    }

    #[test]
    fn the_snap_reach_matches_the_blueprint() {
        assert_eq!(SNAP_RADIUS_PIXELS, 8.0);
        assert_eq!(PREVIEW_WIDTH, 1.5);
        assert_eq!(PREVIEW_COLOR, Color32::from_rgb(0, 255, 255));
        assert_eq!(SNAP_COLOR, Color32::GREEN);
        assert_eq!(SNAP_RADIUS, 4.0);
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
        let scene = scene(&document, &[], None);
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
        let drawn = scene(&document, &[], None);
        assert_eq!(drawn.lines.len(), 3 + 4, "the gnomon and the rectangle's four lines");
        assert!(drawn.lines[3..].iter().all(|line| line.color == WIREFRAME && line.on_top));
        assert_eq!(drawn.meshes.len(), 1);
        assert_eq!(drawn.meshes[0].color, SOLID_FILL);
        assert!(drawn.meshes[0].closed);

        document.set_solid_visible(document.solids()[0].id, false);
        assert!(scene(&document, &[], None).meshes.is_empty(), "a hidden body is not drawn");
        document.set_solid_visible(document.solids()[0].id, true);
        document.set_level_visible(1, false);
        let hidden = scene(&document, &[], None);
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
        let scene = scene(&document, &[], None);
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
        let scene = scene(&document, &[], None);
        assert_eq!(scene.meshes.len(), 2, "the part and the stock body");
        assert_eq!(scene.meshes[1].color, STOCK_FILL);
        assert!(scene.meshes[1].glass());
        assert_eq!(scene.lines.len(), 3 + 4, "a body stock has no box edges of its own");
    }

    #[test]
    fn no_stock_and_an_empty_box_add_nothing() {
        let mut document = CamDocument::default();
        assert!(scene(&document, &[], None).meshes.is_empty());
        document.set_stock(Some(StockDef::BoundingBox { extents: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0] }));
        let scene = scene(&document, &[], None);
        assert!(scene.meshes.is_empty(), "a box with no size is not drawn");
        assert_eq!(scene.lines.len(), 3);
        assert!(!valid_extents([0.0, 0.0, 0.0, 0.0, 1.0, 1.0]));
        assert!(valid_extents([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]));
    }

    /// A calculated 2D Contour over the four lines of a 40 × 30 rectangle.
    fn contour_operation() -> Operation {
        use solidcraft_cam::operations::contour2d::{ChainCurve, Contour2DParams};

        let chain = vec![
            ChainCurve { id: 1, points: vec![[0.0, 0.0, 0.0], [40.0, 0.0, 0.0]] },
            ChainCurve { id: 2, points: vec![[40.0, 0.0, 0.0], [40.0, 30.0, 0.0]] },
            ChainCurve { id: 3, points: vec![[40.0, 30.0, 0.0], [0.0, 30.0, 0.0]] },
            ChainCurve { id: 4, points: vec![[0.0, 30.0, 0.0], [0.0, 0.0, 0.0]] },
        ];
        let params = Contour2DParams::new(vec![1, 2, 3, 4], 1);
        let generated = solidcraft_cam::tool::ToolLibrary::with_presets()
            .assemblies()
            .first()
            .and_then(|assembly| Operation::generated(params, &chain, &assembly.tool).ok());
        generated.unwrap_or_default()
    }

    #[test]
    fn the_scene_draws_the_toolpath_in_the_rapid_and_feed_colours() {
        let document = CamDocument::default();
        let operation = contour_operation();
        assert!(operation.is_generated(), "the fixture generates");
        let drawn = scene(&document, std::slice::from_ref(&operation), None);
        let rapid = drawn.lines.iter().filter(|line| line.color == TOOLPATH_RAPID).count();
        let feed = drawn.lines.iter().filter(|line| line.color == TOOLPATH_FEED).count();
        assert!(rapid >= 2, "the rapid down and the rapid out, got {rapid}");
        assert!(feed >= 4, "the four sides are feeds, got {feed}");
        assert!(drawn.lines.iter().skip(3).all(|line| line.on_top), "the toolpath reads over the model");
        assert_eq!(drawn.lines.len(), 3 + operation.toolpath.len(), "one line per move, after the gnomon");
    }

    #[test]
    fn hiding_an_operation_removes_its_toolpath_from_the_scene() {
        let document = CamDocument::default();
        let mut operation = contour_operation();
        operation.set_visible(false);
        let drawn = scene(&document, &[operation], None);
        assert_eq!(drawn.lines.len(), 3, "only the gnomon is left");
        assert!(drawn.lines.iter().all(|line| line.color != TOOLPATH_RAPID && line.color != TOOLPATH_FEED));
    }

    #[test]
    fn the_toolpath_key_follows_visibility_and_regeneration() {
        let mut operation = contour_operation();
        let empty = toolpaths_key(&[]);
        let generated = toolpaths_key(std::slice::from_ref(&operation));
        assert_ne!(empty, generated, "a shown operation redraws the viewport");
        operation.set_visible(false);
        let hidden = toolpaths_key(std::slice::from_ref(&operation));
        assert_ne!(generated, hidden, "hiding it redraws the viewport");
        operation.set_visible(true);
        assert_ne!(toolpaths_key(std::slice::from_ref(&operation)), hidden, "showing it again redraws too");
        let same_state = contour_operation();
        assert_eq!(toolpaths_key(&[same_state]), generated, "the key is deterministic for the same state");
    }

    /// A simulation of a 40 mm feed at Z 0 through stock whose top is 5 mm, scrubbed halfway.
    fn scrubbed_simulation() -> Simulation {
        use solidcraft_cam::simulation::Simulation;
        use solidcraft_cam::tool::{CuttingTool, ToolType};
        use solidcraft_cam::toolpath::ToolpathBuilder;

        let tool = CuttingTool::new(ToolType::FlatEndmill, 6.0);
        let mut builder = ToolpathBuilder::new();
        builder.place([10.0, 15.0, 10.0]);
        builder.rapid_to([10.0, 15.0, 0.0]);
        builder.feed_to([50.0, 15.0, 0.0]);
        builder.rapid_to([50.0, 15.0, 10.0]);
        let mut simulation = Simulation::new(&builder.finish(), Some(&tool), Some([0.0, 0.0, -5.0, 60.0, 30.0, 5.0]));
        simulation.set_position(simulation.total_length() * 0.75);
        simulation
    }

    #[test]
    fn the_scene_draws_the_machined_stock_and_the_tool_while_simulating() {
        let document = stocked_document();
        let simulation = scrubbed_simulation();
        assert!(simulation.machined() > 0.0, "the fixture has cut");
        let drawn = scene(&document, &[], Some(&simulation));
        assert_eq!(drawn.meshes.len(), 3, "the plate, the machined stock and the cutter");
        let stock = &drawn.meshes[1];
        assert_eq!(stock.color, STOCK_FILL);
        assert!(stock.glass(), "the machined stock is still drawn see-through");
        assert!(stock.mesh.positions.len() > 24, "the stock is a height map, not the untouched box");
        let cutter = &drawn.meshes[2];
        assert_eq!(cutter.color, TOOL_FILL, "the tool joins the scene in its own steel blue");
        assert!(drawn.lines.iter().all(|line| line.color != STOCK_EDGE), "a machined stock has no box edges");
    }

    #[test]
    fn the_stock_grid_mesh_surfaces_the_cut_and_walls_down_to_the_bottom() {
        use solidcraft_cam::simulation::StockGrid;
        use solidcraft_cam::tool::{CuttingTool, ToolType};

        let extents = [0.0, 0.0, -5.0, 60.0, 30.0, 5.0];
        let mut grid = StockGrid::from_extents(extents).expect("the box is usable");
        grid.cut_at([30.0, 15.0, 0.0], &CuttingTool::new(ToolType::FlatEndmill, 6.0));
        let mesh = stock_grid_mesh(&grid);
        assert_eq!(mesh.positions.len(), mesh.normals.len());
        assert!(!mesh.triangles.is_empty());
        assert_eq!(mesh.triangles.len(), mesh.tri_face.len());
        let lowest = mesh.positions.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
        assert!((lowest + 5.0).abs() < 1e-9, "the walls reach the stock's bottom, got {lowest}");
        let cut = mesh.positions.iter().filter(|p| p.z < 5.0).count();
        assert!(cut > 24, "the top carries the cut, not just the walls, got {cut} lowered vertices");
        assert!(mesh.normals.iter().all(|normal| (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z - 1.0).abs() < 1e-9));
    }

    #[test]
    fn the_tool_cylinder_stands_on_the_tip_with_the_flute_length() {
        use solidcraft_cam::tool::{CuttingTool, ToolType};

        let tool = CuttingTool::new(ToolType::FlatEndmill, 10.0);
        let mesh = tool_cylinder_mesh([1.0, 2.0, -3.0], &tool).expect("a usable tool is drawn");
        let (bottom, top) =
            mesh.positions.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| (low.min(point.z), high.max(point.z)));
        assert!((bottom + 3.0).abs() < 1e-9, "the cutter's tip is the move's end, got {bottom}");
        assert!((top - (bottom + tool.flute_length)).abs() < 1e-9, "the cutter is a flute long, got {top}");
        let widest = mesh.positions.iter().map(|point| ((point.x - 1.0).powi(2) + (point.y - 2.0).powi(2)).sqrt()).fold(0.0, f64::max);
        assert!((widest - 5.0).abs() < 1e-9, "the cutter is its diameter wide, got {widest}");
        assert!(tool_cylinder_mesh([0.0; 3], &CuttingTool::new(ToolType::FlatEndmill, 0.0)).is_none(), "a tool with no diameter draws nothing");
    }

    #[test]
    fn the_render_key_follows_the_scrubbed_position() {
        let simulation = scrubbed_simulation();
        assert_eq!(simulation_key(Some(&simulation)), simulation_key(Some(&simulation)), "the key is deterministic");
        assert_ne!(simulation_key(None), simulation_key(Some(&simulation)));
        let mut again = scrubbed_simulation();
        again.set_position(0.0);
        assert_ne!(simulation_key(Some(&simulation)), simulation_key(Some(&again)), "a scrub redraws the picture");
    }
}

//! The workspace screenshot: renders the real Z-CAM shell headlessly and writes the shot the
//! blueprint's milestones record, `target/shots/cam-workspace.png`.
//!
//! There is no window and no GPU here: the app draws into an egui context, and
//! [`raster`] tessellates and software-rasterises the meshes. That makes the shot deterministic
//! and runnable on CI, and it is the same code path the window uses (`CamWorkspaceApp::draw`).

#[path = "support/raster.rs"]
mod raster;

use cam_workspace::app::{CamWorkspaceApp, LEFT_PANEL_MIN_WIDTH, LEFT_PANEL_WIDTH, RIBBON_HEIGHT, STATUS_BAR_HEIGHT};
use cam_workspace::dialogs_cam::ContourPage;
use cam_workspace::document::{DEFAULT_EXTRUDE_DEPTH, WireTool};
use cam_workspace::interactive_geom::{ClickOutcome, SnapKind};
use cam_workspace::left_manager::ManagerTab;
use cam_workspace::ribbon::{Command, RibbonTab, SimulationCommand};
use cam_workspace::simulation_panel::SIMULATION_PANEL_BACKGROUND;
use cam_workspace::viewport::{SCENE_RADIUS, TOOL_FILL, TOOLPATH_FEED, TOOLPATH_RAPID, WIREFRAME};
use solidcore_record::{Plane, Vec3};
use solidcraft_cam::operations::contour2d::{Contour2DParams, LeadKind};
use solidcraft_cam::stock::{StockDef, StockExcess};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The shot's logical size, in points: the window's default size made taller for evidence.
const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// Frames to run before the shot: the first frame sizes panels, the next settles their contents.
const WARMUP_FRAMES: usize = 3;

/// Frames the workspace with the same part a CAM programmer would show: a plate extruded on
/// level 1, a boss on level 2, and the stock around both, so the manager and the viewport both
/// have something to say.
fn seed_part(app: &mut CamWorkspaceApp) {
    // Level 1 (`X`): a rectangular plate.
    app.document.draw(WireTool::Rectangle).expect("the rectangle tool draws on a new document");
    app.document.extrude(DEFAULT_EXTRUDE_DEPTH).expect("the rectangle extrudes into a plate");
    // Level 2: a round boss built on a level of its own.
    app.document.add_level();
    app.document.draw(WireTool::Circle).expect("the circle tool draws on the new level");
    app.document.extrude(DEFAULT_EXTRUDE_DEPTH).expect("the circle extrudes into a boss");
    // The raw material: a box 5 mm larger than the part on every side.
    if let Some((min, max)) = app.document.part_bounds() {
        app.document.set_stock(Some(StockDef::from_bounds(min, max, StockExcess::new(5.0, 5.0, 2.0))));
    }
    // The toolpath the program cuts: a 2D Contour around the plate, two depth cuts deep, with a
    // roughing pass and a lead arc, so the viewport has rapids and feeds to show.
    seed_toolpath(app);
    // Frame the part rather than the whole grid the home view shows.
    app.viewport.camera.target = Vec3::new(70.0, 25.0, 6.0);
    app.viewport.camera.distance = 240.0;
}

/// Defines and calculates the plate's 2D Contour operation.
fn seed_toolpath(app: &mut CamWorkspaceApp) {
    let chain: Vec<usize> = app.document.chain_entities().iter().filter(|entity| entity.level == 1).map(|entity| entity.id).collect();
    let tool = app.tools.assemblies().first().map_or(0, |assembly| assembly.id);
    let mut params = Contour2DParams::new(chain, tool);
    // Cut inside the stock the setup defines: the default depth is below the block, so the
    // verification would have nothing to take off. The clearance keeps the whole program in frame.
    if let Some([_, _, _, _, _, top]) = app.document.stock_extents() {
        params.linking.top_of_stock = top;
        params.linking.feed_plane = top + 2.0;
        params.linking.retract = top + 4.0;
        params.linking.clearance = top + 6.0;
        params.linking.depth = top - 8.0;
    }
    params.depth_cuts.max_rough_step = 4.0;
    params.multipasses.rough_passes = 1;
    params.multipasses.rough_spacing = 2.0;
    params.lead_in_out.entry.kind = LeadKind::Arc;
    params.lead_in_out.exit.kind = LeadKind::Line;
    app.accept_contour2d(params);
}

/// Renders one tab of the workspace and saves it under `target/shots`.
fn shot(name: &str, tab: ManagerTab) -> image::RgbaImage {
    shot_with(name, tab, |_| {})
}

/// Renders the workspace with `prepare` applied to the app first, and saves it under
/// `target/shots`: the way the dialog shots open their dialog before the frames run.
fn shot_with(name: &str, tab: ManagerTab, prepare: impl FnOnce(&mut CamWorkspaceApp)) -> image::RgbaImage {
    let ctx = egui::Context::default();
    let mut app = CamWorkspaceApp::with_context(&ctx);
    app.manager.tab = tab;
    seed_part(&mut app);
    prepare(&mut app);
    // A seeded live tool follows the pointer: hand the frames the pointer it snaps under.
    let pointer = preview_pointer(&app);
    let mut textures = HashMap::new();
    let mut output = None;
    for _ in 0..WARMUP_FRAMES {
        output = Some(raster::paint_at(&ctx, &mut app, &mut textures, SIZE, pointer));
    }
    let Some(output) = output else { panic!("no frame was run") };
    let image = raster::raster(&ctx, &textures, output);
    image.save(shot_path(name)).unwrap();
    image
}

/// The workspace shot: the seeded part with a live line tool, its rubber band and the AutoCursor
/// marker the pointer draws.
fn workspace_shot() -> image::RgbaImage {
    shot_with("cam-workspace", ManagerTab::Levels, seed_preview)
}

/// Leaves a line tool live and anchored at a plate corner, with the pointer over the opposite
/// corner: the shot then records the rubber band and the green endpoint marker.
fn seed_preview(app: &mut CamWorkspaceApp) {
    let wires = app.document.wireframe_lines();
    let corner = wires.first().and_then(|wire| wire.points.first().copied());
    let target = wires.get(2).and_then(|wire| wire.points.first().copied());
    let (Some(corner), Some(target)) = (corner, target) else { return };
    assert!(app.viewport.begin_wireframe_tool(WireTool::Line, Plane::XY), "a line is placed by clicking");
    assert_eq!(app.viewport.interactive.click(corner), ClickOutcome::Progress, "the first click anchors the line");
    app.viewport.interactive.track_point(target);
}

/// Where the live tool's pointer is on screen: the projection of the point it is following into
/// the viewport the panels leave over.
fn preview_pointer(app: &CamWorkspaceApp) -> Option<egui::Pos2> {
    let target = app.viewport.interactive.tool().current_pos()?;
    let rect = viewport_rect();
    let (x, y, _) = app.viewport.camera.to_screen(target, f64::from(rect.width()), f64::from(rect.height()), SCENE_RADIUS)?;
    Some(rect.min + egui::vec2(x as f32, y as f32))
}

/// The screen rectangle the viewport fills: right of the manager, under the ribbon, above the
/// status bar.
fn viewport_rect() -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(LEFT_PANEL_WIDTH, RIBBON_HEIGHT), egui::pos2(SIZE.x, SIZE.y - STATUS_BAR_HEIGHT))
}

/// `target/shots/<name>.png`, created if the folder is missing.
fn shot_path(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.png"))
}

#[test]
fn the_workspace_shot_shows_the_unified_manager() {
    let image = workspace_shot();
    assert_eq!((image.width(), image.height()), (SIZE.x as u32, SIZE.y as u32));
    // A drawn shell has many colours (backgrounds, text, the viewport fill); a blank frame has
    // one. This catches a frame that silently renders nothing.
    let colours = image.pixels().map(|p| p.0).collect::<std::collections::BTreeSet<_>>();
    assert!(colours.len() > 16, "the shot must show drawn UI, got {} colours", colours.len());
}

/// Pixels of the central region: the columns right of the manager panel.
fn viewport_pixels(image: &image::RgbaImage) -> impl Iterator<Item = &image::Rgba<u8>> {
    let width = image.width().max(1);
    let left = (LEFT_PANEL_WIDTH as u32 + 40).min(width);
    image.pixels().enumerate().filter(move |(i, _)| (*i as u32 % width) >= left).map(|(_, pixel)| pixel)
}

#[test]
fn the_shot_shows_the_live_3d_viewport() {
    let image = workspace_shot();
    // The gnomon's X arm is red and the ground grid is grey; a placeholder frame had neither.
    let (mut red, mut grey) = (0usize, 0usize);
    for pixel in viewport_pixels(&image) {
        let [r, g, b, _] = pixel.0;
        if r > 150 && g < 110 && b < 110 {
            red += 1;
        }
        if (150..=215).contains(&r) && (150..=215).contains(&g) && (150..=215).contains(&b) {
            grey += 1;
        }
    }
    assert!(red > 50, "the gnomon's X arm is drawn, got {red} red pixels");
    assert!(grey > 1000, "the ground grid is drawn, got {grey} grey pixels");
}

#[test]
fn the_shot_shows_the_live_wireframe_preview_and_its_snap() {
    let image = workspace_shot();
    // Pure green marks the AutoCursor endpoint square; the Y axis is a muted green and the
    // wireframe, the stock and the bodies carry no green at all.
    let mut green = 0usize;
    for pixel in viewport_pixels(&image) {
        let [r, g, b, _] = pixel.0;
        if g > 200 && r < 80 && b < 80 {
            green += 1;
        }
    }
    assert!(green > 8, "the endpoint snap marker is drawn in green, got {green} green pixels");
}

#[test]
fn the_pointer_snaps_to_the_corner_the_preview_aims_at() {
    let ctx = egui::Context::default();
    let mut app = CamWorkspaceApp::with_context(&ctx);
    seed_part(&mut app);
    seed_preview(&mut app);
    let pointer = preview_pointer(&app).expect("the live tool follows a pointer");
    let mut textures = HashMap::new();
    for _ in 0..WARMUP_FRAMES {
        raster::paint_at(&ctx, &mut app, &mut textures, SIZE, Some(pointer));
    }
    let snap = app.viewport.interactive.snap().expect("the pointer snaps to the plate's corner");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert_eq!(app.viewport.interactive.preview(48).len(), 1, "the rubber band is drawn");
}

#[test]
fn the_shot_shows_the_part_the_ribbon_tools_built() {
    let image = workspace_shot();
    // The wireframe of level 2's circle is drawn in the WIREFRAME blue.
    let (mut wire, mut shaded) = (0usize, 0usize);
    for pixel in viewport_pixels(&image) {
        let [r, g, b, _] = pixel.0;
        if (r as i32 - i32::from(WIREFRAME.0)).abs() < 40 && (b as i32 - i32::from(WIREFRAME.2)).abs() < 60 && b > r {
            wire += 1;
        }
        // Shaded solid faces vary with the light, but stay a cool mid grey.
        if (90..=200).contains(&r) && (100..=210).contains(&g) && (120..=225).contains(&b) && b > g && g > r {
            shaded += 1;
        }
    }
    assert!(wire > 100, "the circle's wireframe is drawn, got {wire} wireframe pixels");
    assert!(shaded > 1000, "the extruded bodies are drawn, got {shaded} shaded pixels");
}

#[test]
fn every_manager_tab_renders_its_view() {
    for (name, tab) in [
        ("cam-workspace-toolpaths", ManagerTab::Toolpaths),
        ("cam-workspace-planes", ManagerTab::Planes),
        ("cam-workspace-solids", ManagerTab::Solids),
    ] {
        let image = shot(name, tab);
        let colours = image.pixels().map(|p| p.0).collect::<std::collections::BTreeSet<_>>();
        assert!(colours.len() > 16, "{name} must show drawn UI, got {} colours", colours.len());
    }
}

#[test]
fn the_tool_library_shot_shows_the_modal_dialog() {
    let mut opened = false;
    let image = shot_with("cam-workspace-tool-library", ManagerTab::Levels, |app| {
        app.ribbon.tab = RibbonTab::Machine;
        app.run(Command::ToolLibrary);
        opened = app.tool_library_open();
    });
    assert!(opened, "the dialog is open while the frames run");
    assert_eq!((image.width(), image.height()), (SIZE.x as u32, SIZE.y as u32));
    // The modal dims the viewport and paints the library, the preset picker and the editor over
    // it; a frame that failed to open it would leave only the plain workspace.
    let colours = image.pixels().map(|p| p.0).collect::<std::collections::BTreeSet<_>>();
    assert!(colours.len() > 32, "the dialog must show drawn UI, got {} colours", colours.len());
}

#[test]
fn the_shot_shows_the_translucent_stock() {
    let image = workspace_shot();
    // The stock is translucent, so it blends its red fill into the scene rather than hiding the
    // part: look for pixels tinted red but still carrying the background and the bodies.
    let mut tinted = 0usize;
    for pixel in viewport_pixels(&image) {
        let [r, g, b, _] = pixel.0;
        let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
        if r > 140 && r > g + 12 && b > g && g > 100 {
            tinted += 1;
        }
    }
    assert!(tinted > 500, "the stock box is drawn in the blueprint's translucent red, got {tinted} tinted pixels");
}

#[test]
fn the_shot_shows_the_calculated_toolpath_in_rapid_and_feed_colours() {
    let image = shot("cam-workspace", ManagerTab::Toolpaths);
    // The rapids are the blueprint's yellow and the feeds its cyan; the wireframe, the stock and
    // the bodies are blues, reds and greys, so the two toolpath colours stand on their own.
    let (mut rapid, mut feed) = (0usize, 0usize);
    for pixel in viewport_pixels(&image) {
        let [r, g, b, _] = pixel.0;
        if r > 180 && g > 180 && b < 160 {
            rapid += 1;
        }
        if r < 160 && g > 170 && b > 170 {
            feed += 1;
        }
    }
    assert!(rapid > 20, "the rapid moves are drawn in {TOOLPATH_RAPID:?}, got {rapid} pixels");
    assert!(feed > 100, "the feed moves are drawn in {TOOLPATH_FEED:?}, got {feed} pixels");
}

#[test]
fn the_stock_setup_shot_shows_the_modal_dialog() {
    let mut opened = false;
    let image = shot_with("cam-workspace-stock-setup", ManagerTab::Levels, |app| {
        app.ribbon.tab = RibbonTab::Simulation;
        app.run(Command::StockSetup);
        opened = app.stock_setup_open();
    });
    assert!(opened, "the dialog is open while the frames run");
    assert_eq!((image.width(), image.height()), (SIZE.x as u32, SIZE.y as u32));
    let colours = image.pixels().map(|p| p.0).collect::<std::collections::BTreeSet<_>>();
    assert!(colours.len() > 32, "the dialog must show drawn UI, got {} colours", colours.len());
}

#[test]
fn the_contour_dialog_shot_shows_the_parameter_pages() {
    let mut opened = false;
    let image = shot_with("cam-workspace-contour2d", ManagerTab::Toolpaths, |app| {
        app.ribbon.tab = RibbonTab::Machine;
        app.run(Command::Contour2D);
        let chain: Vec<usize> = app.document.chain_entities().iter().map(|entity| entity.id).collect();
        if let Some(dialog) = app.contour2d_dialog_mut() {
            dialog.params_mut().chain = chain;
            dialog.params_mut().depth_cuts.max_rough_step = 2.0;
            dialog.params_mut().multipasses.rough_passes = 2;
            dialog.params_mut().lead_in_out.entry.kind = solidcraft_cam::operations::contour2d::LeadKind::Arc;
            dialog.set_page(ContourPage::Linking);
        }
        opened = app.contour2d_open();
    });
    assert!(opened, "the dialog is open while the frames run");
    assert_eq!((image.width(), image.height()), (SIZE.x as u32, SIZE.y as u32));
    // The page list, the linking fields and the footer sit over the dimmed workspace; a frame
    // that failed to open the dialog would leave only the plain workspace.
    let colours = image.pixels().map(|p| p.0).collect::<std::collections::BTreeSet<_>>();
    assert!(colours.len() > 32, "the dialog must show drawn UI, got {} colours", colours.len());
}

#[test]
fn the_toolpaths_shot_lists_the_generated_operation() {
    let mut app = CamWorkspaceApp::with_context(&egui::Context::default());
    seed_part(&mut app);
    assert_eq!(app.operations().len(), 1, "the shot's part has its contour operation");
    let operation = &app.operations()[0];
    assert!(operation.is_generated(), "the operation's toolpath is calculated");
    assert!(operation.toolpath.rapid_moves() > 0 && operation.toolpath.feed_moves() > 0, "the path has rapids and feeds");
    assert!(operation.toolpath.bounds().is_some());
}

#[test]
fn the_solids_shot_lists_the_part_the_tools_built() {
    let ctx = egui::Context::default();
    let app = CamWorkspaceApp::with_context(&ctx);
    assert!(app.document.solids().is_empty(), "a new workspace is empty");
    let mut app = app;
    seed_part(&mut app);
    assert_eq!(app.document.solids().len(), 2);
    assert_eq!(app.document.levels().len(), 2);
    assert_eq!(app.document.solids()[0].level, 1);
    assert_eq!(app.document.solids()[1].level, 2, "the boss belongs to the level it was drawn on");
}

#[test]
fn the_simulation_shot_shows_the_playback_panel_and_the_cut_stock() {
    let mut cut = false;
    let mut advanced = 0.0;
    let image = shot_with("cam-workspace-simulation", ManagerTab::Toolpaths, |app| {
        app.ribbon.tab = RibbonTab::Simulation;
        app.run(Command::Simulation(SimulationCommand::Play));
        let total = app.simulation().total_length();
        app.scrub_simulation(total * 0.62);
        advanced = app.simulation().position();
        cut = app.simulation().grid().is_some_and(|grid| !grid.is_untouched());
    });
    assert!(advanced > 0.0, "the scrub put the tool partway along the program");
    assert!(cut, "the feed moves have taken material off the stock");

    // The playback panel sits above the status bar, in its own background.
    let width = image.width();
    let panel_top = image.height().saturating_sub(24 + 64);
    let panel_bottom = image.height().saturating_sub(24);
    let mut panel = 0usize;
    let mut tool = 0usize;
    for (index, pixel) in image.pixels().enumerate() {
        let y = (index as u32) / width.max(1);
        let [r, g, b, _] = pixel.0;
        if y >= panel_top
            && y < panel_bottom
            && pixel.0 == [SIMULATION_PANEL_BACKGROUND.r(), SIMULATION_PANEL_BACKGROUND.g(), SIMULATION_PANEL_BACKGROUND.b(), 255]
        {
            panel += 1;
        }
        if (r as i32 - i32::from(TOOL_FILL.0)).abs() < 26
            && (g as i32 - i32::from(TOOL_FILL.1)).abs() < 26
            && (b as i32 - i32::from(TOOL_FILL.2)).abs() < 26
        {
            tool += 1;
        }
    }
    assert!(panel > 1000, "the playback panel is drawn, got {panel} panel pixels");
    assert!(tool > 60, "the tool cylinder is drawn at the cut, got {tool} tool pixels");
}

#[test]
fn the_layout_tokens_match_the_blueprint() {
    assert_eq!(RIBBON_HEIGHT, 120.0, "the ribbon's height");
    assert_eq!(LEFT_PANEL_WIDTH, 320.0, "the manager's default width");
    assert_eq!(LEFT_PANEL_MIN_WIDTH, 250.0, "the manager's narrowest width");
}

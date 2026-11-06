//! The workspace screenshot: renders the real Z-CAM shell headlessly and writes the shot the
//! blueprint's milestones record, `target/shots/cam-workspace.png`.
//!
//! There is no window and no GPU here: the app draws into an egui context, and
//! [`raster`] tessellates and software-rasterises the meshes. That makes the shot deterministic
//! and runnable on CI, and it is the same code path the window uses (`CamWorkspaceApp::draw`).

#[path = "support/raster.rs"]
mod raster;

use cam_workspace::app::{CamWorkspaceApp, LEFT_PANEL_MIN_WIDTH, LEFT_PANEL_WIDTH, RIBBON_HEIGHT};
use cam_workspace::dialogs_cam::ContourPage;
use cam_workspace::document::{DEFAULT_EXTRUDE_DEPTH, WireTool};
use cam_workspace::left_manager::ManagerTab;
use cam_workspace::ribbon::{Command, RibbonTab};
use cam_workspace::viewport::WIREFRAME;
use solidcore_record::Vec3;
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
    // Frame the part rather than the whole grid the home view shows.
    app.viewport.camera.target = Vec3::new(70.0, 25.0, 6.0);
    app.viewport.camera.distance = 240.0;
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
    let mut textures = HashMap::new();
    let mut output = None;
    for _ in 0..WARMUP_FRAMES {
        output = Some(raster::paint(&ctx, &mut app, &mut textures, SIZE));
    }
    let Some(output) = output else { panic!("no frame was run") };
    let image = raster::raster(&ctx, &textures, output);
    image.save(shot_path(name)).unwrap();
    image
}

/// `target/shots/<name>.png`, created if the folder is missing.
fn shot_path(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.png"))
}

#[test]
fn the_workspace_shot_shows_the_unified_manager() {
    let image = shot("cam-workspace", ManagerTab::Levels);
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
    let image = shot("cam-workspace", ManagerTab::Levels);
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
fn the_shot_shows_the_part_the_ribbon_tools_built() {
    let image = shot("cam-workspace", ManagerTab::Levels);
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
    let image = shot("cam-workspace", ManagerTab::Levels);
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
fn the_layout_tokens_match_the_blueprint() {
    assert_eq!(RIBBON_HEIGHT, 120.0, "the ribbon's height");
    assert_eq!(LEFT_PANEL_WIDTH, 320.0, "the manager's default width");
    assert_eq!(LEFT_PANEL_MIN_WIDTH, 250.0, "the manager's narrowest width");
}

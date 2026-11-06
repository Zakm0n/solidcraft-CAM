//! The workspace screenshot: renders the real Z-CAM shell headlessly and writes the shot the
//! blueprint's milestones record, `target/shots/cam-workspace.png`.
//!
//! There is no window and no GPU here: the app draws into an egui context, and
//! [`raster`] tessellates and software-rasterises the meshes. That makes the shot deterministic
//! and runnable on CI, and it is the same code path the window uses (`CamWorkspaceApp::draw`).

#[path = "support/raster.rs"]
mod raster;

use cam_workspace::app::{CamWorkspaceApp, LEFT_PANEL_MIN_WIDTH, LEFT_PANEL_WIDTH, RIBBON_HEIGHT};
use cam_workspace::left_manager::ManagerTab;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The shot's logical size, in points: the window's default size made taller for evidence.
const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// Frames to run before the shot: the first frame sizes panels, the next settles their contents.
const WARMUP_FRAMES: usize = 3;

/// Renders one tab of the workspace and saves it under `target/shots`.
fn shot(name: &str, tab: ManagerTab) -> image::RgbaImage {
    let ctx = egui::Context::default();
    let mut app = CamWorkspaceApp::with_context(&ctx);
    app.manager.tab = tab;
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
fn the_layout_tokens_match_the_blueprint() {
    assert_eq!(RIBBON_HEIGHT, 120.0, "the ribbon's height");
    assert_eq!(LEFT_PANEL_WIDTH, 320.0, "the manager's default width");
    assert_eq!(LEFT_PANEL_MIN_WIDTH, 250.0, "the manager's narrowest width");
}

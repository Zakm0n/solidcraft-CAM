//! CAM workspace desktop app.
//!
//! Opens the native window with hardware-accelerated rendering and runs [`CamWorkspaceApp`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![deny(unsafe_code)]

mod app;

use app::CamWorkspaceApp;

/// Default window size: large enough for the ribbon, both manager panels and the viewport.
const WINDOW_SIZE: [f32; 2] = [1280.0, 720.0];

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(WINDOW_SIZE).with_min_inner_size([800.0, 600.0]).with_title("CAM Workspace"),
        ..Default::default()
    };

    eframe::run_native("CAM Workspace", options, Box::new(|cc| Ok(Box::new(CamWorkspaceApp::new(cc)))))
}

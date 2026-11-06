//! Z-CAM desktop workspace.
//!
//! Opens the native window with hardware-accelerated rendering and runs [`CamWorkspaceApp`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![deny(unsafe_code)]

use cam_workspace::app::CamWorkspaceApp;

/// Default window size: large enough for the ribbon, the manager panel and the viewport.
const WINDOW_SIZE: [f32; 2] = [1280.0, 720.0];

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(WINDOW_SIZE).with_min_inner_size([800.0, 600.0]).with_title("Z-CAM"),
        ..Default::default()
    };

    eframe::run_native("Z-CAM", options, Box::new(|cc| Ok(Box::new(CamWorkspaceApp::new(cc)))))
}

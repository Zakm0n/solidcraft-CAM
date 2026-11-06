//! Z-CAM workspace.
//!
//! Opens the native window with hardware-accelerated rendering and runs [`CamWorkspaceApp`];
//! in the browser it starts the same app on the page's canvas.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![deny(unsafe_code)]

use cam_workspace::app::CamWorkspaceApp;

/// Default window size: large enough for the ribbon, the manager panel and the viewport.
#[cfg(not(target_arch = "wasm32"))]
const WINDOW_SIZE: [f32; 2] = [1280.0, 720.0];

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(WINDOW_SIZE).with_min_inner_size([800.0, 600.0]).with_title("Z-CAM"),
        ..Default::default()
    };

    eframe::run_native("Z-CAM", options, Box::new(|cc| Ok(Box::new(CamWorkspaceApp::new(cc)))))
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast as _;

    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("the_canvas_id"))
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok());
        let Some(canvas) = canvas else {
            web_sys::console::error_1(&"Z-CAM: the page has no the_canvas_id canvas".into());
            return;
        };
        // The runner owns the app loop and its event handlers, so it has to outlive this call:
        // one leak for the life of the page, which is how long the app runs for.
        let runner: &'static eframe::WebRunner = Box::leak(Box::new(eframe::WebRunner::new()));
        let started = runner.start(canvas, eframe::WebOptions::default(), Box::new(|cc| Ok(Box::new(CamWorkspaceApp::new(cc)))));
        if let Err(error) = started.await {
            // Report rather than crash: the page stays usable and the console says why.
            web_sys::console::error_1(&format!("Z-CAM: failed to start: {error:?}").into());
        }
    });
}

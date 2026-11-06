//! The Z-CAM workspace: the application shell, its ribbon, its document and its unified left
//! manager.
//!
//! The binary in `main.rs` opens the window; everything it draws lives here, so the shell can be
//! rendered headless by the screenshot test without a window or a GPU.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![deny(unsafe_code)]

pub mod app;
pub mod dialogs_cam;
pub mod document;
pub mod left_manager;
pub mod ribbon;
pub mod viewport;

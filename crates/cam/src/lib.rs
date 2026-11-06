//! Z-CAM's machining data model.
//!
//! The CAM side of Z-CAM is data before it is toolpaths: a library of cutting tools in their
//! holders, the stock a program cuts, and the operations that machine it. This crate holds that
//! data as plain Rust types, with no UI and no geometry, so the workspace's dialogs, the future
//! toolpath calculation and any importer all read the same definitions.
//!
//! Phase 2A of the Z-CAM blueprint adds the tool library ([`tool`]); phase 2B adds the stock the
//! program cuts ([`stock`]); phase 2C adds the 2D Contour operation and its parameter pages
//! ([`operations::contour2d`]); phase 2D adds the toolpath the operation calculates
//! ([`toolpath`]); phase 2E adds running that path over time and the material it removes
//! ([`simulation`]). Operations extend this crate rather than live in the application shell, so
//! the dialog, the toolpath calculation and the simulation read the same definitions.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![deny(unsafe_code)]

pub mod operations;
pub mod simulation;
pub mod stock;
pub mod tool;
pub mod toolpath;

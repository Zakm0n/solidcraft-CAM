//! Z-CAM's machining data model.
//!
//! The CAM side of Z-CAM is data before it is toolpaths: a library of cutting tools in their
//! holders, the stock a program cuts, and the operations that machine it. This crate holds that
//! data as plain Rust types, with no UI and no geometry, so the workspace's dialogs, the future
//! toolpath calculation and any importer all read the same definitions.
//!
//! Phase 2A of the Z-CAM blueprint adds the tool library ([`tool`]); phase 2B adds the stock the
//! program cuts ([`stock`]). The 2D contour operation follows in its own phase; it extends this
//! crate rather than lives in the application shell.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![deny(unsafe_code)]

pub mod stock;
pub mod tool;

//! Z-CAM's machining data model.
//!
//! The CAM side of Z-CAM is data before it is toolpaths: a library of cutting tools in their
//! holders, the stock a program cuts, and the operations that machine it. This crate holds that
//! data as plain Rust types, with no UI and no geometry, so the workspace's dialogs, the future
//! toolpath calculation and any importer all read the same definitions.
//!
//! Phase 2A of the Z-CAM blueprint adds the tool library ([`tool`]). The stock definition and the
//! 2D contour operation follow in their own phases; they extend this crate rather than live in
//! the application shell.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![deny(unsafe_code)]

pub mod tool;

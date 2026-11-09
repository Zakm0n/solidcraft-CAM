//! Named views (`named_views`) save the camera's `roll`, the turn about the viewing direction,
//! so a view taken from a coordinate system keeps its +X across the screen and its +Y upright.
//! Older designs need no transformation: a saved view without `roll` reads as 0, the world
//! upright, which is what every view saved before had. The last release opens such a design
//! with a warning about the newer format number, reads what it can and keeps the rest
//! (`crates/doc/src/newer_file.rs`).

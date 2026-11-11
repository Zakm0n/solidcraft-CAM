//! CAM 2D contour operations (`cam.contours`): the profile operations that follow picked edges,
//! each with an id, a name, the `setup` and `tool` it runs in and on, its `target_edges` as
//! persistent edge names, and the `clearance_height`, `bottom_depth` and `stepdown` it cuts at,
//! in mm.
//! Older designs need no transformation: a design without the key has no contour operations,
//! keeps saving without it, and machines exactly as it did before 2D contours existed. The last
//! release opens such a design with a warning about the newer format number, reads what it can
//! and keeps the rest, the contour operations included (`crates/doc/src/newer_file.rs`).

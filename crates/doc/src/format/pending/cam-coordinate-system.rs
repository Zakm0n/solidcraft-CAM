//! CAM coordinate systems (`coordinate_systems`): the machining origin a toolpath works from,
//! with the machine's own X, Y and Z directions (`origin`, `x_axis`, `y_axis`, `z_axis`, unit
//! vectors in model space), a name and an id (`cam.coordinate_system.create`).
//! Older designs need no transformation: a design without the key has no coordinate systems, and
//! keeps saving without it. The last release opens such a design with a warning about the newer
//! format number, reads what it can and keeps the rest (`crates/doc/src/newer_file.rs`).

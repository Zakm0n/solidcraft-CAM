//! CAM drilling operations (`cam.drills`): the operations that plunge a tool at picked hole
//! centers, each with an id, a name, the `setup` and `tool` it runs in and on, its
//! `hole_centers` as persistent vertex or point names, its `cycle_type` (`standard`, `peck` or
//! `deep_hole`), its `depth_reference` (`hole_bottom`, `model_bottom` or `absolute`), and the
//! `clearance_height`, `peck_depth` and `depth` it drills at, in mm.
//! Older designs need no transformation: a design without the key has no drilling operations,
//! keeps saving without it, and machines exactly as it did before drilling existed. The last
//! release opens such a design with a warning about the newer format number, reads what it can
//! and keeps the rest, the drilling operations included (`crates/doc/src/newer_file.rs`).

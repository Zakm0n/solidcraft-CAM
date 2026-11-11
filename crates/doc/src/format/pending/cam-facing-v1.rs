//! CAM 2D facing operations (`cam.facings`): the operations that flatten the top of the stock,
//! each with an id, a name, the `setup` and `tool` it runs in and on, its `stepover` as a
//! fraction of the tool's diameter, the `pass_extension` each pass runs beyond the stock's edge,
//! the `stock_offset` the raw stock stands off the machined bodies by, and the `depth` it cuts
//! below the stock's top, in mm.
//! Older designs need no transformation: a design without the key has no facing operations, keeps
//! saving without it, and machines exactly as it did before 2D facing existed. The last release
//! opens such a design with a warning about the newer format number, reads what it can and keeps
//! the rest, the facing operations included (`crates/doc/src/newer_file.rs`).

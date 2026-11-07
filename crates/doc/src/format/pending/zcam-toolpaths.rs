//! CAM data (`cam`): the machining setups (`setups`: id, name, `coordinate_system`, the raw
//! `stock` as `size` and `offset` in mm, and the bodies they machine), the tool library (`tools`:
//! id, name, `kind` — `flat_end_mill`, `ball_end_mill` or `bull_nose_end_mill` — `diameter`,
//! `corner_radius`, `flute_length` and `flutes`) and the toolpaths (`toolpaths`: id, name, `kind`
//! — `facing`, `contour` or `pocket` — `setup`, `tool`, `bodies`, `feed_rate`, `plunge_rate`,
//! `spindle_speed`, `stepover`, `depth_per_pass`, `total_depth` and `stock_to_leave`).
//! Older designs need no transformation: a design without the key has no setups, tools or
//! toolpaths, keeps saving without it, and machines exactly as it did before CAM existed. The
//! last release opens such a design with a warning about the newer format number, reads what it
//! can and keeps the rest, CAM data included (`crates/doc/src/newer_file.rs`).

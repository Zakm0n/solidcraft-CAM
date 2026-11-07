//! CAM coordinate systems: the machining origin a toolpath works from, with the machine's own
//! X, Y and Z directions (`cam.coordinate_system.create`).
//!
//! A coordinate system is not part of a body: it places a setup on the stock, and the browser
//! lists it in its *Coordinate Systems* folder.

use serde::{Deserialize, Serialize};

/// A machining origin with the machine's own axes: the machine zero a toolpath works from.
///
/// `origin` is in model space (mm) and the axes are unit vectors in model space, so moving or
/// rotating the model carries the coordinate system with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoordinateSystem {
    /// Its identity in the design; never reused.
    #[serde(default)]
    pub id: u64,
    /// Its name in the browser and in a toolpath's program.
    #[serde(default)]
    pub name: String,
    /// Where the machine zero sits, in mm.
    #[serde(default)]
    pub origin: [f64; 3],
    /// The machine's +X direction: a unit vector.
    #[serde(default = "model_x")]
    pub x_axis: [f64; 3],
    /// The machine's +Y direction: a unit vector.
    #[serde(default = "model_y")]
    pub y_axis: [f64; 3],
    /// The machine's +Z direction: a unit vector.
    #[serde(default = "model_z")]
    pub z_axis: [f64; 3],
}

impl Default for CoordinateSystem {
    fn default() -> Self {
        CoordinateSystem { id: 0, name: String::new(), origin: [0.0; 3], x_axis: model_x(), y_axis: model_y(), z_axis: model_z() }
    }
}

fn model_x() -> [f64; 3] {
    [1.0, 0.0, 0.0]
}

fn model_y() -> [f64; 3] {
    [0.0, 1.0, 0.0]
}

fn model_z() -> [f64; 3] {
    [0.0, 0.0, 1.0]
}

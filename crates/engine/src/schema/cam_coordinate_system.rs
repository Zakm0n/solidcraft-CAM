//! Contracts and result types for the commands in `cmd/cam_coordinate_system.rs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::Point3;

/// Create a coordinate system: a machining origin and the machine's own X, Y and Z directions,
/// which a toolpath uses as its machine zero. The axes are stored as unit vectors in model space
/// and must be square to each other.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CreateCoordinateSystemParams {
    /// Where the machine zero sits, in mm. Default: the model origin [0, 0, 0].
    #[serde(default)]
    pub origin: Option<Point3>,
    /// The machine's +X direction. Default: the model's +X, [1, 0, 0]. Any length; it is stored
    /// as a unit vector.
    #[serde(default)]
    pub x_axis: Option<Point3>,
    /// The machine's +Y direction. Default: computed as `z_axis × x_axis` (the direction square to
    /// both, so a call naming only X and Z always gets a square triad). Any length; it is stored as
    /// a unit vector.
    #[serde(default)]
    pub y_axis: Option<Point3>,
    /// The machine's +Z direction. Default: the model's +Z, [0, 0, 1]. Any length; it is stored
    /// as a unit vector.
    #[serde(default)]
    pub z_axis: Option<Point3>,
    /// Its name. Default: "Coordinate System" and the first number no coordinate system's name
    /// has.
    #[serde(default)]
    pub name: Option<String>,
}

/// What `cam.coordinate_system.create` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CoordinateSystemCreated {
    /// The new coordinate system's id.
    pub coordinate_system: u64,
    /// Its name.
    pub name: String,
    /// Where the machine zero sits, in mm.
    pub origin: [f64; 3],
    /// The machine's +X direction: a unit vector.
    pub x_axis: [f64; 3],
    /// The machine's +Y direction: a unit vector.
    pub y_axis: [f64; 3],
    /// The machine's +Z direction: a unit vector.
    pub z_axis: [f64; 3],
    /// How many coordinate systems the design has now.
    pub count: usize,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[cfg(test)]
#[path = "cam_coordinate_system_tests.rs"]
mod tests;

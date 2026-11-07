//! Contracts and result types for the commands in `cmd/cam_setup.rs` (`cam.setup.create`).
//!
//! A setup names the coordinate system a part is machined from, the raw stock it is cut out of
//! and the bodies it machines. The handler reads its parameters from the raw JSON, so each key
//! here accepts what its handler reads and fails with the handler's message.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A stock dimension is 0 (read the machined bodies' extent) to 1000 mm.
pub const STOCK_SIZE_PROBLEM: &str = "must be a number from 0 to 1000";

/// Create a machining setup: the coordinate system it works from, the raw stock it cuts out of
/// and the bodies it machines. With no coordinate system it works from the model origin and the
/// model's own axes; with no stock size it takes the machined bodies' extent along each axis.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CreateSetupParams {
    /// The id of the coordinate system whose machine zero and axes it works from. Default: 0,
    /// the model origin with the model's own axes.
    #[serde(default)]
    pub coordinate_system: Option<u64>,
    /// The stock's size along the setup's X, Y and Z, in mm. Default: [0, 0, 0], the machined
    /// bodies' extent along each axis.
    #[serde(default)]
    pub stock_size: Option<[f64; 3]>,
    /// Where the stock's corner nearest the machine zero sits, relative to that zero, in mm.
    /// Default: [0, 0, 0].
    #[serde(default)]
    pub stock_offset: Option<[f64; 3]>,
    /// The bodies it machines, by name. Default: every solid body of the design.
    #[serde(default)]
    pub bodies: Option<Vec<String>>,
    /// Its name. Default: "Setup" and the first number no setup's name has.
    #[serde(default)]
    pub name: Option<String>,
}

/// The raw stock a setup cuts out of.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StockReply {
    /// Its size along the setup's X, Y and Z, in mm; 0 along an axis means the machined bodies'
    /// extent there.
    pub size: [f64; 3],
    /// Where its corner nearest the machine zero sits, relative to that zero, in mm.
    pub offset: [f64; 3],
}

/// What `cam.setup.create` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SetupCreated {
    /// The new setup's id.
    pub setup: u64,
    /// Its name.
    pub name: String,
    /// The coordinate system it works from; 0 is the model origin and the model's axes.
    pub coordinate_system: u64,
    /// The raw stock it cuts out of.
    pub stock: StockReply,
    /// The bodies it machines, by name; empty means every solid body.
    pub bodies: Vec<String>,
    /// How many setups the design has now.
    pub count: usize,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[cfg(test)]
#[path = "cam_setup_tests.rs"]
mod tests;

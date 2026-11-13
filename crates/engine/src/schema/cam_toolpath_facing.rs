//! Contracts and result types for the command in `cmd/cam_toolpath_facing.rs`
//! (`cam.toolpath.facing`): the 2D facing operation that flattens the top of the stock, pass by
//! pass across it, down to a depth.
//!
//! The handler reads its parameters from the raw JSON, so each key here accepts what its handler
//! reads and fails with the handler's message. The ranges are shared as `*_PROBLEM` constants so
//! the two cannot drift apart: a value outside one is refused by the contract in strict mode, and
//! by the handler in every mode, with one wording.
//!
//! Cross-field rules (a depth the stock cannot hold, a stock without an area, a pass that leaves
//! the machined bodies) need the whole call and the stock's exact geometry, so the handler keeps
//! them; the contract only checks each value on its own.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use super::types::LengthExpr;

/// A facing pass moves over 0.1 to 1 times the tool's diameter.
pub const FACING_STEPOVER_PROBLEM: &str = "must be a number from 0.1 to 1";
/// A pass extension or a stock offset is 0 to 1000 mm.
pub const FACING_OFFSET_PROBLEM: &str = "must be a number from 0 to 1000";
/// A facing depth is above 0 and at most 1000 mm.
pub const FACING_DEPTH_PROBLEM: &str = "must be a number above 0 and at most 1000";

/// `stepover`: how far the cutter moves over between neighbouring passes, as a fraction of its
/// diameter, 0.1 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FacingStepover(pub f64);

impl<'de> Deserialize<'de> for FacingStepover {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Value::deserialize(d)?
            .as_f64()
            .filter(|x| x.is_finite() && (0.1..=1.0).contains(x))
            .map(FacingStepover)
            .ok_or_else(|| D::Error::custom(FACING_STEPOVER_PROBLEM))
    }
}

impl JsonSchema for FacingStepover {
    fn schema_name() -> Cow<'static, str> {
        "CamFacingStepover".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "How far the cutter moves over between neighbouring passes, as a fraction of its diameter: 0.1 to 1.",
            "type": "number",
            "minimum": 0.1,
            "maximum": 1
        })
    }
}

/// The parameters a 2D facing operation takes: its setup, its tool, and the stepover, extension
/// and depth it cuts at.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CamToolpathFacingParams {
    /// The setup it runs in: its machine zero, axes and stock. Default: the model origin and the
    /// model's own axes, with the stock read from the machined bodies.
    #[serde(default)]
    pub setup: Option<u64>,
    /// The tool it cuts with: a tool's id in this design. Its diameter and the stepover decide
    /// how far apart the passes run.
    pub tool: u64,
    /// How far the cutter moves over between neighbouring passes, as a fraction of its diameter.
    /// Default: 0.7.
    #[serde(default)]
    pub stepover: Option<FacingStepover>,
    /// How far each pass runs beyond the stock's edge, in mm: a length or an expression. Default:
    /// 0.
    #[serde(default)]
    pub pass_extension: Option<LengthExpr>,
    /// How far the raw stock extends beyond the machined bodies on each side, in mm: a length or
    /// an expression, used when the setup states no stock size. Default: 0.
    #[serde(default)]
    pub stock_offset: Option<LengthExpr>,
    /// How deep the operation cuts below the top of the stock, in mm: a length or an expression.
    /// Above 0 and at most the stock's height.
    pub depth: LengthExpr,
    /// Its name in the browser and in its program: 1–128 characters, not another operation's.
    /// Default: "2D Facing" and the first number no facing operation's name has.
    #[serde(default)]
    pub name: Option<String>,
}

/// A 2D facing operation as the command reports it and as the design stores it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamToolpathFacingSummary {
    /// Its identity in the design; never reused.
    pub id: u64,
    /// Its name in the browser and in its program.
    pub name: String,
    /// The setup it runs in; 0 is the model origin.
    pub setup: u64,
    /// The tool it cuts with.
    pub tool: u64,
    /// How far the cutter moves over between neighbouring passes, as a fraction of its diameter.
    pub stepover: f64,
    /// How far each pass runs beyond the stock's edge, in mm.
    pub pass_extension: f64,
    /// How far the raw stock extends beyond the machined bodies on each side, in mm.
    pub stock_offset: f64,
    /// How deep it cuts below the top of the stock, in mm.
    pub depth: f64,
}

/// The box the facing operation clears, in the setup's machine coordinates: its extent along the
/// setup's X, Y and Z.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FacingBounds {
    /// Its corner nearest the machine zero, in mm.
    pub min: [f64; 3],
    /// Its corner furthest from the machine zero, in mm.
    pub max: [f64; 3],
    /// The area of its X/Y footprint, in mm².
    pub area: f64,
}

/// What `cam.toolpath.facing` returns: the stored operation, the box it clears and the toolpath
/// it generated.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamToolpathFacingOutput {
    /// The facing operation as it now is.
    pub facing: CamToolpathFacingSummary,
    /// How many facing operations the design has now.
    pub count: usize,
    /// How many passes it runs across the stock.
    pub passes: usize,
    /// The height of each cut level above the machine zero along the setup's Z, deepest last, in
    /// mm.
    pub cut_levels: Vec<f64>,
    /// The box it clears, in the setup's machine coordinates.
    pub bounds: FacingBounds,
    /// The generated toolpath, in cutting order: its points in the setup's machine coordinates,
    /// in mm. Each pass runs from one end of the stock to the other, the next one back.
    pub waypoints: Vec<[f64; 3]>,
    /// The toolpath's cutting length, in mm.
    pub cutting_length: f64,
    /// The toolpath's rapid length between passes, in mm.
    pub rapid_length: f64,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[cfg(test)]
#[path = "cam_toolpath_facing_tests.rs"]
mod tests;

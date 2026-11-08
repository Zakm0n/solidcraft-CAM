//! Contract and result types for `cam.export.gcode`: the ISO G-code program the design's
//! toolpaths make (`cmd/cam_export.rs`).
//!
//! The handler reads its parameters from the raw JSON, so each key here accepts what its
//! handler reads and fails with the handler's message. Ranges are shared as `*_PROBLEM`
//! constants so the contract and the handler cannot drift apart: a value outside one is refused
//! by the contract in strict mode, and by the handler in every mode, with one wording.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::schema::cam_toolpath::ToolpathKindParam;

/// A safe height is 0 to 1000 mm above the top of the stock.
pub const SAFE_HEIGHT_PROBLEM: &str = "must be a number from 0 to 1000";

/// `safe_height`: the clearance the cutter retracts to between passes, in mm above the stock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SafeHeight(pub f64);

impl<'de> Deserialize<'de> for SafeHeight {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        serde_json::Value::deserialize(d)?
            .as_f64()
            .filter(|x| x.is_finite() && (0.0..=1000.0).contains(x))
            .map(SafeHeight)
            .ok_or_else(|| D::Error::custom(SAFE_HEIGHT_PROBLEM))
    }
}

impl JsonSchema for SafeHeight {
    fn schema_name() -> Cow<'static, str> {
        "CamSafeHeight".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "How far above the top of the stock the cutter retracts between passes, in mm: 0 to 1000.",
            "type": "number",
            "minimum": 0,
            "maximum": 1000
        })
    }
}

/// The parameters an export takes: which toolpaths, where to write, and how the program is
/// named.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct GcodeParams {
    /// The toolpaths to export, by id. Default: every toolpath of the design.
    #[serde(default)]
    pub toolpaths: Option<Vec<u64>>,
    /// Only the toolpaths of this setup, by id. Default: every setup, including the model origin.
    #[serde(default)]
    pub setup: Option<u64>,
    /// The file to write. Default: the program is returned as `gcode` instead.
    #[serde(default)]
    pub path: Option<String>,
    /// The program's name in its header and in `%` markers: 1–128 characters. Default: the
    /// design's name.
    #[serde(default)]
    pub program: Option<String>,
    /// How far above the stock's top the cutter retracts between passes, in mm. Default: 5.
    #[serde(default)]
    pub safe_height: Option<SafeHeight>,
}

/// One toolpath as the program runs it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GcodeOperation {
    /// The toolpath's identity in the design.
    pub toolpath: u64,
    /// Its name, as the program's comments name it.
    pub name: String,
    /// What operation it runs.
    pub kind: ToolpathKindParam,
    /// The tool it cuts with.
    pub tool: u64,
    /// How many passes down the total depth it takes.
    pub passes: usize,
    /// How many cutting moves (G1) it makes.
    pub moves: usize,
}

/// What `cam.export.gcode` returns: the program it made and what it holds.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GcodeExport {
    /// The program's name.
    pub program: String,
    /// The units every coordinate is in: `mm`.
    pub units: &'static str,
    /// The toolpaths it runs, in the design's order.
    pub operations: Vec<GcodeOperation>,
    /// How many passes every operation takes together.
    pub passes: usize,
    /// How many lines the program has.
    pub lines: usize,
    /// How many bytes the program has.
    pub bytes: usize,
    /// Where the program was written, when a `path` was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The program itself, when no `path` was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gcode: Option<String>,
}

#[cfg(test)]
#[path = "cam_export_tests.rs"]
mod tests;

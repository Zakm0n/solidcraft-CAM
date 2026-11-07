//! Contracts and result types for the commands in `cmd/cam_toolpath.rs`: contour and pocket
//! toolpaths (`cam.toolpath.contour` and `cam.toolpath.pocket`).
//!
//! The handlers read their parameters from the raw JSON, so each key here accepts what its
//! handler reads and fails with the handler's message. The ranges are shared as `*_PROBLEM`
//! constants so the two cannot drift apart: a value outside one is refused by the contract in
//! strict mode, and by the handler in every mode, with one wording.
//!
//! Cross-field rules (a stepover no wider than the tool, a cut that stays inside the part) need
//! the whole call and the body's geometry, so the handler keeps them; the contract only checks
//! each value on its own.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use solidcraft_doc::cam::ToolpathKind;

/// A feed, plunge or spindle rate is 0 (not stated) to 1,000,000.
pub const RATE_PROBLEM: &str = "must be a number from 0 to 1000000";
/// A depth or stepover is above 0 and at most 1000 mm.
pub const DEPTH_PROBLEM: &str = "must be a number above 0 and at most 1000";
/// Material left for a finishing pass is 0 to 1000 mm.
pub const STOCK_TO_LEAVE_PROBLEM: &str = "must be a number from 0 to 1000";

/// The kind of operation a toolpath runs, in the spellings a design saves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolpathKindParam {
    /// Flatten the top of the stock, pass by pass across it.
    Facing,
    /// Follow the part's outline with the side of the cutter.
    Contour,
    /// Clear the material inside the part's outline.
    Pocket,
}

impl ToolpathKindParam {
    /// The document's kind for this one.
    pub fn to_doc(self) -> ToolpathKind {
        match self {
            ToolpathKindParam::Facing => ToolpathKind::Facing,
            ToolpathKindParam::Contour => ToolpathKind::Contour,
            ToolpathKindParam::Pocket => ToolpathKind::Pocket,
        }
    }

    /// The call's spelling of a document kind.
    pub fn from_doc(kind: ToolpathKind) -> Self {
        match kind {
            ToolpathKind::Facing => ToolpathKindParam::Facing,
            ToolpathKind::Contour => ToolpathKindParam::Contour,
            ToolpathKind::Pocket => ToolpathKindParam::Pocket,
        }
    }
}

impl<'de> Deserialize<'de> for ToolpathKindParam {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            Value::String(s) => match s.as_str() {
                "facing" => Ok(ToolpathKindParam::Facing),
                "contour" => Ok(ToolpathKindParam::Contour),
                "pocket" => Ok(ToolpathKindParam::Pocket),
                _ => Err(D::Error::custom("must be facing, contour or pocket")),
            },
            _ => Err(D::Error::custom("must be facing, contour or pocket")),
        }
    }
}

impl JsonSchema for ToolpathKindParam {
    fn schema_name() -> Cow<'static, str> {
        "ToolpathKind".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "What operation the toolpath runs: `facing`, `contour` or `pocket`.",
            "type": "string",
            "enum": ["facing", "contour", "pocket"]
        })
    }
}

/// `feed_rate`, `plunge_rate` and `spindle_speed`: 0 (not stated) to 1,000,000.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate(pub f64);

impl<'de> Deserialize<'de> for Rate {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Value::deserialize(d)?
            .as_f64()
            .filter(|x| x.is_finite() && (0.0..=1_000_000.0).contains(x))
            .map(Rate)
            .ok_or_else(|| D::Error::custom(RATE_PROBLEM))
    }
}

impl JsonSchema for Rate {
    fn schema_name() -> Cow<'static, str> {
        "CamRate".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A feed or plunge rate in mm per minute, or a spindle speed in revolutions per minute: 0 (not stated) to 1,000,000.",
            "type": "number",
            "minimum": 0,
            "maximum": 1000000
        })
    }
}

/// `depth_per_pass`, `total_depth` and `stepover`: above 0 and at most 1000 mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Depth(pub f64);

impl<'de> Deserialize<'de> for Depth {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Value::deserialize(d)?
            .as_f64()
            .filter(|x| x.is_finite() && *x > 0.0 && *x <= 1000.0)
            .map(Depth)
            .ok_or_else(|| D::Error::custom(DEPTH_PROBLEM))
    }
}

impl JsonSchema for Depth {
    fn schema_name() -> Cow<'static, str> {
        "CamDepth".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A depth or a stepover in mm: above 0 and at most 1000.",
            "type": "number",
            "exclusiveMinimum": 0,
            "maximum": 1000
        })
    }
}

/// `stock_to_leave`: 0 to 1000 mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StockToLeave(pub f64);

impl<'de> Deserialize<'de> for StockToLeave {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Value::deserialize(d)?
            .as_f64()
            .filter(|x| x.is_finite() && (0.0..=1000.0).contains(x))
            .map(StockToLeave)
            .ok_or_else(|| D::Error::custom(STOCK_TO_LEAVE_PROBLEM))
    }
}

impl JsonSchema for StockToLeave {
    fn schema_name() -> Cow<'static, str> {
        "CamStockToLeave".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "How much material the operation leaves for a finishing pass, in mm: 0 to 1000.",
            "type": "number",
            "minimum": 0,
            "maximum": 1000
        })
    }
}

/// The parameters a contour or pocket toolpath takes: its setup, its tool, the bodies it cuts
/// and the cutting values it runs at.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ToolpathParams {
    /// The setup it runs in: its machine zero, axes and stock. Default: the model origin and the
    /// model's own axes, with the stock read from the machined bodies.
    #[serde(default)]
    pub setup: Option<u64>,
    /// The tool it cuts with: a tool's id in this design.
    pub tool: u64,
    /// The bodies it cuts, by name. Default: the setup's bodies, or every solid body of the model
    /// when the setup names none.
    #[serde(default)]
    pub bodies: Option<Vec<String>>,
    /// Its name in the browser and in its program: 1–128 characters, not another toolpath's.
    /// Default: "Contour" or "Pocket" and the first number no toolpath's name has.
    #[serde(default)]
    pub name: Option<String>,
    /// How fast the cutter moves through material, in mm per minute. Default: 0 (not stated).
    #[serde(default)]
    pub feed_rate: Option<Rate>,
    /// How fast the cutter enters the material, in mm per minute. Default: 0 (not stated).
    #[serde(default)]
    pub plunge_rate: Option<Rate>,
    /// How fast the spindle turns, in revolutions per minute. Default: 0 (not stated).
    #[serde(default)]
    pub spindle_speed: Option<Rate>,
    /// How far the cutter moves over between neighbouring passes, in mm. Default: half the tool's
    /// diameter.
    #[serde(default)]
    pub stepover: Option<Depth>,
    /// How deep one pass cuts, in mm. Default: the whole total depth (one pass).
    #[serde(default)]
    pub depth_per_pass: Option<Depth>,
    /// How deep the operation cuts below the top of the stock, in mm. Default: through the
    /// machined bodies, from the stock's top to their lowest point.
    #[serde(default)]
    pub total_depth: Option<Depth>,
    /// How much material the operation leaves for a finishing pass, in mm. Default: 0 (none).
    #[serde(default)]
    pub stock_to_leave: Option<StockToLeave>,
}

/// One cutter path boundary, as the commands report it: a closed loop of the part's exact
/// outline, sampled into points in the setup's X/Y plane (mm, from the machine zero).
#[derive(Debug, Serialize, JsonSchema)]
pub struct BoundaryLoop {
    /// The body whose outline this loop is.
    pub body: String,
    /// Whether it is the part's outer outline or the edge of an opening inside it.
    pub role: BoundaryRole,
    /// The loop's length around, in mm.
    pub length: f64,
    /// Its points, in the setup's X/Y, without a repeated closing point.
    pub points: Vec<[f64; 2]>,
}

/// What a boundary loop is around an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryRole {
    /// The part's outer outline: cut it with the cutter's side.
    Outer,
    /// The edge of an opening inside the part: an island to keep the cutter out of.
    Hole,
}

/// A toolpath as the commands report it and as the design stores it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ToolpathSummary {
    /// Its identity in the design; never reused.
    pub id: u64,
    /// Its name in the browser and in its program.
    pub name: String,
    /// What operation it runs.
    pub kind: ToolpathKindParam,
    /// The setup it runs in; 0 is the model origin.
    pub setup: u64,
    /// The tool it cuts with.
    pub tool: u64,
    /// The bodies it cuts, by name.
    pub bodies: Vec<String>,
    /// How fast the cutter moves through material, in mm per minute (0 when not stated).
    pub feed_rate: f64,
    /// How fast the cutter enters the material, in mm per minute (0 when not stated).
    pub plunge_rate: f64,
    /// How fast the spindle turns, in revolutions per minute (0 when not stated).
    pub spindle_speed: f64,
    /// How far the cutter moves over between neighbouring passes, in mm.
    pub stepover: f64,
    /// How deep one pass cuts, in mm.
    pub depth_per_pass: f64,
    /// How deep the operation cuts below the top of the stock, in mm.
    pub total_depth: f64,
    /// How much material the operation leaves for a finishing pass, in mm.
    pub stock_to_leave: f64,
}

/// What `cam.toolpath.contour` and `cam.toolpath.pocket` return: the stored toolpath and the
/// exact boundaries it works from.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ToolpathReply {
    /// The toolpath as it now is.
    pub toolpath: ToolpathSummary,
    /// How many toolpaths the design has now.
    pub count: usize,
    /// How many passes down the total depth it takes.
    pub passes: usize,
    /// The height of each pass above the machine zero along the setup's Z, deepest last, in mm.
    pub cut_levels: Vec<f64>,
    /// The part's outline loops, the outer ones before the holes.
    pub boundaries: Vec<BoundaryLoop>,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[cfg(test)]
#[path = "cam_toolpath_tests.rs"]
mod tests;

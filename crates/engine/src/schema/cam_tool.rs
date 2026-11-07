//! Contracts and result types for the commands in `cmd/cam_tool.rs`: the design's tool library
//! (`cam.tool.create`, `cam.tool.edit` and `cam.tool.list`).
//!
//! The handlers read their parameters from the raw JSON, so each key here accepts what its
//! handler reads and fails with the handler's message. The ranges are shared as `*_PROBLEM`
//! constants so the two cannot drift apart: a value outside one is refused by the contract in
//! strict mode, and by the handler in every mode, with one wording.
//!
//! Cross-field rules (a tool's corner radius follows its kind and diameter) need the whole
//! call, so the handler keeps them; the contract only checks each value on its own.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use solidcraft_doc::cam::ToolKind;

/// A diameter is above 0 and at most 1000 mm.
pub const DIAMETER_PROBLEM: &str = "must be a number above 0 and at most 1000";
/// A flute length is 0 (not stated) to 1000 mm.
pub const FLUTE_LENGTH_PROBLEM: &str = "must be a number from 0 to 1000";
/// A corner radius is 0 to 1000 mm; being no more than half the diameter is the handler's check.
pub const CORNER_RADIUS_PROBLEM: &str = "must be a number from 0 to 1000";
/// A tool has 0 (not stated) to 100 cutting edges.
pub const FLUTES_PROBLEM: &str = "must be a whole number from 0 to 100";
/// The three cutter kinds, in the spellings a design saves.
pub const KIND_PROBLEM: &str = "must be flat_end_mill, ball_end_mill or bull_nose_end_mill";

/// A cutter kind as a call writes it: `flat_end_mill`, `ball_end_mill` or `bull_nose_end_mill`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKindParam {
    /// A flat-ended cutter with a square corner.
    FlatEndMill,
    /// A ball-nose cutter: a hemisphere on the end.
    BallEndMill,
    /// A bull-nose cutter: a flat end whose corners are rounded.
    BullNoseEndMill,
}

impl ToolKindParam {
    /// The document's kind for this one.
    pub fn to_doc(self) -> ToolKind {
        match self {
            ToolKindParam::FlatEndMill => ToolKind::FlatEndMill,
            ToolKindParam::BallEndMill => ToolKind::BallEndMill,
            ToolKindParam::BullNoseEndMill => ToolKind::BullNoseEndMill,
        }
    }

    /// The call's spelling of a document kind.
    pub fn from_doc(kind: ToolKind) -> Self {
        match kind {
            ToolKind::FlatEndMill => ToolKindParam::FlatEndMill,
            ToolKind::BallEndMill => ToolKindParam::BallEndMill,
            ToolKind::BullNoseEndMill => ToolKindParam::BullNoseEndMill,
        }
    }
}

impl<'de> Deserialize<'de> for ToolKindParam {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            Value::String(s) => match s.as_str() {
                "flat_end_mill" => Ok(ToolKindParam::FlatEndMill),
                "ball_end_mill" => Ok(ToolKindParam::BallEndMill),
                "bull_nose_end_mill" => Ok(ToolKindParam::BullNoseEndMill),
                _ => Err(D::Error::custom(KIND_PROBLEM)),
            },
            _ => Err(D::Error::custom(KIND_PROBLEM)),
        }
    }
}

impl JsonSchema for ToolKindParam {
    fn schema_name() -> Cow<'static, str> {
        "ToolKind".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "What shape the cutter's cutting end has: `flat_end_mill` (a square corner), `ball_end_mill` (a hemisphere) or `bull_nose_end_mill` (a flat end with rounded corners).",
            "type": "string",
            "enum": ["flat_end_mill", "ball_end_mill", "bull_nose_end_mill"]
        })
    }
}

/// `diameter`: the diameter the cutter cuts, in mm, above 0 and at most 1000.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolDiameter(pub f64);

impl<'de> Deserialize<'de> for ToolDiameter {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Value::deserialize(d)?
            .as_f64()
            .filter(|x| x.is_finite() && *x > 0.0 && *x <= 1000.0)
            .map(ToolDiameter)
            .ok_or_else(|| D::Error::custom(DIAMETER_PROBLEM))
    }
}

impl JsonSchema for ToolDiameter {
    fn schema_name() -> Cow<'static, str> {
        "ToolDiameter".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The diameter the cutter cuts, in mm: above 0 and at most 1000.",
            "type": "number",
            "exclusiveMinimum": 0,
            "maximum": 1000
        })
    }
}

/// `corner_radius`: the radius the cutter's corners are rounded by, in mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolCornerRadius(pub f64);

impl<'de> Deserialize<'de> for ToolCornerRadius {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Value::deserialize(d)?
            .as_f64()
            .filter(|x| x.is_finite() && (0.0..=1000.0).contains(x))
            .map(ToolCornerRadius)
            .ok_or_else(|| D::Error::custom(CORNER_RADIUS_PROBLEM))
    }
}

impl JsonSchema for ToolCornerRadius {
    fn schema_name() -> Cow<'static, str> {
        "ToolCornerRadius".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The radius the cutter's cutting corners are rounded by, in mm: 0 for a square corner, and no more than half the diameter.",
            "type": "number",
            "minimum": 0,
            "maximum": 1000
        })
    }
}

/// `flute_length`: the length of the cutter's flutes, in mm; 0 means not stated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolFluteLength(pub f64);

impl<'de> Deserialize<'de> for ToolFluteLength {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Value::deserialize(d)?
            .as_f64()
            .filter(|x| x.is_finite() && (0.0..=1000.0).contains(x))
            .map(ToolFluteLength)
            .ok_or_else(|| D::Error::custom(FLUTE_LENGTH_PROBLEM))
    }
}

impl JsonSchema for ToolFluteLength {
    fn schema_name() -> Cow<'static, str> {
        "ToolFluteLength".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The length of the cutter's cutting flutes, in mm: the deepest it may cut in one go. 0 means not stated.",
            "type": "number",
            "minimum": 0,
            "maximum": 1000
        })
    }
}

/// `flutes`: how many cutting edges the cutter has; 0 means not stated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolFlutes(pub u32);

impl<'de> Deserialize<'de> for ToolFlutes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Value::deserialize(d)?
            .as_u64()
            .filter(|n| *n <= 100)
            .and_then(|n| u32::try_from(n).ok())
            .map(ToolFlutes)
            .ok_or_else(|| D::Error::custom(FLUTES_PROBLEM))
    }
}

impl JsonSchema for ToolFlutes {
    fn schema_name() -> Cow<'static, str> {
        "ToolFlutes".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "How many cutting edges the cutter has, from 0 (not stated) to 100.",
            "type": "integer",
            "minimum": 0,
            "maximum": 100
        })
    }
}

/// Add a cutter to the design's tool library. Its corner radius follows its kind when it is
/// left out: 0 for a flat end mill, half the diameter for a ball-nose cutter, and 0 for a
/// bull-nose cutter.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CreateToolParams {
    /// Its name in the tool library and in a toolpath's program: 1–128 characters, not another
    /// tool's. Default: "Tool" and the first number no tool's name has.
    #[serde(default)]
    pub name: Option<String>,
    /// What shape its cutting end has. Default: `flat_end_mill`.
    #[serde(default)]
    pub kind: Option<ToolKindParam>,
    /// The diameter it cuts, in mm: above 0 and at most 1000.
    pub diameter: ToolDiameter,
    /// The radius its cutting corners are rounded by, in mm. Default: 0 for a flat end mill, half
    /// the diameter for a ball-nose cutter and 0 for a bull-nose cutter. A flat end mill takes 0
    /// only.
    #[serde(default)]
    pub corner_radius: Option<ToolCornerRadius>,
    /// The length of its cutting flutes, in mm: the deepest it may cut in one go. Default: 0
    /// (not stated).
    #[serde(default)]
    pub flute_length: Option<ToolFluteLength>,
    /// How many cutting edges it has, from 0 to 100. Default: 0 (not stated).
    #[serde(default)]
    pub flutes: Option<ToolFlutes>,
}

/// Change a cutter of the design's tool library. A key left out keeps what the tool has.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct EditToolParams {
    /// The tool to change: its id in this design.
    pub tool: u64,
    /// Its new name: 1–128 characters, not another tool's. Blank or left out keeps its name.
    #[serde(default)]
    pub name: Option<String>,
    /// What shape its cutting end now has. Changing it sets the corner radius to the new kind's
    /// default unless `corner_radius` is also given.
    #[serde(default)]
    pub kind: Option<ToolKindParam>,
    /// Its new diameter, in mm: above 0 and at most 1000. A ball-nose cutter's corner radius
    /// follows it unless `corner_radius` is also given.
    #[serde(default)]
    pub diameter: Option<ToolDiameter>,
    /// Its new corner radius, in mm: 0 for a square corner, and no more than half the diameter.
    #[serde(default)]
    pub corner_radius: Option<ToolCornerRadius>,
    /// Its new flute length, in mm: 0 to 1000, where 0 means not stated.
    #[serde(default)]
    pub flute_length: Option<ToolFluteLength>,
    /// Its new number of cutting edges, from 0 to 100, where 0 means not stated.
    #[serde(default)]
    pub flutes: Option<ToolFlutes>,
}

/// List the design's tool library, in the order its tools were made.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ListToolsParams {}

/// One cutter of the design's tool library, as the commands report it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ToolSummary {
    /// Its identity in the design; never reused.
    pub id: u64,
    /// Its name in the tool library and in a toolpath's program.
    pub name: String,
    /// What shape its cutting end has.
    pub kind: ToolKindParam,
    /// The diameter it cuts, in mm.
    pub diameter: f64,
    /// The radius its cutting corners are rounded by, in mm (0 for a square corner).
    pub corner_radius: f64,
    /// The length of its cutting flutes, in mm (0 when not stated).
    pub flute_length: f64,
    /// How many cutting edges it has (0 when not stated).
    pub flutes: u32,
}

/// What `cam.tool.create` and `cam.tool.edit` return: the tool as it now is.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ToolReply {
    /// The tool as it now is.
    pub tool: ToolSummary,
    /// How many tools the design has now.
    pub count: usize,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// What `cam.tool.list` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ToolsListed {
    /// The design's tools, in the order they were made.
    pub tools: Vec<ToolSummary>,
    /// How many tools the design has.
    pub count: usize,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[cfg(test)]
#[path = "cam_tool_tests.rs"]
mod tests;

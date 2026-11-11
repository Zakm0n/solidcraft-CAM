//! Contracts and result types for the commands in `cmd/cam_toolpath_drill.rs`
//! (`cam.toolpath.drill`): a drilling operation that plunges a tool at picked hole centers, in
//! the cycle and down to the depth the call names.
//!
//! The handler reads its parameters from the raw JSON, so each key here accepts what its
//! handler reads and fails with the handler's message. The ranges are shared as `*_PROBLEM`
//! constants so the two cannot drift apart: a value outside one is refused by the contract in
//! strict mode, and by the handler in every mode, with one wording.
//!
//! Cross-field rules (a hole center that finds no hole, a peck deeper than the depth, a depth
//! the stock cannot hold) need the whole call and the body's geometry, so the handler keeps
//! them; the contract only checks each value on its own.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use solidcraft_doc::cam::{DrillCycle, DrillDepthReference};

use super::cam_toolpath::{ClearanceHeight, Depth, Rate};

/// A hole's name is at most 512 bytes of UTF-8, as a face's or an edge's persistent name is.
pub const HOLE_NAME_PROBLEM: &str = "must be a name of 1 to 512 bytes of UTF-8";
/// A drilling call takes 1 to 1000 hole centers.
pub const HOLE_CENTERS_PROBLEM: &str = "select 1…1000 hole centers";
/// A peck depth is 0 (the tool's flute length) to 1000 mm.
pub const PECK_DEPTH_PROBLEM: &str = "must be a number from 0 to 1000";

/// `hole_centers`: 1 to 1000 exact vertex or point references, each a persistent name as
/// `model.faces` and `model.edges` list their items, so a hole follows the part as it changes.
#[derive(Clone, Debug, PartialEq)]
pub struct HoleCenters(pub Vec<String>);

impl<'de> Deserialize<'de> for HoleCenters {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let items = v.as_array().ok_or_else(|| D::Error::custom(HOLE_CENTERS_PROBLEM))?;
        if items.is_empty() || items.len() > 1000 {
            return Err(D::Error::custom(HOLE_CENTERS_PROBLEM));
        }
        items
            .iter()
            .map(|item| match item.as_str() {
                Some(name) if !name.trim().is_empty() && name.len() <= 512 => Ok(name.to_string()),
                _ => Err(D::Error::custom(HOLE_NAME_PROBLEM)),
            })
            .collect::<Result<_, _>>()
            .map(HoleCenters)
    }
}

impl JsonSchema for HoleCenters {
    fn schema_name() -> Cow<'static, str> {
        "CamHoleCenters".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The hole centers, 1 to 1000: each the persistent name of an exact vertex or point (`crates/doc/src/persistent_name.rs`), so it follows the part as it changes.",
            "type": "array",
            "items": {"type": "string", "minLength": 1, "maxLength": 512},
            "minItems": 1,
            "maxItems": 1000
        })
    }
}

/// `peck_depth`: 0 (the tool's flute length) to 1000 mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeckDepth(pub f64);

impl<'de> Deserialize<'de> for PeckDepth {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Value::deserialize(d)?
            .as_f64()
            .filter(|x| x.is_finite() && (0.0..=1000.0).contains(x))
            .map(PeckDepth)
            .ok_or_else(|| D::Error::custom(PECK_DEPTH_PROBLEM))
    }
}

impl JsonSchema for PeckDepth {
    fn schema_name() -> Cow<'static, str> {
        "CamPeckDepth".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "How deep one peck cuts in a peck or deep hole cycle, in mm: 0 (the tool's flute length) to 1000.",
            "type": "number",
            "minimum": 0,
            "maximum": 1000
        })
    }
}

/// The cycle a drilling call runs, in the spellings a design saves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DrillCycleParam {
    /// Plunge to the bottom in one move, then retract.
    #[default]
    Standard,
    /// Plunge one peck at a time and retract clear of the hole between pecks.
    Peck,
    /// Plunge one peck at a time and retract only just inside the hole between pecks.
    DeepHole,
}

impl DrillCycleParam {
    /// The document's cycle for this one.
    pub fn to_doc(self) -> DrillCycle {
        match self {
            DrillCycleParam::Standard => DrillCycle::Standard,
            DrillCycleParam::Peck => DrillCycle::Peck,
            DrillCycleParam::DeepHole => DrillCycle::DeepHole,
        }
    }

    /// The call's spelling of a document cycle.
    pub fn from_doc(cycle: DrillCycle) -> Self {
        match cycle {
            DrillCycle::Standard => DrillCycleParam::Standard,
            DrillCycle::Peck => DrillCycleParam::Peck,
            DrillCycle::DeepHole => DrillCycleParam::DeepHole,
        }
    }
}

impl<'de> Deserialize<'de> for DrillCycleParam {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            Value::String(s) => match s.as_str() {
                "standard" => Ok(DrillCycleParam::Standard),
                "peck" => Ok(DrillCycleParam::Peck),
                "deep_hole" => Ok(DrillCycleParam::DeepHole),
                _ => Err(D::Error::custom("must be standard, peck or deep_hole")),
            },
            _ => Err(D::Error::custom("must be standard, peck or deep_hole")),
        }
    }
}

impl JsonSchema for DrillCycleParam {
    fn schema_name() -> Cow<'static, str> {
        "CamDrillCycle".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The drilling cycle: `standard` (plunge to the bottom in one move), `peck` (retract clear of the hole between pecks) or `deep_hole` (retract only just inside it between pecks).",
            "type": "string",
            "enum": ["standard", "peck", "deep_hole"]
        })
    }
}

/// Where a drilling call takes the depth it cuts down to from, in the spellings a design saves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DrillDepthReferenceParam {
    /// The bottom of the hole the picked center is in.
    #[default]
    HoleBottom,
    /// The lowest point of the machined bodies.
    ModelBottom,
    /// The depth the call states in `depth`, below the top of the stock.
    Absolute,
}

impl DrillDepthReferenceParam {
    /// The document's reference for this one.
    pub fn to_doc(self) -> DrillDepthReference {
        match self {
            DrillDepthReferenceParam::HoleBottom => DrillDepthReference::HoleBottom,
            DrillDepthReferenceParam::ModelBottom => DrillDepthReference::ModelBottom,
            DrillDepthReferenceParam::Absolute => DrillDepthReference::Absolute,
        }
    }

    /// The call's spelling of a document reference.
    pub fn from_doc(reference: DrillDepthReference) -> Self {
        match reference {
            DrillDepthReference::HoleBottom => DrillDepthReferenceParam::HoleBottom,
            DrillDepthReference::ModelBottom => DrillDepthReferenceParam::ModelBottom,
            DrillDepthReference::Absolute => DrillDepthReferenceParam::Absolute,
        }
    }
}

impl<'de> Deserialize<'de> for DrillDepthReferenceParam {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            Value::String(s) => match s.as_str() {
                "hole_bottom" => Ok(DrillDepthReferenceParam::HoleBottom),
                "model_bottom" => Ok(DrillDepthReferenceParam::ModelBottom),
                "absolute" => Ok(DrillDepthReferenceParam::Absolute),
                _ => Err(D::Error::custom("must be hole_bottom, model_bottom or absolute")),
            },
            _ => Err(D::Error::custom("must be hole_bottom, model_bottom or absolute")),
        }
    }
}

impl JsonSchema for DrillDepthReferenceParam {
    fn schema_name() -> Cow<'static, str> {
        "CamDrillDepthReference".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Where the depth comes from: `hole_bottom` (the picked hole's exact bottom), `model_bottom` (the lowest point of the machined bodies) or `absolute` (`depth` below the top of the stock).",
            "type": "string",
            "enum": ["hole_bottom", "model_bottom", "absolute"]
        })
    }
}

/// The parameters a drilling toolpath takes: its setup, its tool, the holes it drills and the
/// cycle and depths it runs at.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CamToolpathDrillParams {
    /// The setup it runs in: its machine zero, axes and stock. Default: the model origin and the
    /// model's own axes, with the stock read from the drilled holes' bodies.
    #[serde(default)]
    pub setup: Option<u64>,
    /// The tool it drills with: a tool's id in this design.
    pub tool: u64,
    /// The centers it drills at: 1 to 1000 exact vertex or point references, each by its
    /// persistent name.
    pub hole_centers: HoleCenters,
    /// The cycle it runs. Default: `standard` (one plunge to the bottom).
    #[serde(default)]
    pub cycle_type: DrillCycleParam,
    /// Where it takes its depth from. Default: `hole_bottom`.
    #[serde(default)]
    pub depth_reference: DrillDepthReferenceParam,
    /// How high above the top of the stock the cutter travels between holes, in mm. Default: 0
    /// (at the top of the stock).
    #[serde(default)]
    pub clearance_height: Option<ClearanceHeight>,
    /// How deep one peck cuts in a `peck` or `deep_hole` cycle, in mm. Default: 0 (the tool's
    /// flute length).
    #[serde(default)]
    pub peck_depth: Option<PeckDepth>,
    /// How deep it cuts below the top of the stock, in mm, when `depth_reference` is
    /// `absolute`. Default: none, which `absolute` needs.
    #[serde(default)]
    pub depth: Option<Depth>,
    /// How fast the cutter moves through material, in mm per minute. Default: 0 (not stated).
    #[serde(default)]
    pub feed_rate: Option<Rate>,
    /// How fast the cutter enters the material, in mm per minute. Default: 0 (not stated).
    #[serde(default)]
    pub plunge_rate: Option<Rate>,
    /// How fast the spindle turns, in revolutions per minute. Default: 0 (not stated).
    #[serde(default)]
    pub spindle_speed: Option<Rate>,
    /// Its name in the browser and in its program: 1–128 characters, not another operation's.
    /// Default: "Drilling" and the first number no operation's name has.
    #[serde(default)]
    pub name: Option<String>,
}

/// A drilling operation as the command reports it and as the design stores it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamToolpathDrillSummary {
    /// Its identity in the design; never reused.
    pub id: u64,
    /// Its name in the browser and in its program.
    pub name: String,
    /// The setup it runs in; 0 is the model origin.
    pub setup: u64,
    /// The tool it drills with.
    pub tool: u64,
    /// The centers it drills at, by persistent vertex or point name.
    pub hole_centers: Vec<String>,
    /// The cycle it runs.
    pub cycle_type: DrillCycleParam,
    /// Where it takes its depth from.
    pub depth_reference: DrillDepthReferenceParam,
    /// How high above the top of the stock the cutter travels between holes, in mm.
    pub clearance_height: f64,
    /// How deep one peck cuts, in mm; 0 is the tool's flute length.
    pub peck_depth: f64,
    /// How deep it cuts below the top of the stock, in mm (0 when its reference names the
    /// depth).
    pub depth: f64,
}

/// What `cam.toolpath.drill` returns: the stored operation, how much it cuts and how long it
/// takes.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamToolpathDrillOutput {
    /// The drilling operation as it now is.
    pub drill: CamToolpathDrillSummary,
    /// How many drilling operations the design has now.
    pub count: usize,
    /// How many holes it drills.
    pub holes: usize,
    /// How many pecks the whole operation plunges, one per hole in a `standard` cycle.
    pub pecks: usize,
    /// The generated toolpath, in drilling order: its points in the setup's machine coordinates,
    /// in mm (rapid to the clearance height, plunge, retract, for each hole).
    pub waypoints: Vec<[f64; 3]>,
    /// The toolpath's cutting length, in mm.
    pub cutting_length: f64,
    /// The toolpath's rapid length, in mm.
    pub rapid_length: f64,
    /// How long the operation takes, in seconds, from its plunge and rapid rates: 0 when it
    /// states neither.
    pub cycle_time: f64,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[cfg(test)]
#[path = "cam_toolpath_drill_tests.rs"]
mod tests;

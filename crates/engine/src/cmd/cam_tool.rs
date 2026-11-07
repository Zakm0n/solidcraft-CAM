//! CAM ▸ TOOLS: the design's tool library, the cutters its toolpaths cut with
//! (`cam.tool.create`, `cam.tool.edit` and `cam.tool.list`).
//!
//! A tool is named data, not geometry: its kind, diameter, corner radius, flute length and
//! number of cutting edges. A toolpath copies what it needs when it is made, so editing a tool
//! never changes an operation that was already cut with it. Creating and editing a tool are
//! undoable like any other design change, and a refused call changes nothing.

use std::collections::BTreeSet;

use serde_json::Value;
use solidcraft_doc::cam::{Tool, ToolKind};

use super::CommandSpec;
use crate::params::{bad, num, str_};
use crate::schema::TypedContract;
use crate::schema::cam_tool::{
    CORNER_RADIUS_PROBLEM, CreateToolParams, DIAMETER_PROBLEM, EditToolParams, FLUTE_LENGTH_PROBLEM, FLUTES_PROBLEM, KIND_PROBLEM, ListToolsParams,
    ToolKindParam, ToolReply, ToolSummary, ToolsListed,
};
use crate::schema::output_schema_for;
use crate::{Result, Session};

const CREATE_CONTRACT: TypedContract<CreateToolParams> = TypedContract::new();
const EDIT_CONTRACT: TypedContract<EditToolParams> = TypedContract::new();
const LIST_CONTRACT: TypedContract<ListToolsParams> = TypedContract::new();

/// The most tools a design may hold.
const MAX_TOOLS: usize = 1000;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cam.tool.create", "Tool", crate::checked!("cam.tool.create", create))
        .doc_only()
        .contract(&CREATE_CONTRACT)
        .output(output_schema_for::<ToolReply>)
        .params(
            "diameter: mm, above 0 and at most 1000; name?: 1–128 characters (default: \"Tool\" and the first free number); kind?: flat_end_mill (default) | ball_end_mill | bull_nose_end_mill; corner_radius?: mm (default: 0, or half the diameter for a ball-nose cutter); flute_length?: mm, 0 to 1000 (default 0: not stated); flutes?: whole number 0 to 100 (default 0: not stated)",
        ),
    CommandSpec::new("cam.tool.edit", "Edit Tool", crate::checked!("cam.tool.edit", edit))
        .doc_only()
        .contract(&EDIT_CONTRACT)
        .output(output_schema_for::<ToolReply>)
        .params(
            "tool: a tool's id in this design; name?, kind?, diameter?, corner_radius?, flute_length?, flutes?: as cam.tool.create. A key left out keeps what the tool has; changing kind or diameter carries the corner radius with it unless that is given too",
        ),
    CommandSpec::new("cam.tool.list", "Tool Library", crate::checked!("cam.tool.list", list))
        .doc_only()
        .noundo()
        .contract(&LIST_CONTRACT)
        .output(output_schema_for::<ToolsListed>)
        .params("lists the design's tools in the order they were made"),
];

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cam.tool.create";
    if s.doc.cam.tools.len() >= MAX_TOOLS {
        return Err(bad(cmd, "too many tools"));
    }
    let kind = kind(p, cmd)?.unwrap_or(ToolKind::FlatEndMill);
    let diameter = diameter(p, cmd, true)?.ok_or_else(|| bad(cmd, format!("`diameter` {DIAMETER_PROBLEM}")))?;
    let corner_radius = match corner_radius(p, cmd)? {
        Some(r) => r,
        None => default_corner_radius(kind, diameter),
    };
    let flute_length = flute_length(p, cmd)?.unwrap_or(0.0);
    let flutes = flutes(p, cmd)?.unwrap_or(0);
    let mut tool = Tool { id: 0, name: String::new(), kind, diameter, corner_radius, flute_length, flutes };
    check_cutting(cmd, &tool)?;
    tool.name = new_name(s, cmd, str_(p, "name"), None)?;
    let count = {
        let doc = s.doc_mut();
        let id = doc.allocate_id()?;
        tool.id = id;
        doc.cam.tools.push(tool.clone());
        doc.cam.tools.len()
    };
    reply(cmd, &tool, count)
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cam.tool.edit";
    let id = p.get("tool").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "`tool` must be a tool's id in this design"))?;
    let index = s.doc.cam.tools.iter().position(|t| t.id == id).ok_or_else(|| bad(cmd, format!("there is no tool {id}")))?;
    let mut tool = s.doc.cam.tools[index].clone();
    let given_kind = kind(p, cmd)?;
    if let Some(kind) = given_kind {
        tool.kind = kind;
    }
    if let Some(diameter) = diameter(p, cmd, false)? {
        tool.diameter = diameter;
    }
    match corner_radius(p, cmd)? {
        Some(r) => tool.corner_radius = r,
        // A kind or diameter change carries the corner radius with it: a ball-nose cutter's is
        // half its diameter and a flat end mill's is 0.
        None if given_kind.is_some() || p.get("diameter").is_some_and(|d| !d.is_null()) => {
            tool.corner_radius = default_corner_radius(tool.kind, tool.diameter);
        }
        None => {}
    }
    if let Some(length) = flute_length(p, cmd)? {
        tool.flute_length = length;
    }
    if let Some(n) = flutes(p, cmd)? {
        tool.flutes = n;
    }
    check_cutting(cmd, &tool)?;
    if let Some(name) = str_(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        tool.name = new_name(s, cmd, Some(name), Some(id))?;
    }
    let count = {
        let doc = s.doc_mut();
        doc.cam.tools[index] = tool.clone();
        doc.cam.tools.len()
    };
    reply(cmd, &tool, count)
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let tools: Vec<ToolSummary> = s.doc.cam.tools.iter().map(summary).collect();
    let count = tools.len();
    serde_json::to_value(ToolsListed { tools, count, param_warnings: Vec::new() }).map_err(|e| bad("cam.tool.list", e.to_string()))
}

fn reply(cmd: &str, tool: &Tool, count: usize) -> Result<Value> {
    serde_json::to_value(ToolReply { tool: summary(tool), count, param_warnings: Vec::new() }).map_err(|e| bad(cmd, e.to_string()))
}

fn summary(tool: &Tool) -> ToolSummary {
    ToolSummary {
        id: tool.id,
        name: tool.name.clone(),
        kind: ToolKindParam::from_doc(tool.kind),
        diameter: tool.diameter,
        corner_radius: tool.corner_radius,
        flute_length: tool.flute_length,
        flutes: tool.flutes,
    }
}

/// A new tool name: 1–128 trimmed characters, not another tool's (`except`: the one being
/// renamed). A blank or absent name takes "Tool" and the first number no tool's name has.
fn new_name(s: &Session, cmd: &str, given: Option<&str>, except: Option<u64>) -> Result<String> {
    let given = given.map(str::trim).filter(|n| !n.is_empty());
    let name = match given {
        Some(name) => name.to_string(),
        None => {
            let names: BTreeSet<&str> = s.doc.cam.tools.iter().filter(|t| Some(t.id) != except).map(|t| t.name.as_str()).collect();
            let number = (1..=s.doc.cam.tools.len().saturating_add(1)).find(|n| !names.contains(format!("Tool {n}").as_str())).unwrap_or(1);
            format!("Tool {number}")
        }
    };
    if name.chars().count() > 128 {
        return Err(bad(cmd, "`name` must have 1–128 characters"));
    }
    if s.doc.cam.tools.iter().any(|t| Some(t.id) != except && t.name == name) {
        return Err(bad(cmd, format!("a tool named `{name}` already exists")));
    }
    Ok(name)
}

/// The corner radius a kind uses when one is not given.
fn default_corner_radius(kind: ToolKind, diameter: f64) -> f64 {
    match kind {
        ToolKind::BallEndMill => diameter / 2.0,
        ToolKind::FlatEndMill | ToolKind::BullNoseEndMill => 0.0,
    }
}

/// Check what a tool's fields must be together: its diameter, corner radius, flute length and
/// cutting edges.
fn check_cutting(cmd: &str, tool: &Tool) -> Result<()> {
    if !(tool.diameter.is_finite() && tool.diameter > 0.0 && tool.diameter <= 1000.0) {
        return Err(bad(cmd, format!("`diameter` {DIAMETER_PROBLEM}")));
    }
    if !(tool.corner_radius.is_finite() && (0.0..=1000.0).contains(&tool.corner_radius)) {
        return Err(bad(cmd, format!("`corner_radius` {CORNER_RADIUS_PROBLEM}")));
    }
    match tool.kind {
        ToolKind::FlatEndMill => {
            if tool.corner_radius != 0.0 {
                return Err(bad(cmd, "`corner_radius` must be 0 for a flat end mill"));
            }
        }
        ToolKind::BallEndMill => {
            if (tool.corner_radius - tool.diameter / 2.0).abs() > 1e-9 {
                return Err(bad(cmd, "`corner_radius` must be half the diameter for a ball end mill"));
            }
        }
        ToolKind::BullNoseEndMill => {
            if tool.corner_radius * 2.0 > tool.diameter + 1e-9 {
                return Err(bad(cmd, "`corner_radius` must be at most half the diameter"));
            }
        }
    }
    if !(tool.flute_length.is_finite() && (0.0..=1000.0).contains(&tool.flute_length)) {
        return Err(bad(cmd, format!("`flute_length` {FLUTE_LENGTH_PROBLEM}")));
    }
    if tool.flutes > 100 {
        return Err(bad(cmd, format!("`flutes` {FLUTES_PROBLEM}")));
    }
    Ok(())
}

/// `kind` of the call; `None` when left out or `null`.
fn kind(p: &Value, cmd: &str) -> Result<Option<ToolKind>> {
    match p.get("kind") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(kind)) => match kind.as_str() {
            "flat_end_mill" => Ok(Some(ToolKind::FlatEndMill)),
            "ball_end_mill" => Ok(Some(ToolKind::BallEndMill)),
            "bull_nose_end_mill" => Ok(Some(ToolKind::BullNoseEndMill)),
            _ => Err(bad(cmd, format!("`kind` {KIND_PROBLEM}"))),
        },
        Some(_) => Err(bad(cmd, format!("`kind` {KIND_PROBLEM}"))),
    }
}

/// `diameter` of the call. `required` is `true` for a create, where it must be there.
fn diameter(p: &Value, cmd: &str, required: bool) -> Result<Option<f64>> {
    match p.get("diameter") {
        None | Some(Value::Null) if !required => Ok(None),
        _ => {
            let d = num(p, "diameter").filter(|d| *d > 0.0 && *d <= 1000.0).ok_or_else(|| bad(cmd, format!("`diameter` {DIAMETER_PROBLEM}")))?;
            Ok(Some(d))
        }
    }
}

/// `corner_radius` of the call; `None` when left out or `null`.
fn corner_radius(p: &Value, cmd: &str) -> Result<Option<f64>> {
    match p.get("corner_radius") {
        None | Some(Value::Null) => Ok(None),
        Some(_) => {
            let r = num(p, "corner_radius")
                .filter(|r| (0.0..=1000.0).contains(r))
                .ok_or_else(|| bad(cmd, format!("`corner_radius` {CORNER_RADIUS_PROBLEM}")))?;
            Ok(Some(r))
        }
    }
}

/// `flute_length` of the call; `None` when left out or `null`.
fn flute_length(p: &Value, cmd: &str) -> Result<Option<f64>> {
    match p.get("flute_length") {
        None | Some(Value::Null) => Ok(None),
        Some(_) => {
            let length = num(p, "flute_length")
                .filter(|l| (0.0..=1000.0).contains(l))
                .ok_or_else(|| bad(cmd, format!("`flute_length` {FLUTE_LENGTH_PROBLEM}")))?;
            Ok(Some(length))
        }
    }
}

/// `flutes` of the call; `None` when left out or `null`.
fn flutes(p: &Value, cmd: &str) -> Result<Option<u32>> {
    match p.get("flutes") {
        None | Some(Value::Null) => Ok(None),
        Some(_) => p
            .get("flutes")
            .and_then(Value::as_u64)
            .filter(|n| *n <= 100)
            .and_then(|n| u32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| bad(cmd, format!("`flutes` {FLUTES_PROBLEM}"))),
    }
}

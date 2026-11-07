//! CAM ▸ TOOLPATH: machining setups (`cam.setup.create`), the coordinate system a part is
//! machined from, the raw stock it is cut out of and the bodies it machines.
//!
//! A setup names data, not geometry: its coordinate system (0 is the model origin with the
//! model's own axes), its stock and the bodies it machines. Creating one is undoable like any
//! other design change, and a refused call changes nothing.

use std::collections::BTreeSet;

use serde_json::Value;
use solidcraft_doc::cam::{Setup, Stock};

use super::CommandSpec;
use crate::params::{bad, str_, string_list, vec3};
use crate::schema::TypedContract;
use crate::schema::cam_setup::{CreateSetupParams, STOCK_SIZE_PROBLEM, SetupCreated, StockReply};
use crate::schema::output_schema_for;
use crate::{Result, Session};

const CREATE_CONTRACT: TypedContract<CreateSetupParams> = TypedContract::new();

/// The most setups a design may hold.
const MAX_SETUPS: usize = 1000;

/// The longest edge a stock may have, in mm.
const MAX_STOCK: f64 = 1000.0;

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new("cam.setup.create", "Setup", crate::checked!("cam.setup.create", create))
    .at("TOOLPATH", "CREATE")
    .icon("cam")
    .doc_only()
    .contract(&CREATE_CONTRACT)
    .output(output_schema_for::<SetupCreated>)
    .params(
        "coordinate_system?: an existing coordinate system's id (default 0: the model origin and axes); stock_size?: [x,y,z] mm, each 0 (the machined bodies' extent) to 1000; stock_offset?: [x,y,z] mm (default 0,0,0); bodies?: [body names] (default empty: every solid body); name?: 1–128 characters (default: \"Setup\" and the first free number)",
    )];

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cam.setup.create";
    if s.doc.cam.setups.len() >= MAX_SETUPS {
        return Err(bad(cmd, "too many setups"));
    }
    let coordinate_system = p.get("coordinate_system").map_or(Ok(0), |v| {
        v.as_u64().ok_or_else(|| bad(cmd, "`coordinate_system` must be a coordinate system's id in this design"))
    })?;
    if coordinate_system != 0 && !s.doc.coordinate_systems.iter().any(|c| c.id == coordinate_system) {
        return Err(bad(cmd, format!("there is no coordinate system {coordinate_system}")));
    }
    let stock = Stock { size: stock_size(p, cmd)?, offset: point(p, "stock_offset", cmd)? };
    let bodies = bodies(p, cmd)?;
    let name = new_name(s, cmd, str_(p, "name"))?;
    let id = {
        let doc = s.doc_mut();
        let id = doc.allocate_id()?;
        doc.cam.setups.push(Setup { id, name: name.clone(), coordinate_system, stock: stock.clone(), bodies: bodies.clone() });
        id
    };
    let count = s.doc.cam.setups.len();
    serde_json::to_value(SetupCreated {
        setup: id,
        name,
        coordinate_system,
        stock: StockReply { size: stock.size, offset: stock.offset },
        bodies,
        count,
        param_warnings: Vec::new(),
    })
    .map_err(|e| bad(cmd, e.to_string()))
}

/// `stock_size`, checked to be three finite non-negative lengths no longer than [`MAX_STOCK`].
fn stock_size(p: &Value, cmd: &str) -> Result<[f64; 3]> {
    let size = point(p, "stock_size", cmd)?;
    if size.iter().any(|v| !(0.0..=MAX_STOCK).contains(v)) {
        return Err(bad(cmd, format!("`stock_size` {STOCK_SIZE_PROBLEM}")));
    }
    Ok(size)
}

/// A point of the call, or [0, 0, 0] when it is absent. A key of the wrong shape is refused
/// rather than taken as absent.
fn point(p: &Value, k: &str, cmd: &str) -> Result<[f64; 3]> {
    match p.get(k) {
        None | Some(Value::Null) => Ok([0.0; 3]),
        Some(v) => vec3(v).map(|q| [q.x, q.y, q.z]).ok_or_else(|| bad(cmd, format!("`{k}` must be a point [x, y, z]"))),
    }
}

/// The bodies the setup machines, by name: trimmed and without repeats. An empty list means
/// every solid body.
fn bodies(p: &Value, cmd: &str) -> Result<Vec<String>> {
    let wanted = string_list(cmd, p, "bodies")?;
    let mut seen = BTreeSet::new();
    let mut names = Vec::with_capacity(wanted.len());
    for name in wanted {
        let name = name.trim();
        if name.is_empty() {
            return Err(bad(cmd, "`bodies` must name bodies, not blank names"));
        }
        if name.chars().count() > 128 {
            return Err(bad(cmd, "a body name in `bodies` must have 1–128 characters"));
        }
        if !seen.insert(name.to_string()) {
            continue;
        }
        names.push(name.to_string());
    }
    Ok(names)
}

/// A new setup name: 1–128 trimmed characters, not another setup's. A blank or absent name
/// takes "Setup" and the first number no setup's name has.
fn new_name(s: &Session, cmd: &str, given: Option<&str>) -> Result<String> {
    let given = given.map(str::trim).filter(|n| !n.is_empty());
    let name = match given {
        Some(name) => name.to_string(),
        None => {
            let names: BTreeSet<&str> = s.doc.cam.setups.iter().map(|x| x.name.as_str()).collect();
            let number = (1..=s.doc.cam.setups.len().saturating_add(1)).find(|n| !names.contains(format!("Setup {n}").as_str())).unwrap_or(1);
            format!("Setup {number}")
        }
    };
    if name.chars().count() > 128 {
        return Err(bad(cmd, "`name` must have 1–128 characters"));
    }
    if s.doc.cam.setups.iter().any(|x| x.name == name) {
        return Err(bad(cmd, format!("a setup named `{name}` already exists")));
    }
    Ok(name)
}

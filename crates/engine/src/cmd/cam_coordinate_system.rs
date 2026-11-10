//! CAM ▸ TOOLPATH: coordinate systems, the machining origin and axes a toolpath works from.
//!
//! `origin` and the three axes are points in model space (mm); a direction may be given at any
//! length and is stored as a unit vector. The axes must be square to each other, because they are
//! the machine's own X, Y and Z.

use serde_json::Value;
use solidcraft_doc::cam::CoordinateSystem;

use super::CommandSpec;
use crate::params::{bad, str_, vec3};
use crate::schema::TypedContract;
use crate::schema::cam_coordinate_system::{
    CoordinateSystemCreated, CoordinateSystemDeleted, CoordinateSystemEdited, CreateCoordinateSystemParams, DeleteCoordinateSystemParams,
    EditCoordinateSystemParams,
};
use crate::schema::output_schema_for;
use crate::{Result, Session};

const CREATE_CONTRACT: TypedContract<CreateCoordinateSystemParams> = TypedContract::new();
const EDIT_CONTRACT: TypedContract<EditCoordinateSystemParams> = TypedContract::new();
const DELETE_CONTRACT: TypedContract<DeleteCoordinateSystemParams> = TypedContract::new();

/// The most coordinate systems a design may hold.
const MAX_COORDINATE_SYSTEMS: usize = 1000;

/// How far two axes may stray from square, as the cosine of the angle between them.
const SQUARE: f64 = 1e-6;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cam.coordinate_system.create", "Coordinate System", crate::checked!("cam.coordinate_system.create", create))
        .at("TOOLPATH", "CREATE")
        .icon("origin")
        .contract(&CREATE_CONTRACT)
        .output(output_schema_for::<CoordinateSystemCreated>)
        .params("origin?: [x,y,z] mm (default 0,0,0); x_axis?, z_axis?: directions (default the model's, stored as unit vectors); y_axis?: a direction (default z_axis × x_axis); name?")
        .doc_only(),
    CommandSpec::new("cam.coordinate_system.edit", "Edit Coordinate System", crate::checked!("cam.coordinate_system.edit", edit))
        .icon("origin")
        .contract(&EDIT_CONTRACT)
        .output(output_schema_for::<CoordinateSystemEdited>)
        .params(
            "coordinate_system: an existing coordinate system's id; name?, origin?, x_axis?, y_axis?, z_axis?: as cam.coordinate_system.create. A key left out keeps what it has; a y_axis left out is computed as z_axis × x_axis",
        )
        .doc_only(),
    CommandSpec::new("cam.coordinate_system.delete", "Delete Coordinate System", crate::checked!("cam.coordinate_system.delete", delete))
        .icon("delete")
        .contract(&DELETE_CONTRACT)
        .output(output_schema_for::<CoordinateSystemDeleted>)
        .params("coordinate_system: the id of a coordinate system no machining setup works from")
        .doc_only(),
];

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cam.coordinate_system.create";
    let origin = point(p, "origin", cmd, [0.0; 3])?;
    let x = unit(p, "x_axis", cmd, [1.0, 0.0, 0.0])?;
    let z = unit(p, "z_axis", cmd, [0.0, 0.0, 1.0])?;
    // Y is the frame's third direction: given, it must be square to the other two; left out, it
    // is computed from them (Z × X), so a call that names only X and Z always gets a square triad.
    let y = match p.get("y_axis") {
        None | Some(Value::Null) => square_third(z, x).ok_or_else(|| bad(cmd, "x_axis and z_axis must be square to each other"))?,
        Some(_) => unit(p, "y_axis", cmd, [0.0, 1.0, 0.0])?,
    };
    check_square(cmd, x, y, z)?;
    let doc = s.doc_mut();
    if doc.coordinate_systems.len() >= MAX_COORDINATE_SYSTEMS {
        return Err(bad(cmd, "too many coordinate systems"));
    }
    let name = match str_(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) if n.chars().count() <= 128 => n.to_string(),
        Some(_) => return Err(bad(cmd, "`name` is too long")),
        None => {
            let names: std::collections::BTreeSet<_> = doc.coordinate_systems.iter().map(|c| c.name.as_str()).collect();
            let number = (1..=doc.coordinate_systems.len().saturating_add(1))
                .find(|n| !names.contains(format!("Coordinate System {n}").as_str()))
                .unwrap_or(1);
            format!("Coordinate System {number}")
        }
    };
    if doc.coordinate_systems.iter().any(|c| c.name == name) {
        return Err(bad(cmd, format!("a coordinate system named `{name}` already exists")));
    }
    let id = doc.allocate_id()?;
    doc.coordinate_systems.push(CoordinateSystem { id, name: name.clone(), origin, x_axis: x, y_axis: y, z_axis: z });
    let count = doc.coordinate_systems.len();
    serde_json::to_value(CoordinateSystemCreated {
        coordinate_system: id,
        name,
        origin,
        x_axis: x,
        y_axis: y,
        z_axis: z,
        count,
        param_warnings: Vec::new(),
    })
    .map_err(|e| bad(cmd, e.to_string()))
}

/// Change a coordinate system's name, origin or axes. A key left out keeps what it has, so a
/// rename never moves the frame and a move never renames it. A refused call changes nothing.
fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cam.coordinate_system.edit";
    let id = p
        .get("coordinate_system")
        .and_then(Value::as_u64)
        .ok_or_else(|| bad(cmd, "`coordinate_system` must be a coordinate system's id in this design"))?;
    let index = s.doc.coordinate_systems.iter().position(|c| c.id == id).ok_or_else(|| bad(cmd, format!("there is no coordinate system {id}")))?;
    let current = s.doc.coordinate_systems[index].clone();
    let origin = point(p, "origin", cmd, current.origin)?;
    let x = unit(p, "x_axis", cmd, current.x_axis)?;
    let z = unit(p, "z_axis", cmd, current.z_axis)?;
    // As on create: a y_axis left out is the third direction of the X and Z the call leaves, so
    // the stored triad stays square.
    let y = match p.get("y_axis") {
        None | Some(Value::Null) => square_third(z, x).ok_or_else(|| bad(cmd, "x_axis and z_axis must be square to each other"))?,
        Some(_) => unit(p, "y_axis", cmd, current.y_axis)?,
    };
    check_square(cmd, x, y, z)?;
    let name = match str_(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) if n.chars().count() <= 128 => n.to_string(),
        Some(_) => return Err(bad(cmd, "`name` is too long")),
        None => current.name.clone(),
    };
    if s.doc.coordinate_systems.iter().any(|c| c.id != id && c.name == name) {
        return Err(bad(cmd, format!("a coordinate system named `{name}` already exists")));
    }
    let count = {
        let doc = s.doc_mut();
        doc.coordinate_systems[index] = CoordinateSystem { id, name: name.clone(), origin, x_axis: x, y_axis: y, z_axis: z };
        doc.coordinate_systems.len()
    };
    serde_json::to_value(CoordinateSystemEdited {
        coordinate_system: id,
        name,
        origin,
        x_axis: x,
        y_axis: y,
        z_axis: z,
        count,
        param_warnings: Vec::new(),
    })
    .map_err(|e| bad(cmd, e.to_string()))
}

/// Delete a coordinate system. One a machining setup works from stays: the setup would be left
/// without a machining origin, so it is refused and says which setup holds it.
fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cam.coordinate_system.delete";
    let id = p
        .get("coordinate_system")
        .and_then(Value::as_u64)
        .ok_or_else(|| bad(cmd, "`coordinate_system` must be a coordinate system's id in this design"))?;
    let index = s.doc.coordinate_systems.iter().position(|c| c.id == id).ok_or_else(|| bad(cmd, format!("there is no coordinate system {id}")))?;
    if let Some(setup) = s.doc.cam.setups.iter().find(|setup| setup.coordinate_system == id) {
        return Err(bad(cmd, format!("the setup `{}` works from this coordinate system", setup.name)));
    }
    let (name, count, visibility_changed) = {
        let doc = s.doc_mut();
        let removed = doc.coordinate_systems.remove(index);
        (removed.name, doc.coordinate_systems.len(), removed.id)
    };
    // A deleted coordinate system cannot stay in the browser's hidden list.
    s.visibility.hidden_coordinate_systems.retain(|hidden| *hidden != visibility_changed);
    serde_json::to_value(CoordinateSystemDeleted { coordinate_system: id, name, count, param_warnings: Vec::new() })
        .map_err(|e| bad(cmd, e.to_string()))
}

/// The three directions must be square to each other: they are the machine's own X, Y and Z.
fn check_square(cmd: &str, x: [f64; 3], y: [f64; 3], z: [f64; 3]) -> Result<()> {
    for (a, b, name) in [(x, y, "x_axis and y_axis"), (y, z, "y_axis and z_axis"), (z, x, "z_axis and x_axis")] {
        if dot(a, b).abs() > SQUARE {
            return Err(bad(cmd, format!("{name} must be square to each other")));
        }
    }
    Ok(())
}

/// A point of the call, or `default` when it is absent. A key of the wrong shape is refused
/// rather than taken as absent.
fn point(p: &Value, k: &str, cmd: &str, default: [f64; 3]) -> Result<[f64; 3]> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => vec3(v).map(|q| [q.x, q.y, q.z]).ok_or_else(|| bad(cmd, format!("`{k}` must be a point [x, y, z]"))),
    }
}

/// A direction of the call as a unit vector, or `default` when it is absent.
fn unit(p: &Value, k: &str, cmd: &str, default: [f64; 3]) -> Result<[f64; 3]> {
    let v = point(p, k, cmd, default)?;
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if !(length > 1e-9) || !length.is_finite() {
        return Err(bad(cmd, format!("`{k}` must be a direction, not zero")));
    }
    Ok([v[0] / length, v[1] / length, v[2] / length])
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The unit direction square to both `a` and `b`: `a × b`, or `None` where they are parallel (or
/// one is not a finite number), so there is no third direction.
fn square_third(a: [f64; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let c = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let length = dot(c, c).sqrt();
    (length > 1e-9 && length.is_finite()).then(|| [c[0] / length, c[1] / length, c[2] / length])
}

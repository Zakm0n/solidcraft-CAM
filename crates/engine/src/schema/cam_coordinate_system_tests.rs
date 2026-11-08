use serde_json::json;

use super::{CoordinateSystemCreated, CreateCoordinateSystemParams};
use crate::Session;
use crate::schema::tests::{assert_keys, assert_strict_keeps_todays_errors, conforms};
use crate::schema::{ParamCheck, input_schema_for, output_schema_for};

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<CreateCoordinateSystemParams>(), &["name", "origin", "x_axis", "y_axis", "z_axis"]);
}

#[test]
fn strict_keeps_todays_errors() {
    assert_strict_keeps_todays_errors(&[
        ("cam.coordinate_system.create", json!({"x_axis": [0, 0, 0]})),
        ("cam.coordinate_system.create", json!({"x_axis": [1, 0, 0], "y_axis": [1, 0, 0]})),
    ]);
}

/// A coordinate system keeps its origin and a normalized triad in the design, and the reply says
/// what was stored.
#[test]
fn create_stores_a_unit_triad_and_its_reply_matches_the_output_schema() {
    let mut s = Session { param_check: ParamCheck::Warn, ..Session::default() };
    let reply = s
        .execute("cam.coordinate_system.create", &json!({"origin": [10, 20, 30], "x_axis": [0, 2, 0], "y_axis": [-4, 0, 0], "name": "Stock Top"}))
        .unwrap();
    assert!(reply.get("param_warnings").is_none(), "{reply}");
    let stored = &s.doc.coordinate_systems[0];
    assert_eq!(stored.name, "Stock Top");
    assert_eq!(stored.origin, [10.0, 20.0, 30.0]);
    assert_eq!(stored.x_axis, [0.0, 1.0, 0.0], "a direction is stored as a unit vector");
    assert_eq!(stored.y_axis, [-1.0, 0.0, 0.0]);
    assert_eq!(stored.z_axis, [0.0, 0.0, 1.0], "an axis left out is the model's");
    assert_eq!(reply["name"], "Stock Top");
    assert_eq!(reply["count"], 1);
    let schema = output_schema_for::<CoordinateSystemCreated>();
    let schema = schema.as_value();
    assert!(conforms(schema, schema, &reply), "{reply} doesn't match {schema}");

    // The next one without a name takes the first free number, and ids are never reused.
    let second = s.execute("cam.coordinate_system.create", &json!({})).unwrap();
    assert_eq!(second["name"], "Coordinate System 1");
    assert_ne!(second["coordinate_system"], reply["coordinate_system"]);
    assert_eq!(s.doc.coordinate_systems.len(), 2);
}

#[test]
fn a_missing_y_axis_is_computed_from_z_cross_x() {
    let mut s = Session::default();
    let reply = s.execute("cam.coordinate_system.create", &json!({"x_axis": [0, 1, 0], "z_axis": [0, 0, 1]})).unwrap();
    assert_eq!(reply["y_axis"], json!([-1.0, 0.0, 0.0]), "Y is the direction square to X and Z");
    // Parallel X and Z leave no third direction.
    let refused = s.execute("cam.coordinate_system.create", &json!({"x_axis": [0, 0, 1], "z_axis": [0, 0, 2]})).unwrap_err().to_string();
    assert!(refused.contains("square"), "{refused}");
    assert_eq!(s.doc.coordinate_systems.len(), 1);
}

#[test]
fn create_refuses_axes_that_are_not_square_and_repeated_names() {
    let mut s = Session::default();
    let refused = s.execute("cam.coordinate_system.create", &json!({"x_axis": [1, 0, 0], "y_axis": [1, 0, 0]})).unwrap_err().to_string();
    assert!(refused.contains("square"), "{refused}");
    assert!(s.doc.coordinate_systems.is_empty(), "a refused call changes nothing");

    s.execute("cam.coordinate_system.create", &json!({"name": "Top"})).unwrap();
    let repeated = s.execute("cam.coordinate_system.create", &json!({"name": "Top"})).unwrap_err().to_string();
    assert!(repeated.contains("already exists"), "{repeated}");
    assert_eq!(s.doc.coordinate_systems.len(), 1);
}

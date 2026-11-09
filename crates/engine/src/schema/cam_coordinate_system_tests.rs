use serde_json::json;

use super::{CoordinateSystemCreated, CoordinateSystemDeleted, CoordinateSystemEdited, CreateCoordinateSystemParams, EditCoordinateSystemParams};
use crate::Session;
use crate::schema::tests::{assert_keys, assert_strict_keeps_todays_errors, conforms};
use crate::schema::{ParamCheck, input_schema_for, output_schema_for};
use solidcore_record::{Vec2, Vec3};
use solidcraft_geom::prelude::*;

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<CreateCoordinateSystemParams>(), &["name", "origin", "x_axis", "y_axis", "z_axis"]);
    assert_keys(input_schema_for::<EditCoordinateSystemParams>(), &["coordinate_system", "name", "origin", "x_axis", "y_axis", "z_axis"]);
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

/// Editing a coordinate system changes only what the call names: a rename leaves the frame where
/// it is, and a move leaves the name alone.
#[test]
fn edit_changes_the_keys_it_names_and_keeps_the_rest() {
    let mut s = Session::default();
    let created = s.execute("cam.coordinate_system.create", &json!({"origin": [1, 2, 3], "name": "Top"})).unwrap();
    let id = created["coordinate_system"].as_u64().unwrap();
    let renamed = s.execute("cam.coordinate_system.edit", &json!({"coordinate_system": id, "name": "Stock Top"})).unwrap();
    assert_eq!(renamed["name"], "Stock Top");
    assert_eq!(renamed["origin"], json!([1.0, 2.0, 3.0]), "a rename leaves the frame where it is");
    assert_eq!(s.doc.coordinate_systems[0].name, "Stock Top");

    let moved = s.execute("cam.coordinate_system.edit", &json!({"coordinate_system": id, "origin": [10, 0, 0], "x_axis": [0, 1, 0]})).unwrap();
    assert_eq!(moved["name"], "Stock Top");
    assert_eq!(s.doc.coordinate_systems[0].origin, [10.0, 0.0, 0.0]);
    assert_eq!(s.doc.coordinate_systems[0].x_axis, [0.0, 1.0, 0.0], "a direction is stored as a unit vector");
    assert_eq!(s.doc.coordinate_systems[0].y_axis, [-1.0, 0.0, 0.0], "Y follows the Z and X the call leaves");
    let schema = output_schema_for::<CoordinateSystemEdited>();
    let schema = schema.as_value();
    assert!(conforms(schema, schema, &moved), "{moved} doesn't match {schema}");
}

#[test]
fn edit_refuses_a_repeated_name_or_skewed_axes_and_changes_nothing() {
    let mut s = Session::default();
    let first = s.execute("cam.coordinate_system.create", &json!({"name": "Top"})).unwrap()["coordinate_system"].as_u64().unwrap();
    let second = s.execute("cam.coordinate_system.create", &json!({"name": "Side"})).unwrap()["coordinate_system"].as_u64().unwrap();
    let before = s.doc.coordinate_systems.clone();
    let repeated = s.execute("cam.coordinate_system.edit", &json!({"coordinate_system": second, "name": "Top"})).unwrap_err().to_string();
    assert!(repeated.contains("already exists"), "{repeated}");
    let skewed = s
        .execute("cam.coordinate_system.edit", &json!({"coordinate_system": first, "x_axis": [1, 0, 0], "y_axis": [1, 0, 0]}))
        .unwrap_err()
        .to_string();
    assert!(skewed.contains("square"), "{skewed}");
    assert_eq!(s.doc.coordinate_systems, before, "a refused call changes nothing");
    // Its own name is not another's: renaming to the same name is fine.
    s.execute("cam.coordinate_system.edit", &json!({"coordinate_system": first, "name": "Top"})).unwrap();
}

/// A coordinate system a setup works from is not deleted: the setup would be left without a
/// machining origin. Deleting any other takes it out of the design, and out of the hidden list.
#[test]
fn delete_refuses_one_a_setup_holds_and_removes_the_rest() {
    let mut s = Session::default();
    let held = s.execute("cam.coordinate_system.create", &json!({"name": "Held"})).unwrap()["coordinate_system"].as_u64().unwrap();
    let spare = s.execute("cam.coordinate_system.create", &json!({"name": "Spare"})).unwrap()["coordinate_system"].as_u64().unwrap();
    s.execute("cam.setup.create", &json!({"coordinate_system": held})).unwrap();
    let refused = s.execute("cam.coordinate_system.delete", &json!({"coordinate_system": held})).unwrap_err().to_string();
    assert!(refused.contains("Setup 1") || refused.contains("works from"), "{refused}");
    assert_eq!(s.doc.coordinate_systems.len(), 2);

    s.execute("browser.visibility", &json!({"items": [spare], "visible": false})).unwrap();
    let reply = s.execute("cam.coordinate_system.delete", &json!({"coordinate_system": spare})).unwrap();
    assert_eq!(reply["name"], "Spare");
    assert_eq!(reply["count"], 1);
    assert_eq!(s.doc.coordinate_systems.len(), 1);
    assert!(s.visibility.hidden_coordinate_systems.is_empty(), "the deleted coordinate system is no longer hidden");
    let schema = output_schema_for::<CoordinateSystemDeleted>();
    let schema = schema.as_value();
    assert!(conforms(schema, schema, &reply), "{reply} doesn't match {schema}");
}

/// The browser hides and shows a coordinate system's triad by id, as it does a joint's glyph.
#[test]
fn visibility_hides_and_shows_a_coordinate_system() {
    let mut s = Session::default();
    let id = s.execute("cam.coordinate_system.create", &json!({"name": "Top"})).unwrap()["coordinate_system"].as_u64().unwrap();
    s.execute("browser.visibility", &json!({"items": [id], "visible": false})).unwrap();
    assert_eq!(s.visibility.hidden_coordinate_systems, vec![id]);
    s.execute("browser.visibility", &json!({"items": ["Top"], "visible": true})).unwrap();
    assert!(s.visibility.hidden_coordinate_systems.is_empty(), "a name finds the coordinate system too");
}

/// #Z-CAM: the browser's Create Sketch names the plane by the coordinate system's id, as a
/// string; `sketch.create` reads it as the frame's own XY plane, origin and axes included.
#[test]
fn a_sketch_on_a_coordinate_system_keeps_the_frames_origin_and_axes() {
    let mut s = Session::default();
    let id = s
        .execute("cam.coordinate_system.create", &json!({"origin": [10, 20, 30], "x_axis": [0, 1, 0], "z_axis": [1, 0, 0], "name": "Stock"}))
        .unwrap()["coordinate_system"]
        .as_u64()
        .unwrap();
    let sketch = s.execute("sketch.create", &json!({"plane": id.to_string()})).unwrap()["sketch"].as_u64().unwrap();
    let plane = s.model.state().sketch(sketch).expect("an evaluated sketch").plane;
    let close = |a: Vec3, b: Vec3| a.dist(b) < 1e-9;
    assert!(close(plane.to_world(Vec2::new(0.0, 0.0)), Vec3::new(10.0, 20.0, 30.0)), "{plane:?}");
    assert!(close(plane.dir_to_world(Vec2::new(1.0, 0.0)), Vec3::Y), "the sketch x runs along the frame's X");
    assert!(close(plane.dir_to_world(Vec2::new(0.0, 1.0)), Vec3::Z), "the sketch y runs along the frame's Y");
}

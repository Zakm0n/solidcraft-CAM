//! The tool library commands: their contracts, what they store and what they refuse.

use serde_json::json;

use super::{CreateToolParams, EditToolParams, ListToolsParams, ToolReply, ToolsListed};
use crate::Session;
use crate::schema::tests::{assert_keys, assert_strict_keeps_todays_errors, conforms};
use crate::schema::{ParamCheck, input_schema_for, output_schema_for};

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<CreateToolParams>(), &["corner_radius", "diameter", "flute_length", "flutes", "kind", "name"]);
    assert_keys(input_schema_for::<EditToolParams>(), &["corner_radius", "diameter", "flute_length", "flutes", "kind", "name", "tool"]);
    assert_keys(input_schema_for::<ListToolsParams>(), &[]);
}

#[test]
fn strict_keeps_todays_errors() {
    assert_strict_keeps_todays_errors(&[
        ("cam.tool.create", json!({"diameter": 0})),
        ("cam.tool.create", json!({"diameter": 6, "kind": "laser"})),
        ("cam.tool.create", json!({"diameter": 6, "corner_radius": -1})),
        ("cam.tool.create", json!({"diameter": 6, "corner_radius": 1})),
        ("cam.tool.create", json!({"diameter": 6, "flute_length": 2000})),
        ("cam.tool.create", json!({"diameter": 6, "flutes": -1})),
        ("cam.tool.create", json!({"diameter": 6, "kind": "ball_end_mill", "corner_radius": 1})),
        ("cam.tool.edit", json!({"tool": 1})),
    ]);
}

/// A create stores what the call gave, fills in the rest and replies as its output schema says.
#[test]
fn create_stores_a_tool_and_its_reply_matches_the_output_schema() {
    let mut s = Session { param_check: ParamCheck::Warn, ..Session::default() };
    let reply = s.execute("cam.tool.create", &json!({"name": " 6 mm Flat ", "diameter": 6, "flute_length": 18, "flutes": 3})).unwrap();
    assert!(reply.get("param_warnings").is_none(), "{reply}");
    let stored = &s.doc.cam.tools[0];
    assert_eq!(stored.name, "6 mm Flat", "a name is stored trimmed");
    assert_eq!(stored.diameter, 6.0);
    assert_eq!(stored.corner_radius, 0.0, "a flat end mill has a square corner");
    assert_eq!(stored.flute_length, 18.0);
    assert_eq!(stored.flutes, 3);
    assert_eq!(reply["tool"]["id"], stored.id);
    let stored_id = stored.id;
    assert_eq!(reply["tool"]["kind"], "flat_end_mill");
    assert_eq!(reply["count"], 1);
    let schema = output_schema_for::<ToolReply>();
    let schema = schema.as_value();
    assert!(conforms(schema, schema, &reply), "{reply} doesn't match {schema}");

    // A name left out takes the first free number, and ids are never reused.
    let second = s.execute("cam.tool.create", &json!({"diameter": 3})).unwrap();
    assert_eq!(second["tool"]["name"], "Tool 1");
    assert_ne!(second["tool"]["id"], stored_id);
    assert_eq!(s.doc.cam.tools.len(), 2);
}

/// A ball-nose cutter's corner radius is half its diameter, and a bull-nose cutter keeps the
/// radius the call gives.
#[test]
fn a_kind_decides_the_default_corner_radius() {
    let mut s = Session::default();
    let ball = s.execute("cam.tool.create", &json!({"diameter": 6, "kind": "ball_end_mill"})).unwrap();
    assert_eq!(ball["tool"]["corner_radius"], 3.0);
    let bull = s.execute("cam.tool.create", &json!({"diameter": 8, "kind": "bull_nose_end_mill", "corner_radius": 1.5})).unwrap();
    assert_eq!(bull["tool"]["corner_radius"], 1.5);
    let flat = s.execute("cam.tool.create", &json!({"diameter": 4, "corner_radius": 0})).unwrap();
    assert_eq!(flat["tool"]["corner_radius"], 0.0);
}

/// An edit changes only the keys it gives; a kind or diameter change carries the corner radius.
#[test]
fn edit_changes_given_keys_and_carries_the_corner_radius() {
    let mut s = Session { param_check: ParamCheck::Warn, ..Session::default() };
    let created = s.execute("cam.tool.create", &json!({"name": "Rough", "diameter": 6, "flute_length": 20, "flutes": 2})).unwrap();
    let id = created["tool"]["id"].as_u64().unwrap();

    let renamed = s.execute("cam.tool.edit", &json!({"tool": id, "name": "Rough 2"})).unwrap();
    assert!(renamed.get("param_warnings").is_none(), "{renamed}");
    assert_eq!(renamed["tool"]["name"], "Rough 2");
    assert_eq!(renamed["tool"]["diameter"], 6.0, "a key left out keeps what the tool has");
    assert_eq!(renamed["tool"]["flute_length"], 20.0);
    let schema = output_schema_for::<ToolReply>();
    let schema = schema.as_value();
    assert!(conforms(schema, schema, &renamed), "{renamed} doesn't match {schema}");

    let ball = s.execute("cam.tool.edit", &json!({"tool": id, "kind": "ball_end_mill"})).unwrap();
    assert_eq!(ball["tool"]["corner_radius"], 3.0, "a ball-nose cutter's corner radius follows its diameter");
    let wider = s.execute("cam.tool.edit", &json!({"tool": id, "diameter": 10})).unwrap();
    assert_eq!(wider["tool"]["corner_radius"], 5.0, "a diameter change carries a ball-nose cutter's corner radius");
    let flat = s.execute("cam.tool.edit", &json!({"tool": id, "kind": "flat_end_mill"})).unwrap();
    assert_eq!(flat["tool"]["corner_radius"], 0.0);
    assert_eq!(s.doc.cam.tools[0].name, "Rough 2");
}

/// A list gives the tools in the order they were made and matches its output schema.
#[test]
fn list_gives_the_tools_in_order() {
    let mut s = Session { param_check: ParamCheck::Warn, ..Session::default() };
    let empty = s.execute("cam.tool.list", &json!({})).unwrap();
    assert_eq!(empty["count"], 0);
    assert_eq!(empty["tools"], json!([]));
    let schema = output_schema_for::<ToolsListed>();
    let schema = schema.as_value();
    assert!(conforms(schema, schema, &empty), "{empty} doesn't match {schema}");

    s.execute("cam.tool.create", &json!({"name": "First", "diameter": 6})).unwrap();
    s.execute("cam.tool.create", &json!({"name": "Second", "diameter": 3, "kind": "ball_end_mill"})).unwrap();
    let listed = s.execute("cam.tool.list", &json!({})).unwrap();
    assert!(listed.get("param_warnings").is_none(), "{listed}");
    assert_eq!(listed["count"], 2);
    assert_eq!(listed["tools"][0]["name"], "First");
    assert_eq!(listed["tools"][0]["kind"], "flat_end_mill");
    assert_eq!(listed["tools"][1]["name"], "Second");
    assert_eq!(listed["tools"][1]["corner_radius"], 1.5);
    assert!(conforms(schema, schema, &listed), "{listed} doesn't match {schema}");
}

#[test]
fn create_refuses_a_bad_diameter_a_bad_kind_and_a_corner_radius_the_kind_cannot_have() {
    let mut s = Session::default();
    for p in [
        json!({}),
        json!({"diameter": -1}),
        json!({"diameter": 1001}),
        json!({"diameter": 6, "kind": "laser"}),
        json!({"diameter": 6, "corner_radius": 1}),
        json!({"diameter": 6, "kind": "ball_end_mill", "corner_radius": 1}),
        json!({"diameter": 6, "kind": "bull_nose_end_mill", "corner_radius": 4}),
        json!({"diameter": 6, "flute_length": -1}),
        json!({"diameter": 6, "flutes": 101}),
        json!({"diameter": 6, "flutes": 2.5}),
    ] {
        assert!(s.execute("cam.tool.create", &p).is_err(), "{p} should be refused");
    }
    assert!(s.doc.cam.tools.is_empty(), "a refused call changes nothing");
}

#[test]
fn names_are_unique_and_a_missing_tool_is_refused() {
    let mut s = Session::default();
    s.execute("cam.tool.create", &json!({"name": "Rough", "diameter": 6})).unwrap();
    let repeated = s.execute("cam.tool.create", &json!({"name": "Rough", "diameter": 3})).unwrap_err().to_string();
    assert!(repeated.contains("already exists"), "{repeated}");
    let long = "x".repeat(129);
    let refused = s.execute("cam.tool.create", &json!({"name": long, "diameter": 6})).unwrap_err().to_string();
    assert!(refused.contains("128"), "{refused}");

    let missing = s.execute("cam.tool.edit", &json!({"tool": 99, "diameter": 3})).unwrap_err().to_string();
    assert!(missing.contains("there is no tool 99"), "{missing}");
    let id = s.doc.cam.tools[0].id;
    let same = s.execute("cam.tool.edit", &json!({"tool": id, "name": "Rough"})).unwrap();
    assert_eq!(same["tool"]["name"], "Rough", "a tool may keep its own name");
}

/// Editing a ball-nose cutter to a corner radius that is not half its diameter is refused, and
/// the tool keeps what it had.
#[test]
fn edit_refuses_a_corner_radius_its_kind_cannot_have() {
    let mut s = Session::default();
    let ball = s.execute("cam.tool.create", &json!({"diameter": 6, "kind": "ball_end_mill"})).unwrap();
    let id = ball["tool"]["id"].as_u64().unwrap();
    let refused = s.execute("cam.tool.edit", &json!({"tool": id, "corner_radius": 1})).unwrap_err().to_string();
    assert!(refused.contains("half the diameter"), "{refused}");
    assert_eq!(s.doc.cam.tools[0].corner_radius, 3.0);
}

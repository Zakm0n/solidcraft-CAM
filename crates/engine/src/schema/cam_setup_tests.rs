use serde_json::json;

use super::{CreateSetupParams, SetupCreated};
use crate::Session;
use crate::schema::tests::{assert_keys, assert_strict_keeps_todays_errors, conforms};
use crate::schema::{ParamCheck, input_schema_for, output_schema_for};

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<CreateSetupParams>(), &["bodies", "coordinate_system", "name", "stock_offset", "stock_size"]);
}

#[test]
fn strict_keeps_todays_errors() {
    assert_strict_keeps_todays_errors(&[
        ("cam.setup.create", json!({"coordinate_system": 99})),
        ("cam.setup.create", json!({"stock_size": [10, -1, 10]})),
    ]);
}

/// A setup keeps its coordinate system, stock and bodies, and the reply says what was stored.
#[test]
fn create_stores_a_setup_and_its_reply_matches_the_output_schema() {
    let mut s = Session { param_check: ParamCheck::Warn, ..Session::default() };
    let cs = s.execute("cam.coordinate_system.create", &json!({"name": "Top"})).unwrap();
    let cs = cs["coordinate_system"].as_u64().unwrap();
    let reply = s
        .execute(
            "cam.setup.create",
            &json!({
                "coordinate_system": cs,
                "stock_size": [100, 80, 20],
                "stock_offset": [5, 5, 0],
                "bodies": ["Plate", "Plate"],
                "name": "Roughing",
            }),
        )
        .unwrap();
    assert!(reply.get("param_warnings").is_none(), "{reply}");
    let stored = &s.doc.cam.setups[0];
    assert_eq!(stored.name, "Roughing");
    assert_eq!(stored.coordinate_system, cs);
    assert_eq!(stored.stock.size, [100.0, 80.0, 20.0]);
    assert_eq!(stored.stock.offset, [5.0, 5.0, 0.0]);
    assert_eq!(stored.bodies, ["Plate"], "repeated body names are stored once");
    assert_eq!(reply["name"], "Roughing");
    assert_eq!(reply["count"], 1);
    let schema = output_schema_for::<SetupCreated>();
    let schema = schema.as_value();
    assert!(conforms(schema, schema, &reply), "{reply} doesn't match {schema}");

    // The next one without a name takes the first free number, and ids are never reused.
    let second = s.execute("cam.setup.create", &json!({})).unwrap();
    assert_eq!(second["name"], "Setup 1");
    assert_ne!(second["setup"], reply["setup"]);
    assert_eq!(second["coordinate_system"], 0, "no coordinate system is the model origin");
    assert_eq!(s.doc.cam.setups.len(), 2);
}

#[test]
fn create_refuses_an_unknown_coordinate_system_bad_stock_and_repeated_names() {
    let mut s = Session::default();
    let refused = s.execute("cam.setup.create", &json!({"coordinate_system": 7})).unwrap_err().to_string();
    assert!(refused.contains("there is no coordinate system 7"), "{refused}");

    let bad = s.execute("cam.setup.create", &json!({"stock_size": [-1, 10, 10]})).unwrap_err().to_string();
    assert!(bad.contains("stock_size"), "{bad}");

    s.execute("cam.setup.create", &json!({"name": "Top"})).unwrap();
    let repeated = s.execute("cam.setup.create", &json!({"name": "Top"})).unwrap_err().to_string();
    assert!(repeated.contains("already exists"), "{repeated}");
    assert_eq!(s.doc.cam.setups.len(), 1, "a refused call changes nothing");
}

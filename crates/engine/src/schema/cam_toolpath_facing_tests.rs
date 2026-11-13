//! The 2D facing contract (`cam.toolpath.facing`): the keys it takes, the defaults it applies,
//! the values it refuses without the design, and the layout a design saves.

use serde_json::json;

use super::{CamToolpathFacingOutput, CamToolpathFacingParams, FacingStepover};
use crate::Session;
use crate::schema::tests::{assert_keys, conforms};
use crate::schema::{input_schema_for, output_schema_for};

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<CamToolpathFacingParams>(), &["depth", "name", "pass_extension", "setup", "stepover", "stock_offset", "tool"]);
}

/// A call that names a tool and a depth reads, with today's stepover, extension and offset.
#[test]
fn a_facing_call_defaults_its_stepover_extension_and_offset() {
    let p: CamToolpathFacingParams = serde_json::from_value(json!({"tool": 3, "depth": 5})).expect("a tool and a depth are enough");
    assert_eq!(p.tool, 3);
    assert_eq!(p.setup, None);
    assert_eq!(p.stepover, None);
    assert_eq!(p.pass_extension, None);
    assert_eq!(p.stock_offset, None);
    assert_eq!(p.depth.0.as_str(), "5");
    assert_eq!(p.name, None);
}

/// A stepover is a fraction of the tool's diameter from a tenth to one, both ends included.
#[test]
fn a_stepover_is_a_fraction_from_a_tenth_to_one() {
    for (value, accepted) in [(0.1, true), (0.35, true), (1.0, true), (0.09, false), (1.01, false), (-0.5, false)] {
        let read = serde_json::from_value::<FacingStepover>(json!(value));
        assert_eq!(read.is_ok(), accepted, "stepover {value}: {read:?}");
        if let Ok(stepover) = read {
            assert_eq!(stepover.0, value);
        }
    }
    for refused in [json!("0.5"), json!(true), json!([0.5]), json!(null)] {
        assert!(serde_json::from_value::<FacingStepover>(refused.clone()).is_err(), "{refused}");
    }
}

/// Hostile values fail on the value that is wrong, without a design.
#[test]
fn a_hostile_facing_call_is_refused() {
    for value in [
        json!({}),
        json!({"depth": 5}),
        json!({"tool": 3}),
        json!({"tool": "3", "depth": 5}),
        json!({"tool": 3, "depth": 5, "setup": "zero"}),
        json!({"tool": 3, "depth": 5, "stepover": 0}),
        json!({"tool": 3, "depth": 5, "stepover": 1.5}),
        json!({"tool": 3, "depth": 5, "stepover": "0.5"}),
        json!({"tool": 3, "depth": 5, "pass_extension": [1]}),
        json!({"tool": 3, "depth": 5, "stock_offset": {}}),
        json!({"tool": 3, "depth": null}),
    ] {
        assert!(serde_json::from_value::<CamToolpathFacingParams>(value.clone()).is_err(), "{value} read");
    }
}

/// What the command reports matches the schema it publishes, and what it stores reads back as the
/// design's own layout, key for key.
#[test]
fn a_facing_reply_and_its_stored_operation_are_stable() {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length": 60, "width": 40, "height": 10, "body_name": "Plate"})).unwrap();
    let tool = s.execute("cam.tool.create", &json!({"diameter": 10})).unwrap()["tool"]["id"].as_u64().unwrap();
    let reply =
        s.execute("cam.toolpath.facing", &json!({"tool": tool, "stepover": 0.5, "pass_extension": 1.5, "stock_offset": 2, "depth": 4})).unwrap();

    let schema = output_schema_for::<CamToolpathFacingOutput>();
    let schema = schema.as_value();
    assert!(conforms(schema, schema, &reply), "{reply} doesn't match its schema");

    let stored = solidcraft_doc::cam::CamFacing {
        id: 7,
        name: "2D Facing 1".into(),
        setup: 0,
        tool,
        stepover: 0.5,
        pass_extension: 1.5,
        stock_offset: 2.0,
        depth: 4.0,
    };
    let saved = serde_json::to_value(&stored).unwrap();
    assert_eq!(
        saved,
        json!({
            "id": 7,
            "name": "2D Facing 1",
            "setup": 0,
            "tool": tool,
            "stepover": 0.5,
            "pass_extension": 1.5,
            "stock_offset": 2.0,
            "depth": 4.0
        })
    );
    let read: solidcraft_doc::cam::CamFacing = serde_json::from_value(saved).unwrap();
    assert_eq!(read, stored);

    // The reply's summary is the stored operation, reported with the same keys.
    let facing = &reply["facing"];
    for key in ["id", "name", "setup", "tool", "stepover", "pass_extension", "stock_offset", "depth"] {
        assert!(facing.get(key).is_some(), "the summary has no `{key}`: {reply}");
    }
    assert_eq!(facing["setup"], 0);
    assert_eq!(facing["name"], stored.name);
    assert_eq!(s.doc.cam.facings[0].stepover, stored.stepover);
    assert_eq!(s.doc.cam.facings[0].pass_extension, stored.pass_extension);
    assert_eq!(s.doc.cam.facings[0].stock_offset, stored.stock_offset);
    assert_eq!(s.doc.cam.facings[0].depth, stored.depth);
}

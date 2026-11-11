//! The drilling contract (`cam.toolpath.drill`): the keys it takes, the default cycle and
//! reference, and the values it refuses without the design.

use serde_json::json;

use super::{CamToolpathDrillParams, DrillCycleParam, DrillDepthReferenceParam, HoleCenters, PeckDepth};
use crate::schema::tests::{assert_keys, conforms};
use crate::schema::{input_schema_for, output_schema_for};

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(
        input_schema_for::<CamToolpathDrillParams>(),
        &[
            "clearance_height",
            "cycle_type",
            "depth",
            "depth_reference",
            "feed_rate",
            "hole_centers",
            "name",
            "peck_depth",
            "plunge_rate",
            "setup",
            "spindle_speed",
            "tool",
        ],
    );
}

/// A call that names a tool and one hole center reads, with today's cycle and reference.
#[test]
fn a_drill_call_defaults_its_cycle_and_depth_reference() {
    let p: CamToolpathDrillParams =
        serde_json::from_value(json!({"tool": 3, "hole_centers": ["F1:vertex:1"]})).expect("a tool and one hole center are enough");
    assert_eq!(p.tool, 3);
    assert_eq!(p.hole_centers, HoleCenters(vec!["F1:vertex:1".into()]));
    assert_eq!(p.cycle_type, DrillCycleParam::Standard);
    assert_eq!(p.depth_reference, DrillDepthReferenceParam::HoleBottom);
    assert_eq!(p.clearance_height, None);
    assert_eq!(p.peck_depth, None);
    assert_eq!(p.depth, None);
    assert_eq!(p.setup, None);
}

/// Hostile values fail on the value that is wrong, with the wording the handler uses.
#[test]
fn a_hostile_drill_call_is_refused() {
    for value in [
        json!({}),
        json!({"tool": 3, "hole_centers": []}),
        json!({"tool": 3, "hole_centers": ["  "]}),
        json!({"tool": 3, "hole_centers": [""]}),
        json!({"tool": 3, "hole_centers": "F1:vertex:1"}),
        json!({"tool": 3, "hole_centers": [7]}),
        json!({"tool": 3, "hole_centers": ["F1:vertex:1"], "cycle_type": "spiral"}),
        json!({"tool": 3, "hole_centers": ["F1:vertex:1"], "depth_reference": "top"}),
        json!({"tool": 3, "hole_centers": ["F1:vertex:1"], "clearance_height": 1001}),
        json!({"tool": 3, "hole_centers": ["F1:vertex:1"], "peck_depth": -1}),
        json!({"tool": 3, "hole_centers": ["F1:vertex:1"], "depth": 0}),
        json!({"tool": 3, "hole_centers": ["F1:vertex:1"], "feed_rate": 1_000_001}),
        json!({"tool": "6", "hole_centers": ["F1:vertex:1"]}),
    ] {
        assert!(serde_json::from_value::<CamToolpathDrillParams>(value.clone()).is_err(), "{value} should be refused");
    }
    // The cycle and reference are the operands' spellings, and their types round-trip to the
    // document's.
    let p: CamToolpathDrillParams = serde_json::from_value(json!({
        "tool": 3,
        "hole_centers": ["F1:vertex:1"],
        "cycle_type": "deep_hole",
        "depth_reference": "absolute",
        "clearance_height": 5,
        "peck_depth": 2,
        "depth": 8
    }))
    .expect("a deep hole cycle to an absolute depth reads");
    assert_eq!(p.cycle_type, DrillCycleParam::DeepHole);
    assert_eq!(p.cycle_type.to_doc(), solidcraft_doc::cam::DrillCycle::DeepHole);
    assert_eq!(p.depth_reference, DrillDepthReferenceParam::Absolute);
    assert_eq!(p.depth_reference.to_doc(), solidcraft_doc::cam::DrillDepthReference::Absolute);
    assert_eq!(p.clearance_height.map(|c| c.0), Some(5.0));
    assert_eq!(p.peck_depth, Some(PeckDepth(2.0)));
    assert_eq!(p.depth.map(|d| d.0), Some(8.0));
}

/// The output schema accepts what the command reports: the stored operation and its metrics.
#[test]
fn the_output_schema_takes_a_drill_reply() {
    let schema = output_schema_for::<super::CamToolpathDrillOutput>();
    let schema = schema.as_value();
    let reply = json!({
        "drill": {
            "id": 9,
            "name": "Drilling 1",
            "setup": 0,
            "tool": 3,
            "hole_centers": ["F1:vertex:1"],
            "cycle_type": "peck",
            "depth_reference": "hole_bottom",
            "clearance_height": 5.0,
            "peck_depth": 2.0,
            "depth": 0.0
        },
        "count": 1,
        "holes": 1,
        "pecks": 2,
        "waypoints": [[0.0, 0.0, 5.0], [0.0, 0.0, -8.0]],
        "cutting_length": 8.0,
        "rapid_length": 13.0,
        "cycle_time": 1.5
    });
    assert!(conforms(schema, schema, &reply), "{reply} doesn't match {schema}");
    assert_eq!(reply["drill"]["cycle_type"], "peck", "the summary spells the cycle as the design saves it");
}

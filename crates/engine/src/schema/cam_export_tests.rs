//! The `cam.export.gcode` contract: the keys it takes and the schema they make.

use crate::schema::cam_export::{GcodeExport, GcodeParams};
use crate::schema::tests::{assert_keys, conforms};
use crate::schema::{ParamCheck, input_schema_for, output_schema_for};

use crate::Session;
use serde_json::json;

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<GcodeParams>(), &["path", "program", "safe_height", "setup", "toolpaths"]);
}

#[test]
fn strict_refuses_an_unknown_key() {
    let mut s = Session { param_check: ParamCheck::Strict, ..Session::default() };
    let refused = s.execute("cam.export.gcode", &json!({"tool_paths": []})).unwrap_err().to_string();
    assert!(refused.contains("tool_paths"), "{refused}");
}

/// A successful call's reply matches the schema the command publishes.
#[test]
fn the_reply_matches_its_output_schema() {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length": 40, "width": 20, "height": 10, "body_name": "Plate"})).unwrap();
    let tool = s.execute("cam.tool.create", &json!({"diameter": 6})).unwrap()["tool"]["id"].as_u64().unwrap();
    s.execute("cam.toolpath.contour", &json!({"tool": tool, "total_depth": 10, "feed_rate": 800})).unwrap();
    let reply = s.execute("cam.export.gcode", &json!({"program": "Test"})).unwrap();
    let schema = output_schema_for::<GcodeExport>();
    assert!(conforms(schema.as_value(), schema.as_value(), &reply), "{reply} doesn't match its schema");
}

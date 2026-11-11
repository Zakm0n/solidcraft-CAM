//! The contour and pocket toolpath commands: their contracts, the exact boundaries they read
//! from the B-rep, the passes they plan and what they refuse.

use serde_json::json;
use solidcraft_doc::cam::{Setup, Stock};

use super::{CamContour2dOutput, CamContour2dParams, ClearanceHeight, Depth, Pocket2DParams, Pocket2DResult, ToolpathParams, ToolpathReply};
use crate::Session;
use crate::schema::tests::{assert_keys, conforms};
use crate::schema::{ParamCheck, input_schema_for, output_schema_for};

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(
        input_schema_for::<ToolpathParams>(),
        &[
            "bodies",
            "depth_per_pass",
            "feed_rate",
            "name",
            "plunge_rate",
            "setup",
            "spindle_speed",
            "stepover",
            "stock_to_leave",
            "tool",
            "total_depth",
        ],
    );
}

/// A 40 x 20 x 10 plate on the model origin, named "Plate".
fn plate(s: &mut Session) {
    s.execute("solid.box", &json!({"length": 40, "width": 20, "height": 10, "body_name": "Plate"})).unwrap();
}

/// A 6 mm flat end mill, returning its id.
fn mill(s: &mut Session) -> u64 {
    s.execute("cam.tool.create", &json!({"diameter": 6})).unwrap()["tool"]["id"].as_u64().unwrap()
}

/// A contour reads the plate's exact outline and plans its passes.
#[test]
fn contour_reads_the_exact_outline_and_plans_the_passes() {
    let mut s = Session { param_check: ParamCheck::Warn, ..Session::default() };
    plate(&mut s);
    let tool = mill(&mut s);
    let reply = s
        .execute(
            "cam.toolpath.contour",
            &json!({"tool": tool, "total_depth": 10, "depth_per_pass": 4, "feed_rate": 800, "plunge_rate": 200, "spindle_speed": 12000}),
        )
        .unwrap();
    assert!(reply.get("param_warnings").is_none(), "{reply}");
    assert_eq!(reply["toolpath"]["kind"], "contour");
    assert_eq!(reply["toolpath"]["name"], "Contour 1");
    assert_eq!(reply["toolpath"]["tool"], tool);
    assert_eq!(reply["toolpath"]["bodies"], json!(["Plate"]));
    assert_eq!(reply["toolpath"]["feed_rate"], 800.0);
    assert_eq!(reply["toolpath"]["plunge_rate"], 200.0);
    assert_eq!(reply["toolpath"]["spindle_speed"], 12000.0);
    assert_eq!(reply["toolpath"]["stepover"], 3.0, "a stepover left out is half the tool's diameter");
    assert_eq!(reply["passes"], 3);
    assert_eq!(reply["cut_levels"], json!([6.0, 2.0, 0.0]), "the deepest pass cuts the bottom of the plate");
    assert_eq!(reply["count"], 1);
    let boundary = &reply["boundaries"][0];
    assert_eq!(boundary["body"], "Plate");
    assert_eq!(boundary["role"], "outer");
    assert!((boundary["length"].as_f64().unwrap() - 120.0).abs() < 1e-6, "{boundary}");
    assert_eq!(boundary["points"].as_array().map(Vec::len), Some(4), "a rectangle's outline is its four corners");
    let mut corners: Vec<(f64, f64)> = boundary["points"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p.as_array()?.first()?.as_f64()?, p.as_array()?.get(1)?.as_f64()?)))
        .collect();
    corners.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    assert_eq!(corners, vec![(0.0, 0.0), (0.0, 20.0), (40.0, 0.0), (40.0, 20.0)]);
    let stored = &s.doc.cam.toolpaths[0];
    assert_eq!(stored.kind, solidcraft_doc::cam::ToolpathKind::Contour);
    assert_eq!(stored.depth_per_pass, 4.0);
    assert_eq!(stored.total_depth, 10.0);
    let schema = output_schema_for::<ToolpathReply>();
    let schema = schema.as_value();
    assert!(conforms(schema, schema, &reply), "{reply} doesn't match {schema}");
}

/// A pocket reports the openings inside the outline too, as holes.
#[test]
fn pocket_reports_an_opening_as_a_hole() {
    let mut s = Session { param_check: ParamCheck::Warn, ..Session::default() };
    plate(&mut s);
    s.execute("solid.cylinder", &json!({"base": [20, 10, -1], "radius": 5, "height": 12, "operation": "cut", "targets": ["Plate"]})).unwrap();
    let tool = mill(&mut s);
    let reply = s.execute("cam.toolpath.pocket", &json!({"tool": tool, "total_depth": 10, "stepover": 4})).unwrap();
    assert!(reply.get("param_warnings").is_none(), "{reply}");
    assert_eq!(reply["toolpath"]["kind"], "pocket");
    assert_eq!(reply["toolpath"]["name"], "Pocket 1");
    assert_eq!(reply["toolpath"]["stepover"], 4.0);
    let boundaries = reply["boundaries"].as_array().unwrap();
    assert_eq!(boundaries.len(), 2, "the outline and the opening: {reply}");
    assert_eq!(boundaries[0]["role"], "outer");
    assert_eq!(boundaries[1]["role"], "hole");
    let hole_length = boundaries[1]["length"].as_f64().unwrap();
    assert!((hole_length - 2.0 * std::f64::consts::PI * 5.0).abs() < 0.01, "{hole_length}");
    let schema = output_schema_for::<ToolpathReply>();
    let schema = schema.as_value();
    assert!(conforms(schema, schema, &reply), "{reply} doesn't match {schema}");
}

/// A setup turns the boundary into its own machine coordinates.
#[test]
fn a_setup_frames_the_boundary_in_its_own_coordinates() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let cs = s
        .execute("cam.coordinate_system.create", &json!({"name": "Turned", "origin": [5, 0, 0], "x_axis": [0, 1, 0], "y_axis": [-1, 0, 0]}))
        .unwrap();
    let cs_id = cs["coordinate_system"].as_u64().unwrap();
    let setup = {
        let doc = s.doc_mut();
        let id = doc.allocate_id().unwrap();
        doc.cam.setups.push(Setup { id, name: "Setup 1".into(), coordinate_system: cs_id, stock: Stock::default(), bodies: vec!["Plate".into()] });
        id
    };
    let reply = s.execute("cam.toolpath.contour", &json!({"tool": tool, "setup": setup, "total_depth": 10})).unwrap();
    let points = reply["boundaries"][0]["points"].as_array().unwrap();
    let us: Vec<f64> = points.iter().filter_map(|p| p[0].as_f64()).collect();
    let vs: Vec<f64> = points.iter().filter_map(|p| p.get(1)?.as_f64()).collect();
    assert!(us.iter().all(|u| (0.0..=20.0).contains(u)), "u runs along the setup's X: {points:?}");
    assert!(vs.iter().all(|v| (-35.0..=5.0).contains(v)), "v runs along the setup's Y: {points:?}");
    assert!((us.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - 20.0).abs() < 1e-9);
    assert!((vs.iter().cloned().fold(f64::INFINITY, f64::min) + 35.0).abs() < 1e-9);
}

/// Two commands keep distinct default names and explicit names stay unique.
#[test]
fn names_are_unique_and_default_to_the_operation() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    s.execute("cam.toolpath.contour", &json!({"tool": tool})).unwrap();
    s.execute("cam.toolpath.contour", &json!({"tool": tool})).unwrap();
    let pocket = s.execute("cam.toolpath.pocket", &json!({"tool": tool})).unwrap();
    assert_eq!(pocket["toolpath"]["name"], "Pocket 1");
    let named = s.execute("cam.toolpath.contour", &json!({"tool": tool, "name": "  Rough outline  "})).unwrap();
    assert_eq!(named["toolpath"]["name"], "Rough outline", "a name is stored trimmed");
    let repeated = s.execute("cam.toolpath.contour", &json!({"tool": tool, "name": "Rough outline"})).unwrap_err().to_string();
    assert!(repeated.contains("already exists"), "{repeated}");
    let long = "x".repeat(129);
    let refused = s.execute("cam.toolpath.contour", &json!({"tool": tool, "name": long})).unwrap_err().to_string();
    assert!(refused.contains("128"), "{refused}");
}

#[test]
fn a_missing_tool_setup_or_body_is_refused() {
    let mut s = Session::default();
    plate(&mut s);
    let missing_tool = s.execute("cam.toolpath.contour", &json!({"tool": 99})).unwrap_err().to_string();
    assert!(missing_tool.contains("there is no tool 99"), "{missing_tool}");
    let no_tool = s.execute("cam.toolpath.contour", &json!({})).unwrap_err().to_string();
    assert!(no_tool.contains("`tool`"), "{no_tool}");
    let tool = mill(&mut s);
    let missing_setup = s.execute("cam.toolpath.contour", &json!({"tool": tool, "setup": 42})).unwrap_err().to_string();
    assert!(missing_setup.contains("there is no setup 42"), "{missing_setup}");
    let unknown = s.execute("cam.toolpath.contour", &json!({"tool": tool, "bodies": ["Nope"]})).unwrap_err().to_string();
    assert!(unknown.contains("unknown body name `Nope`"), "{unknown}");
    assert_eq!(s.doc.cam.toolpaths.len(), 0, "a refused call changes nothing");
}

#[test]
fn a_body_that_is_not_a_prism_is_refused() {
    let mut s = Session::default();
    s.execute("solid.sphere", &json!({"radius": 10, "body_name": "Ball"})).unwrap();
    let tool = mill(&mut s);
    let refused = s.execute("cam.toolpath.contour", &json!({"tool": tool})).unwrap_err().to_string();
    assert!(refused.contains("not a uniform prism"), "{refused}");
    assert!(s.doc.cam.toolpaths.is_empty());
}

#[test]
fn a_cut_that_leaves_the_part_is_refused() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let too_deep = s.execute("cam.toolpath.contour", &json!({"tool": tool, "total_depth": 12})).unwrap_err().to_string();
    assert!(too_deep.contains("deeper than the stock"), "{too_deep}");
    for depth in [0, -1, 1001] {
        assert!(s.execute("cam.toolpath.contour", &json!({"tool": tool, "total_depth": depth})).is_err(), "{depth}");
    }
    let wide = s.execute("cam.toolpath.pocket", &json!({"tool": tool, "stepover": 7})).unwrap_err().to_string();
    assert!(wide.contains("wider than the tool's diameter"), "{wide}");
    assert!(s.execute("cam.toolpath.contour", &json!({"tool": tool, "feed_rate": -1})).is_err());
    assert!(s.execute("cam.toolpath.contour", &json!({"tool": tool, "stock_to_leave": 1001})).is_err());
    assert!(s.doc.cam.toolpaths.is_empty(), "a refused call changes nothing");
}

/// The 2D contour's contract lists exactly the keys it takes, and its reply matches what it
/// says it returns.
#[test]
fn contour2d_schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<CamContour2dParams>(), &["bottom_depth", "clearance_height", "name", "setup", "stepdown", "target_edges", "tool"]);
    let schema = output_schema_for::<CamContour2dOutput>();
    let schema = schema.as_value();
    let reply = json!({
        "contour": {
            "id": 14,
            "name": "2D Contour 1",
            "setup": 0,
            "tool": 6,
            "target_edges": ["F1:side:l1|F1:top#1~1"],
            "clearance_height": 5.0,
            "bottom_depth": 8.0,
            "stepdown": 2.0
        },
        "count": 1,
        "passes": 4,
        "cut_levels": [6.0, 4.0, 2.0, 0.0],
        "waypoints": [[10.0, 0.0, 6.0], [20.0, 0.0, 6.0]],
        "length": 40.0
    });
    assert!(conforms(schema, schema, &reply), "{reply} doesn't match {schema}");
}

/// The 2D contour's contract reads edge picks and depths, and refuses what the engine could not
/// cut with.
#[test]
fn contour2d_params_take_edge_picks_and_refuse_hostile_values() {
    let params: CamContour2dParams = serde_json::from_value(json!({
        "tool": 6,
        "target_edges": [[10, 0, 4], {"body": "Plate", "index": 2}],
        "clearance_height": 5,
        "bottom_depth": 8,
        "stepdown": 2
    }))
    .expect("a call naming edge picks and depths is accepted");
    assert_eq!(params.target_edges.0.len(), 2);
    assert_eq!(params.clearance_height, Some(ClearanceHeight(5.0)));
    assert_eq!(params.bottom_depth, Depth(8.0));
    assert_eq!(params.stepdown, Depth(2.0));
    assert_eq!(params.name, None);
    for (key, value, why) in [
        ("stepdown", json!(0), "a stepdown of 0"),
        ("bottom_depth", json!(-1), "a negative depth"),
        ("clearance_height", json!(1001), "a clearance above 1000 mm"),
    ] {
        let mut call = json!({"tool": 6, "target_edges": [[0, 0, 0]], "bottom_depth": 8, "stepdown": 2});
        call.as_object_mut().expect("an object").insert(key.to_string(), value);
        assert!(serde_json::from_value::<CamContour2dParams>(call).is_err(), "{why} is refused");
    }
    let no_edges = json!({"tool": 6, "target_edges": [], "bottom_depth": 8, "stepdown": 2});
    assert!(serde_json::from_value::<CamContour2dParams>(no_edges).is_err(), "a call with no edges is refused");
}

/// The 2D pocket's contract lists exactly the keys it takes, and its reply matches what it says
/// it returns.
#[test]
fn pocket2d_schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<Pocket2DParams>(), &["boundary_faces", "depth", "depth_per_pass", "setup", "stepover", "tool"]);
    let schema = output_schema_for::<Pocket2DResult>();
    let schema = schema.as_value();
    let reply = json!({
        "boundaries": [
            {"body": "Plate", "role": "outer", "length": 140.0, "points": [[10.0, 5.0], [50.0, 5.0], [50.0, 35.0], [10.0, 35.0]]},
            {"body": "Plate", "role": "hole", "length": 25.1, "points": [[30.0, 16.0], [34.0, 20.0], [30.0, 24.0]]}
        ],
        "passes": 2,
        "cut_levels": [7.5, 5.0],
        "loops": 3,
        "waypoints": [[11.0, 6.0, 7.5], [49.0, 6.0, 7.5], [11.0, 6.0, 7.5]],
        "length": 36.0
    });
    assert!(conforms(schema, schema, &reply), "{reply} doesn't match {schema}");
}

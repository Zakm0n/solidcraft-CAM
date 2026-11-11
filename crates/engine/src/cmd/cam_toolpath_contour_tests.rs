//! `cam.toolpath.contour2d`: the offset paths it generates from picked edges, the passes it
//! plans, and what it refuses.

use serde_json::{Value, json};

use crate::Session;
use crate::schema::output_schema_for;

/// A 40 x 20 x 10 plate on the model origin, named "Plate".
fn plate(s: &mut Session) {
    s.execute("solid.box", &json!({"length": 40, "width": 20, "height": 10, "body_name": "Plate"})).unwrap();
}

/// A 6 mm flat end mill, returning its id.
fn mill(s: &mut Session) -> u64 {
    s.execute("cam.tool.create", &json!({"diameter": 6})).unwrap()["tool"]["id"].as_u64().unwrap()
}

/// The reply's waypoints as numbers.
fn waypoints(reply: &Value) -> Vec<[f64; 3]> {
    reply["waypoints"]
        .as_array()
        .expect("the reply lists waypoints")
        .iter()
        .map(|point| {
            let point = point.as_array().expect("a waypoint is an array of three numbers");
            [point[0].as_f64().unwrap(), point[1].as_f64().unwrap(), point[2].as_f64().unwrap()]
        })
        .collect()
}

/// A picked top edge is followed at the cutter's radius to its open side, one path per pass, and
/// the operation is stored in the design.
#[test]
fn follows_a_picked_edge_at_the_cutters_radius() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let reply = s
        .execute(
            "cam.toolpath.contour2d",
            &json!({"tool": tool, "target_edges": [[20, 0, 10]], "clearance_height": 5, "bottom_depth": 4, "stepdown": 2}),
        )
        .unwrap();
    assert!(reply.get("param_warnings").is_none(), "{reply}");
    assert_eq!(reply["passes"], 2);
    assert_eq!(reply["cut_levels"], json!([8.0, 6.0]), "the passes step down from the plate's top, deepest last");
    assert_eq!(reply["count"], 1);
    assert_eq!(reply["contour"]["name"], "2D Contour 1");
    assert_eq!(reply["contour"]["tool"], tool);
    assert_eq!(reply["contour"]["setup"], 0);
    assert_eq!(reply["contour"]["clearance_height"], 5.0);
    assert_eq!(reply["contour"]["bottom_depth"], 4.0);
    assert_eq!(reply["contour"]["stepdown"], 2.0);
    // The edge runs along X at y = 0 on the plate's top: the 3 mm radius puts the cutter's centre
    // at y = -3, the open side, from one end of the edge to the other, once per pass.
    let points = waypoints(&reply);
    assert_eq!(points.len(), 4, "{points:?}");
    for point in &points {
        assert!((point[1] + 3.0).abs() < 1e-9, "the cutter's centre is 3 mm outside the wall: {point:?}");
        assert!([0.0, 40.0].iter().any(|end| (point[0] - end).abs() < 1e-9), "the path runs the edge's length: {point:?}");
        assert!([8.0, 6.0].iter().any(|level| (point[2] - level).abs() < 1e-9), "the path is at a pass's depth: {point:?}");
    }
    assert!((reply["length"].as_f64().unwrap() - 80.0).abs() < 1e-6, "40 mm of edge twice: {reply}");
    let stored = &s.doc.cam.contours[0];
    assert_eq!(stored.name, "2D Contour 1");
    assert_eq!(stored.target_edges.len(), 1);
    assert!(!stored.target_edges[0].is_empty(), "the edge is stored by its persistent name");
    assert_eq!(stored.clearance_height, 5.0);
    assert_eq!(stored.bottom_depth, 4.0);
    assert_eq!(stored.stepdown, 2.0);
    let schema = output_schema_for::<crate::schema::cam_toolpath::CamContour2dOutput>();
    assert!(schema.as_value().is_object(), "the reply has a schema");
}

/// An opening's rim is followed from inside it: the open side is the hole, so the cutter's centre
/// runs a radius inside its wall.
#[test]
fn follows_an_openings_rim_from_inside_it() {
    let mut s = Session::default();
    plate(&mut s);
    s.execute("solid.cylinder", &json!({"base": [20, 10, -1], "radius": 5, "height": 12, "operation": "cut", "targets": ["Plate"]})).unwrap();
    let tool = mill(&mut s);
    let reply =
        s.execute("cam.toolpath.contour2d", &json!({"tool": tool, "target_edges": [[25, 10, 10]], "bottom_depth": 5, "stepdown": 5})).unwrap();
    assert_eq!(reply["passes"], 1, "{reply}");
    assert_eq!(reply["cut_levels"], json!([5.0]));
    let points = waypoints(&reply);
    assert!(points.len() > 8, "a round rim is followed closely: {points:?}");
    for point in &points {
        let radius = ((point[0] - 20.0).powi(2) + (point[1] - 10.0).powi(2)).sqrt();
        assert!((radius - 2.0).abs() < 0.01, "the cutter's centre is 3 mm inside the 5 mm opening: {point:?}");
        assert!((point[2] - 5.0).abs() < 1e-9, "{point:?}");
    }
    let length = reply["length"].as_f64().unwrap();
    assert!((length - 4.0 * std::f64::consts::PI).abs() < 0.05, "a 2 mm radius round: {length}");
}

/// Picks and depths the engine cannot cut with are refused, and a refused call changes nothing.
#[test]
fn refuses_picks_and_depths_it_cannot_follow() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let edge = json!([[20, 0, 10]]);
    let run = |s: &mut Session, call: Value| s.execute("cam.toolpath.contour2d", &call).unwrap_err().to_string();
    let missing_tool = run(&mut s, json!({"tool": 99, "target_edges": edge, "bottom_depth": 4, "stepdown": 2}));
    assert!(missing_tool.contains("there is no tool 99"), "{missing_tool}");
    let far = run(&mut s, json!({"tool": tool, "target_edges": [[20, 100, 10]], "bottom_depth": 4, "stepdown": 2}));
    assert!(far.contains("no edge at"), "{far}");
    let corner = run(&mut s, json!({"tool": tool, "target_edges": [[0, 0, 10]], "bottom_depth": 4, "stepdown": 2}));
    assert!(corner.contains("edges meet"), "{corner}");
    let no_body = run(&mut s, json!({"tool": tool, "target_edges": [{"body": "Nope", "index": 0}], "bottom_depth": 4, "stepdown": 2}));
    assert!(no_body.contains("no body `Nope`"), "{no_body}");
    let no_edge = run(&mut s, json!({"tool": tool, "target_edges": [{"body": "Plate", "index": 99}], "bottom_depth": 4, "stepdown": 2}));
    assert!(no_edge.contains("has no edge 99"), "{no_edge}");
    let too_deep = run(&mut s, json!({"tool": tool, "target_edges": edge, "bottom_depth": 12, "stepdown": 2}));
    assert!(too_deep.contains("deeper than the stock"), "{too_deep}");
    for depth in [0, -1, 1001] {
        assert!(
            s.execute("cam.toolpath.contour2d", &json!({"tool": tool, "target_edges": edge, "bottom_depth": depth, "stepdown": 1})).is_err(),
            "{depth}"
        );
    }
    assert!(s.execute("cam.toolpath.contour2d", &json!({"tool": tool, "target_edges": edge, "bottom_depth": 4, "stepdown": 0})).is_err());
    assert!(s.execute("cam.toolpath.contour2d", &json!({"tool": tool, "target_edges": [], "bottom_depth": 4, "stepdown": 2})).is_err());
    assert!(s.doc.cam.contours.is_empty(), "a refused call changes nothing");
}

/// Names default to the operation and stay unique; a second call numbers itself after the first.
#[test]
fn names_default_to_the_operation_and_stay_unique() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let call = json!({"tool": tool, "target_edges": [[20, 0, 10]], "bottom_depth": 4, "stepdown": 2});
    assert_eq!(s.execute("cam.toolpath.contour2d", &call).unwrap()["contour"]["name"], "2D Contour 1");
    assert_eq!(s.execute("cam.toolpath.contour2d", &call).unwrap()["contour"]["name"], "2D Contour 2");
    let mut named = call.clone();
    named.as_object_mut().unwrap().insert("name".into(), json!("  Rough wall  "));
    assert_eq!(s.execute("cam.toolpath.contour2d", &named).unwrap()["contour"]["name"], "Rough wall", "a name is stored trimmed");
    let repeated = s.execute("cam.toolpath.contour2d", &named).unwrap_err().to_string();
    assert!(repeated.contains("already exists"), "{repeated}");
    let mut long = call;
    long.as_object_mut().unwrap().insert("name".into(), json!("x".repeat(129)));
    let refused = s.execute("cam.toolpath.contour2d", &long).unwrap_err().to_string();
    assert!(refused.contains("128"), "{refused}");
    assert_eq!(s.doc.cam.contours.len(), 3, "only the accepted calls are stored");
}

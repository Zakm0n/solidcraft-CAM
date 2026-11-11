//! `cam.toolpath.drill`: the exact holes it reads, the cycles it generates and what it refuses.

use serde_json::{Value, json};

use crate::Session;
use crate::schema::output_schema_for;

/// A 40 x 20 x 10 plate on the model origin, named "Plate".
fn plate(s: &mut Session) {
    s.execute("solid.box", &json!({"length": 40, "width": 20, "height": 10, "body_name": "Plate"})).unwrap();
}

/// A hole of radius `radius` cut through the plate at [`x`, `y`].
fn through_hole(s: &mut Session, x: f64, y: f64, radius: f64) {
    s.execute("solid.cylinder", &json!({"base": [x, y, -1], "radius": radius, "height": 12, "operation": "cut", "targets": ["Plate"]})).unwrap();
}

/// A drill of 8 mm, returning its id.
fn drill(s: &mut Session) -> u64 {
    s.execute("cam.tool.create", &json!({"diameter": 8, "flute_length": 3})).unwrap()["tool"]["id"].as_u64().unwrap()
}

/// The persistent name of the rim of the hole at [`x`, `y`] at level `z` and radius `radius`.
fn rim(s: &mut Session, x: f64, y: f64, z: f64, radius: f64) -> String {
    let reply = s.execute("model.edges", &json!({"body": "Plate"})).unwrap();
    let edges = reply["edges"].as_array().expect("the plate lists its edges");
    let pick = edges
        .iter()
        .filter_map(|edge| {
            let mid = edge["mid"].as_array()?;
            let point = [mid.first()?.as_f64()?, mid.get(1)?.as_f64()?, mid.get(2)?.as_f64()?];
            let off = ((point[0] - x).powi(2) + (point[1] - y).powi(2)).sqrt();
            ((point[2] - z).abs() < 1e-6 && (off - radius).abs() < 1e-3).then_some(edge["name"].as_str()?.to_string())
        })
        .next();
    pick.unwrap_or_else(|| panic!("no rim of the hole at [{x}, {y}] at {z} mm: {reply}"))
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

/// One point on the toolpath.
fn at(point: [f64; 3], x: f64, y: f64, z: f64) {
    assert!((point[0] - x).abs() < 1e-9 && (point[1] - y).abs() < 1e-9 && (point[2] - z).abs() < 1e-9, "expected [{x}, {y}, {z}], got {point:?}");
}

/// A standard cycle rapid-moves above the hole, plunges once through it and retracts, and the
/// operation is stored in the design.
#[test]
fn a_standard_cycle_plunges_once_through_the_hole() {
    let mut s = Session::default();
    plate(&mut s);
    through_hole(&mut s, 20.0, 10.0, 4.0);
    let tool = drill(&mut s);
    let center = rim(&mut s, 20.0, 10.0, 10.0, 4.0);
    let reply = s
        .execute("cam.toolpath.drill", &json!({"tool": tool, "hole_centers": [center], "clearance_height": 5, "feed_rate": 200, "plunge_rate": 100}))
        .unwrap();
    assert!(reply.get("param_warnings").is_none(), "{reply}");
    assert_eq!(reply["holes"], 1);
    assert_eq!(reply["pecks"], 1, "a standard cycle plunges once per hole");
    assert_eq!(reply["drill"]["name"], "Drilling 1");
    assert_eq!(reply["drill"]["setup"], 0);
    assert_eq!(reply["drill"]["tool"], tool);
    assert_eq!(reply["drill"]["cycle_type"], "standard");
    assert_eq!(reply["drill"]["depth_reference"], "hole_bottom");
    assert_eq!(reply["drill"]["clearance_height"], 5.0);
    assert_eq!(reply["drill"]["peck_depth"], 0.0);
    assert_eq!(reply["drill"]["depth"], 0.0);
    // The plate is 10 mm high and the hole runs through it: the cutter travels 5 mm down to the
    // rim at 10 mm, plunges 10 mm to the plate's bottom and retracts 15 mm.
    let points = waypoints(&reply);
    assert_eq!(points.len(), 4, "{points:?}");
    at(points[0], 20.0, 10.0, 15.0);
    at(points[1], 20.0, 10.0, 10.0);
    at(points[2], 20.0, 10.0, 0.0);
    at(points[3], 20.0, 10.0, 15.0);
    assert!((reply["cutting_length"].as_f64().unwrap() - 10.0).abs() < 1e-9, "{reply}");
    assert!((reply["rapid_length"].as_f64().unwrap() - 20.0).abs() < 1e-9, "{reply}");
    // 10 mm at 100 mm/min plunging and 20 mm at 200 mm/min rapid: 6 + 6 seconds.
    assert!((reply["cycle_time"].as_f64().unwrap() - 12.0).abs() < 1e-9, "{reply}");
    let stored = &s.doc.cam.drills[0];
    assert_eq!(stored.name, "Drilling 1");
    assert_eq!(stored.hole_centers, vec![center]);
    assert_eq!(stored.cycle_type, solidcraft_doc::cam::DrillCycle::Standard);
    assert_eq!(stored.depth_reference, solidcraft_doc::cam::DrillDepthReference::HoleBottom);
    let schema = output_schema_for::<crate::schema::cam_toolpath_drill::CamToolpathDrillOutput>();
    assert!(schema.as_value().is_object(), "the reply has a schema");
}

/// A peck cycle plunges one peck at a time and retracts clear of the hole between pecks.
#[test]
fn a_peck_cycle_retracts_clear_between_pecks() {
    let mut s = Session::default();
    plate(&mut s);
    through_hole(&mut s, 20.0, 10.0, 4.0);
    let tool = drill(&mut s);
    let center = rim(&mut s, 20.0, 10.0, 10.0, 4.0);
    let reply = s
        .execute("cam.toolpath.drill", &json!({"tool": tool, "hole_centers": [center], "cycle_type": "peck", "peck_depth": 4, "clearance_height": 5}))
        .unwrap();
    assert_eq!(reply["pecks"], 3, "10 mm in pecks of 4 mm: {reply}");
    let points = waypoints(&reply);
    assert_eq!(points.len(), 10, "{points:?}");
    for (index, level) in [15.0, 10.0, 6.0, 15.0, 6.0, 2.0, 15.0, 2.0, 0.0, 15.0].into_iter().enumerate() {
        at(points[index], 20.0, 10.0, level);
    }
    assert!((reply["cutting_length"].as_f64().unwrap() - 10.0).abs() < 1e-9, "the pecks add up to the hole: {reply}");
    assert!((reply["rapid_length"].as_f64().unwrap() - 64.0).abs() < 1e-9, "{reply}");
}

/// A deep hole cycle retracts only to the hole's top between pecks.
#[test]
fn a_deep_hole_cycle_retracts_inside_the_hole() {
    let mut s = Session::default();
    plate(&mut s);
    through_hole(&mut s, 20.0, 10.0, 4.0);
    let tool = drill(&mut s);
    let center = rim(&mut s, 20.0, 10.0, 10.0, 4.0);
    let reply = s
        .execute(
            "cam.toolpath.drill",
            &json!({"tool": tool, "hole_centers": [center], "cycle_type": "deep_hole", "peck_depth": 4, "clearance_height": 5}),
        )
        .unwrap();
    assert_eq!(reply["pecks"], 3);
    let points = waypoints(&reply);
    assert_eq!(points.len(), 11, "{points:?}");
    for (index, level) in [15.0, 10.0, 6.0, 10.0, 6.0, 2.0, 10.0, 2.0, 0.0, 10.0, 15.0].into_iter().enumerate() {
        at(points[index], 20.0, 10.0, level);
    }
    assert!((reply["cutting_length"].as_f64().unwrap() - 10.0).abs() < 1e-9, "{reply}");
}

/// A blind hole with a flat floor stops exactly on the floor: its exact bottom rim is the depth.
#[test]
fn a_blind_hole_stops_on_its_floor() {
    let mut s = Session::default();
    plate(&mut s);
    // A cylinder cut from 4 mm up: the bore runs from the floor at 4 mm to the plate's top.
    s.execute("solid.cylinder", &json!({"base": [20, 10, 4], "radius": 4, "height": 8, "operation": "cut", "targets": ["Plate"]})).unwrap();
    let tool = drill(&mut s);
    let center = rim(&mut s, 20.0, 10.0, 10.0, 4.0);
    let reply = s.execute("cam.toolpath.drill", &json!({"tool": tool, "hole_centers": [center]})).unwrap();
    let points = waypoints(&reply);
    assert_eq!(points.len(), 3, "a clearance of 0 starts at the plate's top: {points:?}");
    at(points[0], 20.0, 10.0, 10.0);
    at(points[1], 20.0, 10.0, 4.0);
    at(points[2], 20.0, 10.0, 10.0);
    assert!((reply["cutting_length"].as_f64().unwrap() - 6.0).abs() < 1e-9, "{reply}");
    // Picking the floor's rim drills the same hole: it runs from the bore's top, not from the
    // picked rim.
    let floor = rim(&mut s, 20.0, 10.0, 4.0, 4.0);
    let reply = s.execute("cam.toolpath.drill", &json!({"tool": tool, "hole_centers": [floor]})).unwrap();
    let points = waypoints(&reply);
    at(points[0], 20.0, 10.0, 10.0);
    at(points[1], 20.0, 10.0, 4.0);
}

/// `absolute` cuts `depth` below the stock's top and `model_bottom` to the bodies' lowest point.
#[test]
fn depth_references_place_the_cut_exactly() {
    let mut s = Session::default();
    plate(&mut s);
    through_hole(&mut s, 20.0, 10.0, 4.0);
    let tool = drill(&mut s);
    let center = rim(&mut s, 20.0, 10.0, 10.0, 4.0);
    let absolute =
        s.execute("cam.toolpath.drill", &json!({"tool": tool, "hole_centers": [center], "depth_reference": "absolute", "depth": 6})).unwrap();
    assert_eq!(absolute["drill"]["depth"], 6.0);
    let points = waypoints(&absolute);
    at(points[1], 20.0, 10.0, 4.0);
    assert!((absolute["cutting_length"].as_f64().unwrap() - 6.0).abs() < 1e-9, "{absolute}");
    let center = rim(&mut s, 20.0, 10.0, 10.0, 4.0);
    let bottom = s.execute("cam.toolpath.drill", &json!({"tool": tool, "hole_centers": [center], "depth_reference": "model_bottom"})).unwrap();
    let points = waypoints(&bottom);
    at(points[1], 20.0, 10.0, 0.0);
    assert!((bottom["cutting_length"].as_f64().unwrap() - 10.0).abs() < 1e-9, "{bottom}");
}

/// Several holes are drilled in order, with the rapid between them counted.
#[test]
fn several_holes_are_drilled_in_order() {
    let mut s = Session::default();
    plate(&mut s);
    through_hole(&mut s, 10.0, 10.0, 4.0);
    through_hole(&mut s, 30.0, 10.0, 4.0);
    let tool = drill(&mut s);
    let first = rim(&mut s, 10.0, 10.0, 10.0, 4.0);
    let second = rim(&mut s, 30.0, 10.0, 10.0, 4.0);
    let reply = s.execute("cam.toolpath.drill", &json!({"tool": tool, "hole_centers": [first, second]})).unwrap();
    assert_eq!(reply["holes"], 2);
    assert_eq!(reply["pecks"], 2);
    let points = waypoints(&reply);
    assert_eq!(points.len(), 6, "{points:?}");
    at(points[0], 10.0, 10.0, 10.0);
    at(points[1], 10.0, 10.0, 0.0);
    at(points[2], 10.0, 10.0, 10.0);
    at(points[3], 30.0, 10.0, 10.0);
    at(points[4], 30.0, 10.0, 0.0);
    at(points[5], 30.0, 10.0, 10.0);
    assert!((reply["rapid_length"].as_f64().unwrap() - 40.0).abs() < 1e-9, "20 mm between the holes and 20 mm of retracts: {reply}");
}

/// Picks and depths the engine cannot drill are refused, and a refused call changes nothing.
#[test]
fn refuses_picks_and_depths_it_cannot_drill() {
    let mut s = Session::default();
    plate(&mut s);
    through_hole(&mut s, 20.0, 10.0, 4.0);
    let tool = drill(&mut s);
    let center = rim(&mut s, 20.0, 10.0, 10.0, 4.0);
    // A straight edge of the plate's top is no hole's rim.
    let straight = s.execute("model.edges", &json!({"body": "Plate"})).unwrap()["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| (edge["mid"][1].as_f64().unwrap() - 0.0).abs() < 1e-9 && (edge["mid"][2].as_f64().unwrap() - 10.0).abs() < 1e-9)
        .and_then(|edge| edge["name"].as_str())
        .expect("the plate has a top edge along its front")
        .to_string();
    let run = |s: &mut Session, call: Value| s.execute("cam.toolpath.drill", &call).unwrap_err().to_string();
    let missing = run(&mut s, json!({"tool": 99, "hole_centers": [center]}));
    assert!(missing.contains("there is no tool 99"), "{missing}");
    let unnamed = run(&mut s, json!({"tool": tool, "hole_centers": ["no-such-rim"]}));
    assert!(unnamed.contains("no hole center `no-such-rim`"), "{unnamed}");
    let not_circle = run(&mut s, json!({"tool": tool, "hole_centers": [straight]}));
    assert!(not_circle.contains("is not on a circle"), "{not_circle}");
    let twice = run(&mut s, json!({"tool": tool, "hole_centers": [center, center]}));
    assert!(twice.contains("named twice"), "{twice}");
    let no_depth = run(&mut s, json!({"tool": tool, "hole_centers": [center], "depth_reference": "absolute"}));
    assert!(no_depth.contains("needed"), "{no_depth}");
    let too_deep = run(&mut s, json!({"tool": tool, "hole_centers": [center], "depth_reference": "absolute", "depth": 20}));
    assert!(too_deep.contains("deeper than the stock"), "{too_deep}");
    let wide = run(&mut s, json!({"tool": tool, "hole_centers": [center], "cycle_type": "spindle"}));
    assert!(wide.contains("standard, peck or deep_hole"), "{wide}");
    for call in [
        json!({"tool": tool, "hole_centers": []}),
        json!({"tool": tool, "hole_centers": [""]}),
        json!({"tool": tool, "hole_centers": [center], "peck_depth": -1}),
        json!({"tool": tool, "hole_centers": [center], "clearance_height": -1}),
        json!({"tool": tool, "hole_centers": [center], "depth": 0}),
        json!({"tool": tool, "hole_centers": [center], "feed_rate": -1}),
    ] {
        assert!(s.execute("cam.toolpath.drill", &call).is_err(), "{call} should be refused");
    }
    assert!(s.doc.cam.drills.is_empty(), "a refused call changes nothing");
}

/// A peck deeper than the hole, and a hole the tool is wider than, are refused.
#[test]
fn refuses_a_peck_deeper_than_the_hole_and_a_tool_wider_than_it() {
    let mut s = Session::default();
    plate(&mut s);
    through_hole(&mut s, 20.0, 10.0, 4.0);
    let tool = drill(&mut s);
    let center = rim(&mut s, 20.0, 10.0, 10.0, 4.0);
    let deep = s
        .execute("cam.toolpath.drill", &json!({"tool": tool, "hole_centers": [center], "cycle_type": "peck", "peck_depth": 20}))
        .unwrap_err()
        .to_string();
    assert!(deep.contains("deeper than the hole"), "{deep}");
    let big = s.execute("cam.tool.create", &json!({"diameter": 12})).unwrap()["tool"]["id"].as_u64().unwrap();
    let wide = s.execute("cam.toolpath.drill", &json!({"tool": big, "hole_centers": [center]})).unwrap_err().to_string();
    assert!(wide.contains("wider than"), "{wide}");
}

/// A hole that runs across the setup's Z cannot be drilled from above.
#[test]
fn refuses_a_hole_that_is_not_along_the_setups_z() {
    let mut s = Session::default();
    plate(&mut s);
    // A hole through the plate's long side, along X.
    s.execute("solid.cylinder", &json!({"base": [-1, 10, 5], "axis": "X", "radius": 3, "height": 42, "operation": "cut", "targets": ["Plate"]}))
        .unwrap();
    let reply = s.execute("model.edges", &json!({"body": "Plate"})).unwrap();
    let rim = reply["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| {
            (edge["mid"][0].as_f64().unwrap() - 0.0).abs() < 1e-6 && {
                let (y, z) = (edge["mid"][1].as_f64().unwrap(), edge["mid"][2].as_f64().unwrap());
                ((y - 10.0).powi(2) + (z - 5.0).powi(2)).sqrt() - 3.0 < 1e-3
            }
        })
        .and_then(|edge| edge["name"].as_str())
        .expect("the side hole has a rim")
        .to_string();
    let tool = drill(&mut s);
    let refused = s.execute("cam.toolpath.drill", &json!({"tool": tool, "hole_centers": [rim]})).unwrap_err().to_string();
    assert!(refused.contains("not a hole along the setup's Z axis"), "{refused}");
}

/// Names default to the operation and stay unique; a second call numbers itself after the first.
#[test]
fn names_default_to_the_operation_and_stay_unique() {
    let mut s = Session::default();
    plate(&mut s);
    through_hole(&mut s, 20.0, 10.0, 4.0);
    let tool = drill(&mut s);
    let center = rim(&mut s, 20.0, 10.0, 10.0, 4.0);
    let call = json!({"tool": tool, "hole_centers": [center]});
    assert_eq!(s.execute("cam.toolpath.drill", &call).unwrap()["drill"]["name"], "Drilling 1");
    assert_eq!(s.execute("cam.toolpath.drill", &call).unwrap()["drill"]["name"], "Drilling 2");
    let mut named = call.clone();
    named.as_object_mut().unwrap().insert("name".into(), json!("  Rough holes  "));
    assert_eq!(s.execute("cam.toolpath.drill", &named).unwrap()["drill"]["name"], "Rough holes", "a name is stored trimmed");
    let repeated = s.execute("cam.toolpath.drill", &named).unwrap_err().to_string();
    assert!(repeated.contains("already exists"), "{repeated}");
    let mut long = call;
    long.as_object_mut().unwrap().insert("name".into(), json!("x".repeat(129)));
    let refused = s.execute("cam.toolpath.drill", &long).unwrap_err().to_string();
    assert!(refused.contains("128"), "{refused}");
    assert_eq!(s.doc.cam.drills.len(), 3, "only the accepted calls are stored");
}

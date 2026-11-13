//! The 2D facing command (`cam.toolpath.facing`): the stock box it clears, the boustrophedon it
//! plans over it pass by pass, the operation it stores and what it refuses.

use serde_json::{Value, json};

use crate::Session;

/// A 60 x 40 x 10 plate on the model origin, named "Plate".
fn plate(s: &mut Session) {
    s.execute("solid.box", &json!({"length": 60, "width": 40, "height": 10, "body_name": "Plate"})).unwrap();
}

/// A 10 mm flat end mill, returning its id.
fn mill(s: &mut Session) -> u64 {
    s.execute("cam.tool.create", &json!({"diameter": 10})).unwrap()["tool"]["id"].as_u64().unwrap()
}

fn number(v: &Value) -> f64 {
    v.as_f64().expect("a number")
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-6
}

/// The box is the plate's exact extent, and the passes are evenly spaced half a 10 mm tool
/// apart across it: 40 mm of width takes 8 intervals, so 9 passes of 60 mm each.
#[test]
fn facing_generates_valid_bounds() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let reply = s.execute("cam.toolpath.facing", &json!({"tool": tool, "stepover": 0.5, "depth": 5})).unwrap();

    let bounds = &reply["bounds"];
    assert_eq!(bounds["min"], json!([0.0, 0.0, 0.0]), "{reply}");
    assert_eq!(bounds["max"], json!([60.0, 40.0, 10.0]), "{reply}");
    assert!(close(number(&bounds["area"]), 2400.0), "{reply}");

    assert_eq!(reply["passes"], 9, "a 5 mm stepover crosses the 40 mm width in 8 intervals: {reply}");
    assert_eq!(reply["cut_levels"], json!([5.0]), "one level, 5 mm below the 10 mm top: {reply}");
    assert!(close(number(&reply["cutting_length"]), 540.0), "9 passes of 60 mm: {reply}");
    assert!(close(number(&reply["rapid_length"]), 40.0), "8 rapids of 5 mm: {reply}");

    let waypoints = reply["waypoints"].as_array().expect("waypoints");
    assert_eq!(waypoints.len(), 18, "a start and an end for each of 9 passes: {reply}");
    assert_eq!(waypoints[0], json!([0.0, 0.0, 5.0]), "{reply}");
    assert_eq!(waypoints[1], json!([60.0, 0.0, 5.0]), "the first pass runs along +X: {reply}");
    assert_eq!(waypoints[2], json!([60.0, 5.0, 5.0]), "{reply}");
    assert_eq!(waypoints[3], json!([0.0, 5.0, 5.0]), "the next one runs back along -X: {reply}");
    assert_eq!(waypoints[16], json!([0.0, 40.0, 5.0]), "{reply}");
    assert_eq!(waypoints[17], json!([60.0, 40.0, 5.0]), "the last pass is an even one: {reply}");
    for point in waypoints {
        assert!(close(number(&point[2]), 5.0), "every pass is cut at the level: {point}");
    }

    let facing = &reply["facing"];
    assert_eq!(facing["name"], "2D Facing 1");
    assert_eq!(facing["setup"], 0, "no setup named is the machine origin");
    assert_eq!(facing["tool"], tool);
    assert!(close(number(&facing["stepover"]), 0.5), "{reply}");
    assert!(close(number(&facing["pass_extension"]), 0.0), "{reply}");
    assert!(close(number(&facing["stock_offset"]), 0.0), "{reply}");
    assert!(close(number(&facing["depth"]), 5.0), "{reply}");
    assert_eq!(reply["count"], 1);
    assert_eq!(s.doc.cam.facings.len(), 1, "the operation is stored in the design");
    assert_eq!(s.doc.cam.facings[0].name, "2D Facing 1");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc.cam.facings.is_empty(), "undo takes the operation back out");
}

/// A pass extension runs each pass beyond the stock's edge on both sides, while the stock box it
/// reports stays the stock itself.
#[test]
fn a_pass_extension_clears_beyond_the_stock_edge() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let reply = s.execute("cam.toolpath.facing", &json!({"tool": tool, "stepover": 0.5, "pass_extension": 2, "depth": 5})).unwrap();

    // 64 mm of X and 44 mm of Y: 44 / 5 = 8.8 takes 9 intervals, so 10 passes.
    assert_eq!(reply["passes"], 10, "{reply}");
    assert!(close(number(&reply["cutting_length"]), 10.0 * 64.0), "10 passes of 60 + 2 + 2 mm: {reply}");
    assert!(close(number(&reply["rapid_length"]), 44.0), "9 rapids across 40 + 2 + 2 mm: {reply}");

    assert_eq!(reply["bounds"]["min"], json!([0.0, 0.0, 0.0]), "the extension is not stock: {reply}");
    assert_eq!(reply["bounds"]["max"], json!([60.0, 40.0, 10.0]), "{reply}");

    let waypoints = reply["waypoints"].as_array().expect("waypoints");
    assert_eq!(waypoints.len(), 20, "{reply}");
    assert_eq!(waypoints[0], json!([-2.0, -2.0, 5.0]), "the first pass starts past the corner: {reply}");
    assert_eq!(waypoints[1], json!([62.0, -2.0, 5.0]), "{reply}");
    let last = &waypoints[19];
    assert_eq!(last, &json!([-2.0, 42.0, 5.0]), "the last pass ends past the far corner: {reply}");
}

/// Without a stock size, `stock_offset` is the raw stock standing off the machined bodies on each
/// side, along X and Y only.
#[test]
fn a_stock_offset_enlarges_the_stock_around_the_bodies() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let reply = s.execute("cam.toolpath.facing", &json!({"tool": tool, "stepover": 0.5, "stock_offset": 1, "depth": 5})).unwrap();

    let bounds = &reply["bounds"];
    assert_eq!(bounds["min"], json!([-1.0, -1.0, 0.0]), "{reply}");
    assert_eq!(bounds["max"], json!([61.0, 41.0, 10.0]), "the offset moves X and Y, not the top: {reply}");
    assert!(close(number(&bounds["area"]), 62.0 * 42.0), "{reply}");

    assert_eq!(reply["passes"], 10, "42 mm of width takes 9 intervals at 5 mm: {reply}");
    assert!(close(number(&reply["cutting_length"]), 620.0), "10 passes of 62 mm: {reply}");
    assert!(close(number(&reply["rapid_length"]), 42.0), "{reply}");
}

/// A call that states an expression reads it in the design's units, like every other length.
#[test]
fn a_depth_expression_is_evaluated() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let reply = s.execute("cam.toolpath.facing", &json!({"tool": tool, "depth": "2 + 3"})).unwrap();
    assert_eq!(reply["cut_levels"], json!([5.0]), "{reply}");
    assert!(close(number(&reply["facing"]["depth"]), 5.0), "{reply}");
}

/// Refused values are named and change nothing.
#[test]
fn a_hostile_facing_call_is_refused() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let cases: [(Value, &str); 10] = [
        (json!({"tool": 0, "depth": 5}), "there is no tool 0"),
        (json!({"tool": tool, "depth": 5, "stepover": 0.05}), "`stepover`"),
        (json!({"tool": tool, "depth": 5, "stepover": 1.5}), "`stepover`"),
        (json!({"tool": tool, "depth": 0}), "`depth`"),
        (json!({"tool": tool, "depth": -1}), "`depth`"),
        (json!({"tool": tool, "depth": 11}), "deeper than the stock"),
        (json!({"tool": tool, "depth": 5, "pass_extension": -1}), "`pass_extension`"),
        (json!({"tool": tool, "depth": 5, "stock_offset": 1001}), "`stock_offset`"),
        (json!({"tool": tool, "depth": 5, "name": "x".repeat(129)}), "1–128 characters"),
        (json!({"tool": tool, "depth": "w / 0"}), "`depth`"),
    ];
    for (params, expected) in cases {
        let refused = s.execute("cam.toolpath.facing", &params).unwrap_err().to_string();
        assert!(refused.contains(expected), "{params}: {refused}");
    }
    assert!(s.doc.cam.facings.is_empty(), "a refused call stored an operation");
}

/// A design with no solid body has nothing to face, and two operations may not share a name.
#[test]
fn a_design_with_nothing_to_face_and_a_repeated_name_are_refused() {
    let mut s = Session::default();
    let tool = s.execute("cam.tool.create", &json!({"diameter": 10})).unwrap()["tool"]["id"].as_u64().unwrap();
    let refused = s.execute("cam.toolpath.facing", &json!({"tool": tool, "depth": 5})).unwrap_err().to_string();
    assert!(refused.contains("no solid body"), "{refused}");

    plate(&mut s);
    s.execute("cam.toolpath.facing", &json!({"tool": tool, "depth": 5})).unwrap();
    let refused = s.execute("cam.toolpath.facing", &json!({"tool": tool, "depth": 5, "name": "2D Facing 1"})).unwrap_err().to_string();
    assert!(refused.contains("already exists"), "{refused}");

    let reply = s.execute("cam.toolpath.facing", &json!({"tool": tool, "depth": 5})).unwrap();
    assert_eq!(reply["facing"]["name"], "2D Facing 2", "the default name takes the first free number: {reply}");
    assert_eq!(reply["count"], 2);
}

//! The 2D pocket command: the exact region it reads from the faces a call picks, the offset
//! passes it plans and what it refuses.

use serde_json::{Value, json};

use crate::Session;
use crate::schema::ParamCheck;

/// A 60 x 40 x 10 plate on the model origin, named "Plate".
fn plate(s: &mut Session) {
    s.execute("solid.box", &json!({"length": 60, "width": 40, "height": 10, "body_name": "Plate"})).unwrap();
}

/// The plate with a 40 x 30 pocket cut into its top, 5 mm deep: its floor is the face at z = 5
/// from (10, 5) to (50, 35).
fn pocketed_plate(s: &mut Session) {
    plate(s);
    s.execute("solid.box", &json!({"corner": [10, 5, 5], "length": 40, "width": 30, "height": 5, "operation": "cut", "targets": ["Plate"]})).unwrap();
}

/// A 2 mm end mill, returning its id: its radius of 1 mm fits the 40 x 30 pocket corner to corner.
fn mill(s: &mut Session) -> u64 {
    s.execute("cam.tool.create", &json!({"diameter": 2})).unwrap()["tool"]["id"].as_u64().unwrap()
}

/// A pocket clears its picked face's exact region with concentric offset passes, one stepover
/// apart, at every cut level down to the depth.
#[test]
fn a_pocket_clears_the_face_with_offset_passes() {
    let mut s = Session { param_check: ParamCheck::Warn, ..Session::default() };
    pocketed_plate(&mut s);
    let tool = mill(&mut s);
    // The floor of the cavity, picked by a point on it.
    let reply = s
        .execute(
            "cam.toolpath.pocket_2d",
            &json!({
                "tool": tool,
                "boundary_faces": [[15, 10, 5]],
                "stepover": 2,
                "depth": 5
            }),
        )
        .unwrap();
    assert!(reply.get("param_warnings").is_none(), "{reply}");
    let boundaries = reply["boundaries"].as_array().expect("boundaries");
    assert_eq!(boundaries.len(), 1, "the floor has no opening: {reply}");
    assert_eq!(boundaries[0]["body"], "Plate");
    assert_eq!(boundaries[0]["role"], "outer");
    assert!((boundaries[0]["length"].as_f64().unwrap_or_default() - 140.0).abs() < 1e-6, "{reply}");
    assert_eq!(boundaries[0]["points"].as_array().map(Vec::len), Some(4), "a rectangle's floor is its four corners: {reply}");
    assert_eq!(reply["passes"], 1, "a stepdown left out cuts the whole depth in one pass");
    assert_eq!(reply["cut_levels"], json!([5.0]), "the floor's height above the machine zero");
    // The boundary inset by the tool's 1 mm radius, then 3 mm, then 5 mm: the passes that still
    // fit inside the 40 x 30 region. Each rings the boundary and its opening.
    assert_eq!(reply["loops"], 7, "seven passes fit, each one ring: {reply}");
    let waypoints = reply["waypoints"].as_array().expect("waypoints");
    let per_ring = 4 + 1;
    assert_eq!(waypoints.len(), reply["loops"].as_u64().unwrap_or_default() as usize * per_ring, "{reply}");
    for (index, pass) in waypoints.chunks(per_ring).enumerate() {
        let inset = 1.0 + 2.0 * index as f64;
        assert!((pass[0][2].as_f64().unwrap_or_default() - 5.0).abs() < 1e-9, "every ring is cut at the floor: {pass:?}");
        assert_eq!(pass[0], pass[per_ring - 1], "each ring closes on itself");
        let xs: Vec<f64> = pass.iter().filter_map(|point| point[0].as_f64()).collect();
        let ys: Vec<f64> = pass.iter().filter_map(|point| point[1].as_f64()).collect();
        let (low_x, high_x) = (xs.iter().cloned().fold(f64::INFINITY, f64::min), xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
        let (low_y, high_y) = (ys.iter().cloned().fold(f64::INFINITY, f64::min), ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
        assert!((low_x - (10.0 + inset)).abs() < 1e-6 && (high_x - (50.0 - inset)).abs() < 1e-6, "pass {index}: {pass:?}");
        assert!((low_y - (5.0 + inset)).abs() < 1e-6 && (high_y - (35.0 - inset)).abs() < 1e-6, "pass {index}: {pass:?}");
    }
    let length = reply["length"].as_f64().unwrap_or_default();
    let expected: f64 = (0..7).map(|index| 2.0 * ((40.0 - 2.0 * (1.0 + 2.0 * index as f64)) + (30.0 - 2.0 * (1.0 + 2.0 * index as f64)))).sum();
    assert!((length - expected).abs() < 1e-6, "{length} is not {expected}");
    assert!(s.doc.cam.toolpaths.is_empty(), "the command stores nothing in the design");
}

/// An opening inside the pocket is reported as a hole and the cutter stops before it.
#[test]
fn an_opening_inside_the_pocket_is_a_hole_the_cutter_stays_out_of() {
    let mut s = Session { param_check: ParamCheck::Warn, ..Session::default() };
    pocketed_plate(&mut s);
    // A boss standing on the cavity floor: the floor becomes a face with an opening in it.
    s.execute("solid.cylinder", &json!({"base": [30, 20, 5], "radius": 4, "height": 5, "operation": "union", "targets": ["Plate"]})).unwrap();
    let tool = mill(&mut s);
    let reply = s
        .execute(
            "cam.toolpath.pocket_2d",
            &json!({
                "tool": tool,
                "boundary_faces": [[15, 10, 5]],
                "stepover": 2,
                "depth": 5
            }),
        )
        .unwrap();
    let boundaries = reply["boundaries"].as_array().expect("boundaries");
    assert_eq!(boundaries.len(), 2, "the floor and the boss: {reply}");
    assert_eq!(boundaries[0]["role"], "outer");
    assert_eq!(boundaries[1]["role"], "hole");
    let round = boundaries[1]["length"].as_f64().unwrap_or_default();
    assert!((round - 2.0 * std::f64::consts::PI * 4.0).abs() < 0.01, "the opening is the boss's circle: {round}");
    assert!(reply["loops"].as_u64().unwrap_or_default() >= 1, "{reply}");
    let waypoints = reply["waypoints"].as_array().expect("waypoints");
    for point in waypoints {
        let (x, y) = (point[0].as_f64().unwrap_or_default(), point[1].as_f64().unwrap_or_default());
        let clear = ((x - 30.0).powi(2) + (y - 20.0).powi(2)).sqrt();
        assert!(clear >= 4.0 + 1.0 - 1e-6, "[{x}, {y}] is inside the boss's 4 mm circle");
    }
}

/// A tool too big for the region is refused rather than guessed at.
#[test]
fn a_tool_that_does_not_fit_is_refused() {
    let mut s = Session::default();
    pocketed_plate(&mut s);
    let wide = s.execute("cam.tool.create", &json!({"diameter": 40})).unwrap()["tool"]["id"].as_u64().unwrap();
    let refused = s
        .execute("cam.toolpath.pocket_2d", &json!({"tool": wide, "boundary_faces": [[15, 10, 5]], "stepover": 4, "depth": 5}))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("does not fit"), "{refused}");
}

#[test]
fn the_call_is_refused_when_a_value_or_a_pick_is_wrong() {
    let mut s = Session::default();
    pocketed_plate(&mut s);
    let tool = mill(&mut s);
    let call = |extra: Value| {
        let mut call = json!({"tool": tool, "boundary_faces": [[15, 10, 5]], "stepover": 2, "depth": 5});
        call.as_object_mut().expect("an object").extend(extra.as_object().cloned().unwrap_or_default());
        call
    };
    for (extra, why) in [
        (json!({"boundary_faces": []}), "no face is picked"),
        (json!({"stepover": 0}), "a stepover of 0"),
        (json!({"stepover": -2}), "a negative stepover"),
        (json!({"stepover": 21}), "a stepover wider than the tool"),
        (json!({"depth": 0}), "a depth of 0"),
        (json!({"depth": 101}), "a depth deeper than the stock"),
        (json!({"tool": 99}), "a tool that does not exist"),
        (json!({"boundary_faces": [{"corner": "Plate"}]}), "a pick that is neither a point nor a name"),
    ] {
        assert!(s.execute("cam.toolpath.pocket_2d", &call(extra.clone())).is_err(), "{why} is refused: {extra}");
    }
    // A wall of the cavity does not look along the setup's Z: it cannot be a 2D pocket's boundary.
    let wall = s
        .execute("cam.toolpath.pocket_2d", &json!({"tool": tool, "boundary_faces": [[10, 20, 8]], "stepover": 2, "depth": 5}))
        .unwrap_err()
        .to_string();
    assert!(wall.contains("parallel to the setup's X/Y"), "{wall}");
    // The same face picked twice is a mistake, not a bigger pocket.
    let twice = s
        .execute("cam.toolpath.pocket_2d", &json!({"tool": tool, "boundary_faces": [[15, 10, 5], [45, 30, 5]], "stepover": 2, "depth": 5}))
        .unwrap_err()
        .to_string();
    assert!(twice.contains("picked twice"), "{twice}");
    // A face name that finds no face says so.
    let named = s
        .execute("cam.toolpath.pocket_2d", &json!({"tool": tool, "boundary_faces": ["Nowhere"], "stepover": 2, "depth": 5}))
        .unwrap_err()
        .to_string();
    assert!(named.contains("Nowhere"), "{named}");
    // A cut that reaches below the machined bodies is refused.
    let deep = s
        .execute("cam.toolpath.pocket_2d", &json!({"tool": tool, "boundary_faces": [[15, 10, 5]], "stepover": 2, "depth": 11}))
        .unwrap_err()
        .to_string();
    assert!(deep.contains("deeper than the stock"), "{deep}");
}

/// The steps and the depth are expressions, read in the design's length unit.
#[test]
fn the_steps_and_the_depth_may_be_expressions() {
    let mut s = Session::default();
    pocketed_plate(&mut s);
    let tool = mill(&mut s);
    let reply = s
        .execute(
            "cam.toolpath.pocket_2d",
            &json!({
                "tool": tool,
                "boundary_faces": [[15, 10, 5]],
                "stepover": "2 mm",
                "depth": "5 mm",
                "depth_per_pass": "2.5 mm"
            }),
        )
        .unwrap();
    assert_eq!(reply["passes"], 2, "5 mm at 2.5 mm a pass: {reply}");
    let levels: Vec<f64> = reply["cut_levels"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
    assert_eq!(levels, vec![7.5, 5.0], "the passes end at the 5 mm-deep floor: {reply}");
    let bad = s
        .execute(
            "cam.toolpath.pocket_2d",
            &json!({
                "tool": tool,
                "boundary_faces": [[15, 10, 5]],
                "stepover": "nope",
                "depth": 5
            }),
        )
        .unwrap_err()
        .to_string();
    assert!(bad.contains("stepover"), "{bad}");
}

//! `cam.export.gcode`: the ISO G-code a design's toolpaths make, its refusals, and the file it
//! writes.

use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::json;

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

/// A contour with the rates a program needs, returning its toolpath id.
fn contour(s: &mut Session, tool: u64) -> u64 {
    let reply = s
        .execute(
            "cam.toolpath.contour",
            &json!({"tool": tool, "total_depth": 10, "depth_per_pass": 5, "feed_rate": 800, "plunge_rate": 200, "spindle_speed": 12000}),
        )
        .unwrap();
    reply["toolpath"]["id"].as_u64().unwrap()
}

/// The program has the header, the tool change, the passes and the tape markers; the contour is
/// the plate's outline moved out by the cutter's radius.
#[test]
fn contour_exports_a_complete_program() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let id = contour(&mut s, tool);
    let reply = s.execute("cam.export.gcode", &json!({"program": "Bracket"})).unwrap();
    let text = reply["gcode"].as_str().unwrap();
    assert!(text.starts_with("%\n(Bracket)\n(Z-CAM)\nG21\nG90\nG17\nG94\nG54\n"), "{text}");
    assert!(text.ends_with("M30\n%\n"), "{text}");
    assert_eq!(reply["program"], "Bracket");
    assert_eq!(reply["units"], "mm");
    assert_eq!(reply["operations"].as_array().unwrap().len(), 1);
    assert_eq!(reply["operations"][0]["toolpath"], id);
    assert_eq!(reply["operations"][0]["kind"], "contour");
    assert_eq!(reply["operations"][0]["passes"], 2);
    assert_eq!(reply["passes"], 2);
    assert_eq!(reply["lines"], text.lines().count());
    assert_eq!(reply["bytes"], text.len());
    assert!(reply.get("path").is_none() && reply.get("gcode").is_some());
    assert!(text.contains(&format!("T{tool}\nM6\n")), "{text}");
    assert!(text.contains("S12000 M3\n"), "{text}");
    assert!(text.contains("G1 Z5 F200\n"), "the first pass at 5 mm: {text}");
    assert!(text.contains("G1 Z0 F200\n"), "the second pass reaches the bottom: {text}");
    assert!(text.contains("F800"), "the cutting moves run at the feed rate: {text}");
    // The 40 x 20 outline, offset outward by the 3 mm radius on both sides.
    for coordinate in ["-3", "43", "23"] {
        assert!(text.contains(coordinate), "the cutter's centre is clear of the wall by 3 mm: {coordinate}\n{text}");
    }
    let schema = output_schema_for::<crate::schema::cam_export::GcodeExport>();
    assert!(schema.as_value().is_object(), "the reply has a schema");
}

/// A pocket clears the inside with cutting moves, keeping clear of the outline's wall.
#[test]
fn pocket_exports_cutting_moves_inside_the_outline() {
    let mut s = Session::default();
    plate(&mut s);
    s.execute("solid.cylinder", &json!({"radius": 5, "height": 20, "base": [20, 10, -5], "operation": "cut"})).unwrap();
    let tool = mill(&mut s);
    let reply = s
        .execute(
            "cam.toolpath.pocket",
            &json!({"tool": tool, "total_depth": 10, "depth_per_pass": 5, "stepover": 3, "feed_rate": 600, "spindle_speed": 9000}),
        )
        .unwrap();
    let text = s.execute("cam.export.gcode", &json!({})).unwrap()["gcode"].as_str().unwrap().to_string();
    assert_eq!(reply["toolpath"]["kind"], "pocket");
    let moves = s.execute("cam.export.gcode", &json!({})).unwrap()["operations"][0]["moves"].as_u64().unwrap();
    assert!(moves > 5, "the pocket has a raster of cutting moves: {moves}\n{text}");
    // Every cut stays inside the plate, a radius clear of the outline's wall.
    for point in cut_points(&text) {
        assert!((3.0 - 1e-6..=37.0 + 1e-6).contains(&point.0), "x {} is clear of the wall", point.0);
        assert!((3.0 - 1e-6..=17.0 + 1e-6).contains(&point.1), "y {} is clear of the wall", point.1);
    }
}

/// The X/Y of every `G1 X… Y…` move in a program.
fn cut_points(text: &str) -> Vec<(f64, f64)> {
    let mut points = Vec::new();
    for line in text.lines().filter(|line| line.starts_with("G1 X")) {
        let mut x = None;
        let mut y = None;
        for word in line.split_whitespace() {
            if let Some(value) = word.strip_prefix('X') {
                x = value.parse::<f64>().ok();
            }
            if let Some(value) = word.strip_prefix('Y') {
                y = value.parse::<f64>().ok();
            }
        }
        if let (Some(x), Some(y)) = (x, y) {
            points.push((x, y));
        }
    }
    points
}

/// With several toolpaths, only those the call names (or its setup holds) are exported.
#[test]
fn the_call_exports_the_toolpaths_it_names() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    let first = contour(&mut s, tool);
    let second =
        s.execute("cam.toolpath.pocket", &json!({"tool": tool, "total_depth": 10, "stepover": 4, "feed_rate": 500})).unwrap()["toolpath"]["id"]
            .as_u64()
            .unwrap();
    let one = s.execute("cam.export.gcode", &json!({"toolpaths": [first]})).unwrap();
    assert_eq!(one["operations"].as_array().unwrap().len(), 1);
    assert_eq!(one["operations"][0]["toolpath"], first);
    let all = s.execute("cam.export.gcode", &json!({})).unwrap();
    assert_eq!(all["operations"].as_array().unwrap().len(), 2);
    assert_eq!(all["operations"][1]["toolpath"], second);
    let setup = s.execute("cam.setup.create", &json!({"name": "Mill", "bodies": ["Plate"]})).unwrap()["setup"].as_u64().unwrap();
    let in_setup =
        s.execute("cam.toolpath.contour", &json!({"tool": tool, "setup": setup, "total_depth": 10, "feed_rate": 400})).unwrap()["toolpath"]["id"]
            .as_u64()
            .unwrap();
    let only_setup = s.execute("cam.export.gcode", &json!({"setup": setup})).unwrap();
    assert_eq!(only_setup["operations"].as_array().unwrap().len(), 1);
    assert_eq!(only_setup["operations"][0]["toolpath"], in_setup);
}

/// A toolpath that cannot make a runnable program is refused, and a refused call writes nothing.
#[test]
fn a_program_that_cannot_be_run_is_refused() {
    let mut s = Session::default();
    plate(&mut s);
    let no_toolpaths = s.execute("cam.export.gcode", &json!({})).unwrap_err().to_string();
    assert!(no_toolpaths.contains("no toolpaths to export"), "{no_toolpaths}");
    let tool = mill(&mut s);
    s.execute("cam.toolpath.contour", &json!({"tool": tool, "total_depth": 10})).unwrap();
    let no_feed = s.execute("cam.export.gcode", &json!({})).unwrap_err().to_string();
    assert!(no_feed.contains("no feed rate"), "{no_feed}");
    let no_such = s.execute("cam.export.gcode", &json!({"toolpaths": [9999]})).unwrap_err().to_string();
    assert!(no_such.contains("there is no toolpath 9999"), "{no_such}");
    let no_setup = s.execute("cam.export.gcode", &json!({"setup": 42})).unwrap_err().to_string();
    assert!(no_setup.contains("there is no setup 42"), "{no_setup}");
    let long = "x".repeat(129);
    let bad_program = s.execute("cam.export.gcode", &json!({"program": long})).unwrap_err().to_string();
    assert!(bad_program.contains("128"), "{bad_program}");
    assert!(s.execute("cam.export.gcode", &json!({"safe_height": 1001})).is_err());
    assert!(s.execute("cam.export.gcode", &json!({"toolpaths": "all"})).is_err());
}

/// A toolpath whose tool is gone says so instead of writing an empty tool change.
#[test]
fn a_toolpath_whose_tool_is_gone_is_refused() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    contour(&mut s, tool);
    s.doc_mut().cam.tools.clear();
    let gone = s.execute("cam.export.gcode", &json!({})).unwrap_err().to_string();
    assert!(gone.contains("which does not exist"), "{gone}");
}

static FILE: AtomicU32 = AtomicU32::new(0);

/// A path writes the program to a file and the file is exactly the text.
#[test]
fn a_path_writes_the_program_to_a_file() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = mill(&mut s);
    contour(&mut s, tool);
    let text = s.execute("cam.export.gcode", &json!({"program": "File test"})).unwrap()["gcode"].as_str().unwrap().to_string();
    let path = std::env::temp_dir().join(format!("zcam-gcode-{}-{}.nc", std::process::id(), FILE.fetch_add(1, Ordering::Relaxed)));
    let path_text = path.to_string_lossy().into_owned();
    let reply = s.execute("cam.export.gcode", &json!({"path": path_text, "program": "File test"})).unwrap();
    assert_eq!(reply["path"], path_text);
    assert!(reply.get("gcode").is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "the file is the program");
    let _ = std::fs::remove_file(&path);
}

/// A tool's name and the program's name cannot end a G-code comment early.
#[test]
fn comments_are_kept_inside_their_parentheses() {
    let mut s = Session::default();
    plate(&mut s);
    let tool = s.execute("cam.tool.create", &json!({"diameter": 6, "name": "End mill (rough)"})).unwrap()["tool"]["id"].as_u64().unwrap();
    contour(&mut s, tool);
    let text = s.execute("cam.export.gcode", &json!({"program": "Part (left) %"})).unwrap()["gcode"].as_str().unwrap().to_string();
    assert!(text.contains("(Part left)"), "{text}");
    assert!(text.contains("End mill rough 6 mm"), "{text}");
    for line in text.lines().filter(|line| line.starts_with('(')) {
        assert_eq!(line.matches('(').count(), 1, "{line}");
        assert_eq!(line.matches(')').count(), 1, "{line}");
    }
}

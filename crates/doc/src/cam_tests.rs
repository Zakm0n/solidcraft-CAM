//! The CAM data a design keeps: setups, the tool library and toolpaths (`crate::cam`).

use crate::Document;
use crate::cam::{
    CamContour2d, CamDrill, CamFacing, CamSettings, DrillCycle, DrillDepthReference, Setup, Stock, Tool, ToolKind, Toolpath, ToolpathKind,
};

fn example_setup() -> Setup {
    Setup {
        id: 5,
        name: "Setup 1".into(),
        coordinate_system: 3,
        stock: Stock { size: [80.0, 60.0, 20.0], offset: [0.0, 0.0, -20.0] },
        bodies: vec!["Bracket".into()],
    }
}

fn example_tool() -> Tool {
    Tool { id: 6, name: "6 mm bull nose".into(), kind: ToolKind::BullNoseEndMill, diameter: 6.0, corner_radius: 1.0, flute_length: 20.0, flutes: 3 }
}

fn example_contour() -> CamContour2d {
    CamContour2d {
        id: 8,
        name: "2D Contour 1".into(),
        setup: 5,
        tool: 6,
        target_edges: vec!["F1:side:l1|F1:top#1~1".into()],
        clearance_height: 5.0,
        bottom_depth: 8.0,
        stepdown: 2.0,
    }
}

fn example_drill() -> CamDrill {
    CamDrill {
        id: 9,
        name: "Drilling 1".into(),
        setup: 5,
        tool: 6,
        hole_centers: vec!["F1:vertex:1".into(), "F1:vertex:2".into()],
        cycle_type: DrillCycle::Peck,
        depth_reference: DrillDepthReference::HoleBottom,
        clearance_height: 5.0,
        peck_depth: 2.0,
        depth: 0.0,
    }
}

fn example_facing() -> CamFacing {
    CamFacing { id: 10, name: "2D Facing 1".into(), setup: 5, tool: 6, stepover: 0.7, pass_extension: 2.0, stock_offset: 1.0, depth: 1.0 }
}

fn example_toolpath() -> Toolpath {
    Toolpath {
        id: 7,
        name: "Pocket 1".into(),
        kind: ToolpathKind::Pocket,
        setup: 5,
        tool: 6,
        bodies: vec!["Bracket".into()],
        feed_rate: 900.0,
        plunge_rate: 300.0,
        spindle_speed: 12_000.0,
        stepover: 3.0,
        depth_per_pass: 2.0,
        total_depth: 8.0,
        stock_to_leave: 0.2,
    }
}

#[test]
fn cam_data_round_trips_through_a_design_file() {
    let mut doc = Document::new("Bracket");
    doc.cam.setups.push(example_setup());
    doc.cam.tools.push(example_tool());
    doc.cam.toolpaths.push(example_toolpath());
    doc.cam.contours.push(example_contour());
    doc.cam.drills.push(example_drill());
    doc.cam.facings.push(example_facing());
    let file = serde_json::to_value(&doc).unwrap();
    assert_eq!(file["cam"]["setups"][0]["coordinate_system"], 3);
    assert_eq!(file["cam"]["setups"][0]["stock"]["size"][2], 20.0);
    assert_eq!(file["cam"]["tools"][0]["kind"], "bull_nose_end_mill");
    assert_eq!(file["cam"]["tools"][0]["flutes"], 3);
    assert_eq!(file["cam"]["toolpaths"][0]["kind"], "pocket");
    assert_eq!(file["cam"]["toolpaths"][0]["stock_to_leave"], 0.2);
    assert_eq!(file["cam"]["contours"][0]["target_edges"][0], "F1:side:l1|F1:top#1~1");
    assert_eq!(file["cam"]["contours"][0]["clearance_height"], 5.0);
    assert_eq!(file["cam"]["contours"][0]["stepdown"], 2.0);
    assert_eq!(file["cam"]["drills"][0]["hole_centers"][0], "F1:vertex:1");
    assert_eq!(file["cam"]["drills"][0]["cycle_type"], "peck");
    assert_eq!(file["cam"]["drills"][0]["depth_reference"], "hole_bottom");
    assert_eq!(file["cam"]["drills"][0]["peck_depth"], 2.0);
    assert_eq!(file["cam"]["facings"][0]["stepover"], 0.7);
    assert_eq!(file["cam"]["facings"][0]["pass_extension"], 2.0);
    assert_eq!(file["cam"]["facings"][0]["stock_offset"], 1.0);
    assert_eq!(file["cam"]["facings"][0]["depth"], 1.0);
    let opened: Document = serde_json::from_value(file).unwrap();
    assert_eq!(opened.cam, doc.cam);
}

#[test]
fn a_design_without_cam_saves_without_the_key_and_opens_empty() {
    let doc = Document::new("Plain");
    assert!(doc.cam.is_empty());
    let file = serde_json::to_value(&doc).unwrap();
    assert!(file.get("cam").is_none());
    let opened: Document = serde_json::from_value(file).unwrap();
    assert_eq!(opened.cam, CamSettings::default());
}

#[test]
fn a_saved_cam_record_missing_its_newer_fields_opens_with_defaults() {
    let cam: CamSettings = serde_json::from_value(serde_json::json!({
        "setups": [{ "id": 11, "name": "Top" }],
        "tools": [{ "id": 12 }],
        "toolpaths": [{ "id": 13, "kind": "contour" }]
    }))
    .unwrap();
    assert_eq!(cam.setups[0].stock, Stock::default());
    assert_eq!(cam.tools[0].kind, ToolKind::FlatEndMill);
    assert_eq!(cam.toolpaths[0].kind, ToolpathKind::Contour);
    assert_eq!(cam.toolpaths[0].total_depth, 0.0);
    assert!(cam.contours.is_empty(), "a design saved before 2D contours has none");
    assert!(cam.drills.is_empty(), "a design saved before drilling has none");
    assert!(cam.facings.is_empty(), "a design saved before 2D facing has none");
}

#[test]
fn an_unknown_tool_or_toolpath_kind_is_refused() {
    assert!(serde_json::from_value::<Tool>(serde_json::json!({ "id": 1, "kind": "laser" })).is_err());
    assert!(serde_json::from_value::<Toolpath>(serde_json::json!({ "id": 1, "kind": "drilling" })).is_err());
}

#[test]
fn cam_items_take_ids_from_the_designs_one_counter() {
    let mut doc = Document::new("Ids");
    doc.cam.setups.push(example_setup());
    doc.cam.tools.push(example_tool());
    doc.cam.toolpaths.push(example_toolpath());
    assert_eq!(doc.allocate_id().unwrap(), 8);
}

#[test]
fn a_contour_round_trips_and_takes_an_id_from_the_designs_one_counter() {
    let mut doc = Document::new("Contour");
    doc.cam.contours.push(example_contour());
    let opened: Document = serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap();
    assert_eq!(opened.cam.contours, doc.cam.contours);
    assert_eq!(doc.cam.contour(8), Some(&example_contour()));
    assert_eq!(doc.cam.contour(9), None);
    assert_eq!(doc.allocate_id().unwrap(), 9);
}

#[test]
fn a_drill_round_trips_and_takes_an_id_from_the_designs_one_counter() {
    let mut doc = Document::new("Drill");
    doc.cam.drills.push(example_drill());
    assert!(!doc.cam.is_empty(), "drilling alone is CAM data the design saves");
    let opened: Document = serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap();
    assert_eq!(opened.cam.drills, doc.cam.drills);
    assert_eq!(doc.cam.drill(9), Some(&example_drill()));
    assert_eq!(doc.cam.drill(10), None);
    assert_eq!(doc.allocate_id().unwrap(), 10);
}

#[test]
fn a_facing_round_trips_and_takes_an_id_from_the_designs_one_counter() {
    let mut doc = Document::new("Facing");
    doc.cam.facings.push(example_facing());
    assert!(!doc.cam.is_empty(), "facing alone is CAM data the design saves");
    let opened: Document = serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap();
    assert_eq!(opened.cam.facings, doc.cam.facings);
    assert_eq!(doc.cam.facing(10), Some(&example_facing()));
    assert_eq!(doc.cam.facing(11), None);
    assert_eq!(doc.allocate_id().unwrap(), 11);
}

#[test]
fn a_setup_lists_its_own_toolpaths_in_the_order_they_were_made() {
    let mut cam = CamSettings::default();
    let first = Toolpath { id: 7, setup: 5, ..Default::default() };
    let other = Toolpath { id: 8, setup: 9, ..Default::default() };
    let second = Toolpath { id: 10, setup: 5, ..Default::default() };
    cam.toolpaths.extend([first.clone(), other, second.clone()]);
    let listed: Vec<u64> = cam.toolpaths_in(5).map(|p| p.id).collect();
    assert_eq!(listed, [first.id, second.id]);
    assert_eq!(cam.setup(5), None);
    assert_eq!(cam.tool(6), None);
}

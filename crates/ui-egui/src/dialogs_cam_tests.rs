use std::cell::Cell;
use std::rc::Rc;

use serde_json::json;
use solidcore_record::Vec3;
use solidcraft_engine::Session;

use super::*;
use crate::Services;
use crate::dialogs::{Kind, apply_commands, preview_commands};

/// A grounded base, a cam (a disc of radius 30 about (10, 0)) turning on a revolute joint about +Z
/// through the origin, a tappet on a slider along +X, its end on the cam, and a spare part on a
/// revolute joint of its own.
fn valve() -> SolveApp {
    let mut s = Session::default();
    let mut run = |id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run("component.create", json!({"name": "Base"}));
    run("solid.box", json!({"length": 4, "width": 4, "height": 4, "corner": [-80, -80, -20]}));
    run("component.activate", json!({"component": "root"}));
    run("component.create", json!({"name": "Cam"}));
    run("solid.cylinder", json!({"base": [10, 0, 0], "axis": [0, 0, 1], "radius": 30, "height": 5}));
    run("component.activate", json!({"component": "root"}));
    run("component.create", json!({"name": "Tappet"}));
    run("solid.box", json!({"length": 20, "width": 4, "height": 5, "corner": [40, -2, 0]}));
    run("component.activate", json!({"component": "root"}));
    run("component.create", json!({"name": "Spare"}));
    run("solid.box", json!({"length": 4, "width": 4, "height": 4, "corner": [-80, 60, 0]}));
    run("component.activate", json!({"component": "root"}));
    run("occurrence.ground", json!({"occurrence": "Base:1", "grounded": true}));
    for (kind, name, z) in [("revolute", "Cam", [0, 0, 1]), ("slider", "Tappet", [1, 0, 0]), ("revolute", "Spare", [0, 0, 1])] {
        let at = |o: &str| json!({"occurrence": o, "point": [0, 0, 2.5], "z": z});
        run("joint.create", json!({"type": kind, "a": at("Base"), "b": at(name), "flip": true, "name": name}));
    }
    SolveApp::new(s, Services::default())
}

/// A pick of the mesh edge nearest a point.
fn edge(app: &SolveApp, p: Vec3) -> Sel {
    let st = app.session.world_state().unwrap();
    let mut best: Option<(f64, Sel)> = None;
    for b in &st.bodies {
        for (index, e) in b.mesh().edges.iter().enumerate() {
            let d = e.windows(2).filter_map(|w| Some(p.dist_to_segment(*w.first()?, *w.get(1)?))).fold(f64::MAX, f64::min);
            if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((d, Sel::Edge { body: b.name.clone(), index, point: p }));
            }
        }
    }
    best.map(|x| x.1).unwrap()
}

fn joint(app: &SolveApp, name: &str) -> u64 {
    app.session.doc.assembly.joints.iter().find(|j| j.name == name).map(|j| j.id).unwrap_or(0)
}

/// Picking the cam's edge chooses the joint it turns on; OK makes the cam, and Play turns the cam
/// in the preview.
#[test]
fn a_picked_cam_edge_makes_the_cam() {
    let mut app = valve();
    app.start("joint.cam");
    let mut d = app.dialog.take().unwrap();
    // A second revolute joint for the cam to start on: the pick puts the cam right.
    if let Kind::Assembly(Asm::Cam(f)) = &mut d.kind {
        (f.a, f.b) = (joint(&app, "Spare"), joint(&app, "Tappet"));
    }
    d.active = 0;
    // The far side of the cam's top edge (its near side touches the tappet).
    let pick = edge(&app, Vec3::new(-20.0, 0.0, 5.0));
    d.pick(&app.session, pick);
    app.dialog = Some(d);
    let ctx = crate::fonts::test_context();
    for _ in 0..3 {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| crate::dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
    }
    let mut d = app.dialog.clone().expect("the dialog stays open");
    if let Kind::Assembly(Asm::Cam(f)) = &mut d.kind {
        // The joint the picked part turns on, and a follower that is another joint.
        assert_eq!((f.a, f.b), (joint(&app, "Cam"), joint(&app, "Tappet")));
        (f.follower, f.radius) = (1, "4 mm".into());
    }
    let c = apply_commands(&app, &d).unwrap();
    let (id, p) = &c[0];
    assert_eq!((id.as_str(), p["follower"].clone(), p["radius"].clone()), ("joint.cam", json!("roller"), json!("4 mm")), "{p}");
    assert!(p["profile"]["edges"][0].is_array() && p["profile"]["occurrence"].is_u64(), "{p}");
    if let Kind::Assembly(Asm::Cam(f)) = &mut d.kind {
        (f.follower, f.animate, f.phase) = (0, true, 0.25);
    }
    let cmds = preview_commands(&app, &d).unwrap();
    assert_eq!(cmds.get(1).map(|c| (c.0.as_str(), c.1["values"].clone())), Some(("joint.drive", json!(["90 deg"]))), "{cmds:?}");
    let c = apply_commands(&app, &d).unwrap();
    for (id, p) in c {
        app.run(&id, p).unwrap();
    }
    let l = app.session.doc.assembly.links.first().cloned().unwrap();
    assert_eq!(l.lift.len(), 360);
    // A quarter turn lowers the knife tappet from 40 to √(30² − 10²).
    app.run("joint.drive", json!({"joint": "Cam", "value": 90})).unwrap();
    let v = solidcraft_engine::doc::joints::joint_values(&app.session.doc.assembly).get(&joint(&app, "Tappet")).cloned().unwrap_or_default();
    assert!((v.first().copied().unwrap_or(f64::NAN) - (800f64.sqrt() - 40.0)).abs() < 1e-3, "{v:?}");
}

/// The dialog's rows pass their English text, which the catalogs translate by its key.
#[test]
fn the_cam_dialog_labels_translate() {
    let de = crate::i18n::Locale::for_language("de");
    for (english, german) in [("Cam", "Nocken"), ("Follower", "Stößel"), ("Roller", "Rolle"), ("Tip (optional)", "Spitze (optional)")] {
        assert_eq!(de.label(english), german);
    }
    assert_eq!(crate::i18n::Locale::for_language("zh").label("Knife edge"), "尖顶");
}

/// The Toolpath tab's Setup dialog builds `cam.setup.create` from its fields and bodies.
#[test]
fn the_setup_dialog_creates_a_setup() {
    let mut app = SolveApp::new(Session::default(), Services::default());
    let mut d = crate::dialogs::Dialog::for_command(&app, "cam.setup.create").expect("the Setup command opens a dialog");
    if let Kind::Cam(Zcam::Setup(f)) = &mut d.kind {
        f.name = "Roughing".into();
        f.stock_x = "100 mm".into();
        f.stock_y = "80 mm".into();
        f.stock_z = "20 mm".into();
        f.offset_z = "5 mm".into();
    } else {
        panic!("cam.setup.create has the wrong dialog");
    }
    d.inputs[0].items = vec![Sel::Body { name: "Plate".into() }];
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(cmds[0].0, "cam.setup.create");
    assert_eq!(cmds[0].1["name"], "Roughing");
    assert_eq!(cmds[0].1["stock_size"], json!([100.0, 80.0, 20.0]));
    assert_eq!(cmds[0].1["stock_offset"], json!([0.0, 0.0, 5.0]));
    assert_eq!(cmds[0].1["bodies"], json!(["Plate"]));
    for (id, params) in cmds {
        app.run(&id, params).unwrap();
    }
    assert_eq!(app.session.doc.cam.setups.len(), 1);
    assert_eq!(app.session.doc.cam.setups[0].name, "Roughing");
}

/// The Toolpath tab's Contour dialog builds `cam.toolpath.contour` from its setup, tool and
/// cutting values; a field left empty is not sent, so the command keeps its default.
#[test]
fn the_contour_dialog_builds_the_operation() {
    let mut app = SolveApp::new(Session::default(), Services::default());
    app.run("cam.tool.create", json!({"name": "6 flat", "diameter": 6.0})).unwrap();
    app.run("cam.setup.create", json!({"name": "Roughing"})).unwrap();
    let mut d = crate::dialogs::Dialog::for_command(&app, "cam.toolpath.contour").expect("the Contour command opens a dialog");
    if let Kind::Cam(Zcam::Contour(f)) = &mut d.kind {
        // The design's only setup is chosen, so the operation cuts in it.
        assert_eq!((f.setup, f.tool), (1, 0));
        f.name = "Outline".into();
        f.feed_rate = "500".into();
        f.spindle_speed = "12000".into();
        f.stepover = "3 mm".into();
        f.depth_per_pass = "2 mm".into();
    } else {
        panic!("cam.toolpath.contour has the wrong dialog");
    }
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(cmds[0].0, "cam.toolpath.contour");
    assert_eq!(cmds[0].1["name"], "Outline");
    assert_eq!(cmds[0].1["tool"], app.session.doc.cam.tools[0].id);
    assert_eq!(cmds[0].1["setup"], app.session.doc.cam.setups[0].id);
    assert_eq!(cmds[0].1["feed_rate"], 500.0);
    assert_eq!(cmds[0].1["spindle_speed"], 12000.0);
    assert_eq!(cmds[0].1["stepover"], 3.0);
    assert_eq!(cmds[0].1["depth_per_pass"], 2.0);
    assert!(cmds[0].1.get("total_depth").is_none(), "an empty field takes the command's default: {}", cmds[0].1);
    assert!(cmds[0].1.get("stock_to_leave").is_none(), "{}", cmds[0].1);
}

/// The Pocket dialog sends `cam.toolpath.pocket`, and without setups it names the model origin
/// (setup 0 is left out, as the command defaults to it).
#[test]
fn the_pocket_dialog_names_the_model_origin_without_a_setup() {
    let mut app = SolveApp::new(Session::default(), Services::default());
    app.run("cam.tool.create", json!({"name": "3 flat", "diameter": 3.0})).unwrap();
    let mut d = crate::dialogs::Dialog::for_command(&app, "cam.toolpath.pocket").expect("the Pocket command opens a dialog");
    if let Kind::Cam(Zcam::Pocket(f)) = &mut d.kind {
        assert_eq!(f.setup, 0, "without setups the operation cuts from the model origin");
        f.stepover = "2.5 mm".into();
        f.total_depth = "10 mm".into();
        f.stock_to_leave = "0.2 mm".into();
    } else {
        panic!("cam.toolpath.pocket has the wrong dialog");
    }
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(cmds[0].0, "cam.toolpath.pocket");
    assert!(cmds[0].1.get("setup").is_none(), "{}", cmds[0].1);
    assert_eq!(cmds[0].1["stepover"], 2.5);
    assert_eq!(cmds[0].1["total_depth"], 10.0);
    assert_eq!(cmds[0].1["stock_to_leave"], 0.2);
}

/// The Output button asks where to save the program, then writes the G-code there; with no
/// toolpath it says so without opening a picker.
#[test]
fn the_output_button_asks_where_to_save_and_writes_the_gcode() {
    let mut app = SolveApp::new(Session::default(), Services::default());
    app.run("solid.box", json!({"length": 40, "width": 20, "height": 10, "body_name": "Plate"})).unwrap();
    let tool = app.run("cam.tool.create", json!({"diameter": 6})).unwrap()["tool"]["id"].as_u64().unwrap();
    app.run("cam.toolpath.contour", json!({"tool": tool, "total_depth": 10, "feed_rate": 800, "spindle_speed": 12000})).unwrap();
    let path = std::env::temp_dir().join(format!("zcam-ui-{}.nc", std::process::id()));
    let chosen = path.to_string_lossy().into_owned();
    let asked = Rc::new(Cell::new(0));
    let count = asked.clone();
    let answer = chosen.clone();
    app.services.pick_save = Some(Box::new(move |name, extensions| {
        assert!(name.ends_with(".nc"), "{name}");
        assert!(extensions.contains(&"nc"), "{extensions:?}");
        count.set(count.get() + 1);
        Box::pin(std::future::ready(Some(answer.clone())))
    }));
    app.start("cam.export.gcode");
    crate::file_picker::tick(&mut app, &crate::fonts::test_context());
    assert_eq!(asked.get(), 1, "the command asks where to save");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("%\n") && text.contains("G21") && text.contains("M30"), "{text}");
    assert!(app.status.as_ref().is_some_and(|(text, _, error)| text.starts_with("Exported ") && !error), "{:?}", app.status);
    let _ = std::fs::remove_file(&path);

    // A design without toolpaths reports why instead of asking for a file.
    let mut empty = SolveApp::new(Session::default(), Services::default());
    let asked = Rc::new(Cell::new(0));
    let count = asked.clone();
    empty.services.pick_save = Some(Box::new(move |_, _| {
        count.set(count.get() + 1);
        Box::pin(std::future::ready(None))
    }));
    empty.start("cam.export.gcode");
    assert_eq!(asked.get(), 0, "nothing to name, so no picker");
    assert!(empty.status.as_ref().is_some_and(|(_, _, error)| *error), "{:?}", empty.status);
}

/// A toolpath's menu offers its G-code export once it has a feed rate.
#[test]
fn a_toolpaths_menu_exports_its_gcode() {
    let mut app = SolveApp::new(Session::default(), Services::default());
    app.run("solid.box", json!({"length": 40, "width": 20, "height": 10, "body_name": "Plate"})).unwrap();
    let tool = app.run("cam.tool.create", json!({"diameter": 6})).unwrap()["tool"]["id"].as_u64().unwrap();
    let id = app.run("cam.toolpath.contour", json!({"tool": tool, "total_depth": 10})).unwrap()["toolpath"]["id"].as_u64().unwrap();
    let items = crate::cam_browser::toolpath_items(&app, id);
    let export = items.iter().find(|item| item.id == "ui.cam.export_gcode").expect("the menu exports the program");
    assert!(!export.enabled, "a toolpath without a feed rate cannot be exported: {:?}", export.tip);
    let _ = app.run("cam.export.gcode", json!({}));
    app.run("cam.toolpath.contour", json!({"tool": tool, "total_depth": 10, "feed_rate": 500})).unwrap();
    let ready = app.session.doc.cam.toolpaths.last().unwrap().id;
    assert!(crate::cam_browser::toolpath_items(&app, ready).iter().any(|item| item.id == "ui.cam.export_gcode" && item.enabled));
}

/// The Tool Library opens on a new tool, creates it, and opens again on that tool to edit it.
#[test]
fn the_tool_library_dialog_creates_and_edits_tools() {
    let mut app = SolveApp::new(Session::default(), Services::default());
    let mut d = crate::dialogs::Dialog::for_command(&app, "cam.tool.list").expect("the Tool Library command opens a dialog");
    if let Kind::Cam(Zcam::ToolLibrary(f)) = &mut d.kind {
        assert_eq!(f.editing, None, "an empty library makes a new tool");
        f.name = "6 flat".into();
        f.diameter = "6 mm".into();
    } else {
        panic!("cam.tool.list has the wrong dialog");
    }
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(cmds[0].0, "cam.tool.create");
    assert_eq!(cmds[0].1["diameter"], 6.0);
    for (id, params) in cmds {
        app.run(&id, params).unwrap();
    }
    assert_eq!(app.session.doc.cam.tools.len(), 1);

    // Reopened, the library selects the tool it has, so OK edits it.
    let mut d = crate::dialogs::Dialog::for_command(&app, "cam.tool.list").expect("the Tool Library reopens");
    if let Kind::Cam(Zcam::ToolLibrary(f)) = &mut d.kind {
        assert!(f.editing.is_some(), "the only tool is selected for editing");
        f.diameter = "8 mm".into();
    }
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(cmds[0].0, "cam.tool.edit");
    assert_eq!(cmds[0].1["tool"], app.session.doc.cam.tools[0].id);
    assert_eq!(cmds[0].1["diameter"], 8.0);
    for (id, params) in cmds {
        app.run(&id, params).unwrap();
    }
    assert_eq!(app.session.doc.cam.tools[0].diameter, 8.0);
}

/// The Coordinate System dialog sends `cam.coordinate_system.create` with the exact frame its
/// gnomon holds: the origin and three square axes.
#[test]
fn the_coordinate_system_dialog_places_the_frame() {
    let mut app = SolveApp::new(Session::default(), Services::default());
    let mut d = crate::dialogs::Dialog::for_command(&app, "cam.coordinate_system.create")
        .expect("the Coordinate System command opens a dialog");
    {
        let Kind::Cam(Zcam::CoordinateSystem(f)) = &mut d.kind else {
            panic!("cam.coordinate_system.create has the wrong dialog");
        };
        f.name = "Stock Top".into();
        assert!(f.gnomon.set_origin(Vec3::new(10.0, 20.0, 30.0)));
        assert!(f.gnomon.set_z_axis(Vec3::new(0.0, 0.0, 2.0)));
        assert!(f.gnomon.set_x_axis(Vec3::new(0.0, 1.0, 0.0)));
    }
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(cmds[0].0, "cam.coordinate_system.create");
    assert_eq!(cmds[0].1["name"], "Stock Top");
    assert_eq!(cmds[0].1["origin"], json!([10.0, 20.0, 30.0]));
    assert_eq!(cmds[0].1["x_axis"], json!([0.0, 1.0, 0.0]));
    assert_eq!(cmds[0].1["y_axis"], json!([-1.0, 0.0, 0.0]), "Y is Z × X");
    assert_eq!(cmds[0].1["z_axis"], json!([0.0, 0.0, 1.0]));
    for (id, params) in cmds {
        app.run(&id, params).unwrap();
    }
    let cs = &app.session.doc.coordinate_systems[0];
    assert_eq!(cs.name, "Stock Top");
    assert_eq!(cs.origin, [10.0, 20.0, 30.0]);
}

/// A picked planar face gives the frame its Z, a picked straight edge its X, and a picked point
/// its origin; in Free mode a pick moves nothing.
#[test]
fn a_picked_face_or_edge_aligns_the_frame() {
    let mut app = SolveApp::new(Session::default(), Services::default());
    app.run("solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "Block"})).unwrap();
    let body = app.session.world_state().unwrap().bodies[0].name.clone();
    let bottom = (0..64).find(|&i| super::planar_normal(&app, &body, i).is_some_and(|n| n.z < -0.999)).expect("the box's bottom face");
    let edge = {
        let st = app.session.world_state().unwrap();
        let b = st.body(&body).unwrap();
        (0..b.body.record().edges.len())
            .find(|&i| {
                matches!(&b.body.record().edges[i].curve, solidcore_record::brep::Curve::Line { start, end }
                    if (end[0] - start[0]).abs() < 1e-9
                        && (end[2] - start[2]).abs() < 1e-9
                        && (end[1] - start[1]).abs() > 1e-9)
            })
            .expect("an edge along Y")
    };
    let mut d = crate::dialogs::Dialog::for_command(&app, "cam.coordinate_system.create").expect("the dialog opens");
    let Kind::Cam(Zcam::CoordinateSystem(f)) = &mut d.kind else { panic!("the dialog's kind") };
    let face = Sel::Face { body: body.clone(), index: bottom, point: Vec3::new(0.0, 0.0, 0.0) };
    assert!(!super::align_to(&app, f, &face), "Free mode takes no pick");
    f.gnomon.set_align_mode(GnomonAlignMode::AlignZFace);
    assert!(super::align_to(&app, f, &face));
    assert!(f.gnomon.z_axis().vec().dist(Vec3::new(0.0, 0.0, -1.0)) < 1e-9, "{:?}", f.gnomon.z_axis().vec());
    f.gnomon.set_align_mode(GnomonAlignMode::AlignXEdge);
    assert!(super::align_to(&app, f, &Sel::Edge { body: body.clone(), index: edge, point: Vec3::ZERO }));
    assert!((f.gnomon.x_axis().vec().y.abs() - 1.0).abs() < 1e-9, "{:?}", f.gnomon.x_axis().vec());
    f.gnomon.set_align_mode(GnomonAlignMode::SnapOrigin);
    let at = Vec3::new(5.0, 6.0, 0.0);
    assert!(super::align_to(&app, f, &Sel::Vertex { body, point: at }));
    assert_eq!(f.gnomon.origin(), at);
}

/// #Z-CAM: a construction plane converts to a coordinate system: its origin, its own X and Y (so
/// Z is the plane's normal) and its name go into the frame, with Origin Snap on.
#[test]
fn a_construction_plane_seeds_a_coordinate_system() {
    let mut app = SolveApp::new(Session::default(), Services::default());
    app.run("construct.plane.offset", json!({"base": "XY", "offset": 20, "name": "Shelf"})).unwrap();
    let feature = app.session.doc.features.iter().find(|f| f.name == "Shelf").map(|f| f.id).expect("the plane");
    crate::browser::coordinate_system_from_plane(&mut app, feature);
    let d = app.dialog.as_ref().expect("the coordinate system dialog opens");
    let Kind::Cam(Zcam::CoordinateSystem(f)) = &d.kind else { panic!("the coordinate system dialog") };
    assert_eq!(f.name, "Shelf");
    assert_eq!(f.gnomon.align_mode(), GnomonAlignMode::SnapOrigin, "Origin Snap is on");
    assert!((f.gnomon.origin().z - 20.0).abs() < 1e-9, "{:?}", f.gnomon.origin());
    assert!(f.gnomon.z_axis().vec().dist(Vec3::Z) < 1e-9, "{:?}", f.gnomon.z_axis().vec());
    assert!(f.gnomon.x_axis().vec().dist(Vec3::X) < 1e-9, "{:?}", f.gnomon.x_axis().vec());
    let cmds = apply_commands(&app, d).unwrap();
    assert_eq!(cmds[0].0, "cam.coordinate_system.create");
    assert_eq!(cmds[0].1["origin"], json!([0.0, 0.0, 20.0]));
    assert_eq!(cmds[0].1["name"], "Shelf");
}

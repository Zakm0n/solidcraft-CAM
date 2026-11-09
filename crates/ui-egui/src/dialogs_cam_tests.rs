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

// The gnomon's click modes, its snaps and its value entry: the frame a click on its origin ball,
// a cone, an arm's shaft or a wheel opens, and what its pick then does.

use crate::dynamic_gnomon::{Axis, DynamicGnomon, GnomonHandle, InputState};
use solidcraft_engine::render::Camera;

/// A view where the frame's three arms, cones and wheels stand apart on screen, so a test can
/// click each of them.
fn gnomon_projection() -> crate::viewport::Proj {
    let cam = Camera::default();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0));
    // The same view and matrix the viewport makes, so a screen point and its ray agree: a drag
    // along an arm must move the frame along that arm.
    let vp = cam.view_proj(rect.width() as f64 / rect.height() as f64, 100.0);
    crate::viewport::Proj { cam, vp, rect }
}

fn origin_on_screen(g: &DynamicGnomon, proj: &crate::viewport::Proj) -> egui::Pos2 {
    proj.to_screen(g.origin()).expect("the frame's origin is in view")
}

/// The screen direction of model direction `d`, from the frame's origin.
fn direction_on_screen(g: &DynamicGnomon, proj: &crate::viewport::Proj, d: Vec3) -> egui::Vec2 {
    let o = g.origin();
    let from = origin_on_screen(g, proj);
    let to = proj.to_screen(o + d * 0.05).expect("the arm's far end is in view");
    (to - from).normalized()
}

/// The farthest point on `axis`'s arm that takes `want`: a cone, or the shaft beneath it.
fn on_arm(g: &DynamicGnomon, proj: &crate::viewport::Proj, axis: Axis, want: GnomonHandle) -> egui::Pos2 {
    let centre = origin_on_screen(g, proj);
    let d = direction_on_screen(g, proj, g.axes()[axis.index()]);
    (1..=250)
        .rev()
        .map(|r| centre + d * r as f32)
        .find(|p| g.handle_at(proj, *p) == want)
        .unwrap_or_else(|| panic!("no {want:?} on the {axis:?} arm"))
}

/// A point on the wheel about `axis`: a direction square to it where no arm runs.
fn on_wheel(g: &DynamicGnomon, proj: &crate::viewport::Proj, axis: Axis) -> egui::Pos2 {
    let frame = g.axes();
    let (i1, i2) = match axis {
        Axis::X => (1, 2),
        Axis::Y => (2, 0),
        Axis::Z => (0, 1),
    };
    let centre = origin_on_screen(g, proj);
    let d = (direction_on_screen(g, proj, frame[i1]) + direction_on_screen(g, proj, frame[i2])).normalized();
    (1..=250)
        .map(|r| centre + d * r as f32)
        .find(|p| g.handle_at(proj, *p) == GnomonHandle::Rotate(axis))
        .unwrap_or_else(|| panic!("no {axis:?} wheel in view"))
}

fn press(g: &mut DynamicGnomon, proj: &crate::viewport::Proj, at: egui::Pos2) {
    g.interact(proj, &InputState { pointer: Some(at), primary_pressed: true, primary_down: true, primary_released: false, escape: false });
}

fn release(g: &mut DynamicGnomon, proj: &crate::viewport::Proj, at: egui::Pos2) {
    g.interact(proj, &InputState { pointer: Some(at), primary_pressed: false, primary_down: false, primary_released: true, escape: false });
}

/// A click on the ball, a cone, a shaft or a wheel with the pointer held still.
fn click(g: &mut DynamicGnomon, proj: &crate::viewport::Proj, at: egui::Pos2) {
    press(g, proj, at);
    release(g, proj, at);
}

/// #Z-CAM: clicking the gnomon's origin ball opens Origin Snap Mode, its cones Axis Orbit Mode,
/// and its shafts and wheels their value entries; a drag that moves places the frame and opens
/// no mode.
#[test]
fn a_click_on_the_gnomon_opens_its_mode_or_its_value_entry() {
    let proj = gnomon_projection();
    let mut g = DynamicGnomon::default();
    let centre = origin_on_screen(&g, &proj);
    click(&mut g, &proj, centre);
    assert_eq!(g.active_state(), GnomonActiveState::OriginSnap, "the ball opens Origin Snap Mode");
    let cone = on_arm(&g, &proj, Axis::X, GnomonHandle::Cone(Axis::X));
    click(&mut g, &proj, cone);
    assert_eq!(g.active_state(), GnomonActiveState::AxisOrbit(Axis::X), "a cone opens Axis Orbit Mode");
    let shaft = on_arm(&g, &proj, Axis::Y, GnomonHandle::Translate(Axis::Y));
    click(&mut g, &proj, shaft);
    assert_eq!(g.active_state(), GnomonActiveState::RelativeInput { handle: GnomonHandle::Translate(Axis::Y), pos: shaft });
    let wheel = on_wheel(&g, &proj, Axis::Z);
    click(&mut g, &proj, wheel);
    assert_eq!(g.active_state(), GnomonActiveState::RelativeInput { handle: GnomonHandle::Rotate(Axis::Z), pos: wheel });
    // A drag along the shaft moves the frame, so it asks for no value.
    press(&mut g, &proj, shaft);
    let moved = shaft + direction_on_screen(&g, &proj, Vec3::Y) * 20.0;
    g.interact(&proj, &InputState { pointer: Some(moved), primary_pressed: false, primary_down: true, primary_released: false, escape: false });
    release(&mut g, &proj, moved);
    assert_eq!(g.active_state(), GnomonActiveState::Idle, "a drag opens no mode");
    assert!(
        g.origin().x.abs() < 1e-9 && g.origin().z.abs() < 1e-9 && g.origin().y.abs() > 0.01,
        "the drag moves the frame along its arm only: {:?}",
        g.origin()
    );
    // Escape ends a mode.
    let centre = origin_on_screen(&g, &proj);
    click(&mut g, &proj, centre);
    assert_eq!(g.active_state(), GnomonActiveState::OriginSnap);
    g.interact(&proj, &InputState { escape: true, ..InputState::default() });
    assert_eq!(g.active_state(), GnomonActiveState::Idle, "Escape ends the click mode");
}

/// #Z-CAM: Axis Orbit turns the clicked axis to the picked point, about the short way round, and
/// keeps the frame square.
#[test]
fn an_axis_orbit_turns_the_axis_to_the_point() {
    let mut g = DynamicGnomon::default();
    assert!(g.aim_axis(Axis::Y, Vec3::new(0.0, -3.0, 4.0)));
    assert!(g.y_axis().vec().dist(Vec3::new(0.0, -0.6, 0.8)) < 1e-9, "{:?}", g.y_axis().vec());
    let a = g.axes();
    assert!(a[0].dot(a[1]).abs() < 1e-9 && a[1].dot(a[2]).abs() < 1e-9, "the frame stays square: {a:?}");
    assert!(!g.aim_axis(Axis::X, g.origin()), "the frame's own origin names no direction");
}

/// A box 10 mm on a side, and its body's name.
fn block() -> (SolveApp, String) {
    let mut app = SolveApp::new(Session::default(), Services::default());
    app.run("solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "Block"})).unwrap();
    let body = app.session.world_state().unwrap().bodies[0].name.clone();
    (app, body)
}

/// #Z-CAM: a snap uses exact geometry: a face's centroid and outward normal, a straight edge's
/// midpoint, and the centre of the circle a circular edge lies on.
#[test]
fn a_snap_uses_the_centroid_midpoint_and_circle_centre() {
    let (app, body) = block();
    let st = app.session.world_state().unwrap();
    let faces = st.body(&body).unwrap().body.faces(0.01).unwrap();
    let top = faces.iter().find(|f| f.plane_normal.is_some_and(|n| n.z > 0.999)).expect("the box's top face").index;
    let snap = super::snap_target(&app, &Sel::Face { body: body.clone(), index: top, point: Vec3::new(1.0, 2.0, 10.0) }).expect("a face snaps");
    assert!(snap.point.dist(Vec3::new(5.0, 5.0, 10.0)) < 1e-6, "the face's centroid: {:?}", snap.point);
    assert_eq!(snap.normal, Some(Vec3::Z), "a planar face gives its outward normal");
    let st = app.session.world_state().unwrap();
    let b = st.body(&body).unwrap();
    let (edge, mid) = (0..b.body.record().edges.len())
        .find_map(|i| match &b.body.record().edges[i].curve {
            solidcore_record::brep::Curve::Line { start, end } => {
                let mid = (Vec3::new(start[0], start[1], start[2]) + Vec3::new(end[0], end[1], end[2])) * 0.5;
                Some((i, mid))
            }
            _ => None,
        })
        .expect("a straight edge");
    let snap = super::snap_target(&app, &Sel::Edge { body: body.clone(), index: edge, point: Vec3::ZERO }).expect("an edge snaps");
    assert!(snap.point.dist(mid) < 1e-9, "a straight edge snaps to its midpoint: {:?} for {mid:?}", snap.point);
    assert_eq!(snap.normal, None, "an edge gives no normal");
    // A cylinder's rim is a circle: a pick on it snaps to the circle's centre.
    let mut app = app;
    app.run("solid.cylinder", json!({"base": [30, 0, 0], "axis": [0, 0, 1], "radius": 4, "height": 10, "body_name": "Pin"})).unwrap();
    let st = app.session.world_state().unwrap();
    let pin = st.body("Pin").expect("the pin's body");
    let (circular, centre) = (0..pin.body.record().edges.len())
        .find_map(|i| Some((i, solidcraft_engine::kernel::edge_circle(&pin.body, i)?.center)))
        .expect("a circular edge");
    let snap = super::snap_target(&app, &Sel::Edge { body: "Pin".into(), index: circular, point: Vec3::ZERO }).expect("a circular edge snaps");
    assert!(snap.point.dist(centre) < 1e-9, "a circular edge snaps to its circle's centre: {:?} for {centre:?}", snap.point);
}

/// #Z-CAM: the origin ball's click turns Origin Snap on, and its pick puts the origin on the
/// face's centroid and the frame's Z on the face's outward normal, once.
#[test]
fn an_origin_snap_centres_a_face_and_turns_its_z_to_the_normal() {
    let (app, body) = block();
    let st = app.session.world_state().unwrap();
    let top = st
        .body(&body)
        .unwrap()
        .body
        .faces(0.01)
        .unwrap()
        .into_iter()
        .find(|f| f.plane_normal.is_some_and(|n| n.z > 0.999))
        .expect("the box's top face")
        .index;
    let mut d = crate::dialogs::Dialog::for_command(&app, "cam.coordinate_system.create").expect("the dialog opens");
    let Kind::Cam(Zcam::CoordinateSystem(f)) = &mut d.kind else { panic!("the dialog's kind") };
    f.gnomon.set_active_state(GnomonActiveState::OriginSnap);
    f.follow_click();
    assert_eq!(f.gnomon.align_mode(), GnomonAlignMode::SnapOrigin, "the ball's click turns Origin Snap on");
    assert!(f.take_pick(&app, &Sel::Face { body, index: top, point: Vec3::new(1.0, 2.0, 10.0) }));
    assert!(f.gnomon.origin().dist(Vec3::new(5.0, 5.0, 10.0)) < 1e-6, "{:?}", f.gnomon.origin());
    assert!(f.gnomon.z_axis().vec().dist(Vec3::Z) < 1e-9, "the face's outward normal, not its inward one: {:?}", f.gnomon.z_axis().vec());
    assert_eq!(f.gnomon.active_state(), GnomonActiveState::Idle, "one pick per click");
}

/// #Z-CAM: the value entry a shaft's or a wheel's click opens moves the frame a relative distance
/// along the arm, or turns it a relative angle about the wheel.
#[test]
fn the_value_entry_moves_the_frame_or_turns_it() {
    let app = SolveApp::new(Session::default(), Services::default());
    let mut d = crate::dialogs::Dialog::for_command(&app, "cam.coordinate_system.create").expect("the dialog opens");
    let Kind::Cam(Zcam::CoordinateSystem(f)) = &mut d.kind else { panic!("the dialog's kind") };
    f.entry = Some(Box::new(RelativeEntry { handle: GnomonHandle::Translate(Axis::X), text: "5".into(), focus: false }));
    f.apply_entry(&app);
    assert_eq!(f.gnomon.origin(), Vec3::new(5.0, 0.0, 0.0), "5 mm along +X");
    assert!(f.entry.is_none(), "the entry closes once its value is taken");
    f.entry = Some(Box::new(RelativeEntry { handle: GnomonHandle::Rotate(Axis::Z), text: "90 deg".into(), focus: false }));
    f.apply_entry(&app);
    assert!(f.gnomon.x_axis().vec().dist(Vec3::Y) < 1e-9, "+X turns to +Y: {:?}", f.gnomon.x_axis().vec());
    assert_eq!(f.gnomon.origin(), Vec3::new(5.0, 0.0, 0.0), "turning moves no origin");
    // A value that is not a number keeps the entry open.
    f.entry = Some(Box::new(RelativeEntry { handle: GnomonHandle::Translate(Axis::Y), text: "not a number".into(), focus: false }));
    f.apply_entry(&app);
    assert!(f.entry.is_some(), "an unreadable value keeps the entry open");
    assert_eq!(f.gnomon.origin(), Vec3::new(5.0, 0.0, 0.0));
}

/// #Z-CAM: the floating value entry opens empty with the keyboard in its field, and a press
/// inside it belongs to its field, so it stays open; only a press outside it ends it (Enter and
/// Escape are handled by the field itself).
#[test]
fn a_click_inside_the_value_entry_leaves_it_open() {
    let app = SolveApp::new(Session::default(), Services::default());
    let mut d = crate::dialogs::Dialog::for_command(&app, "cam.coordinate_system.create").expect("the dialog opens");
    let Kind::Cam(Zcam::CoordinateSystem(f)) = &mut d.kind else { panic!("the dialog's kind") };
    let proj = gnomon_projection();
    let shaft = on_arm(&f.gnomon, &proj, Axis::Y, GnomonHandle::Translate(Axis::Y));
    click(&mut f.gnomon, &proj, shaft);
    f.follow_click();
    let entry = f.entry.as_ref().expect("a shaft's click opens its value entry");
    assert!(entry.text.is_empty(), "the entry starts empty: {:?}", entry.text);
    assert!(entry.focus, "the entry's field asks for the keyboard as it opens");
    // A press inside the entry's own rectangle belongs to its field, not to the gnomon or a pick.
    let rect = egui::Rect::from_min_size(shaft + egui::vec2(8.0, 8.0), egui::vec2(90.0, 28.0));
    f.entry_rect = Some(rect);
    f.gnomon.set_entry_rect(f.entry_rect);
    let inside = rect.center();
    f.gnomon.interact(&proj, &InputState { pointer: Some(inside), primary_pressed: true, primary_down: true, ..InputState::default() });
    f.gnomon.interact(&proj, &InputState { pointer: Some(inside), primary_released: true, ..InputState::default() });
    assert!(matches!(f.gnomon.active_state(), GnomonActiveState::RelativeInput { .. }), "a press inside the entry keeps it open");
    f.follow_click();
    assert!(f.entry.is_some(), "the entry stays open");
    // A press outside the entry ends it.
    let outside = proj.rect.left_top() + egui::vec2(5.0, 5.0);
    f.gnomon.interact(&proj, &InputState { pointer: Some(outside), primary_pressed: true, primary_down: true, ..InputState::default() });
    f.follow_click();
    assert_eq!(f.gnomon.active_state(), GnomonActiveState::Idle, "a click outside the entry drops it");
    assert!(f.entry.is_none(), "the entry closes with its mode");
}

/// #Z-CAM: Enter and Escape typed into the gnomon's floating value entry are the entry's own:
/// they apply or drop its value, and the coordinate system dialog they float over neither
/// commits on Enter nor closes on Escape.
#[test]
fn the_value_entry_keeps_enter_and_escape_from_the_dialog() {
    let mut app = SolveApp::new(Session::default(), Services::default());
    app.home.open = false;
    let dialog = crate::dialogs::Dialog::for_command(&app, "cam.coordinate_system.create").expect("the dialog opens");
    app.dialog = Some(dialog);
    let proj = gnomon_projection();
    let ctx = crate::fonts::test_context();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0));
    let frame = |app: &mut SolveApp, events: Vec<egui::Event>| {
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        crate::test_support::frame(&ctx, input, |ui| {
            // The real frame's order: the global shortcuts, the view's gnomon, then the dialog.
            crate::toolbar::shortcuts(app, &ctx);
            let painter = ui.painter().clone();
            if let Some(mut d) = app.dialog.take() {
                if matches!(d.kind, Kind::Cam(Zcam::CoordinateSystem(_))) {
                    overlay(app, ui, &painter, &proj, &mut d);
                }
                app.dialog = Some(d);
            }
            crate::dialogs::show(app, ui.ctx());
        })
    };
    let open_entry = |app: &mut SolveApp, text: &str| {
        let mut d = app.dialog.take().expect("the dialog is open");
        let Kind::Cam(Zcam::CoordinateSystem(f)) = &mut d.kind else { panic!("the dialog's kind") };
        // A rect from an earlier entry must not swallow this press.
        f.gnomon.set_entry_rect(None);
        let shaft = on_arm(&f.gnomon, &proj, Axis::Y, GnomonHandle::Translate(Axis::Y));
        click(&mut f.gnomon, &proj, shaft);
        f.follow_click();
        f.entry.as_mut().expect("a shaft's click opens its value entry").text = text.into();
        app.dialog = Some(d);
    };
    // The dialog's own first frames give its form the keyboard; the entry opens after that.
    frame(&mut app, Vec::new());
    frame(&mut app, Vec::new());
    open_entry(&mut app, "5");
    // The entry's field asks for the keyboard as it opens; these frames put the keyboard in it.
    frame(&mut app, Vec::new());
    frame(&mut app, Vec::new());
    assert_eq!(ctx.memory(|m| m.focused()), Some(entry_field_id()), "the entry's field holds the keyboard");
    // Enter applies 5 mm along +Y and leaves the dialog open.
    frame(&mut app, crate::test_support::key(egui::Key::Enter));
    {
        let d = app.dialog.as_ref().expect("Enter in the entry does not submit the dialog");
        let Kind::Cam(Zcam::CoordinateSystem(f)) = &d.kind else { panic!("the dialog's kind") };
        assert!(f.entry.is_none(), "Enter applies the entry's value");
        assert!((f.gnomon.origin().y - 5.0).abs() < 1e-9, "5 mm along +Y: {:?}", f.gnomon.origin());
    }
    // Escape drops the entry without turning the frame and without closing the dialog.
    open_entry(&mut app, "9");
    frame(&mut app, Vec::new());
    frame(&mut app, Vec::new());
    frame(&mut app, crate::test_support::key(egui::Key::Escape));
    assert!(app.dialog.is_some(), "Escape in the entry does not cancel the dialog");
    assert!(!app.esc_handled, "the dialog never saw the entry's Escape");
    let d = app.dialog.as_ref().expect("the dialog stays open");
    let Kind::Cam(Zcam::CoordinateSystem(f)) = &d.kind else { panic!("the dialog's kind") };
    assert!(f.entry.is_none(), "Escape drops the entry");
    assert!((f.gnomon.origin().y - 5.0).abs() < 1e-9, "Escape moves nothing: {:?}", f.gnomon.origin());
}

/// #Z-CAM: the gnomon's handles are strictly separated: walking out along an arm, the ball takes
/// the presses nearest the origin, the shaft the line between, and the cone only the arrowhead at
/// the tip, with no region of one inside another.
#[test]
fn the_ball_the_shaft_and_the_cone_never_steal_each_others_clicks() {
    let proj = gnomon_projection();
    let g = DynamicGnomon::default();
    let centre = origin_on_screen(&g, &proj);
    let d = direction_on_screen(&g, &proj, Vec3::X);
    let walked: Vec<(f32, GnomonHandle)> = (0..=110).map(|r| (r as f32, g.handle_at(&proj, centre + d * r as f32))).collect();
    let first = |want: GnomonHandle| walked.iter().position(|(_, h)| *h == want);
    let (origin, shaft, cone) = (first(GnomonHandle::Origin), first(GnomonHandle::Translate(Axis::X)), first(GnomonHandle::Cone(Axis::X)));
    let (Some(origin), Some(shaft), Some(cone)) = (origin, shaft, cone) else { panic!("the arm shows a ball, a shaft and a cone: {walked:?}") };
    assert!(origin < shaft && shaft < cone, "the regions run origin, shaft, cone: {walked:?}");
    assert!(walked[..origin].iter().all(|(_, h)| *h == GnomonHandle::None), "nothing comes before the ball: {walked:?}");
    // Past the first cone point the shaft never returns: the arrowhead owns its tip outright.
    assert!(
        !walked[cone..].iter().any(|(_, h)| *h == GnomonHandle::Translate(Axis::X)),
        "no shaft press inside the arrowhead: {walked:?}"
    );
    assert!(walked[cone..].iter().any(|(_, h)| *h == GnomonHandle::Cone(Axis::X)), "the arrowhead takes its tip");
}

/// #Z-CAM: a click on the handle that opened a snap mode drops it again, so the cone's Axis
/// Orbit and the ball's Origin Snap toggle on and off; a click in the empty view also drops one.
#[test]
fn a_snap_mode_toggles_on_and_off() {
    let proj = gnomon_projection();
    let mut g = DynamicGnomon::default();
    let centre = origin_on_screen(&g, &proj);
    click(&mut g, &proj, centre);
    assert_eq!(g.active_state(), GnomonActiveState::OriginSnap);
    click(&mut g, &proj, centre);
    assert_eq!(g.active_state(), GnomonActiveState::Idle, "a second click on the ball drops Origin Snap");
    let cone = on_arm(&g, &proj, Axis::X, GnomonHandle::Cone(Axis::X));
    click(&mut g, &proj, cone);
    assert_eq!(g.active_state(), GnomonActiveState::AxisOrbit(Axis::X));
    click(&mut g, &proj, cone);
    assert_eq!(g.active_state(), GnomonActiveState::Idle, "a second click on the cone drops Axis Orbit");
    // Another cone switches the axis rather than dropping the mode.
    click(&mut g, &proj, cone);
    let other = on_arm(&g, &proj, Axis::Z, GnomonHandle::Cone(Axis::Z));
    click(&mut g, &proj, other);
    assert_eq!(g.active_state(), GnomonActiveState::AxisOrbit(Axis::Z), "another cone orbits another axis");
    // A click in the empty view drops the mode.
    let empty = proj.rect.left_top() + egui::vec2(5.0, 5.0);
    g.interact(&proj, &InputState { pointer: Some(empty), primary_pressed: true, primary_down: true, ..InputState::default() });
    assert_eq!(g.active_state(), GnomonActiveState::Idle, "a click in the view drops the snap mode");
}

/// #Z-CAM: the frame tells the viewport when the marquee must not start: a handle is hovered, a
/// drag is in progress, or a snap mode waits for its pick.
#[test]
fn the_gnomon_blocks_the_viewport_marquee() {
    let proj = gnomon_projection();
    let mut g = DynamicGnomon::default();
    let empty = proj.rect.left_top() + egui::vec2(5.0, 5.0);
    assert!(!g.blocks_viewport_marquee(), "a frame the pointer is away from blocks nothing");
    let shaft = on_arm(&g, &proj, Axis::Y, GnomonHandle::Translate(Axis::Y));
    g.interact(&proj, &InputState { pointer: Some(shaft), ..InputState::default() });
    assert!(g.blocks_viewport_marquee(), "a hovered handle blocks the marquee");
    g.interact(&proj, &InputState { pointer: Some(empty), ..InputState::default() });
    assert!(!g.blocks_viewport_marquee(), "the empty view leaves the marquee to the viewport");
    let centre = origin_on_screen(&g, &proj);
    click(&mut g, &proj, centre);
    g.interact(&proj, &InputState { pointer: Some(empty), ..InputState::default() });
    assert_eq!(g.active_state(), GnomonActiveState::OriginSnap);
    assert!(g.blocks_viewport_marquee(), "a snap mode waiting for its pick blocks the marquee");
    g.interact(&proj, &InputState { escape: true, ..InputState::default() });
    // The snap mode followed the pointer to the view's corner; put the frame back in view so its
    // arm can be pressed as before.
    let _ = g.set_origin(Vec3::ZERO);
    let shaft = on_arm(&g, &proj, Axis::Y, GnomonHandle::Translate(Axis::Y));
    press(&mut g, &proj, shaft);
    assert!(g.blocks_viewport_marquee(), "a drag blocks the marquee");
}

// Live snapping: while Origin Snap or Axis Orbit is on, the frame follows the pointer on hover,
// with no drag needed: it takes a candidate within 20 screen points, and otherwise unprojects the
// pointer onto the view plane through where the mode began.

use crate::dynamic_gnomon::{Change, SnapCandidate};

/// A view zoomed in on a 10 mm block, so each vertex and face centre stands well apart on screen
/// and a 20-point snap reach can take exactly one of them. The angle is off the standard iso one,
/// where points on a diagonal of the block project onto each other.
fn live_projection() -> crate::viewport::Proj {
    let cam = Camera { target: Vec3::new(5.0, 5.0, 5.0), distance: 24.0, yaw: 0.62, pitch: 0.48, ..Camera::default() };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0));
    let vp = cam.view_proj(rect.width() as f64 / rect.height() as f64, 24.0);
    crate::viewport::Proj { cam, vp, rect }
}

/// A pointer hovering `at`, with no button down: what a live snap reads each frame.
fn hover(g: &mut DynamicGnomon, proj: &crate::viewport::Proj, at: egui::Pos2) -> Option<Change> {
    g.interact(proj, &InputState { pointer: Some(at), ..InputState::default() })
}

/// #Z-CAM: Origin Snap follows the pointer on hover: a candidate within reach takes the frame's
/// origin exactly, a planar face's centre turns its Z to the face's outward normal with the other
/// axes kept square (Gram-Schmidt), and a pointer away from every candidate is unprojected onto
/// the view plane through where the mode began.
#[test]
fn a_live_origin_snap_takes_the_point_under_the_pointer() {
    let (app, _body) = block();
    let candidates = super::snap_candidates(&app);
    let face = *candidates
        .iter()
        .find(|c| c.normal.is_some_and(|n| n.dist(Vec3::X) < 1e-6) && c.point.dist(Vec3::new(10.0, 5.0, 5.0)) < 1e-6)
        .expect("the box's +X face is a candidate, with its outward normal");
    let proj = live_projection();
    let at = proj.to_screen(face.point).expect("the face is in view");
    assert!(proj.rect.contains(at), "the face shows in the view: {at:?}");
    let mut g = DynamicGnomon::new(Vec3::ZERO, Vec3::X, Vec3::Z).expect("a frame at the origin");
    g.set_snap_candidates(candidates);
    g.set_active_state(GnomonActiveState::OriginSnap);
    // A hover within the 20-point reach takes the face's centre, though the pointer is off it.
    assert_eq!(hover(&mut g, &proj, at + egui::vec2(8.0, -6.0)), Some(Change::Frame), "the hover moves the frame");
    assert!(g.origin().dist(face.point) < 1e-9, "the face's exact centre: {:?}", g.origin());
    assert!(g.z_axis().vec().dist(Vec3::X) < 1e-9, "the face's outward normal: {:?}", g.z_axis().vec());
    assert_eq!(g.snap_mark(), Some(at), "the mark sits on the point it took");
    let a = g.axes();
    assert!(a[0].dot(a[1]).abs() < 1e-9 && a[1].dot(a[2]).abs() < 1e-9 && a[2].dot(a[0]).abs() < 1e-9, "the frame stays square: {a:?}");
    assert!((a[0].cross(a[1]).dot(a[2]) - 1.0).abs() < 1e-9, "the frame stays right-handed: {a:?}");
    // Away from every candidate the frame follows the pointer on the plane facing the camera
    // through where the mode began, keeping the axes it began with.
    let empty = proj.rect.left_top() + egui::vec2(60.0, 60.0);
    assert_eq!(hover(&mut g, &proj, empty), Some(Change::Frame));
    assert_eq!(g.snap_mark(), None, "no candidate, no mark");
    let back = proj.to_screen(g.origin()).expect("the origin stays in view");
    assert!(back.distance(empty) < 1e-3, "the frame follows the pointer: {back:?} for {empty:?}");
    assert!(g.z_axis().vec().dist(Vec3::Z) < 1e-9, "a point away from a face keeps the frame's Z: {:?}", g.z_axis().vec());
    // A press takes that place and ends the mode: the frame stays where the snap put it.
    let kept = g.origin();
    g.interact(&proj, &InputState { pointer: Some(empty), primary_pressed: true, primary_down: true, ..InputState::default() });
    assert_eq!(g.active_state(), GnomonActiveState::Idle, "a click commits and ends the mode");
    assert_eq!(g.origin(), kept, "the commit keeps the place the snap found");
    assert_eq!(g.snap_mark(), None, "the mark goes with the mode");
}

/// #Z-CAM: Axis Orbit follows the pointer on hover: the orbited axis points at the candidate
/// within reach (or at the unprojected pointer), the origin stays put, and the other two axes stay
/// rigidly square to it from the frame the mode began with.
#[test]
fn a_live_axis_orbit_turns_the_axis_towards_the_hovered_point() {
    let proj = live_projection();
    let target = Vec3::new(0.0, -3.0, 4.0);
    let mut g = DynamicGnomon::new(Vec3::ZERO, Vec3::X, Vec3::Z).expect("a frame at the origin");
    g.set_snap_candidates(vec![SnapCandidate { point: target, normal: None }]);
    g.set_active_state(GnomonActiveState::AxisOrbit(Axis::Y));
    let at = proj.to_screen(target).expect("the point is in view");
    assert_eq!(hover(&mut g, &proj, at + egui::vec2(6.0, 6.0)), Some(Change::Frame), "the hover turns the frame");
    let want = target.normalized().expect("a direction");
    assert!(g.y_axis().vec().dist(want) < 1e-9, "the axis points at the point: {:?} for {want:?}", g.y_axis().vec());
    assert_eq!(g.origin(), Vec3::ZERO, "Axis Orbit keeps the origin");
    let a = g.axes();
    assert!(a[0].dot(a[1]).abs() < 1e-9 && a[1].dot(a[2]).abs() < 1e-9 && a[2].dot(a[0]).abs() < 1e-9, "the frame stays square: {a:?}");
    assert!((a[0].cross(a[1]).dot(a[2]) - 1.0).abs() < 1e-9, "the frame stays right-handed: {a:?}");
    assert!(a[0].dist(Vec3::X) < 1e-9, "the axis the mode did not turn keeps its direction: {:?}", a[0]);
    // Away from a candidate the axis follows the pointer, the origin still locked.
    let away = proj.rect.center() + egui::vec2(240.0, 120.0);
    assert_eq!(hover(&mut g, &proj, away), Some(Change::Frame));
    assert_eq!(g.origin(), Vec3::ZERO);
    assert_eq!(g.snap_mark(), None);
    let centre = proj.to_screen(Vec3::ZERO).expect("the origin is in view");
    let tip = proj.to_screen(g.origin() + g.y_axis().vec() * 40.0).expect("the axis tip is in view");
    assert!(((tip - centre).normalized() - (away - centre).normalized()).length() < 1e-3, "the axis points at the pointer: {tip:?} for {away:?}");
    // The frame the mode began from is what the other axes are taken from, so a second frame at
    // the same pointer does not creep.
    let axes = g.axes();
    hover(&mut g, &proj, away);
    assert!(g.axes()[0].dist(axes[0]) < 1e-12 && g.axes()[2].dist(axes[2]) < 1e-12, "the same pointer gives the same frame");
}

/// #Z-CAM: the geometry a live snap takes is the B-rep's own: a box offers its eight vertices, its
/// twelve straight edges' midpoints and its six faces' centroids, the planar faces with their
/// outward normals.
#[test]
fn live_snap_candidates_hold_the_vertices_midpoints_and_face_centres() {
    let (app, _body) = block();
    let candidates = super::snap_candidates(&app);
    let there = |p: Vec3| candidates.iter().any(|c| c.point.dist(p) < 1e-6);
    let corners: Vec<Vec3> = [0.0, 10.0].into_iter().flat_map(|x| [0.0, 10.0].into_iter().flat_map(move |y| [0.0, 10.0].map(move |z| Vec3::new(x, y, z)))).collect();
    assert_eq!(corners.len(), 8);
    assert!(corners.iter().all(|p| there(*p)), "every vertex is a candidate");
    let midpoints = [Vec3::new(5.0, 0.0, 0.0), Vec3::new(5.0, 10.0, 0.0), Vec3::new(5.0, 0.0, 10.0), Vec3::new(5.0, 10.0, 10.0), Vec3::new(0.0, 5.0, 0.0), Vec3::new(10.0, 5.0, 0.0), Vec3::new(0.0, 5.0, 10.0), Vec3::new(10.0, 5.0, 10.0), Vec3::new(0.0, 0.0, 5.0), Vec3::new(10.0, 0.0, 5.0), Vec3::new(0.0, 10.0, 5.0), Vec3::new(10.0, 10.0, 5.0)];
    assert!(midpoints.iter().all(|p| there(*p)), "every edge's midpoint is a candidate");
    let faces = [(Vec3::new(10.0, 5.0, 5.0), Vec3::X), (Vec3::new(5.0, 10.0, 5.0), Vec3::Y), (Vec3::new(5.0, 5.0, 10.0), Vec3::Z)];
    for (point, normal) in faces {
        let found = candidates.iter().find(|c| c.point.dist(point) < 1e-6).unwrap_or_else(|| panic!("the face centre {point:?} is a candidate"));
        assert!(found.normal.is_some_and(|n| n.dist(normal) < 1e-6), "the face's outward normal: {found:?}");
    }
    assert_eq!(candidates.iter().filter(|c| c.normal.is_some()).count(), 6, "six planar faces, six normals");
    assert_eq!(candidates.iter().filter(|c| c.normal.is_none()).count(), 20, "eight vertices and twelve edge midpoints, no normals");
}

/// #Z-CAM: a click mode keeps the view repainting every frame, so live snapping sees the pointer
/// move even when nothing else would draw; an idle frame asks for none.
#[test]
fn a_click_mode_keeps_the_view_repainting() {
    let mut g = DynamicGnomon::default();
    assert!(!g.tracks_pointer(), "an idle frame asks for no repaint");
    g.set_active_state(GnomonActiveState::OriginSnap);
    assert!(g.tracks_pointer(), "Origin Snap asks to repaint");
    g.set_active_state(GnomonActiveState::AxisOrbit(Axis::Y));
    assert!(g.tracks_pointer(), "Axis Orbit asks to repaint");
    g.set_active_state(GnomonActiveState::RelativeInput { handle: GnomonHandle::Translate(Axis::X), pos: egui::Pos2::ZERO });
    assert!(!g.tracks_pointer(), "a value entry follows no pointer");
    g.set_active_state(GnomonActiveState::Idle);
    assert!(!g.tracks_pointer(), "a mode that ended asks for no repaint");
}

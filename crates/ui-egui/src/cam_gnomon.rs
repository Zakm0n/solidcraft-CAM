//! The CAM coordinate systems in the 3D viewport: each machining origin with the machine's own
//! X, Y and Z directions, drawn as a short triad where it sits (`cam.coordinate_system.create`).
//!
//! The arms are model points, so the view's camera puts them in place and they turn with the
//! model; their lengths follow the zoom, as the joint origins' triads do. The colours are the
//! view's own X, Y and Z colours, so a machine axis reads the same as the world axis it shares a
//! direction with.

use egui::{Color32, Painter, Stroke};
use solidcore_record::Vec3;

use crate::SolveApp;
use crate::theme::Tokens;
use crate::viewport::Proj;

/// How long an arm is, as a share of the view's height: short next to the model, long enough to
/// name a direction.
const ARM_SHARE: f64 = 0.045;

/// One coordinate system's triad: where the machining origin sits and where each machine axis
/// points, one arm away, all in model space.
#[derive(Clone, Debug, PartialEq)]
pub struct Gnomon {
    /// Its name in the Browser, which also names its handles.
    pub name: String,
    /// The machine zero, in model space (mm).
    pub origin: Vec3,
    /// The +X, +Y and +Z arm tips, in model space (mm).
    pub tips: [Vec3; 3],
    /// The colour each arm is drawn in: the view's X, Y and Z colours.
    pub colours: [Color32; 3],
}

/// The triads the view draws now, in the design's own order. A coordinate system whose points are
/// not finite numbers is left out: a design file is input like any other.
pub fn gnomons(app: &SolveApp) -> Vec<Gnomon> {
    let t = Tokens::get();
    let len = arm_length(app);
    let colours = [t.axis_x, t.axis_y, t.axis_z];
    app.session
        .doc
        .coordinate_systems
        .iter()
        .filter(|c| !app.session.visibility.hidden_coordinate_systems.contains(&c.id))
        .filter_map(|c| {
            let origin = point(c.origin)?;
            let axes = [point(c.x_axis)?, point(c.y_axis)?, point(c.z_axis)?];
            let tips = axes.map(|a| origin + a * len);
            tips.iter().all(finite).then_some(Gnomon { name: c.name.clone(), origin, tips, colours })
        })
        .collect()
}

/// The length of an arm in model units, from the view's height. A camera that cannot say (not
/// laid out, hostile numbers) keeps the arms a fixed share of the home distance.
fn arm_length(app: &SolveApp) -> f64 {
    let half = app.cam.half_height();
    if half.is_finite() && half > 0.0 { half * 2.0 * ARM_SHARE } else { 4.5 }
}

/// A stored point, unless it holds a number the view cannot place.
fn point(p: [f64; 3]) -> Option<Vec3> {
    p.iter().all(|v| v.is_finite()).then(|| Vec3::new(p[0], p[1], p[2]))
}

fn finite(v: &Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

/// Draw every coordinate system's triad over the view. The origin is a small disc, as a joint
/// origin's is; each arm is a segment in its axis's colour. Handles are published so a scenario
/// can find an origin and its arms.
pub fn show(app: &SolveApp, painter: &Painter, proj: &Proj) {
    let t = Tokens::get();
    let triads = gnomons(app);
    crate::scenario::publish_count("cam:gnomons", triads.len() as f64);
    let stroke = Stroke::new(2.0, t.text);
    for g in &triads {
        let Some(centre) = proj.to_screen(g.origin) else { continue };
        crate::scenario::publish_handle(&format!("cam:gnomon:{}", g.name), centre);
        for ((tip, colour), axis) in g.tips.iter().zip(g.colours).zip(["x", "y", "z"]) {
            let Some(at) = proj.to_screen(*tip) else { continue };
            painter.line_segment([centre, at], Stroke::new(2.0, colour));
            crate::scenario::publish_handle(&format!("cam:gnomon:{}:{axis}", g.name), at);
        }
        painter.circle(centre, 4.5, t.panel, stroke);
    }
}

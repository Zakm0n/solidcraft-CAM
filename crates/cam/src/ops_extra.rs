//! The remaining 2D operations: blend, chamfer, helical bore and thread mill.
//!
//! These are deliberately small, geometry-driven generators on top of the existing routing
//! parameters and Toolpath type. Helical moves carry a changing Z on one arc block, so they
//! post and simulate through the same pipeline as everything else.

use thiserror::Error;

use crate::geometry::{Chain, Point2};
use crate::offset::{OffsetError, OffsetOptions, offset_chain};
use crate::routing::{Compensation, RoutingError, RoutingParams, contour, depth_levels};
use crate::tool::Tool;
use crate::toolpath::{Heights, Toolpath, ToolpathError};

#[derive(Debug, Error)]
pub enum ExtraError {
    #[error("toolpath error: {0}")]
    Toolpath(#[from] ToolpathError),
    #[error("offset error: {0}")]
    Offset(#[from] OffsetError),
    #[error("routing error: {0}")]
    Routing(#[from] RoutingError),
    #[error("geometry error: {0}")]
    Geometry(#[from] crate::geometry::GeometryError),
    #[error("{0} is not valid for this operation")]
    BadParameter(&'static str),
    #[error("the tool does not fit the feature")]
    ToolTooLarge,
}

/// Helical bore from a centre point and diameter.
pub fn helix_bore(centre: Point2, diameter_mm: f64, pitch_mm: f64, params: &RoutingParams, tool: &Tool) -> Result<Toolpath, ExtraError> {
    if !(diameter_mm.is_finite() && diameter_mm > tool.diameter_mm()) {
        return Err(ExtraError::ToolTooLarge);
    }
    if !(pitch_mm.is_finite() && pitch_mm > 0.0) {
        return Err(ExtraError::BadParameter("pitch"));
    }
    let radius = (diameter_mm - tool.diameter_mm()) / 2.0;
    let heights = Heights::new(params.clearance_mm, params.retract_mm).unwrap_or_default();
    let start = [centre[0] + radius, centre[1]];
    let mut path = Toolpath::new([start[0], start[1], params.clearance_mm], heights)?;
    path.lift_to_clearance()?;
    path.rapid([start[0], start[1], params.retract_mm])?;
    path.feed([start[0], start[1], 0.0], params.plunge_feed_mm_min)?;

    let mut z = 0.0;
    while z > -params.depth_mm + 1e-9 {
        let next = (z - pitch_mm).max(-params.depth_mm);
        path.arc([start[0], start[1], next], [centre[0], centre[1], next], false, params.feed_mm_min)?;
        z = next;
    }
    // A final flat circle cleans the bore floor.
    path.arc([start[0], start[1], -params.depth_mm], [centre[0], centre[1], -params.depth_mm], false, params.feed_mm_min)?;
    path.lift_to_clearance()?;
    Ok(path)
}

/// Thread mill: a helix at the thread pitch around the bore, with a short lead-in and lead-out.
pub fn thread_mill(centre: Point2, major_diameter_mm: f64, pitch_mm: f64, params: &RoutingParams, tool: &Tool) -> Result<Toolpath, ExtraError> {
    if !(major_diameter_mm.is_finite() && major_diameter_mm > tool.diameter_mm()) {
        return Err(ExtraError::ToolTooLarge);
    }
    if !(pitch_mm.is_finite() && pitch_mm > 0.0) {
        return Err(ExtraError::BadParameter("pitch"));
    }
    let radius = (major_diameter_mm - tool.diameter_mm()) / 2.0;
    let heights = Heights::new(params.clearance_mm, params.retract_mm).unwrap_or_default();
    let start = [centre[0] + radius, centre[1]];
    let mut path = Toolpath::new([start[0], start[1], params.clearance_mm], heights)?;
    path.lift_to_clearance()?;
    path.rapid([start[0], start[1], params.retract_mm])?;
    path.feed([start[0], start[1], 0.0], params.plunge_feed_mm_min)?;

    let mut z = 0.0;
    while z > -params.depth_mm + 1e-9 {
        let next = (z - pitch_mm).max(-params.depth_mm);
        path.arc([start[0], start[1], next], [centre[0], centre[1], next], false, params.feed_mm_min)?;
        z = next;
    }
    // Lead out: one more turn at the final depth.
    path.arc([start[0], start[1], -params.depth_mm], [centre[0], centre[1], -params.depth_mm], false, params.feed_mm_min)?;
    path.lift_to_clearance()?;
    Ok(path)
}

/// Model chamfer: follow the selected chain, offset inward by half the chamfer width, at the
/// routed depth(s). A bull-nose or V-bit tool turns this into the chamfer surface.
pub fn model_chamfer(chain: &Chain, width_mm: f64, angle_deg: f64, params: &RoutingParams, tool: &Tool) -> Result<Toolpath, ExtraError> {
    if !(width_mm.is_finite() && width_mm > 0.0) {
        return Err(ExtraError::BadParameter("chamfer width"));
    }
    if !(angle_deg.is_finite() && angle_deg > 0.0 && angle_deg < 90.0) {
        return Err(ExtraError::BadParameter("chamfer angle"));
    }
    let inset = width_mm * 0.5 * angle_deg.to_radians().cos();
    let guide = offset_chain(chain, -inset, OffsetOptions::default())?.into_iter().next().ok_or(ExtraError::BadParameter("chamfer geometry"))?;
    let mut routed = params.clone();
    routed.compensation = Compensation::None;
    routed.lead_in_mm = 0.0;
    routed.finish_pass = false;
    Ok(contour(&guide, &routed, tool)?)
}

/// Blend mill: passes spanning two open chains, spaced by the stepover along them.
pub fn blend_mill(first: &Chain, second: &Chain, stepover_mm: f64, params: &RoutingParams, _tool: &Tool) -> Result<Toolpath, ExtraError> {
    if !(stepover_mm.is_finite() && stepover_mm > 0.0) {
        return Err(ExtraError::BadParameter("stepover"));
    }
    let length = first.length().max(second.length());
    let passes = (length / stepover_mm).ceil().clamp(2.0, 2_000.0) as usize;
    let a = resample(first, passes);
    let b = resample(second, passes);
    let heights = Heights::new(params.clearance_mm, params.retract_mm).unwrap_or_default();
    let start = [a[0][0], a[0][1], params.clearance_mm];
    let mut path = Toolpath::new(start, heights)?;
    for level in depth_levels(params.depth_mm, params.depth_per_pass_mm) {
        for index in 0..passes {
            let (from, to) = if index % 2 == 0 { (a[index], b[index]) } else { (b[index], a[index]) };
            path.lift_to_clearance()?;
            path.rapid([from[0], from[1], params.clearance_mm])?;
            path.rapid([from[0], from[1], params.retract_mm])?;
            path.feed([from[0], from[1], level], params.plunge_feed_mm_min)?;
            path.feed([to[0], to[1], level], params.feed_mm_min)?;
        }
    }
    path.lift_to_clearance()?;
    Ok(path)
}

fn resample(chain: &Chain, count: usize) -> Vec<Point2> {
    let points = chain.tessellate(0.02);
    if points.len() < 2 {
        return vec![points.first().copied().unwrap_or([0.0, 0.0]); count.max(2)];
    }
    let mut lengths = Vec::with_capacity(points.len());
    let mut total = 0.0;
    lengths.push(0.0);
    for pair in points.windows(2) {
        total += distance(pair[0], pair[1]);
        lengths.push(total);
    }
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let target = if count <= 1 { 0.0 } else { total * index as f64 / (count - 1) as f64 };
        out.push(point_at(&points, &lengths, target));
    }
    out
}

fn point_at(points: &[Point2], lengths: &[f64], target: f64) -> Point2 {
    match lengths.binary_search_by(|value| value.partial_cmp(&target).unwrap_or(std::cmp::Ordering::Equal)) {
        Ok(index) => points[index.min(points.len() - 1)],
        Err(index) => {
            if index == 0 {
                return points[0];
            }
            if index >= points.len() {
                return points[points.len() - 1];
            }
            let start = lengths[index - 1];
            let end = lengths[index];
            let t = if end > start { (target - start) / (end - start) } else { 0.0 };
            [
                points[index - 1][0] + (points[index][0] - points[index - 1][0]) * t,
                points[index - 1][1] + (points[index][1] - points[index - 1][1]) * t,
            ]
        }
    }
}

fn distance(a: Point2, b: Point2) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Curve2D;
    use crate::tool::Cutter;
    use crate::toolpath::Move;

    fn tool() -> Tool {
        Tool::new("6mm flat", Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap()
    }

    fn params(depth: f64) -> RoutingParams {
        let cutting = crate::tool::CuttingData::new(10_000.0, 1_200.0, 300.0, 2.0, 1.5).unwrap();
        RoutingParams::new(&tool(), &cutting, depth)
    }

    #[test]
    fn helix_bore_uses_the_bore_radius_and_pitch() {
        let path = helix_bore([10.0, 10.0], 20.0, 2.0, &params(6.0), &tool()).unwrap();
        let mut radii = Vec::new();
        let mut zs = Vec::new();
        for motion in path.moves() {
            if let Move::Arc { to, centre, .. } = motion {
                radii.push(((to[0] - centre[0]).powi(2) + (to[1] - centre[1]).powi(2)).sqrt());
                zs.push(to[2]);
            }
        }
        assert!(radii.iter().all(|radius| (*radius - 7.0).abs() < 1e-6), "{radii:?}");
        // Each turn drops the pitch; the final floor circle repeats the last depth.
        assert!(zs.windows(2).any(|pair| (pair[1] - pair[0] + 2.0).abs() < 1e-6));
        assert!((zs[zs.len() - 1] + 6.0).abs() < 1e-9);
    }

    #[test]
    fn thread_mill_drops_by_the_pitch_each_turn() {
        let path = thread_mill([0.0, 0.0], 20.0, 1.5, &params(6.0), &tool()).unwrap();
        let mut zs = Vec::new();
        for motion in path.moves() {
            if let Move::Arc { to, .. } = motion {
                zs.push(to[2]);
            }
        }
        assert!(zs.len() >= 4);
        for pair in zs.windows(2).take(zs.len() - 2) {
            assert!((pair[0] - pair[1] - 1.5).abs() < 1e-6, "{zs:?}");
        }
    }

    #[test]
    fn chamfer_follows_an_inset_chain() {
        let square = Chain::closed(vec![Curve2D::polyline(vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]], true).unwrap()], 1e-6).unwrap();
        let path = model_chamfer(&square, 4.0, 45.0, &params(1.0), &tool()).unwrap();
        let (min, max) = path.bounds();
        assert!((min[0] - 1.414).abs() < 0.2, "min x {}", min[0]);
        assert!((max[0] - 18.586).abs() < 0.2, "max x {}", max[0]);
    }

    #[test]
    fn blend_mill_spans_between_the_chains() {
        let first = Chain::open(vec![Curve2D::line([0.0, 0.0], [10.0, 0.0]).unwrap()], 1e-6).unwrap();
        let second = Chain::open(vec![Curve2D::line([0.0, 8.0], [10.0, 8.0]).unwrap()], 1e-6).unwrap();
        let path = blend_mill(&first, &second, 2.0, &params(1.0), &tool()).unwrap();
        assert!(path.cutting_length_mm() > 30.0);
        let (min, max) = path.bounds();
        assert!(min[1] <= 0.01 && max[1] >= 7.99);
    }
}

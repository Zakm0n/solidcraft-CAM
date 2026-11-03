//! 2D offsets, booleans and clipping: the shared geometry every routing operation uses.
//!
//! Closed loops are offset and combined with `clipper2-rust`; open chains get a single-side
//! offset (left of the direction of travel, as a contour sees it) because a balanced both-sides
//! offset is useless for cutter compensation. Everything is returned in CAM's own
//! [`Chain`] type, as closed polylines within the arc tolerance.

use clipper2_rust::{EndType, FillRule, JoinType, PathD, PathsD, PointD, difference_d, inflate_paths_d, union_d};
use thiserror::Error;

use crate::geometry::{Chain, Curve2D, GeometryError, Point2};

/// How corners are joined when a loop is offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinStyle {
    Round,
    Miter,
    Square,
    Bevel,
}

impl JoinStyle {
    fn clipper(self) -> JoinType {
        match self {
            JoinStyle::Round => JoinType::Round,
            JoinStyle::Miter => JoinType::Miter,
            JoinStyle::Square => JoinType::Square,
            JoinStyle::Bevel => JoinType::Bevel,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffsetOptions {
    pub join: JoinStyle,
    /// Decimal places clipper keeps (4 = hundredth of a micron at mm scale).
    pub precision: i32,
    /// Chord tolerance for the polylines clipper produces.
    pub arc_tolerance: f64,
    /// Chord tolerance for turning CAM curves into polygons.
    pub flatten_tolerance: f64,
    /// Keep an exact arc when the source loop is a single circular arc; set false to force
    /// polylines everywhere.
    pub preserve_arcs: bool,
}

impl Default for OffsetOptions {
    fn default() -> Self {
        Self { join: JoinStyle::Round, precision: 4, arc_tolerance: 0.01, flatten_tolerance: 0.02, preserve_arcs: true }
    }
}

#[derive(Debug, Error)]
pub enum OffsetError {
    #[error("geometry error: {0}")]
    Geometry(#[from] GeometryError),
    #[error("offset distance must be finite, got {0}")]
    BadDistance(f64),
    #[error("an open chain needs at least two points")]
    ShortChain,
    #[error("the offset collapsed: the geometry has no room for this distance")]
    Collapsed,
}

/// Offset a chain. A closed chain is offset outward for positive distances (and inward for
/// negative); an open chain is offset to the left of its direction of travel.
///
/// Returns every loop clipper produces: a pocket that splits into pieces returns more than one.
pub fn offset_chain(chain: &Chain, distance: f64, options: OffsetOptions) -> Result<Vec<Chain>, OffsetError> {
    if !distance.is_finite() {
        return Err(OffsetError::BadDistance(distance));
    }
    if distance == 0.0 {
        return Ok(vec![chain.clone()]);
    }
    if !chain.is_closed() {
        let shifted = offset_open(chain, distance, options.flatten_tolerance)?;
        return Ok(vec![shifted]);
    }

    // Lines and arcs that meet tangentially (rounded rectangles, slots, circles) offset
    // analytically, so every arc stays an arc. Sharp corners need trimming and fall back to
    // clipper below.
    if options.preserve_arcs
        && let Some(loop_) = analytic_offset(chain, distance)
    {
        return Ok(vec![loop_]);
    }

    let path = chain_to_path(chain, options.flatten_tolerance);
    let original_area = chain.area(options.flatten_tolerance);
    let mut loops = offset_paths(&[path], distance, options)?;
    // Clipper's sign follows the polygon orientation. CAM means "positive grows", so if the
    // loop shrank, run the other way.
    let area = total_area(&loops);
    let wrong_way = (distance > 0.0 && area < original_area) || (distance < 0.0 && area > original_area);
    if wrong_way {
        loops = offset_paths(&[chain_to_path(chain, options.flatten_tolerance)], -distance, options)?;
    }
    if loops.is_empty() {
        return Err(OffsetError::Collapsed);
    }
    Ok(loops)
}

/// Offset every loop of a region by the same distance.
pub fn offset_region(region: &crate::geometry::Region, distance: f64, options: OffsetOptions) -> Result<Vec<Chain>, OffsetError> {
    let mut paths = Vec::new();
    paths.push(chain_to_path(region.outer(), options.flatten_tolerance));
    for island in region.islands() {
        paths.push(chain_to_path(island, options.flatten_tolerance));
    }
    let loops = offset_paths(&paths, distance, options)?;
    if loops.is_empty() {
        return Err(OffsetError::Collapsed);
    }
    Ok(loops)
}

/// Region difference: `subject` minus `clip`.
pub fn boolean_difference(subject: &[Chain], clip: &[Chain], options: OffsetOptions) -> Result<Vec<Chain>, OffsetError> {
    let subject = closed_paths(subject, options.flatten_tolerance);
    let clip = closed_paths(clip, options.flatten_tolerance);
    let result = difference_d(&subject, &clip, FillRule::NonZero, options.precision);
    paths_to_chains(result, options.flatten_tolerance).map_err(OffsetError::from)
}

/// Region union.
pub fn boolean_union(first: &[Chain], second: &[Chain], options: OffsetOptions) -> Result<Vec<Chain>, OffsetError> {
    let first = closed_paths(first, options.flatten_tolerance);
    let second = closed_paths(second, options.flatten_tolerance);
    let result = union_d(&first, &second, FillRule::NonZero, options.precision);
    paths_to_chains(result, options.flatten_tolerance).map_err(OffsetError::from)
}

/// Clip loops to an axis-aligned stock window.
pub fn clip_to_stock(loops: &[Chain], min: Point2, max: Point2, options: OffsetOptions) -> Result<Vec<Chain>, OffsetError> {
    let stock = Chain::closed(vec![Curve2D::polyline(vec![[min[0], min[1]], [max[0], min[1]], [max[0], max[1]], [min[0], max[1]]], true)?], 1e-6)?;
    // Everything outside the stock is the clip set: a very large box minus the stock.
    let far = 1.0e6;
    let outside = boolean_difference(
        &[Chain::closed(
            vec![Curve2D::polyline(
                vec![[min[0] - far, min[1] - far], [max[0] + far, min[1] - far], [max[0] + far, max[1] + far], [min[0] - far, max[1] + far]],
                true,
            )?],
            1e-6,
        )?],
        &[stock],
        options,
    )?;
    boolean_difference(loops, &outside, options)
}

/// The largest distance a closed chain can be offset inward before it collapses.
pub fn minimum_inward_radius(chain: &Chain, options: OffsetOptions) -> f64 {
    let mut low = 0.0;
    let mut high = chain.area(options.flatten_tolerance).sqrt();
    for _ in 0..24 {
        let mid = (low + high) / 2.0;
        let survives = offset_chain(chain, -mid, options).is_ok();
        if survives {
            low = mid;
        } else {
            high = mid;
        }
    }
    low
}

/// Offset a closed loop of lines and arcs that meet tangentially, preserving each curve's type.
///
/// Positive distance grows the loop: outward means the right of travel on a counter-clockwise
/// loop. Returns `None` when the loop is not a tangent line/arc chain, when a radius would
/// vanish, or when the result would not stay closed — all cases for the clipper fallback.
fn analytic_offset(chain: &Chain, distance: f64) -> Option<Chain> {
    if chain.curves().is_empty() || distance == 0.0 {
        return None;
    }
    let ccw = chain.signed_area(0.01) >= 0.0;
    let mut curves = Vec::with_capacity(chain.curves().len());
    for curve in chain.curves() {
        match curve {
            Curve2D::Line { from, to } => {
                let dx = to[0] - from[0];
                let dy = to[1] - from[1];
                let length = (dx * dx + dy * dy).sqrt();
                if length < 1e-12 {
                    return None;
                }
                let (nx, ny) = if ccw { (dy / length, -dx / length) } else { (-dy / length, dx / length) };
                curves.push(Curve2D::Line {
                    from: [from[0] + nx * distance, from[1] + ny * distance],
                    to: [to[0] + nx * distance, to[1] + ny * distance],
                });
            }
            Curve2D::Arc { center, radius, start_angle, sweep } => {
                // A counter-clockwise arc on a counter-clockwise loop bulges outward, so an
                // outward offset grows its radius; the other combinations behave oppositely.
                let sign = if ccw == (*sweep > 0.0) { 1.0 } else { -1.0 };
                let new_radius = radius + sign * distance;
                if new_radius <= 1e-9 {
                    return None;
                }
                curves.push(Curve2D::Arc { center: *center, radius: new_radius, start_angle: *start_angle, sweep: *sweep });
            }
            Curve2D::Polyline { .. } => return None,
        }
    }
    let offset = Chain::closed(curves, 1e-6).ok()?;
    // Inward offsets in tight areas fold over themselves; let clipper trim those instead.
    if self_intersects(&offset, 0.02) {
        return None;
    }
    let before = chain.area(0.02);
    let after = offset.area(0.02);
    if distance > 0.0 && after <= before + 1e-9 {
        return None;
    }
    if distance < 0.0 && after >= before - 1e-9 {
        return None;
    }
    Some(offset)
}

fn segments_cross(a: Point2, b: Point2, c: Point2, d: Point2) -> bool {
    let cross = |p: Point2, q: Point2, r: Point2| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let eps = 1e-9;
    let d1 = cross(a, b, c);
    let d2 = cross(a, b, d);
    let d3 = cross(c, d, a);
    let d4 = cross(c, d, b);
    ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps)) && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
}

fn self_intersects(chain: &Chain, tol: f64) -> bool {
    let points = chain.tessellate(tol);
    let count = points.len();
    if count < 4 {
        return false;
    }
    for i in 0..count {
        let a = points[i];
        let b = points[(i + 1) % count];
        for j in (i + 1)..count {
            if j == (i + 1) % count || (j + 1) % count == i {
                continue;
            }
            let c = points[j];
            let d = points[(j + 1) % count];
            if segments_cross(a, b, c, d) {
                return true;
            }
        }
    }
    false
}

fn offset_paths(paths: &[PathD], distance: f64, options: OffsetOptions) -> Result<Vec<Chain>, OffsetError> {
    let result = inflate_paths_d(&paths.to_vec(), distance, options.join.clipper(), EndType::Polygon, 2.0, options.precision, options.arc_tolerance);
    paths_to_chains(result, options.flatten_tolerance).map_err(OffsetError::from)
}

fn offset_open(chain: &Chain, distance: f64, tol: f64) -> Result<Chain, OffsetError> {
    let points = chain.tessellate(tol);
    if points.len() < 2 {
        return Err(OffsetError::ShortChain);
    }
    let mut shifted = Vec::with_capacity(points.len());
    for index in 0..points.len() {
        let previous = points[index.saturating_sub(1)];
        let next = points[(index + 1).min(points.len() - 1)];
        let dx = next[0] - previous[0];
        let dy = next[1] - previous[1];
        let length = (dx * dx + dy * dy).sqrt();
        if length < 1e-12 {
            shifted.push(points[index]);
            continue;
        }
        // Left normal of the direction of travel.
        let nx = -dy / length;
        let ny = dx / length;
        shifted.push([points[index][0] + nx * distance, points[index][1] + ny * distance]);
    }
    let curve = Curve2D::polyline(shifted, false)?;
    Ok(Chain::open(vec![curve], 1e-6)?)
}

fn chain_to_path(chain: &Chain, tol: f64) -> PathD {
    chain.tessellate(tol).into_iter().map(|point| PointD { x: point[0], y: point[1] }).collect()
}

fn closed_paths(chains: &[Chain], tol: f64) -> PathsD {
    chains.iter().filter(|chain| chain.is_closed()).map(|chain| chain_to_path(chain, tol)).collect()
}

fn paths_to_chains(paths: PathsD, tol: f64) -> Result<Vec<Chain>, GeometryError> {
    let mut chains = Vec::new();
    for path in paths {
        if path.len() < 3 {
            continue;
        }
        let points: Vec<Point2> = path.iter().map(|point| [point.x, point.y]).collect();
        if (points[0][0] - points[points.len() - 1][0]).abs() < 1e-9 && (points[0][1] - points[points.len() - 1][1]).abs() < 1e-9 {
            continue;
        }
        let curve = Curve2D::polyline(points, true)?;
        chains.push(Chain::closed(vec![curve], 1e-6)?);
    }
    let _ = tol;
    Ok(chains)
}

fn total_area(loops: &[Chain]) -> f64 {
    loops.iter().map(|chain| chain.signed_area(0.02)).sum::<f64>().abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, size: f64) -> Chain {
        Chain::closed(vec![Curve2D::polyline(vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]], true).unwrap()], 1e-6).unwrap()
    }

    #[test]
    fn a_closed_loop_offsets_out_and_in() {
        let options = OffsetOptions::default();
        let chain = square(0.0, 0.0, 10.0);
        let out = offset_chain(&chain, 1.0, options).unwrap();
        let outward_area: f64 = out.iter().map(|loop_| loop_.area(0.02)).sum();
        assert!((outward_area - 144.0).abs() < 3.0, "outward area {outward_area}");
        let inward = offset_chain(&chain, -1.0, options).unwrap();
        let inward_area: f64 = inward.iter().map(|loop_| loop_.signed_area(0.02)).sum::<f64>().abs();
        assert!((inward_area - 64.0).abs() < 3.0, "inward area {inward_area}");
    }

    #[test]
    fn an_open_chain_offsets_to_the_left() {
        let chain = Chain::open(vec![Curve2D::line([0.0, 0.0], [10.0, 0.0]).unwrap()], 1e-6).unwrap();
        let shifted = offset_open(&chain, 2.0, 0.02).unwrap();
        assert!((shifted.start()[1] - 2.0).abs() < 1e-9);
        assert!((shifted.end()[1] - 2.0).abs() < 1e-9);
        assert!((shifted.length() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn booleans_and_clipping_hold_the_expected_area() {
        let options = OffsetOptions::default();
        let outer = square(0.0, 0.0, 20.0);
        let hole = square(5.0, 5.0, 5.0);
        let cut = boolean_difference(std::slice::from_ref(&outer), &[hole], options).unwrap();
        let area: f64 = cut.iter().map(|chain| chain.signed_area(0.02)).sum::<f64>().abs();
        assert!((area - 375.0).abs() < 2.0, "difference area {area}");

        let clipped = clip_to_stock(&[square(-5.0, -5.0, 20.0)], [0.0, 0.0], [10.0, 10.0], options).unwrap();
        let clipped_area: f64 = clipped.iter().map(|chain| chain.signed_area(0.02)).sum::<f64>().abs();
        assert!((clipped_area - 100.0).abs() < 2.0, "clipped area {clipped_area}");
    }

    fn rounded_rectangle() -> Chain {
        let (w, h, r) = (40.0, 20.0, 5.0);
        Chain::closed(
            vec![
                Curve2D::line([r, 0.0], [w - r, 0.0]).unwrap(),
                Curve2D::arc([w - r, r], r, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2).unwrap(),
                Curve2D::line([w, r], [w, h - r]).unwrap(),
                Curve2D::arc([w - r, h - r], r, 0.0, std::f64::consts::FRAC_PI_2).unwrap(),
                Curve2D::line([w - r, h], [r, h]).unwrap(),
                Curve2D::arc([r, h - r], r, std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2).unwrap(),
                Curve2D::line([0.0, h - r], [0.0, r]).unwrap(),
                Curve2D::arc([r, r], r, std::f64::consts::PI, std::f64::consts::FRAC_PI_2).unwrap(),
            ],
            1e-6,
        )
        .unwrap()
    }

    fn rounded_area(w: f64, h: f64, r: f64) -> f64 {
        w * h - (4.0 - std::f64::consts::PI) * r * r
    }

    #[test]
    fn a_rounded_rectangle_keeps_its_arcs_when_offset() {
        let chain = rounded_rectangle();
        let outward = offset_chain(&chain, 1.0, OffsetOptions::default()).unwrap();
        assert_eq!(outward.len(), 1);
        let curves = outward[0].curves();
        assert_eq!(curves.iter().filter(|curve| matches!(curve, Curve2D::Arc { .. })).count(), 4);
        assert_eq!(curves.iter().filter(|curve| matches!(curve, Curve2D::Line { .. })).count(), 4);
        for curve in curves {
            if let Curve2D::Arc { radius, .. } = curve {
                assert!((radius - 6.0).abs() < 1e-9, "corner radius {radius}");
            }
        }
        let expected = rounded_area(42.0, 22.0, 6.0);
        assert!((outward[0].area(0.01) - expected).abs() < 0.5, "area {}", outward[0].area(0.01));

        let inward = offset_chain(&chain, -1.0, OffsetOptions::default()).unwrap();
        for curve in inward[0].curves() {
            if let Curve2D::Arc { radius, .. } = curve {
                assert!((radius - 4.0).abs() < 1e-9, "corner radius {radius}");
            }
        }
        let expected = rounded_area(38.0, 18.0, 4.0);
        assert!((inward[0].area(0.01) - expected).abs() < 0.5);
    }

    fn rounded_rect(width: f64, height: f64, radius: f64) -> Chain {
        let pi = std::f64::consts::FRAC_PI_2;
        Chain::closed(
            vec![
                Curve2D::line([radius, 0.0], [width - radius, 0.0]).unwrap(),
                Curve2D::arc([width - radius, radius], radius, -pi, pi).unwrap(),
                Curve2D::line([width, radius], [width, height - radius]).unwrap(),
                Curve2D::arc([width - radius, height - radius], radius, 0.0, pi).unwrap(),
                Curve2D::line([width - radius, height], [radius, height]).unwrap(),
                Curve2D::arc([radius, height - radius], radius, pi, pi).unwrap(),
                Curve2D::line([0.0, height - radius], [0.0, radius]).unwrap(),
                Curve2D::arc([radius, radius], radius, 2.0 * pi, pi).unwrap(),
            ],
            1e-6,
        )
        .unwrap()
    }

    #[test]
    fn a_clockwise_rounded_rectangle_still_keeps_its_arcs() {
        let ccw = rounded_rect(30.0, 20.0, 5.0);
        let cw = ccw.reversed();
        let outward = offset_chain(&cw, 2.0, OffsetOptions::default()).unwrap();
        assert_eq!(outward.len(), 1);
        let arcs: Vec<f64> = outward[0]
            .curves()
            .iter()
            .filter_map(|curve| match curve {
                Curve2D::Arc { radius, .. } => Some(*radius),
                _ => None,
            })
            .collect();
        assert_eq!(arcs.len(), 4, "clockwise loop lost its arcs");
        assert!(arcs.iter().all(|radius| (radius - 7.0).abs() < 1e-6));
        assert!(outward[0].area(0.02) > cw.area(0.02));
    }

    #[test]
    fn sharp_corners_and_flattening_still_fall_back_to_polylines() {
        let square = Chain::closed(vec![Curve2D::polyline(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], true).unwrap()], 1e-6).unwrap();
        let preserving = offset_chain(&square, 1.0, OffsetOptions::default()).unwrap();
        assert!(matches!(preserving[0].curves()[0], Curve2D::Polyline { .. }));
        let rounded = rounded_rectangle();
        let flattened = offset_chain(&rounded, 1.0, OffsetOptions { preserve_arcs: false, ..OffsetOptions::default() }).unwrap();
        assert!(flattened[0].curves().iter().all(|curve| matches!(curve, Curve2D::Polyline { .. })));
    }

    #[test]
    fn a_circular_arc_stays_an_arc_when_offset() {
        let circle = Chain::closed(vec![Curve2D::arc([0.0, 0.0], 10.0, 0.0, std::f64::consts::TAU).unwrap()], 1e-6).unwrap();
        let offset = offset_chain(&circle, 1.0, OffsetOptions::default()).unwrap();
        assert_eq!(offset.len(), 1);
        match &offset[0].curves()[0] {
            Curve2D::Arc { radius, .. } => assert!((radius - 11.0).abs() < 1e-9),
            other => panic!("offset circle became {other:?}"),
        }
        // Flattening is still available when wanted.
        let flattened = offset_chain(&circle, 1.0, OffsetOptions { preserve_arcs: false, ..OffsetOptions::default() }).unwrap();
        assert!(matches!(flattened[0].curves()[0], Curve2D::Polyline { .. }));
        assert!((flattened[0].area(0.02) - std::f64::consts::PI * 121.0).abs() < 3.0);
    }

    #[test]
    fn a_collapsing_offset_is_reported() {
        let options = OffsetOptions::default();
        let chain = square(0.0, 0.0, 1.0);
        assert!(matches!(offset_chain(&chain, -10.0, options), Err(OffsetError::Collapsed)));
        assert!((minimum_inward_radius(&chain, options) - 0.5).abs() < 0.15);
    }
}

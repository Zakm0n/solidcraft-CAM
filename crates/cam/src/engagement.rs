//! Constant-engagement (dynamic) toolpaths.
//!
//! Mastercam's dynamic operations keep the cutter's radial engagement roughly constant by
//! moving in overlapping trochoidal loops. Three parameters shape that motion and are modelled
//! here:
//!
//! - **Micro lift** raises the tool off the floor for the part of each loop that is not
//!   removing material, clearing chips and avoiding heat. It is applied on the back half of
//!   each loop and while moving to the next loop.
//! - **Back feed** is the feed used for those non-cutting back moves; it is independent of the
//!   cutting feed and usually faster.
//! - **Minimum toolpath radius** clamps the loop radius so dynamic motion never asks the machine
//!   for an arc tighter than it can follow (Mastercam expresses it as a percentage of the tool
//!   diameter; `minimum_radius_from_percent` converts).
//!
//! The engine produces the shared [`Toolpath`] type, so caching, posting, cutrail simulation
//! and the G-code comparison loop all work unchanged.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::geometry::{Chain, Point2, Region};
use crate::offset::{OffsetError, OffsetOptions, boolean_difference, offset_chain, offset_region};
use crate::routing::depth_levels;
use crate::tool::{CuttingData, Tool};
use crate::toolpath::{Heights, Move, Toolpath, ToolpathError};

#[derive(Debug, Error)]
pub enum EngagementError {
    #[error("toolpath error: {0}")]
    Toolpath(#[from] ToolpathError),
    #[error("offset error: {0}")]
    Offset(#[from] OffsetError),
    #[error("geometry error: {0}")]
    Geometry(#[from] crate::geometry::GeometryError),
    #[error("{0} must be finite and greater than zero")]
    BadPositive(&'static str),
    #[error("{0} must not be negative")]
    BadNegative(&'static str),
    #[error("the guide chain is empty")]
    EmptyGuide,
}

/// Parameters for constant-engagement motion.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EngagementParams {
    /// Radial advance between loop centres, mm.
    pub stepover_mm: f64,
    /// Requested trochoidal loop radius, mm.
    pub loop_radius_mm: f64,
    /// Smallest loop radius the machine may be asked for, mm (Mastercam's minimum toolpath radius).
    pub min_toolpath_radius_mm: f64,
    /// Lift on the non-cutting back half of a loop and between loops, mm.
    pub micro_lift_mm: f64,
    /// Feed for back moves, mm/min.
    pub back_feed_mm_min: f64,
    /// Feed for cutting arcs, mm/min.
    pub feed_mm_min: f64,
    /// Plunge/entry feed, mm/min.
    pub plunge_feed_mm_min: f64,
    pub depth_mm: f64,
    pub depth_per_pass_mm: f64,
    pub clearance_mm: f64,
    pub retract_mm: f64,
}

impl EngagementParams {
    /// Practical starting values from the tool and its cutting data.
    ///
    /// The loop radius defaults to 45% of the tool diameter, the minimum radius to 10% of it,
    /// and the back feed to 1.5× the cutting feed. Micro lift defaults to 0.2 mm.
    pub fn new(tool: &Tool, cutting: &CuttingData, depth_mm: f64) -> Self {
        let diameter = tool.diameter_mm();
        let min_radius = diameter * 0.10;
        Self {
            stepover_mm: cutting.stepover_mm().min(diameter * 0.5).max(0.05),
            loop_radius_mm: (diameter * 0.45).max(min_radius),
            min_toolpath_radius_mm: min_radius,
            micro_lift_mm: 0.2,
            back_feed_mm_min: cutting.feed_mm_min() * 1.5,
            feed_mm_min: cutting.feed_mm_min(),
            plunge_feed_mm_min: cutting.plunge_feed_mm_min(),
            depth_mm,
            depth_per_pass_mm: cutting.depth_of_cut_mm(),
            clearance_mm: 10.0,
            retract_mm: 2.0,
        }
    }

    pub fn with_stepover(mut self, stepover_mm: f64) -> Self {
        self.stepover_mm = stepover_mm;
        self
    }

    pub fn with_loop_radius(mut self, radius_mm: f64) -> Self {
        self.loop_radius_mm = radius_mm;
        self
    }

    pub fn with_min_radius(mut self, radius_mm: f64) -> Self {
        self.min_toolpath_radius_mm = radius_mm;
        self
    }

    pub fn with_micro_lift(mut self, lift_mm: f64) -> Self {
        self.micro_lift_mm = lift_mm;
        self
    }

    pub fn with_back_feed(mut self, feed_mm_min: f64) -> Self {
        self.back_feed_mm_min = feed_mm_min;
        self
    }

    /// The effective loop radius: the requested radius, raised to the minimum toolpath radius.
    pub fn effective_radius(&self) -> f64 {
        self.loop_radius_mm.max(self.min_toolpath_radius_mm).max(0.05)
    }

    fn validate(&self) -> Result<(), EngagementError> {
        for (name, value) in [
            ("stepover", self.stepover_mm),
            ("loop radius", self.loop_radius_mm),
            ("back feed", self.back_feed_mm_min),
            ("feed", self.feed_mm_min),
            ("plunge feed", self.plunge_feed_mm_min),
            ("depth", self.depth_mm),
            ("depth per pass", self.depth_per_pass_mm),
            ("clearance", self.clearance_mm),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(EngagementError::BadPositive(name));
            }
        }
        for (name, value) in
            [("minimum toolpath radius", self.min_toolpath_radius_mm), ("micro lift", self.micro_lift_mm), ("retract", self.retract_mm)]
        {
            if !value.is_finite() || value < 0.0 {
                return Err(EngagementError::BadNegative(name));
            }
        }
        Ok(())
    }
}

/// Mastercam's minimum toolpath radius as a percentage of tool diameter.
pub fn minimum_radius_from_percent(tool: &Tool, percent: f64) -> f64 {
    tool.diameter_mm() * percent.max(0.0) / 100.0
}

/// Radial engagement angle in degrees for a tool of `tool_radius` taking `stepover` radially:
/// `cos(theta) = 1 - stepover / tool_radius`.
pub fn engagement_angle_deg(tool_radius: f64, stepover: f64) -> f64 {
    if tool_radius <= 0.0 || stepover <= 0.0 {
        return 0.0;
    }
    let ratio = (1.0 - stepover / tool_radius).clamp(-1.0, 1.0);
    ratio.acos().to_degrees()
}

/// The radial stepover that gives `angle_deg` of engagement.
pub fn stepover_for_angle(tool_radius: f64, angle_deg: f64) -> f64 {
    let angle = angle_deg.clamp(0.1, 179.9).to_radians();
    (tool_radius * (1.0 - angle.cos())).max(0.0)
}

/// Trochoidal motion along a guide chain, one full loop per `stepover_mm` of travel.
///
/// Each loop is split into a cutting half and a lifted back half so micro lift and back feed
/// are visible in the path. The path is emitted for every depth pass.
pub fn trochoidal(guide: &Chain, params: &EngagementParams, _tool: &Tool) -> Result<Toolpath, EngagementError> {
    params.validate()?;
    let radius = params.effective_radius();
    let centres = loop_centres(guide, params.stepover_mm)?;
    if centres.is_empty() {
        return Err(EngagementError::EmptyGuide);
    }
    let heights = Heights::new(params.clearance_mm, params.retract_mm).unwrap_or_default();
    let start = [centres[0][0] + radius, centres[0][1], params.clearance_mm];
    let mut path = Toolpath::new(start, heights)?;
    let lift = params.micro_lift_mm;

    for level in depth_levels(params.depth_mm, params.depth_per_pass_mm) {
        for (index, centre) in centres.iter().enumerate() {
            let loop_start = [centre[0] + radius, centre[1], level];
            if index == 0 {
                path.lift_to_clearance()?;
                path.rapid([loop_start[0], loop_start[1], params.clearance_mm])?;
                path.rapid([loop_start[0], loop_start[1], params.retract_mm])?;
                path.feed([loop_start[0], loop_start[1], level], params.plunge_feed_mm_min)?;
            } else {
                // The previous back half left the tool at level + micro lift.
                path.feed([loop_start[0], loop_start[1], level + lift], params.back_feed_mm_min)?;
                path.feed([loop_start[0], loop_start[1], level], params.plunge_feed_mm_min)?;
            }
            // Cutting half: 0° → 180° around the loop centre, at depth.
            path.arc([centre[0] - radius, centre[1], level], [centre[0], centre[1], level], false, params.feed_mm_min)?;
            // Back half: 180° → 360°, lifted by the micro lift, at the back feed.
            path.arc([centre[0] + radius, centre[1], level + lift], [centre[0], centre[1], level + lift], false, params.back_feed_mm_min)?;
        }
        path.lift_to_clearance()?;
    }
    Ok(path)
}

/// Dynamic contour: constant-engagement loops along the compensated wall.
pub fn dynamic_contour(
    chain: &Chain,
    side: crate::routing::Compensation,
    params: &EngagementParams,
    tool: &Tool,
) -> Result<Toolpath, EngagementError> {
    use crate::routing::Compensation;
    let radius = tool.diameter_mm() / 2.0;
    let distance = match side {
        Compensation::None => 0.0,
        Compensation::Left => -radius,
        Compensation::Right => radius,
    };
    let guide = if distance == 0.0 {
        chain.clone()
    } else {
        offset_chain(chain, distance, OffsetOptions::default())?.into_iter().next().ok_or(EngagementError::EmptyGuide)?
    };
    trochoidal(&guide, params, tool)
}

/// Dynamic mill: inward offset loops over a region, each run as a trochoid. Islands in the
/// region are honoured by the offset step.
pub fn dynamic_mill(region: &Region, params: &EngagementParams, tool: &Tool) -> Result<Toolpath, EngagementError> {
    params.validate()?;
    let radius = tool.diameter_mm() / 2.0;
    let mut path: Option<Toolpath> = None;
    let mut distance = radius;
    let mut emitted = 0;
    while emitted < 500 {
        let loops = match offset_region(region, -distance, OffsetOptions::default()) {
            Ok(loops) => loops,
            Err(OffsetError::Collapsed) => break,
            Err(error) => return Err(error.into()),
        };
        if loops.is_empty() {
            break;
        }
        for guide in loops {
            let piece = trochoidal(&guide, params, tool)?;
            match path.as_mut() {
                Some(target) => append_path(target, &piece)?,
                None => path = Some(piece),
            }
            emitted += 1;
        }
        distance += params.stepover_mm;
    }
    path.ok_or(EngagementError::EmptyGuide)
}

/// Peel mill: trochoidal slotting along a centreline; the loop never exceeds half the slot
/// width.
pub fn peel_mill(centreline: &Chain, width_mm: f64, params: &EngagementParams, tool: &Tool) -> Result<Toolpath, EngagementError> {
    params.validate()?;
    let half = width_mm.max(0.1) / 2.0;
    let mut adjusted = *params;
    adjusted.min_toolpath_radius_mm = adjusted.min_toolpath_radius_mm.min(half);
    adjusted.loop_radius_mm = adjusted.loop_radius_mm.min(half).max(adjusted.min_toolpath_radius_mm);
    adjusted.stepover_mm = adjusted.stepover_mm.min(half).max(0.01);
    trochoidal(centreline, &adjusted, tool)
}

/// Area mill: clear what a previous operation left. With no previous region it behaves like a
/// dynamic mill of the whole region.
pub fn area_mill(region: &Region, previous: Option<&Region>, params: &EngagementParams, tool: &Tool) -> Result<Toolpath, EngagementError> {
    let Some(previous) = previous else {
        return dynamic_mill(region, params, tool);
    };
    let rest = boolean_difference(std::slice::from_ref(region.outer()), std::slice::from_ref(previous.outer()), OffsetOptions::default())?;
    let mut path: Option<Toolpath> = None;
    let inset = params.effective_radius();
    for guide in rest {
        // Keep the trochoid's loops inside the region: run the guide a loop radius inward.
        let guide = match offset_chain(&guide, -inset, OffsetOptions::default()) {
            Ok(mut loops) if !loops.is_empty() => loops.remove(0),
            _ => guide,
        };
        let piece = trochoidal(&guide, params, tool)?;
        match path.as_mut() {
            Some(target) => append_path(target, &piece)?,
            None => path = Some(piece),
        }
    }
    path.ok_or(EngagementError::EmptyGuide)
}

fn append_path(target: &mut Toolpath, source: &Toolpath) -> Result<(), ToolpathError> {
    for motion in source.moves() {
        match *motion {
            Move::Rapid { to } => target.rapid(to)?,
            Move::Feed { to, feed_mm_min } => target.feed(to, feed_mm_min)?,
            Move::Arc { to, centre, clockwise, feed_mm_min } => target.arc(to, centre, clockwise, feed_mm_min)?,
        }
    }
    Ok(())
}

/// Loop centres spaced `stepover` apart along the guide's arc length.
fn loop_centres(guide: &Chain, stepover: f64) -> Result<Vec<Point2>, EngagementError> {
    let mut points = guide.tessellate(0.02);
    if points.len() < 2 {
        return Err(EngagementError::EmptyGuide);
    }
    if guide.is_closed()
        && let (Some(first), Some(last)) = (points.first().copied(), points.last().copied())
        && distance(first, last) > 1e-9
    {
        points.push(first);
    }
    let spacing = stepover.max(0.01);
    let mut lengths = Vec::with_capacity(points.len());
    let mut total = 0.0;
    lengths.push(0.0);
    for pair in points.windows(2) {
        total += distance(pair[0], pair[1]);
        lengths.push(total);
    }
    if total <= 1e-9 {
        return Ok(vec![points[0]]);
    }
    let mut centres = Vec::new();
    let mut travelled = 0.0;
    while travelled < total {
        centres.push(point_at(&points, &lengths, travelled));
        travelled += spacing;
    }
    Ok(centres)
}

fn point_at(points: &[Point2], lengths: &[f64], travelled: f64) -> Point2 {
    match lengths.binary_search_by(|value| value.partial_cmp(&travelled).unwrap_or(std::cmp::Ordering::Equal)) {
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
            let t = if end > start { (travelled - start) / (end - start) } else { 0.0 };
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

    fn cutting() -> CuttingData {
        CuttingData::new(10_000.0, 1_200.0, 300.0, 2.0, 1.5).unwrap()
    }

    fn line(length: f64) -> Chain {
        Chain::open(vec![Curve2D::line([0.0, 0.0], [length, 0.0]).unwrap()], 1e-6).unwrap()
    }

    fn square(size: f64) -> Chain {
        Chain::closed(vec![Curve2D::polyline(vec![[0.0, 0.0], [size, 0.0], [size, size], [0.0, size]], true).unwrap()], 1e-6).unwrap()
    }

    #[test]
    fn engagement_angle_matches_the_stepover() {
        // A 3 mm tool radius taking 1.5 mm radially engages 60°.
        assert!((engagement_angle_deg(3.0, 1.5) - 60.0).abs() < 1e-9);
        let stepover = stepover_for_angle(3.0, 60.0);
        assert!((stepover - 1.5).abs() < 1e-9);
        assert_eq!(engagement_angle_deg(3.0, 0.0), 0.0);
    }

    #[test]
    fn the_minimum_toolpath_radius_clamps_the_loop() {
        let mut params = EngagementParams::new(&tool(), &cutting(), 2.0).with_loop_radius(0.5).with_min_radius(2.75);
        params.stepover_mm = 2.0;
        let path = trochoidal(&line(20.0), &params, &tool()).unwrap();
        let mut arc_radii = Vec::new();
        for motion in path.moves() {
            if let Move::Arc { to, centre, .. } = motion {
                arc_radii.push(((to[0] - centre[0]).powi(2) + (to[1] - centre[1]).powi(2)).sqrt());
            }
        }
        assert!(!arc_radii.is_empty());
        assert!(arc_radii.iter().all(|radius| (*radius - 2.75).abs() < 1e-6), "{arc_radii:?}");
    }

    #[test]
    fn back_moves_are_lifted_and_use_the_back_feed() {
        let params = EngagementParams::new(&tool(), &cutting(), 2.0).with_micro_lift(0.3).with_back_feed(2_000.0);
        let path = trochoidal(&line(10.0), &params, &tool()).unwrap();
        let mut lifted = 0;
        let mut back_fed = 0;
        for motion in path.moves() {
            if let Move::Arc { to, feed_mm_min, .. } = motion {
                if (to[2] - (-2.0 + 0.3)).abs() < 1e-9 {
                    lifted += 1;
                }
                if (*feed_mm_min - 2_000.0).abs() < 1e-9 {
                    back_fed += 1;
                }
            }
        }
        assert!(lifted > 0, "no lifted back moves");
        assert_eq!(lifted, back_fed, "lift and back feed should coincide");
    }

    #[test]
    fn loops_advance_by_the_stepover() {
        let params = EngagementParams::new(&tool(), &cutting(), 1.0).with_stepover(2.0);
        let path = trochoidal(&line(10.0), &params, &tool()).unwrap();
        // 10 mm at 2 mm advance: centres at 0, 2, 4, 6, 8 → five cutting arcs.
        let cutting_arcs = path.moves().iter().filter(|motion| matches!(motion, Move::Arc { to, .. } if (to[2] + 1.0).abs() < 1e-9)).count();
        assert_eq!(cutting_arcs, 5);
    }

    #[test]
    fn a_dynamic_contour_follows_the_offset_wall() {
        let params = EngagementParams::new(&tool(), &cutting(), 2.0);
        let path = dynamic_contour(&square(20.0), crate::routing::Compensation::Left, &params, &tool()).unwrap();
        assert!(path.cutting_length_mm() > 100.0);
        let (min, max) = path.bounds();
        // Left compensation keeps the whole trochoid inside the 20 mm wall.
        assert!(min[0] >= -0.01 && max[0] <= 20.01, "bounds {min:?}..{max:?}");
        assert!(min[1] >= -0.01 && max[1] <= 20.01, "bounds {min:?}..{max:?}");
    }

    #[test]
    fn dynamic_mill_clears_inside_the_region() {
        let region = Region::new(square(20.0), vec![]).unwrap();
        let params = EngagementParams::new(&tool(), &cutting(), 2.0).with_stepover(2.0);
        let path = dynamic_mill(&region, &params, &tool()).unwrap();
        assert!(path.cutting_length_mm() > 100.0);
        let (min, max) = path.bounds();
        assert!(min[0] >= -0.01 && max[0] <= 20.01, "bounds {min:?}..{max:?}");
        assert!(min[1] >= -0.01 && max[1] <= 20.01, "bounds {min:?}..{max:?}");
    }

    #[test]
    fn peel_mill_keeps_the_loop_inside_the_slot() {
        let centre = line(20.0);
        let params = EngagementParams::new(&tool(), &cutting(), 2.0);
        let path = peel_mill(&centre, 10.0, &params, &tool()).unwrap();
        let mut radii = Vec::new();
        for motion in path.moves() {
            if let Move::Arc { to, centre, .. } = motion {
                radii.push(((to[0] - centre[0]).powi(2) + (to[1] - centre[1]).powi(2)).sqrt());
            }
        }
        assert!(!radii.is_empty());
        assert!(radii.iter().all(|radius| *radius <= 5.0 + 1e-9), "{radii:?}");
    }

    #[test]
    fn area_mill_clears_only_the_rest_of_the_region() {
        let outer = Region::new(square(20.0), vec![]).unwrap();
        let previous = Region::new(square(10.0), vec![]).unwrap();
        let params = EngagementParams::new(&tool(), &cutting(), 1.0).with_stepover(2.0);
        let path = area_mill(&outer, Some(&previous), &params, &tool()).unwrap();
        assert!(path.cutting_length_mm() > 20.0);
        let (min, max) = path.bounds();
        assert!(min[0] >= -0.01 && max[0] <= 20.01, "bounds {min:?}..{max:?}");
    }

    #[test]
    fn bad_geometry_and_parameters_are_rejected() {
        let params = EngagementParams::new(&tool(), &cutting(), 2.0);
        let empty = Chain::open(vec![], 1e-6).unwrap_err();
        assert!(matches!(empty, crate::geometry::GeometryError::EmptyChain));
        let mut bad = params;
        bad.stepover_mm = 0.0;
        assert!(matches!(trochoidal(&line(5.0), &bad, &tool()), Err(EngagementError::BadPositive("stepover"))));
    }
}

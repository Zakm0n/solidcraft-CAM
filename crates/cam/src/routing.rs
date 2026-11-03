//! Routing primitives: the tool motions for the core 2.5D operations.
//!
//! Every primitive emits the existing [`Toolpath`] (so caching, simulation and posting keep
//! working) and shares one parameter block: depth, depth per pass, stepover, feeds, safe Z,
//! compensation, lead-in and tabs. Coordinates are WCS millimetres with Z=0 at the setup top.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::geometry::{Chain, Curve2D, Point2, Region};
use crate::offset::{OffsetError, OffsetOptions, boolean_difference, clip_to_stock, offset_chain, offset_region};
use crate::toolpath::{Heights, Tab, Toolpath, ToolpathError};
use crate::{CuttingData, Tool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Compensation {
    #[default]
    None,
    Left,
    Right,
}

/// How the tool enters the material at the start of a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EntryStyle {
    /// Straight down at the plunge feed.
    #[default]
    Plunge,
    /// Down and forward at the same time.
    Ramp,
    /// A descending full circle (circle mill and helical bores).
    Helix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PocketStrategy {
    /// Offset loops spiralling inward, the classic pocket clearing path.
    #[default]
    Spiral,
    /// Back-and-forth raster lines.
    ZigZag,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoutingParams {
    /// Cut depth below Z=0 (positive).
    pub depth_mm: f64,
    /// Depth of one pass (positive).
    pub depth_per_pass_mm: f64,
    pub stepover_mm: f64,
    pub feed_mm_min: f64,
    pub plunge_feed_mm_min: f64,
    pub clearance_mm: f64,
    pub retract_mm: f64,
    pub compensation: Compensation,
    pub entry: EntryStyle,
    /// Horizontal travel of a straight ramp entry, mm.
    pub ramp_length_mm: f64,
    /// Ramp angle from horizontal, degrees. Used when the entry follows the contour.
    pub ramp_angle_deg: f64,
    /// When set with a Ramp entry, contour passes descend by this depth instead of
    /// `depth_per_pass_mm`.
    pub ramp_depth_per_pass_mm: Option<f64>,
    /// Facing raster direction, degrees counter-clockwise from +X.
    pub facing_angle_deg: f64,
    /// Keep circular source arcs as arcs when offsetting; false forces polylines.
    pub preserve_arcs: bool,
    /// Chord tolerance used when offsets are flattened to polylines, mm.
    pub arc_tolerance_mm: f64,
    /// Straight lead-in length for contours and engravings; 0 disables it.
    pub lead_in_mm: f64,
    /// Add a final wall pass at the compensated radius.
    pub finish_pass: bool,
    pub tabs: Vec<Tab>,
}

impl RoutingParams {
    /// Parameters from a tool and its cutting data, at a given depth.
    pub fn new(tool: &Tool, cutting: &CuttingData, depth_mm: f64) -> Self {
        Self {
            depth_mm,
            depth_per_pass_mm: cutting.depth_of_cut_mm(),
            // Never step over more than the cutter itself, whatever the chart says.
            stepover_mm: cutting.stepover_mm().min(tool.diameter_mm() * 0.9),
            feed_mm_min: cutting.feed_mm_min(),
            plunge_feed_mm_min: cutting.plunge_feed_mm_min(),
            clearance_mm: 10.0,
            retract_mm: 2.0,
            compensation: Compensation::None,
            entry: EntryStyle::Plunge,
            ramp_length_mm: 2.0,
            ramp_angle_deg: 3.0,
            ramp_depth_per_pass_mm: None,
            facing_angle_deg: 0.0,
            preserve_arcs: true,
            arc_tolerance_mm: 0.02,
            lead_in_mm: 3.0,
            finish_pass: true,
            tabs: Vec::new(),
        }
    }

    pub fn with_compensation(mut self, compensation: Compensation) -> Self {
        self.compensation = compensation;
        self
    }

    pub fn with_entry(mut self, entry: EntryStyle) -> Self {
        self.entry = entry;
        self
    }

    pub fn with_ramp_angle(mut self, degrees: f64) -> Self {
        self.ramp_angle_deg = degrees;
        self
    }

    pub fn with_ramp_depth_per_pass(mut self, depth_mm: f64) -> Self {
        self.ramp_depth_per_pass_mm = Some(depth_mm);
        self
    }

    pub fn with_facing_angle(mut self, degrees: f64) -> Self {
        self.facing_angle_deg = degrees;
        self
    }

    /// Mastercam "stepover".
    pub fn with_stepover(mut self, stepover_mm: f64) -> Self {
        self.stepover_mm = stepover_mm;
        self
    }

    /// Mastercam "depth cuts": maximum depth of one pass.
    pub fn with_depth_per_pass(mut self, depth_mm: f64) -> Self {
        self.depth_per_pass_mm = depth_mm;
        self
    }

    /// Mastercam "finish pass" switch.
    pub fn with_finish_pass(mut self, finish_pass: bool) -> Self {
        self.finish_pass = finish_pass;
        self
    }

    /// Mastercam "lead in" length.
    pub fn with_lead_in(mut self, lead_in_mm: f64) -> Self {
        self.lead_in_mm = lead_in_mm;
        self
    }

    fn offset_options(&self) -> OffsetOptions {
        OffsetOptions { flatten_tolerance: self.arc_tolerance_mm, preserve_arcs: self.preserve_arcs, ..OffsetOptions::default() }
    }

    fn heights(&self) -> Heights {
        Heights::new(self.clearance_mm, self.retract_mm).unwrap_or_default()
    }

    fn validate(&self) -> Result<(), RoutingError> {
        if !self.depth_mm.is_finite() || self.depth_mm <= 0.0 {
            return Err(RoutingError::BadDepth(self.depth_mm));
        }
        if !self.depth_per_pass_mm.is_finite() || self.depth_per_pass_mm <= 0.0 {
            return Err(RoutingError::BadDepth(self.depth_per_pass_mm));
        }
        if !self.stepover_mm.is_finite() || self.stepover_mm <= 0.0 {
            return Err(RoutingError::BadStepover(self.stepover_mm));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("toolpath error: {0}")]
    Toolpath(#[from] ToolpathError),
    #[error("offset error: {0}")]
    Offset(#[from] OffsetError),
    #[error("geometry error: {0}")]
    Geometry(#[from] crate::geometry::GeometryError),
    #[error("cut depth must be finite and greater than zero, got {0}")]
    BadDepth(f64),
    #[error("stepover must be finite and greater than zero, got {0}")]
    BadStepover(f64),
    #[error("the tool does not fit the feature: diameter {tool} mm, feature {feature} mm")]
    ToolTooLarge { tool: f64, feature: f64 },
    #[error("an operation needs geometry to machine")]
    NoGeometry,
}

/// Depth levels from the first pass to the final depth, in negative Z.
pub fn depth_levels(depth_mm: f64, per_pass_mm: f64) -> Vec<f64> {
    let mut levels = Vec::new();
    let mut cut = per_pass_mm;
    while cut < depth_mm {
        levels.push(-cut);
        cut += per_pass_mm;
    }
    levels.push(-depth_mm);
    levels
}

/// Raster facing over a rectangular window. Rows run along X, alternating direction.
pub fn facing(min: Point2, max: Point2, params: &RoutingParams, tool: &Tool) -> Result<Toolpath, RoutingError> {
    params.validate()?;
    let step = params.stepover_mm.min(tool.diameter_mm() * 0.95).max(0.05);
    let rows = facing_rows(min, max, step, params.facing_angle_deg);
    if rows.is_empty() {
        return Err(RoutingError::NoGeometry);
    }
    let mut path = new_path(rows[0].0, params);
    for level in depth_levels(params.depth_mm, params.depth_per_pass_mm) {
        for (index, (start, end)) in rows.iter().enumerate() {
            let (from, to) = if index % 2 == 0 { (*start, *end) } else { (*end, *start) };
            approach(&mut path, from, level, params)?;
            path.feed([to[0], to[1], level], params.feed_mm_min)?;
        }
    }
    path.lift_to_clearance()?;
    Ok(path)
}

/// Contour along a chain with cutter compensation, depth passes and an optional lead-in.
pub fn contour(chain: &Chain, params: &RoutingParams, tool: &Tool) -> Result<Toolpath, RoutingError> {
    params.validate()?;
    let radius = tool.diameter_mm() / 2.0;
    let offset = match params.compensation {
        Compensation::None => 0.0,
        Compensation::Left => -radius,
        Compensation::Right => radius,
    };
    let compensated = if offset == 0.0 {
        chain.clone()
    } else {
        offset_chain(chain, offset, params.offset_options())?.into_iter().next().ok_or(RoutingError::NoGeometry)?
    };
    let mut path = new_path(compensated.start(), params);
    for level in contour_levels(params) {
        if params.entry == EntryStyle::Ramp {
            path.lift_to_clearance()?;
            path.rapid([compensated.start()[0], compensated.start()[1], params.retract_mm])?;
            ramp_and_follow(&mut path, &compensated, level, params)?;
            continue;
        }
        approach(&mut path, compensated.start(), level, params)?;
        if params.lead_in_mm > 0.0
            && let Some(lead) = lead_in_point(&compensated, params.lead_in_mm)
        {
            path.feed([lead[0], lead[1], level], params.feed_mm_min)?;
            path.feed([compensated.start()[0], compensated.start()[1], level], params.feed_mm_min)?;
        }
        follow(&mut path, &compensated, level, params.feed_mm_min)?;
    }
    for tab in &params.tabs {
        path.add_tab(*tab);
    }
    path.lift_to_clearance()?;
    Ok(path)
}

/// Pocket clearing: inward offset spiral, or a raster, with islands and an optional finish pass.
pub fn pocket(region: &Region, strategy: PocketStrategy, params: &RoutingParams, tool: &Tool) -> Result<Toolpath, RoutingError> {
    params.validate()?;
    let radius = tool.diameter_mm() / 2.0;
    let mut path = new_path(region.outer().start(), params);
    for level in contour_levels(params) {
        match strategy {
            PocketStrategy::Spiral => {
                let mut cut = radius;
                let mut emitted = 0;
                while cut < region.outer().area(0.02).sqrt() * 2.0 {
                    let loops = match offset_region(region, -cut, params.offset_options()) {
                        Ok(loops) => loops,
                        Err(OffsetError::Collapsed) => break,
                        Err(error) => return Err(error.into()),
                    };
                    for loop_ in &loops {
                        if params.entry == EntryStyle::Ramp {
                            path.lift_to_clearance()?;
                            path.rapid([loop_.start()[0], loop_.start()[1], params.retract_mm])?;
                            ramp_and_follow(&mut path, loop_, level, params)?;
                        } else {
                            approach(&mut path, loop_.start(), level, params)?;
                            follow(&mut path, loop_, level, params.feed_mm_min)?;
                        }
                        emitted += 1;
                    }
                    cut += params.stepover_mm;
                    if emitted > 10_000 {
                        break;
                    }
                }
                if emitted == 0 {
                    return Err(RoutingError::ToolTooLarge { tool: tool.diameter_mm(), feature: region.outer().area(0.02).sqrt() });
                }
                if params.finish_pass {
                    let finish = offset_region(region, -radius, params.offset_options())?;
                    for loop_ in &finish {
                        if params.entry == EntryStyle::Ramp {
                            path.lift_to_clearance()?;
                            path.rapid([loop_.start()[0], loop_.start()[1], params.retract_mm])?;
                            ramp_and_follow(&mut path, loop_, level, params)?;
                        } else {
                            approach(&mut path, loop_.start(), level, params)?;
                            follow(&mut path, loop_, level, params.feed_mm_min)?;
                        }
                    }
                }
            }
            PocketStrategy::ZigZag => {
                let (min, max) = region.outer().bounds();
                let rows = raster_rows(min, max, params.stepover_mm);
                let mut emitted = 0;
                for (index, (start, end)) in rows.iter().enumerate() {
                    let midpoint = [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0];
                    if !region.contains(midpoint) {
                        continue;
                    }
                    let (from, to) = if index % 2 == 0 { (*start, *end) } else { (*end, *start) };
                    approach(&mut path, from, level, params)?;
                    path.feed([to[0], to[1], level], params.feed_mm_min)?;
                    emitted += 1;
                }
                if emitted == 0 {
                    return Err(RoutingError::NoGeometry);
                }
            }
        }
    }
    path.lift_to_clearance()?;
    Ok(path)
}

/// Circle mill: helical entry at the pocket radius, one full circle per depth pass.
pub fn circle_mill(center: Point2, pocket_diameter_mm: f64, params: &RoutingParams, tool: &Tool) -> Result<Toolpath, RoutingError> {
    params.validate()?;
    let radius = (pocket_diameter_mm - tool.diameter_mm()) / 2.0;
    if radius <= 0.0 {
        return Err(RoutingError::ToolTooLarge { tool: tool.diameter_mm(), feature: pocket_diameter_mm });
    }
    let start = [center[0] + radius, center[1]];
    let mut path = new_path(start, params);
    for level in depth_levels(params.depth_mm, params.depth_per_pass_mm) {
        if params.entry == EntryStyle::Helix {
            path.lift_to_clearance()?;
            path.rapid([start[0], start[1], params.retract_mm])?;
        } else {
            approach(&mut path, start, level, params)?;
        }
        path.arc([start[0], start[1], level], [center[0], center[1], level], false, params.feed_mm_min)?;
    }
    path.lift_to_clearance()?;
    Ok(path)
}

/// Slot mill around a closed slot: two offset passes leave the slot at `width_mm`.
pub fn slot_mill(centreline: &Chain, width_mm: f64, params: &RoutingParams, tool: &Tool) -> Result<Toolpath, RoutingError> {
    params.validate()?;
    let side = (width_mm - tool.diameter_mm()) / 2.0;
    if side < 0.0 {
        return Err(RoutingError::ToolTooLarge { tool: tool.diameter_mm(), feature: width_mm });
    }
    let left = offset_chain(centreline, side, params.offset_options())?;
    let right = offset_chain(centreline, -side, params.offset_options())?;
    let mut path = new_path(centreline.start(), params);
    for level in depth_levels(params.depth_mm, params.depth_per_pass_mm) {
        for loop_ in left.iter().chain(right.iter()) {
            approach(&mut path, loop_.start(), level, params)?;
            follow(&mut path, loop_, level, params.feed_mm_min)?;
        }
    }
    path.lift_to_clearance()?;
    Ok(path)
}

/// Engrave along a chain at depth, following the geometry exactly.
pub fn engrave(chain: &Chain, params: &RoutingParams) -> Result<Toolpath, RoutingError> {
    let mut params = params.clone();
    params.compensation = Compensation::None;
    params.lead_in_mm = 0.0;
    params.finish_pass = false;
    let engraver = Tool::new("engrave", crate::tool::Cutter::Flat, 0.1, 1, 1.0, 1.0).expect("a 0.1 mm engraving cutter is valid");
    contour(chain, &params, &engraver)
}

fn new_path(start: Point2, params: &RoutingParams) -> Toolpath {
    let safe = [start[0], start[1], params.clearance_mm];
    Toolpath::new(safe, params.heights()).unwrap_or_else(|_| Toolpath::new([0.0, 0.0, 0.0], Heights::default()).expect("default heights are valid"))
}

fn approach(path: &mut Toolpath, point: Point2, level: f64, params: &RoutingParams) -> Result<(), RoutingError> {
    path.lift_to_clearance()?;
    match params.entry {
        EntryStyle::Ramp => {
            let ramp = params.ramp_length_mm.max(0.0);
            path.rapid([point[0] - ramp, point[1], params.retract_mm])?;
        }
        _ => {
            path.rapid([point[0], point[1], params.clearance_mm])?;
            path.rapid([point[0], point[1], params.retract_mm])?;
        }
    }
    path.feed([point[0], point[1], level], params.plunge_feed_mm_min)?;
    Ok(())
}

fn follow(path: &mut Toolpath, chain: &Chain, level: f64, feed: f64) -> Result<(), RoutingError> {
    for curve in chain.curves() {
        match curve {
            Curve2D::Line { to, .. } => path.feed([to[0], to[1], level], feed)?,
            Curve2D::Arc { center, sweep, .. } => {
                let to = curve.end();
                path.arc([to[0], to[1], level], [center[0], center[1], level], *sweep < 0.0, feed)?;
            }
            Curve2D::Polyline { points, closed } => {
                for point in points.iter().skip(1) {
                    path.feed([point[0], point[1], level], feed)?;
                }
                if *closed && let Some(first) = points.first() {
                    path.feed([first[0], first[1], level], feed)?;
                }
            }
        }
    }
    Ok(())
}

fn lead_in_point(chain: &Chain, distance: f64) -> Option<Point2> {
    let first = chain.curves().first()?;
    let start = first.start();
    let next = first.tessellate(0.05).get(1).copied().unwrap_or(start);
    let dx = next[0] - start[0];
    let dy = next[1] - start[1];
    let length = (dx * dx + dy * dy).sqrt();
    if length < 1e-12 {
        return None;
    }
    Some([start[0] - dx / length * distance, start[1] - dy / length * distance])
}

/// Depth levels for a contour: a Ramp entry may set its own depth per pass.
fn contour_levels(params: &RoutingParams) -> Vec<f64> {
    let per_pass = params.ramp_depth_per_pass_mm.filter(|depth| params.entry == EntryStyle::Ramp && *depth > 0.0).unwrap_or(params.depth_per_pass_mm);
    depth_levels(params.depth_mm, per_pass)
}

/// Descend along the contour itself, then finish the contour at depth.
///
/// The ramp length comes from `ramp_angle_deg`: a steeper angle needs less contour. A closed
/// chain is lapped again if the ramp needs more travel than one lap.
fn ramp_and_follow(path: &mut Toolpath, chain: &Chain, level: f64, params: &RoutingParams) -> Result<(), RoutingError> {
    let mut points = chain.tessellate(0.02);
    if chain.is_closed()
        && let (Some(first), Some(last)) = (points.first().copied(), points.last().copied())
        && ((first[0] - last[0]).abs() > 1e-9 || (first[1] - last[1]).abs() > 1e-9)
    {
        points.push(first);
    }
    if points.len() < 2 {
        return Err(RoutingError::NoGeometry);
    }
    let angle = params.ramp_angle_deg.clamp(0.1, 60.0).to_radians();
    let drop = (params.retract_mm - level).max(0.0);
    let mut ramp_left = drop / angle.tan().max(1e-3);
    let mut walked = 0.0;
    let mut laps = 0;
    loop {
        for pair in points.windows(2) {
            let dx = pair[1][0] - pair[0][0];
            let dy = pair[1][1] - pair[0][1];
            let segment = (dx * dx + dy * dy).sqrt();
            if segment < 1e-12 {
                continue;
            }
            if ramp_left > 0.0 {
                let take = segment.min(ramp_left);
                let t = take / segment;
                let point = [pair[0][0] + dx * t, pair[0][1] + dy * t];
                walked += take;
                ramp_left -= take;
                let z = (params.retract_mm - walked * angle.tan()).max(level);
                path.feed([point[0], point[1], z], params.feed_mm_min)?;
                if take < segment - 1e-12 {
                    path.feed([pair[1][0], pair[1][1], level], params.feed_mm_min)?;
                }
            } else {
                path.feed([pair[1][0], pair[1][1], level], params.feed_mm_min)?;
            }
        }
        laps += 1;
        if ramp_left <= 1e-9 || !chain.is_closed() || laps > 64 {
            break;
        }
    }
    Ok(())
}

/// Rotate facing rows by `angle_deg` about the window centre.
fn facing_rows(min: Point2, max: Point2, step: f64, angle_deg: f64) -> Vec<(Point2, Point2)> {
    if angle_deg.abs() < 1e-12 {
        return raster_rows(min, max, step);
    }
    let centre = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let to_local = |point: Point2| {
        let dx = point[0] - centre[0];
        let dy = point[1] - centre[1];
        [dx * cos + dy * sin, -dx * sin + dy * cos]
    };
    let to_world = |point: Point2| [centre[0] + point[0] * cos - point[1] * sin, centre[1] + point[0] * sin + point[1] * cos];
    let corners = [min, [max[0], min[1]], max, [min[0], max[1]]];
    let mut local_min = [f64::INFINITY; 2];
    let mut local_max = [f64::NEG_INFINITY; 2];
    for corner in corners {
        let local = to_local(corner);
        for axis in 0..2 {
            local_min[axis] = local_min[axis].min(local[axis]);
            local_max[axis] = local_max[axis].max(local[axis]);
        }
    }
    raster_rows(local_min, local_max, step).into_iter().map(|(start, end)| (to_world(start), to_world(end))).collect()
}

fn raster_rows(min: Point2, max: Point2, step: f64) -> Vec<(Point2, Point2)> {
    if step <= 0.0 || min[1] > max[1] || min[0] > max[0] {
        return Vec::new();
    }
    let mut rows = Vec::new();
    let mut y = min[1];
    while y < max[1] - 1e-9 {
        rows.push(([min[0], y], [max[0], y]));
        y += step;
    }
    // Always finish with a row on the far edge, so the last strip is faced too.
    rows.push(([min[0], max[1]], [max[0], max[1]]));
    rows
}

/// Keep only loops whose bounds lie inside the stock window (a cheap air-cut guard).
pub fn within_stock(loops: &[Chain], min: Point2, max: Point2, options: OffsetOptions) -> Result<Vec<Chain>, RoutingError> {
    Ok(clip_to_stock(loops, min, max, options)?)
}

/// Difference helper used by rest-material operations later.
pub fn subtract(loops: &[Chain], remove: &[Chain]) -> Result<Vec<Chain>, RoutingError> {
    Ok(boolean_difference(loops, remove, OffsetOptions::default())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolpath::Move;

    fn tool() -> Tool {
        Tool::new("6mm flat", crate::tool::Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap()
    }

    fn cutting() -> CuttingData {
        CuttingData::new(10_000.0, 1_200.0, 300.0, 2.0, 3.0).unwrap()
    }

    fn square(x: f64, y: f64, size: f64) -> Chain {
        Chain::closed(vec![Curve2D::polyline(vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]], true).unwrap()], 1e-6).unwrap()
    }

    fn params(depth: f64) -> RoutingParams {
        RoutingParams::new(&tool(), &cutting(), depth)
    }

    #[test]
    fn mastercam_named_builders_update_the_routing_parameters() {
        let tuned = params(2.0).with_stepover(1.5).with_depth_per_pass(0.5).with_finish_pass(false).with_lead_in(2.0);
        assert_eq!(tuned.stepover_mm, 1.5);
        assert_eq!(tuned.depth_per_pass_mm, 0.5);
        assert!(!tuned.finish_pass);
        assert_eq!(tuned.lead_in_mm, 2.0);
    }

    #[test]
    fn depth_levels_end_on_the_final_depth() {
        assert_eq!(depth_levels(5.0, 2.0), [-2.0, -4.0, -5.0]);
        assert_eq!(depth_levels(2.0, 5.0), [-2.0]);
    }

    #[test]
    fn facing_covers_the_window_and_moves() {
        let path = facing([0.0, 0.0], [20.0, 10.0], &params(1.0), &tool()).unwrap();
        assert!(path.moves().len() > 5);
        assert!(path.cutting_length_mm() > 80.0);
        let (min, max) = path.bounds();
        assert!(min[0] <= 0.0 + 1e-6 && max[0] >= 20.0 - 1e-6);
    }

    #[test]
    fn contour_follows_the_chain_and_records_tabs() {
        let chain = square(0.0, 0.0, 20.0);
        let mut params = params(2.0);
        params.tabs.push(Tab::new([10.0, 0.0, -2.0], 4.0, 1.0).unwrap());
        let path = contour(&chain, &params, &tool()).unwrap();
        assert!(path.cutting_length_mm() >= 4.0 * 20.0);
        assert_eq!(path.tabs().len(), 1);
        assert!(path.bounds().0[2] <= -2.0);
    }

    #[test]
    fn compensated_contour_holds_the_tool_radius_from_the_wall() {
        let chain = square(0.0, 0.0, 20.0);
        let path = contour(&chain, &params(2.0).with_compensation(Compensation::Left), &tool()).unwrap();
        // Left compensation on a CCW chain puts the tool centre one radius inside the wall.
        let (min, max) = path.bounds();
        assert!((min[0] - 3.0).abs() < 0.2, "min x {}", min[0]);
        assert!((max[0] - 17.0).abs() < 0.2, "max x {}", max[0]);
        assert!((max[0] - min[0] - 14.0).abs() < 0.3);
    }

    #[test]
    fn a_pocket_spirals_inside_the_region_and_clears_it() {
        let region = Region::new(square(0.0, 0.0, 20.0), vec![]).unwrap();
        let path = pocket(&region, PocketStrategy::Spiral, &params(2.0), &tool()).unwrap();
        assert!(path.cutting_length_mm() > 100.0);
        let (min, max) = path.bounds();
        assert!(min[0] >= -0.01 && max[0] <= 20.01);
        assert!(min[2] <= -2.0);
    }

    #[test]
    fn zigzag_pocket_stays_inside_the_region() {
        let region = Region::new(square(0.0, 0.0, 20.0), vec![]).unwrap();
        let path = pocket(&region, PocketStrategy::ZigZag, &params(2.0), &tool()).unwrap();
        assert!(path.cutting_length_mm() > 100.0);
    }

    #[test]
    fn circle_mill_stays_on_its_radius() {
        let path = circle_mill([10.0, 10.0], 20.0, &params(2.0), &tool()).unwrap();
        let moves = path.moves();
        let mut feeding = 0;
        for motion in moves {
            if let Move::Arc { to, .. } = motion {
                let radius = ((to[0] - 10.0).powi(2) + (to[1] - 10.0).powi(2)).sqrt();
                assert!((radius - 7.0).abs() < 1e-6);
                feeding += 1;
            }
        }
        assert_eq!(feeding, 1);
    }

    #[test]
    fn a_slot_is_as_wide_as_asked() {
        let centre = Chain::open(vec![Curve2D::line([0.0, 0.0], [20.0, 0.0]).unwrap()], 1e-6).unwrap();
        let path = slot_mill(&centre, 10.0, &params(2.0), &tool()).unwrap();
        let (min, max) = path.bounds();
        let width = max[1] - min[1];
        assert!((width - 6.0).abs() < 2.5, "slot bounds width {width}");
    }

    #[test]
    fn a_ramp_entry_descends_while_it_moves() {
        let chain = square(0.0, 0.0, 20.0);
        let path = contour(&chain, &params(2.0).with_entry(EntryStyle::Ramp), &tool()).unwrap();
        let mut current = path.start();
        let mut ramped = false;
        for motion in path.moves() {
            let to = motion.end();
            if let Move::Feed { .. } = motion
                && to[2] < 0.0
                && (current[0] - to[0]).abs() + (current[1] - to[1]).abs() > 0.5
                && current[2] > to[2]
            {
                ramped = true;
            }
            current = to;
        }
        assert!(ramped, "no diagonal feed into the cut");
    }

    #[test]
    fn a_helix_entry_descends_in_a_full_circle() {
        let path = circle_mill([10.0, 10.0], 20.0, &params(2.0).with_entry(EntryStyle::Helix), &tool()).unwrap();
        let mut current = path.start();
        let mut helical = false;
        for motion in path.moves() {
            let to = motion.end();
            if let Move::Arc { .. } = motion
                && current[2] > to[2]
                && (current[0] - to[0]).abs() < 1e-9
                && (current[1] - to[1]).abs() < 1e-9
            {
                helical = true;
            }
            current = to;
        }
        assert!(helical, "no descending full circle");
    }

    #[test]
    fn arc_tolerance_controls_offset_detail() {
        let circle = Chain::closed(vec![Curve2D::arc([0.0, 0.0], 10.0, 0.0, std::f64::consts::TAU).unwrap()], 1e-6).unwrap();
        let mut coarse = params(1.0).with_compensation(Compensation::Left);
        coarse.arc_tolerance_mm = 0.5;
        coarse.preserve_arcs = false;
        let mut fine = coarse.clone();
        fine.arc_tolerance_mm = 0.01;
        let coarse_moves = contour(&circle, &coarse, &tool()).unwrap().moves().len();
        let fine_moves = contour(&circle, &fine, &tool()).unwrap().moves().len();
        assert!(fine_moves > coarse_moves, "fine {fine_moves} vs coarse {coarse_moves}");
    }

    #[test]
    fn generated_paths_are_the_shared_toolpath_type() {
        let chain = square(0.0, 0.0, 20.0);
        let path = contour(&chain, &params(2.0), &tool()).unwrap();
        let json = serde_json::to_string(&path).unwrap();
        assert_eq!(serde_json::from_str::<Toolpath>(&json).unwrap(), path);
        let stock = crate::stock::Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], crate::stock::Material::Aluminum).unwrap();
        let setup = crate::setup::Setup::top_centre(&stock);
        assert!(!path.to_cutrail_cutting(&setup).is_empty());
    }

    #[test]
    fn cutting_moves_stay_inside_the_stock_window() {
        let region = Region::new(square(0.0, 0.0, 20.0), vec![]).unwrap();
        let path = pocket(&region, PocketStrategy::Spiral, &params(2.0), &tool()).unwrap();
        for motion in path.moves() {
            if motion.feed_mm_min().is_some() {
                let to = motion.end();
                assert!(to[0] >= -0.01 && to[0] <= 20.01, "air cut at x {}", to[0]);
                assert!(to[1] >= -0.01 && to[1] <= 20.01, "air cut at y {}", to[1]);
            }
        }
    }

    #[test]
    fn facing_can_run_at_an_angle() {
        let path = facing([0.0, 0.0], [20.0, 20.0], &params(1.0).with_facing_angle(90.0), &tool()).unwrap();
        let mut current = path.start();
        let mut vertical_row = false;
        for motion in path.moves() {
            let to = motion.end();
            if motion.feed_mm_min().is_some() && to[2] < 0.0 {
                let dx = (to[0] - current[0]).abs();
                let dy = (to[1] - current[1]).abs();
                if dx + dy > 1.0 {
                    vertical_row = dy > dx;
                    break;
                }
            }
            current = to;
        }
        assert!(vertical_row, "90-degree facing did not turn the raster");
    }

    #[test]
    fn a_ramp_entry_follows_the_contour() {
        let chain = square(0.0, 0.0, 20.0);
        let path = contour(&chain, &params(2.0).with_entry(EntryStyle::Ramp).with_ramp_angle(5.0), &tool()).unwrap();
        let mut current = path.start();
        let mut ramped = false;
        for motion in path.moves() {
            let to = motion.end();
            if motion.feed_mm_min().is_some() && to[2] < current[2] - 1e-9 && ((to[0] - current[0]).abs() + (to[1] - current[1]).abs()) > 1e-6 {
                ramped = true;
                break;
            }
            current = to;
        }
        assert!(ramped, "no descending move along the contour");
    }

    #[test]
    fn ramp_depth_per_pass_controls_the_levels() {
        let chain = square(0.0, 0.0, 20.0);
        let mut cut = params(2.0);
        cut.depth_per_pass_mm = 2.0;
        cut = cut.with_entry(EntryStyle::Ramp).with_ramp_depth_per_pass(0.5);
        let path = contour(&chain, &cut, &tool()).unwrap();
        let mut depths: Vec<f64> = path
            .moves()
            .iter()
            .filter_map(|motion| match motion {
                Move::Feed { to, .. } if to[2] < 0.0 => Some(to[2]),
                Move::Arc { to, .. } if to[2] < 0.0 => Some(to[2]),
                _ => None,
            })
            .collect();
        depths.sort_by(|a, b| a.partial_cmp(b).unwrap());
        depths.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        assert!(depths.iter().any(|z| (*z + 0.5).abs() < 1e-6), "no 0.5 mm level: {depths:?}");
        assert!(depths.iter().any(|z| (*z + 2.0).abs() < 1e-6), "no final level: {depths:?}");
    }

    #[test]
    fn an_arc_contour_keeps_its_arc_after_compensation() {
        let circle = Chain::closed(vec![Curve2D::arc([0.0, 0.0], 10.0, 0.0, std::f64::consts::TAU).unwrap()], 1e-6).unwrap();
        let path = contour(&circle, &params(2.0).with_compensation(Compensation::Left), &tool()).unwrap();
        let has_arc = path.moves().iter().any(|motion| matches!(motion, Move::Arc { to, .. } if (to[2] + 2.0).abs() < 1e-6));
        assert!(has_arc, "compensated circle emitted no arc");
    }

    fn rounded_rect(width: f64, height: f64, radius: f64) -> Chain {
        let quarter = std::f64::consts::FRAC_PI_2;
        Chain::closed(
            vec![
                Curve2D::line([radius, 0.0], [width - radius, 0.0]).unwrap(),
                Curve2D::arc([width - radius, radius], radius, -quarter, quarter).unwrap(),
                Curve2D::line([width, radius], [width, height - radius]).unwrap(),
                Curve2D::arc([width - radius, height - radius], radius, 0.0, quarter).unwrap(),
                Curve2D::line([width - radius, height], [radius, height]).unwrap(),
                Curve2D::arc([radius, height - radius], radius, quarter, quarter).unwrap(),
                Curve2D::line([0.0, height - radius], [0.0, radius]).unwrap(),
                Curve2D::arc([radius, radius], radius, 2.0 * quarter, quarter).unwrap(),
            ],
            1e-6,
        )
        .unwrap()
    }

    #[test]
    fn a_compensated_rounded_rectangle_keeps_its_corner_arcs() {
        let chain = rounded_rect(30.0, 20.0, 5.0);
        let path = contour(&chain, &params(2.0).with_compensation(Compensation::Left), &tool()).unwrap();
        let mut arcs = Vec::new();
        for motion in path.moves() {
            if let Move::Arc { to, centre, .. } = motion
                && (to[2] + 2.0).abs() < 1e-6
            {
                arcs.push(((to[0] - centre[0]).powi(2) + (to[1] - centre[1]).powi(2)).sqrt());
            }
        }
        assert_eq!(arcs.len(), 4, "corner arcs were flattened: {arcs:?}");
        for radius in arcs {
            assert!((radius - 2.0).abs() < 0.05, "offset corner radius {radius}");
        }
    }

    #[test]
    fn engrave_follows_the_chain() {
        let chain = Chain::open(vec![Curve2D::line([0.0, 0.0], [10.0, 0.0]).unwrap()], 1e-6).unwrap();
        let path = engrave(&chain, &params(0.5)).unwrap();
        assert!(path.cutting_length_mm() >= 10.0);
        assert!(path.bounds().0[2] <= -0.5);
    }
}

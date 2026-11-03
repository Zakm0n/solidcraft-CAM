//! Toolpaths: the curves the tool actually follows.
//!
//! A [`Toolpath`] is a start point plus an ordered list of [`Move`]s (rapid, feed, arc) in WCS
//! millimetres, with the job's clearance heights, an optional lead-in and any tabs. It converts
//! to cutrail [`MotionSegment`]s for simulation, using a [`Setup`] to turn WCS points into model
//! poses.
//!
//! Conventions: rapid moves traverse in air, feed moves cut material, and arcs are measured from
//! a centre point. Heights (`clearance_mm`, `retract_mm`) are Z values in WCS, where 0 is the
//! setup's Z zero.

use cutrail::{MotionSegment, Pose};
use thiserror::Error;

use crate::setup::Setup;
use serde::{Deserialize, Serialize};

/// Safe Z levels, in WCS millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Heights {
    clearance_mm: f64,
    retract_mm: f64,
}

impl Heights {
    /// `clearance` is the travel height (well above the part); `retract` is the height between
    /// cuts. Clearance must be above retract, and retract must not be negative.
    pub fn new(clearance_mm: f64, retract_mm: f64) -> Result<Self, ToolpathError> {
        if !clearance_mm.is_finite() || !retract_mm.is_finite() {
            return Err(ToolpathError::BadHeight);
        }
        if retract_mm < 0.0 || clearance_mm <= retract_mm {
            return Err(ToolpathError::BadHeight);
        }
        Ok(Self { clearance_mm, retract_mm })
    }

    pub fn clearance_mm(&self) -> f64 {
        self.clearance_mm
    }

    pub fn retract_mm(&self) -> f64 {
        self.retract_mm
    }
}

impl Default for Heights {
    fn default() -> Self {
        Self { clearance_mm: 10.0, retract_mm: 2.0 }
    }
}

/// A tab: a bridge of material left standing to hold the part during a contour.
///
/// Tabs are recorded on the path and used by the post-processor (and by the simulation layer
/// when it learns to lift over them); they do not yet change the swept volume themselves.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    at: [f64; 3],
    width_mm: f64,
    height_mm: f64,
}

impl Tab {
    pub fn new(at: [f64; 3], width_mm: f64, height_mm: f64) -> Result<Self, ToolpathError> {
        if !at.iter().all(|c| c.is_finite()) {
            return Err(ToolpathError::NotFinite);
        }
        if !width_mm.is_finite() || width_mm <= 0.0 {
            return Err(ToolpathError::BadTabWidth(width_mm));
        }
        if !height_mm.is_finite() || height_mm <= 0.0 {
            return Err(ToolpathError::BadTabHeight(height_mm));
        }
        Ok(Self { at, width_mm, height_mm })
    }

    pub fn at(&self) -> [f64; 3] {
        self.at
    }

    pub fn width_mm(&self) -> f64 {
        self.width_mm
    }

    pub fn height_mm(&self) -> f64 {
        self.height_mm
    }
}

/// A lead-in: how the tool enters the material before the first cutting move.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LeadIn {
    from: [f64; 3],
    feed_mm_min: f64,
}

impl LeadIn {
    /// A straight lead-in from `from` to the toolpath's start point.
    pub fn line(from: [f64; 3], feed_mm_min: f64) -> Result<Self, ToolpathError> {
        if !from.iter().all(|c| c.is_finite()) {
            return Err(ToolpathError::NotFinite);
        }
        if !feed_mm_min.is_finite() || feed_mm_min <= 0.0 {
            return Err(ToolpathError::BadFeed(feed_mm_min));
        }
        Ok(Self { from, feed_mm_min })
    }

    pub fn from(&self) -> [f64; 3] {
        self.from
    }

    pub fn feed_mm_min(&self) -> f64 {
        self.feed_mm_min
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ToolpathError {
    #[error("toolpath point is not finite")]
    NotFinite,
    #[error("feed must be finite and greater than zero, got {0} mm/min")]
    BadFeed(f64),
    #[error("heights must be finite, retract >= 0 and clearance > retract")]
    BadHeight,
    #[error("tab width must be finite and greater than zero, got {0} mm")]
    BadTabWidth(f64),
    #[error("tab height must be finite and greater than zero, got {0} mm")]
    BadTabHeight(f64),
    #[error("a move cannot end where it starts")]
    ZeroLengthMove,
}

/// One motion of the tool.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Move {
    /// Travel in air.
    Rapid { to: [f64; 3] },
    /// Cut along a straight line.
    Feed { to: [f64; 3], feed_mm_min: f64 },
    /// Cut along an arc around a centre (helical when Z also changes).
    Arc { to: [f64; 3], centre: [f64; 3], clockwise: bool, feed_mm_min: f64 },
}

impl Move {
    pub fn end(&self) -> [f64; 3] {
        match self {
            Move::Rapid { to } | Move::Feed { to, .. } | Move::Arc { to, .. } => *to,
        }
    }

    /// The move is cutting when it has a feed.
    pub fn feed_mm_min(&self) -> Option<f64> {
        match self {
            Move::Rapid { .. } => None,
            Move::Feed { feed_mm_min, .. } | Move::Arc { feed_mm_min, .. } => Some(*feed_mm_min),
        }
    }

    /// Path length from `from` to the move's end, in mm.
    pub fn length_from(&self, from: [f64; 3]) -> f64 {
        match self {
            Move::Rapid { to } | Move::Feed { to, .. } => distance(from, *to),
            Move::Arc { to, centre, clockwise, .. } => {
                let r_start = distance(from, *centre);
                let r_end = distance(*to, *centre);
                if r_start <= f64::EPSILON {
                    return distance(from, *to);
                }
                let a = sub(from, *centre);
                let b = sub(*to, *centre);
                let mut angle = (dot(a, b) / (norm(a) * norm(b))).clamp(-1.0, 1.0).acos();
                if *clockwise {
                    angle = std::f64::consts::TAU - angle;
                }
                angle * (r_start + r_end) / 2.0
            }
        }
    }
}

/// A complete toolpath for one operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Toolpath {
    start: [f64; 3],
    moves: Vec<Move>,
    heights: Heights,
    lead_in: Option<LeadIn>,
    tabs: Vec<Tab>,
}

impl Toolpath {
    pub fn new(start: [f64; 3], heights: Heights) -> Result<Self, ToolpathError> {
        if !start.iter().all(|c| c.is_finite()) {
            return Err(ToolpathError::NotFinite);
        }
        Ok(Self { start, moves: Vec::new(), heights, lead_in: None, tabs: Vec::new() })
    }

    pub fn start(&self) -> [f64; 3] {
        self.start
    }

    /// Where the tool is after the last move.
    pub fn end(&self) -> [f64; 3] {
        self.moves.last().map(Move::end).unwrap_or(self.start)
    }

    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    pub fn heights(&self) -> Heights {
        self.heights
    }

    pub fn lead_in(&self) -> Option<LeadIn> {
        self.lead_in
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn rapid(&mut self, to: [f64; 3]) -> Result<(), ToolpathError> {
        self.check_point(to)?;
        self.moves.push(Move::Rapid { to });
        Ok(())
    }

    pub fn feed(&mut self, to: [f64; 3], feed_mm_min: f64) -> Result<(), ToolpathError> {
        self.check_point(to)?;
        self.check_feed(feed_mm_min)?;
        self.moves.push(Move::Feed { to, feed_mm_min });
        Ok(())
    }

    pub fn arc(&mut self, to: [f64; 3], centre: [f64; 3], clockwise: bool, feed_mm_min: f64) -> Result<(), ToolpathError> {
        self.check_point(to)?;
        self.check_point(centre)?;
        self.check_feed(feed_mm_min)?;
        self.moves.push(Move::Arc { to, centre, clockwise, feed_mm_min });
        Ok(())
    }

    pub fn set_lead_in(&mut self, lead_in: LeadIn) {
        self.lead_in = Some(lead_in);
    }

    pub fn add_tab(&mut self, tab: Tab) {
        self.tabs.push(tab);
    }

    /// Retract straight up to the retract height at the current XY.
    pub fn retract(&mut self) -> Result<(), ToolpathError> {
        let mut p = self.end();
        p[2] = self.heights.retract_mm;
        self.rapid(p)
    }

    /// Move straight up to the clearance height at the current XY.
    pub fn lift_to_clearance(&mut self) -> Result<(), ToolpathError> {
        let mut p = self.end();
        p[2] = self.heights.clearance_mm;
        self.rapid(p)
    }

    /// Total distance travelled, rapid and cutting together.
    pub fn length_mm(&self) -> f64 {
        let mut current = self.start;
        let mut total = self.lead_in.map(|lead| distance(lead.from, self.start)).unwrap_or(0.0);
        for mv in &self.moves {
            total += mv.length_from(current);
            current = mv.end();
        }
        total
    }

    /// Distance travelled while cutting (feed and arc moves only).
    pub fn cutting_length_mm(&self) -> f64 {
        let mut current = self.start;
        let mut total = self.lead_in.map(|lead| distance(lead.from, self.start)).unwrap_or(0.0);
        for mv in &self.moves {
            if mv.feed_mm_min().is_some() {
                total += mv.length_from(current);
            }
            current = mv.end();
        }
        total
    }

    /// Axis-aligned bounds of every point the path visits, as `(min, max)`.
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let mut min = self.start;
        let mut max = self.start;
        let mut points = Vec::with_capacity(self.moves.len() + 1);
        if let Some(lead) = self.lead_in {
            points.push(lead.from);
        }
        points.push(self.start);
        points.extend(self.moves.iter().map(Move::end));
        for p in points {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        (min, max)
    }

    /// The path as cutrail motion segments, in model coordinates.
    ///
    /// Arcs use the setup's Z axis as their plane normal; a clockwise arc is the same motion
    /// about the opposite normal, as cutrail defines arcs counterclockwise about `normal`.
    pub fn to_cutrail(&self, setup: &Setup) -> Vec<MotionSegment> {
        let axis = setup.z_axis().vector();
        let mut segments = Vec::with_capacity(self.moves.len() + 1);
        let mut current = self.start;
        if let Some(lead) = self.lead_in {
            segments.push(MotionSegment::linear(Pose::new(setup.to_model(lead.from), axis), Pose::new(setup.to_model(self.start), axis)));
        }
        for mv in &self.moves {
            let from = Pose::new(setup.to_model(current), axis);
            let to = Pose::new(setup.to_model(mv.end()), axis);
            match mv {
                Move::Rapid { .. } | Move::Feed { .. } => {
                    segments.push(MotionSegment::linear(from, to));
                }
                Move::Arc { centre, clockwise, .. } => {
                    let normal = if *clockwise { [-axis[0], -axis[1], -axis[2]] } else { axis };
                    segments.push(MotionSegment::arc(from, to, setup.to_model(*centre), normal));
                }
            }
            current = mv.end();
        }
        segments
    }

    /// The cutting moves as cutrail segments: feed and arc moves plus the lead-in, with rapid
    /// traverses left out (they run in air, and cutting them would remove material that the
    /// real machine never touches).
    pub fn to_cutrail_cutting(&self, setup: &Setup) -> Vec<MotionSegment> {
        let axis = setup.z_axis().vector();
        let mut segments = Vec::with_capacity(self.moves.len() + 1);
        let mut current = self.start;
        if let Some(lead) = self.lead_in {
            segments.push(MotionSegment::linear(Pose::new(setup.to_model(lead.from), axis), Pose::new(setup.to_model(self.start), axis)));
        }
        for mv in &self.moves {
            match mv {
                Move::Rapid { .. } => {}
                Move::Feed { .. } => {
                    segments.push(MotionSegment::linear(Pose::new(setup.to_model(current), axis), Pose::new(setup.to_model(mv.end()), axis)));
                }
                Move::Arc { centre, clockwise, .. } => {
                    let normal = if *clockwise { [-axis[0], -axis[1], -axis[2]] } else { axis };
                    segments.push(MotionSegment::arc(
                        Pose::new(setup.to_model(current), axis),
                        Pose::new(setup.to_model(mv.end()), axis),
                        setup.to_model(*centre),
                        normal,
                    ));
                }
            }
            current = mv.end();
        }
        segments
    }

    fn check_point(&self, p: [f64; 3]) -> Result<(), ToolpathError> {
        if !p.iter().all(|c| c.is_finite()) {
            return Err(ToolpathError::NotFinite);
        }
        Ok(())
    }

    fn check_feed(&self, feed: f64) -> Result<(), ToolpathError> {
        if !feed.is_finite() || feed <= 0.0 {
            return Err(ToolpathError::BadFeed(feed));
        }
        Ok(())
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = sub(a, b);
    dot(d, d).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::Direction;
    use crate::stock::{Material, Stock};

    fn setup() -> Setup {
        let stock = Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap();
        Setup::top_centre(&stock)
    }

    fn heights() -> Heights {
        Heights::new(10.0, 2.0).unwrap()
    }

    #[test]
    fn heights_must_be_sane() {
        assert!(Heights::new(2.0, 10.0).is_err());
        assert!(Heights::new(10.0, -1.0).is_err());
        assert_eq!(Heights::default().clearance_mm(), 10.0);
    }

    #[test]
    fn a_straight_path_reports_cutting_and_total_length() {
        let mut path = Toolpath::new([0.0, 0.0, 0.0], heights()).unwrap();
        path.rapid([0.0, 0.0, 5.0]).unwrap();
        path.feed([10.0, 0.0, 0.0], 500.0).unwrap();
        assert!((path.length_mm() - (5.0 + (100.0f64 + 25.0).sqrt())).abs() < 1e-9);
        let cutting = path.cutting_length_mm();
        assert!((cutting - (100.0f64 + 25.0).sqrt()).abs() < 1e-9);
    }

    #[test]
    fn a_quarter_arc_is_a_quarter_circle() {
        let mut path = Toolpath::new([10.0, 0.0, 0.0], heights()).unwrap();
        path.arc([0.0, 10.0, 0.0], [0.0, 0.0, 0.0], false, 400.0).unwrap();
        let expected = std::f64::consts::FRAC_PI_2 * 10.0;
        assert!((path.cutting_length_mm() - expected).abs() < 1e-9);
    }

    #[test]
    fn a_lead_in_is_the_first_cutting_move() {
        let mut path = Toolpath::new([0.0, 0.0, 0.0], heights()).unwrap();
        path.feed([10.0, 0.0, 0.0], 500.0).unwrap();
        path.set_lead_in(LeadIn::line([-5.0, 0.0, 0.0], 300.0).unwrap());
        assert!((path.cutting_length_mm() - 15.0).abs() < 1e-9);
        assert_eq!(path.to_cutrail(&setup()).len(), 2);
    }

    #[test]
    fn tabs_are_recorded_and_validated() {
        let mut path = Toolpath::new([0.0, 0.0, 0.0], heights()).unwrap();
        path.add_tab(Tab::new([10.0, 0.0, 0.0], 4.0, 1.5).unwrap());
        assert_eq!(path.tabs().len(), 1);
        assert_eq!(path.tabs()[0].width_mm(), 4.0);
        assert!(Tab::new([0.0; 3], 0.0, 1.0).is_err());
        assert!(Tab::new([0.0; 3], 4.0, 0.0).is_err());
    }

    #[test]
    fn retract_and_clearance_move_straight_up() {
        let mut path = Toolpath::new([5.0, 5.0, -3.0], heights()).unwrap();
        path.retract().unwrap();
        assert_eq!(path.end(), [5.0, 5.0, 2.0]);
        path.lift_to_clearance().unwrap();
        assert_eq!(path.end(), [5.0, 5.0, 10.0]);
    }

    #[test]
    fn bad_moves_are_rejected() {
        let mut path = Toolpath::new([0.0, 0.0, 0.0], heights()).unwrap();
        assert_eq!(path.feed([0.0, 0.0, 0.0], 0.0), Err(ToolpathError::BadFeed(0.0)));
        assert_eq!(path.feed([f64::INFINITY, 0.0, 0.0], 100.0), Err(ToolpathError::NotFinite));
    }

    #[test]
    fn converts_to_cutrail_segments_in_model_space() {
        let mut path = Toolpath::new([0.0, 0.0, 0.0], heights()).unwrap();
        path.feed([10.0, 0.0, 0.0], 500.0).unwrap();
        path.arc([10.0, 10.0, 0.0], [10.0, 5.0, 0.0], true, 500.0).unwrap();
        let segments = path.to_cutrail(&setup());
        assert_eq!(segments.len(), 2);
        match &segments[0] {
            MotionSegment::Linear { from, to } => {
                assert_eq!(from.tip, [30.0, 20.0, 20.0]);
                assert_eq!(to.tip, [40.0, 20.0, 20.0]);
                assert_eq!(from.axis, [0.0, 0.0, 1.0]);
            }
            other => panic!("expected a linear segment, got {other:?}"),
        }
        match &segments[1] {
            MotionSegment::Arc { normal, .. } => assert_eq!(*normal, [0.0, 0.0, -1.0]),
            other => panic!("expected an arc segment, got {other:?}"),
        }
    }

    #[test]
    fn bounds_cover_the_lead_in_and_the_path() {
        let mut path = Toolpath::new([0.0, 0.0, 0.0], heights()).unwrap();
        path.rapid([5.0, 5.0, 5.0]).unwrap();
        path.set_lead_in(LeadIn::line([-2.0, 0.0, 0.0], 300.0).unwrap());
        let (min, max) = path.bounds();
        assert_eq!(min, [-2.0, 0.0, 0.0]);
        assert_eq!(max, [5.0, 5.0, 5.0]);
    }

    #[test]
    fn a_clockwise_arc_sweeps_the_long_way() {
        let mut path = Toolpath::new([10.0, 0.0, 0.0], heights()).unwrap();
        path.arc([0.0, 10.0, 0.0], [0.0, 0.0, 0.0], true, 400.0).unwrap();
        let expected = 1.5 * std::f64::consts::PI * 10.0;
        assert!((path.cutting_length_mm() - expected).abs() < 1e-9);
    }

    #[test]
    fn setup_axis_is_used_for_flipped_setups() {
        let mut path = Toolpath::new([0.0, 0.0, 0.0], heights()).unwrap();
        path.feed([0.0, 10.0, 0.0], 500.0).unwrap();
        let flipped = setup().flipped();
        let segments = path.to_cutrail(&flipped);
        match &segments[0] {
            MotionSegment::Linear { from, .. } => assert_eq!(from.axis, [0.0, 0.0, -1.0]),
            other => panic!("expected a linear segment, got {other:?}"),
        }
        assert_eq!(Direction::PosZ.flipped(), Direction::NegZ);
    }
}

//! Toolpath moves: the machine instructions a calculated operation produces.
//!
//! A toolpath is an ordered list of moves between points of the part. Each move is either a
//! rapid (G0), which travels without cutting, or a feed (G1), which cuts; a feed that follows an
//! arc is a G2 (clockwise) or a G3 (counter-clockwise). The post processor turns the list into
//! program text, and the viewport draws the same list in the blueprint's colours — yellow and
//! red for rapids, cyan and blue for feeds — so what is verified is what will be cut.
//!
//! The moves carry their own start and end rather than relying on their order, so a toolpath
//! stays readable when a move is inspected on its own (the first move starts at the clearance
//! plane, the last retracts from it). Every length is millimetres, as everywhere else in this
//! crate, and every point is `[x, y, z]` in the part's coordinates.

use std::f64::consts::TAU;

/// A point in the part's coordinates, in millimetres: `[x, y, z]`.
pub type Point = [f64; 3];

/// The largest angle one arc is sampled at, in radians: five degrees keeps a lead arc round at
/// any scale the viewport shows without flooding the scene with segments.
const ARC_STEP: f64 = 5.0 * std::f64::consts::PI / 180.0;

/// Most segments one arc is sampled into, so a hostile radius cannot grow the scene unbounded.
const MAX_ARC_STEPS: usize = 720;

/// Moves shorter than this, in millimetres, are not emitted: a repeated point is not a move.
pub const MOVE_EPSILON: f64 = 1e-9;

/// How the machine travels along a move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    /// G0: travel between cuts, at the machine's rapid rate, with the tool clear of the part.
    Rapid,
    /// G1: a cutting move along the programmed feed rate (G2/G3 when it follows an arc).
    Feed,
}

impl MoveKind {
    /// Both kinds, in the order a program lists them.
    pub const ALL: [Self; 2] = [Self::Rapid, Self::Feed];

    /// The kind as the legend and the status bar name it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rapid => "Rapid",
            Self::Feed => "Feed",
        }
    }

    /// The G-code word the kind starts with: G0 for a rapid, G1 for a straight feed.
    pub const fn gcode(self) -> &'static str {
        match self {
            Self::Rapid => "G0",
            Self::Feed => "G1",
        }
    }

    /// Whether the move cuts material.
    pub const fn cuts(self) -> bool {
        matches!(self, Self::Feed)
    }
}

/// The arc a G2 or G3 move travels along: its centre and which way round it goes.
///
/// The radius is whatever the start point's distance to [`Arc::centre`] is, as a control reads
/// it from the `I`/`J` words; the arc lies in the XY plane and keeps the start's Z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arc {
    /// The arc's centre, in millimetres.
    pub centre: Point,
    /// Whether the move goes clockwise about the centre (G2) or counter-clockwise (G3).
    pub clockwise: bool,
}

impl Arc {
    /// An arc about `centre`, clockwise when `clockwise`.
    pub const fn new(centre: Point, clockwise: bool) -> Self {
        Self { centre, clockwise }
    }

    /// The G-code word a feed along this arc is: G2 clockwise, G3 counter-clockwise.
    pub const fn gcode(&self) -> &'static str {
        if self.clockwise { "G2" } else { "G3" }
    }

    /// The arc's radius, in millimetres: the distance from `from` to the centre in the XY plane.
    pub fn radius(&self, from: Point) -> f64 {
        let (dx, dy) = (from[0] - self.centre[0], from[1] - self.centre[1]);
        (dx * dx + dy * dy).sqrt()
    }
}

/// One move of a toolpath: where it starts, where it ends and whether it cuts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolpathMove {
    /// Whether the move is a rapid or a feed.
    pub kind: MoveKind,
    /// The point the move starts at, in millimetres.
    pub from: Point,
    /// The point the move ends at, in millimetres.
    pub to: Point,
    /// The arc the move follows, for a G2 or G3 feed; `None` for a straight move.
    pub arc: Option<Arc>,
}

impl ToolpathMove {
    /// A straight move of `kind` from `from` to `to`.
    pub const fn line(kind: MoveKind, from: Point, to: Point) -> Self {
        Self { kind, from, to, arc: None }
    }

    /// A feed from `from` to `to` along `arc`.
    pub const fn arc_feed(from: Point, to: Point, arc: Arc) -> Self {
        Self { kind: MoveKind::Feed, from, to, arc: Some(arc) }
    }

    /// The G-code word the move starts with: G0, G1, G2 or G3.
    pub fn gcode(&self) -> &'static str {
        match self.arc {
            Some(arc) => arc.gcode(),
            None => self.kind.gcode(),
        }
    }

    /// The distance the machine travels, in millimetres: the chord for a straight move, the arc
    /// length for an arc.
    pub fn length(&self) -> f64 {
        match self.arc {
            None => distance(self.from, self.to),
            Some(arc) => {
                let radius = arc.radius(self.from);
                let sweep = arc_sweep(self.from, self.to, arc);
                if radius.is_finite() && radius > MOVE_EPSILON && sweep.is_finite() && sweep.abs() > MOVE_EPSILON {
                    radius * sweep.abs()
                } else {
                    distance(self.from, self.to)
                }
            }
        }
    }

    /// The move as a polyline a renderer draws: its ends for a straight move, the sampled arc
    /// for a curve.
    pub fn polyline(&self) -> Vec<Point> {
        let Some(arc) = self.arc else {
            return vec![self.from, self.to];
        };
        let radius = arc.radius(self.from);
        let sweep = arc_sweep(self.from, self.to, arc);
        if !radius.is_finite() || radius <= MOVE_EPSILON || !sweep.is_finite() || sweep.abs() <= MOVE_EPSILON {
            return vec![self.from, self.to];
        }
        let steps = ((sweep.abs() / ARC_STEP).ceil() as usize).clamp(2, MAX_ARC_STEPS);
        let start = (self.from[1] - arc.centre[1]).atan2(self.from[0] - arc.centre[0]);
        let mut points = Vec::with_capacity(steps + 1);
        for step in 0..=steps {
            let t = step as f64 / steps as f64;
            let angle = start + sweep * t;
            points.push([arc.centre[0] + radius * angle.cos(), arc.centre[1] + radius * angle.sin(), self.from[2] + (self.to[2] - self.from[2]) * t]);
        }
        // The sampled curve ends a rounding error away from `to`; the move does not.
        if let Some(last) = points.last_mut() {
            *last = self.to;
        }
        points
    }
}

/// The centre angle an arc sweeps, in radians: positive counter-clockwise. Zero when the arc is
/// too small to name a direction.
fn arc_sweep(from: Point, to: Point, arc: Arc) -> f64 {
    let (r0x, r0y) = (from[0] - arc.centre[0], from[1] - arc.centre[1]);
    let (r1x, r1y) = (to[0] - arc.centre[0], to[1] - arc.centre[1]);
    if ![r0x, r0y, r1x, r1y].iter().all(|value| value.is_finite()) {
        return 0.0;
    }
    let start = r0y.atan2(r0x);
    let end = r1y.atan2(r1x);
    let mut sweep = end - start;
    if arc.clockwise {
        while sweep >= 0.0 {
            sweep -= TAU;
        }
    } else {
        while sweep <= 0.0 {
            sweep += TAU;
        }
    }
    sweep
}

/// The straight distance between two points, in millimetres.
pub fn distance(a: Point, b: Point) -> f64 {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// A calculated toolpath: the moves an operation cuts, in machining order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Toolpath {
    moves: Vec<ToolpathMove>,
}

impl Toolpath {
    /// An empty toolpath: an operation that has not been generated.
    pub fn new() -> Self {
        Self::default()
    }

    /// The moves, in machining order.
    pub fn moves(&self) -> &[ToolpathMove] {
        &self.moves
    }

    /// How many moves the path makes.
    pub fn len(&self) -> usize {
        self.moves.len()
    }

    /// Whether the path has no move at all.
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
    }

    /// Appends a move.
    pub fn push(&mut self, motion: ToolpathMove) {
        self.moves.push(motion);
    }

    /// How many moves are rapids.
    pub fn rapid_moves(&self) -> usize {
        self.moves.iter().filter(|motion| motion.kind == MoveKind::Rapid).count()
    }

    /// How many moves cut.
    pub fn feed_moves(&self) -> usize {
        self.moves.iter().filter(|motion| motion.kind.cuts()).count()
    }

    /// The distance the machine travels in rapids, in millimetres.
    pub fn rapid_length(&self) -> f64 {
        self.moves.iter().filter(|motion| motion.kind == MoveKind::Rapid).map(ToolpathMove::length).sum()
    }

    /// The distance the tool cuts, in millimetres.
    pub fn feed_length(&self) -> f64 {
        self.moves.iter().filter(|motion| motion.kind.cuts()).map(ToolpathMove::length).sum()
    }

    /// The whole distance the machine travels, in millimetres.
    pub fn total_length(&self) -> f64 {
        self.moves.iter().map(ToolpathMove::length).sum()
    }

    /// The box the path reaches, as `(min, max)` corners, with arc samples included.
    /// `None` for an empty path.
    pub fn bounds(&self) -> Option<(Point, Point)> {
        let mut bounds: Option<(Point, Point)> = None;
        for (_, points) in self.segments() {
            for point in points {
                bounds = Some(match bounds {
                    None => (point, point),
                    Some((min, max)) => (
                        [min[0].min(point[0]), min[1].min(point[1]), min[2].min(point[2])],
                        [max[0].max(point[0]), max[1].max(point[1]), max[2].max(point[2])],
                    ),
                });
            }
        }
        bounds
    }

    /// The path as the polylines a renderer draws, each with the kind of the move it comes
    /// from: one per move, so a rapid and a feed that share a segment keep their colours.
    pub fn segments(&self) -> Vec<(MoveKind, Vec<Point>)> {
        self.moves.iter().map(|motion| (motion.kind, motion.polyline())).collect()
    }

    /// The path as the toolpath tree and the status bar summarise it, such as
    /// `84 moves · 312.4 mm cutting`.
    pub fn summary(&self) -> String {
        format!("{} moves · {:.1} mm cutting", self.len(), self.feed_length())
    }
}

/// Builds a [`Toolpath`] while keeping track of where the tool is, so the calculation names only
/// the point a move goes to and the builder works out its start.
#[derive(Clone, Debug, Default)]
pub struct ToolpathBuilder {
    path: Toolpath,
    at: Option<Point>,
}

impl ToolpathBuilder {
    /// A builder with the tool nowhere: the first move sets the start position.
    pub fn new() -> Self {
        Self::default()
    }

    /// Where the tool is after the moves so far.
    pub fn position(&self) -> Option<Point> {
        self.at
    }

    /// Puts the tool at `at` without cutting or travelling there: the start of the program.
    pub fn place(&mut self, at: Point) {
        self.at = Some(at);
    }

    /// Travels to `to` at the rapid rate.
    pub fn rapid_to(&mut self, to: Point) {
        self.push(MoveKind::Rapid, to, None);
    }

    /// Cuts in a straight line to `to`.
    pub fn feed_to(&mut self, to: Point) {
        self.push(MoveKind::Feed, to, None);
    }

    /// Cuts along `arc` to `to`.
    pub fn arc_to(&mut self, to: Point, arc: Arc) {
        self.push(MoveKind::Feed, to, Some(arc));
    }

    /// The path as calculated.
    pub fn finish(self) -> Toolpath {
        self.path
    }

    /// Emits the move, or only remembers the position when the tool was nowhere yet or the move
    /// has no length.
    fn push(&mut self, kind: MoveKind, to: Point, arc: Option<Arc>) {
        if !to.iter().all(|value| value.is_finite()) {
            return;
        }
        if let Some(from) = self.at
            && distance(from, to) > MOVE_EPSILON
        {
            self.path.push(ToolpathMove { kind, from, to, arc });
        }
        self.at = Some(to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gcode_word_names_the_kind_and_the_arc_direction() {
        assert_eq!(MoveKind::Rapid.gcode(), "G0");
        assert_eq!(MoveKind::Feed.gcode(), "G1");
        assert_eq!(ToolpathMove::line(MoveKind::Rapid, [0.0; 3], [1.0; 3]).gcode(), "G0");
        assert_eq!(ToolpathMove::line(MoveKind::Feed, [0.0; 3], [1.0; 3]).gcode(), "G1");
        let clockwise = ToolpathMove::arc_feed([10.0, 0.0, 0.0], [0.0, 10.0, 0.0], Arc::new([0.0; 3], true));
        assert_eq!(clockwise.gcode(), "G2");
        let counter = ToolpathMove::arc_feed([10.0, 0.0, 0.0], [0.0, 10.0, 0.0], Arc::new([0.0; 3], false));
        assert_eq!(counter.gcode(), "G3");
    }

    #[test]
    fn an_arc_lengths_and_samples_its_quarter_circle() {
        let quarter = ToolpathMove::arc_feed([10.0, 0.0, 0.0], [0.0, 10.0, 0.0], Arc::new([0.0; 3], false));
        assert!((quarter.length() - 10.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-9, "length {}", quarter.length());
        let points = quarter.polyline();
        assert!(points.len() > 4, "an arc is sampled into segments");
        assert_eq!(points.first(), Some(&[10.0, 0.0, 0.0]));
        assert_eq!(points.last(), Some(&[0.0, 10.0, 0.0]));
        for point in &points {
            let radius = (point[0] * point[0] + point[1] * point[1]).sqrt();
            assert!((radius - 10.0).abs() < 1e-6, "every sample lies on the arc, got {point:?}");
        }
    }

    #[test]
    fn a_clockwise_arc_runs_the_other_way_round() {
        // From +X to +Y the short way is counter-clockwise; clockwise goes the long way round,
        // through -Y and -X, which a control reading a G2 word cuts.
        let clockwise = ToolpathMove::arc_feed([10.0, 0.0, 0.0], [0.0, 10.0, 0.0], Arc::new([0.0; 3], true));
        let points = clockwise.polyline();
        let midpoint = &points[points.len() / 2];
        assert!(midpoint[0] < 0.0 && midpoint[1] < 0.0, "a clockwise turn from +X to +Y runs through -X-Y, got {midpoint:?}");
        assert!((clockwise.length() - 30.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-9, "three quarters, got {}", clockwise.length());
        let counter = ToolpathMove::arc_feed([10.0, 0.0, 0.0], [0.0, 10.0, 0.0], Arc::new([0.0; 3], false));
        assert!((counter.length() - 10.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-9, "the short way is a quarter");
    }

    #[test]
    fn a_degenerate_arc_falls_back_to_its_chord() {
        let flat = ToolpathMove::arc_feed([0.0, 0.0, 0.0], [5.0, 0.0, 0.0], Arc::new([0.0; 3], false));
        assert_eq!(flat.polyline(), vec![[0.0; 3], [5.0, 0.0, 0.0]], "a zero radius has no curve to sample");
        assert!((flat.length() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn the_builder_skips_repeated_points_and_keeps_the_position() {
        let mut builder = ToolpathBuilder::new();
        builder.place([0.0, 0.0, 10.0]);
        builder.rapid_to([0.0, 0.0, 10.0]);
        assert!(builder.finish().is_empty(), "a move to where the tool already is is not a move");
        let mut builder = ToolpathBuilder::new();
        builder.rapid_to([0.0, 0.0, 10.0]);
        builder.feed_to([0.0, 0.0, 0.0]);
        assert_eq!(builder.position(), Some([0.0, 0.0, 0.0]));
        let path = builder.finish();
        assert_eq!(path.len(), 1, "the first move sets where the tool is rather than travelling from nowhere");
        assert_eq!(path.moves()[0].from, [0.0, 0.0, 10.0]);
        assert_eq!(path.rapid_moves(), 0);
        assert_eq!(path.feed_moves(), 1);
    }

    #[test]
    fn a_path_summarises_its_moves_and_cutting_length() {
        let mut path = Toolpath::new();
        path.push(ToolpathMove::line(MoveKind::Rapid, [0.0; 3], [0.0, 0.0, 10.0]));
        path.push(ToolpathMove::line(MoveKind::Feed, [0.0; 3], [40.0, 0.0, 0.0]));
        assert_eq!(path.summary(), "2 moves · 40.0 mm cutting");
        assert!((path.rapid_length() - 10.0).abs() < 1e-9);
        assert!((path.total_length() - 50.0).abs() < 1e-9);
        assert_eq!(path.bounds(), Some(([0.0; 3], [40.0, 0.0, 10.0])));
        assert_eq!(path.segments().len(), 2);
    }
}

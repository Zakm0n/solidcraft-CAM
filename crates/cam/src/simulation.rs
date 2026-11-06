//! Simulation: running a calculated toolpath over time and the stock it removes.
//!
//! Phase 2E of the Z-CAM blueprint asks for more than coloured lines: the program is verified by
//! watching the machine cut. This module holds that verification as numbers — the playback clock,
//! the tool's position along the moves and the material its flutes have taken off — so the
//! viewport draws a cylinder at the same point and the stock's surface from the same map, and the
//! headless snapshot shows exactly what the window shows.
//!
//! The material removal is the stub the blueprint allows: a height map sampled over the stock's
//! bounding box. Every feed move lowers the samples under the tool's cutting profile, which is
//! enough to read a contour clearing its channel without asking the kernel for an exact
//! subtraction on every frame. Rapids travel above the part and remove nothing.
//!
//! Every length is millimetres and every time is seconds, as everywhere else in this crate.

use crate::tool::CuttingTool;
use crate::toolpath::{MOVE_EPSILON, MoveKind, Point, Toolpath, ToolpathMove, distance};

/// The rapid traverse rate the playback assumes, in millimetres per minute: the machine travels
/// without cutting, so a rapid is over quickly and is not where verification looks.
pub const SIMULATION_RAPID_RATE: f64 = 5000.0;

/// The feed rate the playback assumes, in millimetres per minute: the operation's own feed rate is
/// a later phase's page, so the clock uses the rate a 6 mm endmill is usually programmed at.
pub const SIMULATION_FEED_RATE: f64 = 600.0;

/// How many samples the stock's longest side is divided into: enough that a 6 mm cutter leaves a
/// readable channel, few enough that rebuilding the map is cheap in the frame loop.
pub const STOCK_SAMPLES: usize = 40;

/// The closest two height-map samples may be, in millimetres, so a tiny stock does not ask for an
/// unreasonable grid.
pub const MIN_STOCK_CELL: f64 = 0.25;

/// Most positions one cutting move is sampled at, so a hostile radius cannot stall the frame.
pub const MAX_CUT_STEPS: usize = 4096;

/// The slowest playback speed, as a multiple of the clock.
pub const MIN_PLAYBACK_SPEED: f64 = 0.1;

/// The fastest playback speed, as a multiple of the clock.
pub const MAX_PLAYBACK_SPEED: f64 = 100.0;

/// Two distances closer than this, in millimetres, are the same place on the path.
const EPSILON: f64 = 1e-9;

/// Where the tool is at one moment of the program: the point its tip has reached, and the move it
/// is travelling along.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolSample {
    /// The tool tip's position in the part's coordinates, in millimetres.
    pub position: Point,
    /// Whether the move in progress is a rapid or a feed.
    pub kind: MoveKind,
    /// The index of the move in progress.
    pub index: usize,
}

/// A toolpath laid out along a clock: every move's start distance and start time, so a clock time
/// and a distance along the path convert into one another and neither has to be walked.
#[derive(Clone, Debug, PartialEq)]
pub struct SimulationPath {
    moves: Vec<ToolpathMove>,
    /// Distance from the path's start to the start of each move, in millimetres.
    starts: Vec<f64>,
    /// Time from the path's start to the start of each move, in seconds.
    times: Vec<f64>,
    /// The path's length, in millimetres.
    total: f64,
    /// How long the path takes, in seconds.
    duration: f64,
    /// The traverse rate, in millimetres per minute.
    rapid_rate: f64,
    /// The cutting rate, in millimetres per minute.
    feed_rate: f64,
}

impl Default for SimulationPath {
    /// An empty path timed at the crate's default rates.
    fn default() -> Self {
        Self::new(&Toolpath::new(), SIMULATION_RAPID_RATE, SIMULATION_FEED_RATE)
    }
}

impl SimulationPath {
    /// Lays `toolpath` out along a clock that runs rapids at `rapid_rate` and feeds at `feed_rate`,
    /// in millimetres per minute. A rate that is not a positive number falls back to the default,
    /// so a clock always exists.
    pub fn new(toolpath: &Toolpath, rapid_rate: f64, feed_rate: f64) -> Self {
        let rapid_rate = usable_rate(rapid_rate, SIMULATION_RAPID_RATE);
        let feed_rate = usable_rate(feed_rate, SIMULATION_FEED_RATE);
        let moves = toolpath.moves().to_vec();
        let mut starts = Vec::with_capacity(moves.len());
        let mut times = Vec::with_capacity(moves.len());
        let (mut at, mut clock) = (0.0, 0.0);
        for motion in &moves {
            starts.push(at);
            times.push(clock);
            let length = motion.length().max(0.0);
            at += length;
            clock += length / (rate_of(motion.kind, rapid_rate, feed_rate) / 60.0);
        }
        Self { moves, starts, times, total: at, duration: clock, rapid_rate, feed_rate }
    }

    /// The moves, in machining order.
    pub fn moves(&self) -> &[ToolpathMove] {
        &self.moves
    }

    /// How many moves the path makes.
    pub fn len(&self) -> usize {
        self.moves.len()
    }

    /// Whether the path has no move to run.
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
    }

    /// The path's length, in millimetres.
    pub fn total_length(&self) -> f64 {
        self.total
    }

    /// How long the whole path takes at the playback's rates, in seconds.
    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// The rate a move travels at, in millimetres per minute.
    pub fn move_rate(&self, kind: MoveKind) -> f64 {
        rate_of(kind, self.rapid_rate, self.feed_rate)
    }

    /// The tool at `distance` millimetres along the path, or `None` for an empty path. A distance
    /// outside the path is clamped to its ends, so a scrub past either end stops at it.
    pub fn sample_at(&self, distance: f64) -> Option<ToolSample> {
        if self.moves.is_empty() || !distance.is_finite() {
            return None;
        }
        let distance = distance.clamp(0.0, self.total);
        let index = self.move_index(distance);
        let motion = self.moves.get(index)?;
        let start = self.starts.get(index).copied().unwrap_or(0.0);
        let position = point_along(motion, distance - start);
        Some(ToolSample { position, kind: motion.kind, index })
    }

    /// The distance from the path's start after `seconds` of playback, clamped to the path.
    pub fn distance_at_time(&self, seconds: f64) -> f64 {
        if self.moves.is_empty() || !(seconds > 0.0) {
            return 0.0;
        }
        if !seconds.is_finite() || seconds >= self.duration {
            return self.total;
        }
        let index = self.time_index(seconds);
        let start_time = self.times.get(index).copied().unwrap_or(0.0);
        let start = self.starts.get(index).copied().unwrap_or(0.0);
        let Some(motion) = self.moves.get(index) else { return self.total };
        let rate = self.move_rate(motion.kind);
        (start + (seconds - start_time) * rate / 60.0).clamp(start, self.move_end(index))
    }

    /// The time from the path's start at which the tool has travelled `distance` millimetres.
    pub fn time_at_distance(&self, distance: f64) -> f64 {
        if self.moves.is_empty() || !(distance > 0.0) || !distance.is_finite() {
            return 0.0;
        }
        if distance >= self.total {
            return self.duration;
        }
        let index = self.move_index(distance);
        let start = self.starts.get(index).copied().unwrap_or(0.0);
        let start_time = self.times.get(index).copied().unwrap_or(0.0);
        let Some(motion) = self.moves.get(index) else { return self.duration };
        let rate = self.move_rate(motion.kind);
        start_time + (distance - start) * 60.0 / rate
    }

    /// The end distance of the move the tool is on, so a step advances one whole move.
    pub fn next_move_end(&self, distance: f64) -> f64 {
        if self.moves.is_empty() || !distance.is_finite() || distance >= self.total {
            return self.total;
        }
        let index = self.move_index(distance);
        self.move_end(index)
    }

    /// The cutting spans between `from` and `to` millimetres along the path, as point pairs the
    /// stock's height map takes material off along. Rapids contribute nothing, and a curve is
    /// split where it is sampled, so an arc cuts the same shape it is drawn as.
    pub fn feed_spans(&self, from: f64, to: f64) -> Vec<(Point, Point)> {
        let mut spans = Vec::new();
        if self.moves.is_empty() || !from.is_finite() || !to.is_finite() {
            return spans;
        }
        let (low, high) = (from.min(to).clamp(0.0, self.total), from.max(to).clamp(0.0, self.total));
        if !(high > low) {
            return spans;
        }
        for (index, motion) in self.moves.iter().enumerate() {
            if !motion.kind.cuts() {
                continue;
            }
            let start = self.starts.get(index).copied().unwrap_or(0.0);
            let end = self.move_end(index);
            let (a, b) = (low.max(start), high.min(end));
            if !(b > a) {
                continue;
            }
            let points = polyline_between(motion, a - start, b - start);
            for pair in points.windows(2) {
                if let [one, two] = pair
                    && distance(*one, *two) > MOVE_EPSILON
                {
                    spans.push((*one, *two));
                }
            }
        }
        spans
    }

    /// The index of the move containing `distance`.
    fn move_index(&self, distance: f64) -> usize {
        let count = self.starts.partition_point(|start| *start <= distance + EPSILON);
        count.saturating_sub(1).min(self.moves.len().saturating_sub(1))
    }

    /// The index of the move being travelled at `seconds`.
    fn time_index(&self, seconds: f64) -> usize {
        let count = self.times.partition_point(|time| *time <= seconds + EPSILON);
        count.saturating_sub(1).min(self.moves.len().saturating_sub(1))
    }

    /// The distance at which move `index` ends.
    fn move_end(&self, index: usize) -> f64 {
        self.starts.get(index + 1).copied().unwrap_or(self.total)
    }
}

/// A rate as the clock uses it: `raw` when it is a positive number, `fallback` otherwise.
fn usable_rate(raw: f64, fallback: f64) -> f64 {
    if raw.is_finite() && raw > 0.0 { raw } else { fallback }
}

/// The rate a move of `kind` travels at, in millimetres per minute.
fn rate_of(kind: MoveKind, rapid_rate: f64, feed_rate: f64) -> f64 {
    match kind {
        MoveKind::Rapid => rapid_rate,
        MoveKind::Feed => feed_rate,
    }
}

/// The point `local` millimetres along `motion`, following its sampled curve.
fn point_along(motion: &ToolpathMove, local: f64) -> Point {
    let polyline = motion.polyline();
    if polyline.len() < 2 {
        return motion.from;
    }
    let cumulative = cumulative_lengths(&polyline);
    let span = cumulative.last().copied().unwrap_or(0.0);
    let length = motion.length();
    if !(span > MOVE_EPSILON) || !(length > MOVE_EPSILON) {
        return motion.from;
    }
    let wanted = (local / length).clamp(0.0, 1.0) * span;
    point_on_polyline(&polyline, &cumulative, wanted)
}

/// The polyline points of `motion` between `from` and `to` millimetres along it, with the ends
/// interpolated, so a span never cuts a corner the move does not.
fn polyline_between(motion: &ToolpathMove, from: f64, to: f64) -> Vec<Point> {
    let polyline = motion.polyline();
    if polyline.len() < 2 {
        return Vec::new();
    }
    let cumulative = cumulative_lengths(&polyline);
    let span = cumulative.last().copied().unwrap_or(0.0);
    let length = motion.length();
    if !(span > MOVE_EPSILON) || !(length > MOVE_EPSILON) {
        return Vec::new();
    }
    let scale = span / length;
    let (low, high) = (from.clamp(0.0, length) * scale, to.clamp(0.0, length) * scale);
    let mut points = vec![point_on_polyline(&polyline, &cumulative, low)];
    for (index, point) in polyline.iter().enumerate() {
        let at = cumulative.get(index).copied().unwrap_or(0.0);
        if at > low + EPSILON && at < high - EPSILON {
            points.push(*point);
        }
    }
    points.push(point_on_polyline(&polyline, &cumulative, high));
    points
}

/// The distance from the polyline's start to each of its points.
fn cumulative_lengths(points: &[Point]) -> Vec<f64> {
    let mut cumulative = Vec::with_capacity(points.len());
    let mut total = 0.0;
    for (index, point) in points.iter().enumerate() {
        if index > 0
            && let Some(previous) = points.get(index - 1)
        {
            total += distance(*previous, *point);
        }
        cumulative.push(total);
    }
    cumulative
}

/// The point `at` millimetres along a polyline, interpolating inside its segment.
fn point_on_polyline(points: &[Point], cumulative: &[f64], at: f64) -> Point {
    let Some(first) = points.first() else { return [0.0; 3] };
    if at <= 0.0 {
        return *first;
    }
    for (index, point) in points.iter().enumerate() {
        let start = cumulative.get(index.saturating_sub(1)).copied().unwrap_or(0.0);
        let end = cumulative.get(index).copied().unwrap_or(0.0);
        if at <= end + EPSILON && end > start {
            let Some(previous) = points.get(index.saturating_sub(1)) else { return *point };
            let t = ((at - start) / (end - start)).clamp(0.0, 1.0);
            return [
                previous[0] + (point[0] - previous[0]) * t,
                previous[1] + (point[1] - previous[1]) * t,
                previous[2] + (point[2] - previous[2]) * t,
            ];
        }
    }
    points.last().copied().unwrap_or(*first)
}

/// The stock's surface as a height map: a grid of samples over the stock's bounding box, each at
/// the Z the tool has left there. It starts at the box's top and only ever drops, which is what
/// a milling cutter does to raw material.
#[derive(Clone, Debug, PartialEq)]
pub struct StockGrid {
    /// `[min x, min y, min z, max x, max y, max z]`, in millimetres.
    extents: [f64; 6],
    /// Samples along X, counting both edges.
    columns: usize,
    /// Samples along Y, counting both edges.
    rows: usize,
    /// The surface Z at each sample, row-major (`row * columns + column`).
    heights: Vec<f64>,
}

impl StockGrid {
    /// A height map over `extents`, sampled so the longest side has [`STOCK_SAMPLES`] cells.
    /// `None` unless the extents describe a box with a positive, finite size.
    pub fn from_extents(extents: [f64; 6]) -> Option<Self> {
        let [x0, y0, z0, x1, y1, z1] = extents;
        if ![x0, y0, z0, x1, y1, z1].iter().all(|value| value.is_finite()) || !(x1 > x0) || !(y1 > y0) || !(z1 > z0) {
            return None;
        }
        let longest = (x1 - x0).max(y1 - y0);
        // The cell the samples land on: the longest side is STOCK_SAMPLES cells wide.
        let cell = (longest / STOCK_SAMPLES as f64).max(MIN_STOCK_CELL);
        let columns = ((x1 - x0) / cell).round().max(1.0) as usize + 1;
        let rows = ((y1 - y0) / cell).round().max(1.0) as usize + 1;
        let mut grid = Self { extents, columns, rows, heights: Vec::with_capacity(columns * rows) };
        grid.reset();
        Some(grid)
    }

    /// Puts every sample back at the stock's top: raw material that nothing has cut.
    pub fn reset(&mut self) {
        self.heights.clear();
        self.heights.resize(self.columns * self.rows, self.extents[5]);
    }

    /// The box the map covers, `[min x, min y, min z, max x, max y, max z]` in millimetres.
    pub fn extents(&self) -> [f64; 6] {
        self.extents
    }

    /// Samples along X, counting both edges.
    pub fn columns(&self) -> usize {
        self.columns
    }

    /// Samples along Y, counting both edges.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The sample spacing along X and Y, in millimetres.
    pub fn cell_size(&self) -> (f64, f64) {
        let [x0, y0, _, x1, y1, _] = self.extents;
        let cell_x = if self.columns > 1 { (x1 - x0) / (self.columns - 1) as f64 } else { x1 - x0 };
        let cell_y = if self.rows > 1 { (y1 - y0) / (self.rows - 1) as f64 } else { y1 - y0 };
        (cell_x, cell_y)
    }

    /// The X of the sample in `column`.
    pub fn x_of(&self, column: usize) -> f64 {
        let [x0, _, _, _, _, _] = self.extents;
        let (cell_x, _) = self.cell_size();
        x0 + cell_x * column as f64
    }

    /// The Y of the sample in `row`.
    pub fn y_of(&self, row: usize) -> f64 {
        let [_, y0, _, _, _, _] = self.extents;
        let (_, cell_y) = self.cell_size();
        y0 + cell_y * row as f64
    }

    /// The surface Z of the sample at `column`, `row`.
    pub fn height(&self, column: usize, row: usize) -> f64 {
        self.heights.get(row * self.columns + column).copied().unwrap_or(self.extents[5])
    }

    /// The highest surface of the stock: the uncut top.
    pub fn top(&self) -> f64 {
        self.extents[5]
    }

    /// The lowest surface the tool has left.
    pub fn min_height(&self) -> f64 {
        self.heights.iter().copied().fold(self.extents[5], f64::min)
    }

    /// Whether nothing has been cut: every sample still at the top.
    pub fn is_untouched(&self) -> bool {
        self.heights.iter().all(|height| (*height - self.extents[5]).abs() <= EPSILON)
    }

    /// Cuts the material from `from` to `to` as `tool` passes: the samples under its cutting
    /// profile drop to the surface the flutes leave. A straight or sampled move is stepped along
    /// closely enough that no sample is missed between two positions.
    pub fn cut_segment(&mut self, from: Point, to: Point, tool: &CuttingTool) {
        let radius = tool.diameter / 2.0;
        if !(radius > 0.0) || !radius.is_finite() || !from.iter().chain(to.iter()).all(|value| value.is_finite()) {
            return;
        }
        let (cell_x, cell_y) = self.cell_size();
        let step = (radius.min(cell_x.min(cell_y)) * 0.5).max(0.05);
        let length = distance(from, to);
        let steps = ((length / step).ceil() as usize).clamp(1, MAX_CUT_STEPS);
        for index in 0..=steps {
            let t = index as f64 / steps as f64;
            let point = [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t, from[2] + (to[2] - from[2]) * t];
            self.cut_at(point, tool);
        }
    }

    /// Cuts the samples under the tool standing with its tip at `tip`.
    pub fn cut_at(&mut self, tip: Point, tool: &CuttingTool) {
        if !tip.iter().all(|value| value.is_finite()) {
            return;
        }
        let radius = tool.diameter / 2.0;
        if !(radius > 0.0) || !radius.is_finite() {
            return;
        }
        let corner = tool.effective_corner_radius().clamp(0.0, radius);
        let [x0, y0, z0, x1, y1, z1] = self.extents;
        let (cell_x, cell_y) = self.cell_size();
        if !(cell_x > 0.0) || !(cell_y > 0.0) {
            return;
        }
        let left = (((tip[0] - radius - x0) / cell_x).floor() as isize).clamp(0, self.columns as isize - 1);
        let right = (((tip[0] + radius - x0) / cell_x).ceil() as isize).clamp(0, self.columns as isize - 1);
        let bottom = (((tip[1] - radius - y0) / cell_y).floor() as isize).clamp(0, self.rows as isize - 1);
        let top = (((tip[1] + radius - y0) / cell_y).ceil() as isize).clamp(0, self.rows as isize - 1);
        for row in bottom as usize..=top as usize {
            let y = (self.y_of(row)).clamp(y0, y1);
            for column in left as usize..=right as usize {
                let x = (self.x_of(column)).clamp(x0, x1);
                let (dx, dy) = (x - tip[0], y - tip[1]);
                let offset = (dx * dx + dy * dy).sqrt();
                if offset > radius + EPSILON {
                    continue;
                }
                // A flat tool leaves its tip's Z; a corner radius lifts the surface away from the
                // tip the further the sample is from the centre.
                let shoulder = (offset - (radius - corner)).max(0.0);
                let drop = (corner * corner - shoulder * shoulder).max(0.0).sqrt();
                let surface = (tip[2] + corner - drop).clamp(z0, z1);
                let index = row * self.columns + column;
                if let Some(height) = self.heights.get_mut(index)
                    && surface < *height - EPSILON
                {
                    *height = surface;
                }
            }
        }
    }
}

/// The simulated program: a calculated path, the tool that cuts it, the stock's height map and
/// the clock that carries the tool along it.
///
/// The map is advanced as the clock does, so playback costs only the newly machined span; a scrub
/// backwards rebuilds it and runs it again to the new position, which keeps what is shown a
/// function of the position alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Simulation {
    path: SimulationPath,
    tool: Option<CuttingTool>,
    grid: Option<StockGrid>,
    /// The path distance the height map has been cut to, in millimetres.
    machined: f64,
    /// Seconds elapsed since the program's start.
    elapsed: f64,
    playing: bool,
    speed: f64,
}

impl Default for Simulation {
    /// A simulation with nothing to run: what a new workspace holds until an operation exists.
    fn default() -> Self {
        Self { path: SimulationPath::default(), tool: None, grid: None, machined: 0.0, elapsed: 0.0, playing: false, speed: 1.0 }
    }
}

impl Simulation {
    /// Simulates `toolpath` cut by `tool` out of stock with `stock_extents`, stopped at the start.
    /// A `None` tool or stock leaves that part out rather than refusing the program: the path can
    /// still be played, if not cut.
    pub fn new(toolpath: &Toolpath, tool: Option<&CuttingTool>, stock_extents: Option<[f64; 6]>) -> Self {
        let grid = stock_extents.and_then(StockGrid::from_extents);
        Self {
            path: SimulationPath::new(toolpath, SIMULATION_RAPID_RATE, SIMULATION_FEED_RATE),
            tool: tool.cloned(),
            grid,
            machined: 0.0,
            elapsed: 0.0,
            playing: false,
            speed: 1.0,
        }
    }

    /// The path being run.
    pub fn path(&self) -> &SimulationPath {
        &self.path
    }

    /// The tool that cuts the path, if the library still holds the operation's tool.
    pub fn tool(&self) -> Option<&CuttingTool> {
        self.tool.as_ref()
    }

    /// The stock's height map, if the setup defines a box the map can be sampled over.
    pub fn grid(&self) -> Option<&StockGrid> {
        self.grid.as_ref()
    }

    /// Whether the clock is running.
    pub fn playing(&self) -> bool {
        self.playing
    }

    /// The playback speed, as a multiple of real time.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Sets the playback speed, clamped to the range a person can follow.
    pub fn set_speed(&mut self, speed: f64) {
        if speed.is_finite() {
            self.speed = speed.clamp(MIN_PLAYBACK_SPEED, MAX_PLAYBACK_SPEED);
        }
    }

    /// The path's length, in millimetres.
    pub fn total_length(&self) -> f64 {
        self.path.total_length()
    }

    /// How long the whole program takes at the playback's rates, in seconds.
    pub fn duration(&self) -> f64 {
        self.path.duration()
    }

    /// The distance the tool has travelled from the path's start, in millimetres.
    pub fn position(&self) -> f64 {
        self.path.distance_at_time(self.elapsed)
    }

    /// The tool's tip and the move it is on, or `None` for a program with no moves.
    pub fn sample(&self) -> Option<ToolSample> {
        self.path.sample_at(self.position())
    }

    /// The distance the height map is cut to, in millimetres.
    pub fn machined(&self) -> f64 {
        self.machined
    }

    /// How much of the path has been run, from 0 to 1.
    pub fn fraction(&self) -> f64 {
        let total = self.total_length();
        if total > 0.0 { (self.position() / total).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// Whether the program has been run to its end.
    pub fn finished(&self) -> bool {
        let total = self.total_length();
        total > 0.0 && self.position() >= total - EPSILON
    }

    /// Whether there is a move to run at all.
    pub fn is_empty(&self) -> bool {
        self.path.is_empty()
    }

    /// Starts the clock. A program that has already finished starts again from the beginning, so
    /// pressing Play at the end replays it rather than doing nothing.
    pub fn play(&mut self) {
        if self.is_empty() {
            return;
        }
        if self.finished() {
            self.elapsed = 0.0;
            self.refresh();
        }
        self.playing = true;
    }

    /// Stops the clock where it is.
    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Starts or stops the clock.
    pub fn toggle(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Moves the clock on by `seconds` of real time at the playback's speed, and stops it at the
    /// end of the program. Does nothing while paused.
    pub fn advance(&mut self, seconds: f64) {
        if !self.playing || !(seconds > 0.0) || !seconds.is_finite() {
            return;
        }
        self.elapsed += seconds * self.speed;
        if self.elapsed >= self.duration() {
            self.elapsed = self.duration();
            self.playing = false;
        }
        self.refresh();
    }

    /// Steps the tool to the end of the next move and stops the clock there.
    pub fn step(&mut self) {
        if self.is_empty() {
            return;
        }
        self.playing = false;
        let target = self.path.next_move_end(self.position());
        self.elapsed = self.path.time_at_distance(target);
        self.refresh();
    }

    /// Puts the tool at `distance` millimetres along the path and stops the clock there: what a
    /// scrub of the timeline does.
    pub fn set_position(&mut self, distance: f64) {
        self.playing = false;
        self.elapsed = self.path.time_at_distance(distance);
        self.refresh();
    }

    /// Runs the program back to its start and restores the stock to raw material.
    pub fn reset(&mut self) {
        self.playing = false;
        self.elapsed = 0.0;
        self.machined = 0.0;
        if let Some(grid) = self.grid.as_mut() {
            grid.reset();
        }
    }

    /// The program as the playback panel names it, such as `24.5 / 312.4 mm · 3 of 84 moves`.
    pub fn summary(&self) -> String {
        if self.is_empty() {
            return "no toolpath".to_string();
        }
        let moves = self.path.len();
        let index = self.sample().map_or(0, |sample| sample.index + 1);
        format!("{:.1} / {:.1} mm · {index} of {moves} moves", self.position(), self.total_length())
    }

    /// Brings the height map up to the clock's position: cuts forwards, or rebuilds and recuts
    /// when the scrub went backwards.
    fn refresh(&mut self) {
        let target = self.position();
        if target + EPSILON < self.machined {
            if let Some(grid) = self.grid.as_mut() {
                grid.reset();
            }
            self.machined = 0.0;
        }
        if !(target > self.machined) {
            return;
        }
        if let Some(grid) = self.grid.as_mut()
            && let Some(tool) = self.tool.as_ref()
        {
            for (from, to) in self.path.feed_spans(self.machined, target) {
                grid.cut_segment(from, to, tool);
            }
        }
        self.machined = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{CuttingTool, ToolType};
    use crate::toolpath::ToolpathBuilder;

    /// A program that rapids down, cuts 40 mm, then rapids out.
    fn program() -> Toolpath {
        let mut builder = ToolpathBuilder::new();
        builder.place([10.0, 0.0, 10.0]);
        builder.rapid_to([10.0, 0.0, 0.0]);
        builder.feed_to([50.0, 0.0, 0.0]);
        builder.rapid_to([50.0, 0.0, 10.0]);
        builder.finish()
    }

    fn tool() -> CuttingTool {
        CuttingTool::new(ToolType::FlatEndmill, 6.0)
    }

    #[test]
    fn a_paths_clock_knows_its_length_and_duration() {
        let path = SimulationPath::new(&program(), 6000.0, 600.0);
        assert_eq!(path.len(), 3);
        // 10 mm rapid at 6000 mm/min, 40 mm feed at 600 mm/min, 10 mm rapid back.
        assert!((path.total_length() - 60.0).abs() < 1e-9);
        assert!((path.duration() - (10.0 / 100.0 + 40.0 / 10.0 + 10.0 / 100.0)).abs() < 1e-9, "duration {}", path.duration());
        assert_eq!(SimulationPath::default().len(), 0);
        assert!(SimulationPath::default().is_empty());
    }

    #[test]
    fn the_tool_is_sampled_along_each_move() {
        let path = SimulationPath::new(&program(), 6000.0, 600.0);
        let start = path.sample_at(0.0).expect("the path has moves");
        assert_eq!(start.position, [10.0, 0.0, 10.0], "the path starts where the builder placed the tool");
        assert_eq!(start.kind, MoveKind::Rapid);
        let middle = path.sample_at(30.0).expect("the path has moves");
        assert_eq!(middle.kind, MoveKind::Feed);
        assert!((middle.position[0] - 30.0).abs() < 1e-9, "halfway along the 40 mm feed, got {:?}", middle.position);
        assert_eq!(path.sample_at(1e9).map(|sample| sample.position), Some([50.0, 0.0, 10.0]), "past the end is the end");
        assert!(SimulationPath::default().sample_at(0.0).is_none());
    }

    #[test]
    fn time_and_distance_convert_both_ways() {
        let path = SimulationPath::new(&program(), 6000.0, 600.0);
        for distance in [0.0, 5.0, 20.0, 55.0, 60.0] {
            let seconds = path.time_at_distance(distance);
            assert!((path.distance_at_time(seconds) - distance).abs() < 1e-6, "distance {distance} round-trips");
        }
        assert_eq!(path.distance_at_time(-1.0), 0.0);
        assert!((path.distance_at_time(1e9) - 60.0).abs() < 1e-9);
    }

    #[test]
    fn stepping_lands_on_the_end_of_each_move() {
        let path = SimulationPath::new(&program(), 6000.0, 600.0);
        assert!((path.next_move_end(0.0) - 10.0).abs() < 1e-9);
        assert!((path.next_move_end(10.0) - 50.0).abs() < 1e-9);
        assert!((path.next_move_end(30.0) - 50.0).abs() < 1e-9, "a step finishes the move it is in");
        assert!((path.next_move_end(60.0) - 60.0).abs() < 1e-9);
    }

    #[test]
    fn only_feeds_machine_material() {
        let path = SimulationPath::new(&program(), 6000.0, 600.0);
        let spans = path.feed_spans(0.0, 60.0);
        assert_eq!(spans.len(), 1, "the rapid contributes no span");
        assert_eq!(spans[0], ([10.0, 0.0, 0.0], [50.0, 0.0, 0.0]));
        let half = path.feed_spans(10.0, 30.0);
        assert_eq!(half, vec![([10.0, 0.0, 0.0], [30.0, 0.0, 0.0])], "a span is clipped to the run");
    }

    #[test]
    fn a_height_map_starts_at_the_stock_top_and_cuts_under_the_tool() {
        let mut grid = StockGrid::from_extents([0.0, 0.0, -5.0, 40.0, 30.0, 0.0]).expect("the box is usable");
        assert!(grid.is_untouched());
        assert_eq!(grid.top(), 0.0);
        grid.cut_at([20.0, 15.0, -2.0], &tool());
        assert!(!grid.is_untouched());
        assert!((grid.min_height() + 2.0).abs() < 1e-9, "the flat tool leaves its tip's Z, got {}", grid.min_height());
        // Far from the tool the sample stays at the top.
        assert_eq!(grid.height(0, 0), 0.0);
        assert_eq!(StockGrid::from_extents([0.0, 0.0, 0.0, 0.0, 1.0, 1.0]), None);
    }

    #[test]
    fn a_ball_tool_leaves_a_dished_surface() {
        let mut ball = CuttingTool::new(ToolType::BallEndmill, 10.0);
        ball.corner_radius = 5.0;
        let mut grid = StockGrid::from_extents([0.0, 0.0, -5.0, 40.0, 30.0, 0.0]).expect("the box is usable");
        grid.cut_at([20.0, 15.0, -1.0], &ball);
        let centre = grid.min_height();
        assert!((centre + 1.0).abs() < 1e-9, "the ball's tip reaches its centre, got {centre}");
        // A sample near the tool's edge sits higher than the centre.
        let mut edge = f64::INFINITY;
        for column in 0..grid.columns() {
            let x = grid.x_of(column);
            if (x - 24.0).abs() < 1.0 {
                edge = edge.min(grid.height(column, grid.rows() / 2));
            }
        }
        assert!(edge > centre, "the ball's shoulder is above its tip, got {edge} vs {centre}");
    }

    #[test]
    fn a_simulation_scrubs_forwards_backwards_and_resets() {
        let stock = Some([0.0, 0.0, -5.0, 40.0, 30.0, 5.0]);
        let mut simulation = Simulation::new(&program(), Some(&tool()), stock);
        assert!((simulation.total_length() - 60.0).abs() < 1e-9);
        assert!(simulation.grid().is_some_and(StockGrid::is_untouched));

        simulation.set_position(50.0);
        assert!((simulation.position() - 50.0).abs() < 1e-9);
        assert!(simulation.grid().is_some_and(|grid| !grid.is_untouched()), "the feed removes material");
        let cut = simulation.grid().map(|grid| grid.min_height()).unwrap_or(f64::NAN);
        assert!((cut - 0.0).abs() < 1e-9, "the feed's tip is at Z 0, got {cut}");

        simulation.set_position(0.0);
        assert!(simulation.grid().is_some_and(StockGrid::is_untouched), "a scrub back restores raw material");

        simulation.set_position(60.0);
        assert!(simulation.finished());
        simulation.reset();
        assert_eq!(simulation.position(), 0.0);
        assert!(!simulation.playing());
        assert!(simulation.grid().is_some_and(StockGrid::is_untouched));
    }

    #[test]
    fn playback_advances_the_clock_and_stops_at_the_end() {
        let mut simulation = Simulation::new(&program(), Some(&tool()), None);
        assert_eq!(simulation.summary().split(" · ").nth(1), Some("1 of 3 moves"));
        simulation.play();
        assert!(simulation.playing());
        simulation.advance(0.033);
        assert!(simulation.position() > 0.0, "the clock moves the tool");
        simulation.advance(1e6);
        assert!(!simulation.playing(), "playback stops at the end");
        assert!(simulation.finished());
        simulation.play();
        assert!((simulation.position() - 0.0).abs() < 1e-9, "playing at the end replays from the start");
        simulation.pause();
        assert!(!simulation.playing());

        let mut empty = Simulation::default();
        empty.play();
        assert!(!empty.playing(), "an empty program does not run");
        assert_eq!(empty.summary(), "no toolpath");
    }

    #[test]
    fn a_step_finishes_the_move_the_tool_is_on() {
        let mut simulation = Simulation::new(&program(), Some(&tool()), None);
        simulation.step();
        assert!((simulation.position() - 10.0).abs() < 1e-9);
        simulation.step();
        assert!((simulation.position() - 50.0).abs() < 1e-9);
        simulation.step();
        assert!(simulation.finished());
        assert!(!simulation.playing());
    }

    #[test]
    fn the_playback_speed_is_kept_in_a_readable_range() {
        let mut simulation = Simulation::default();
        simulation.set_speed(1e9);
        assert_eq!(simulation.speed(), MAX_PLAYBACK_SPEED);
        simulation.set_speed(-2.0);
        assert_eq!(simulation.speed(), MIN_PLAYBACK_SPEED);
        simulation.set_speed(f64::NAN);
        assert_eq!(simulation.speed(), MIN_PLAYBACK_SPEED);
    }
}

//! The 2D Contour operation: cut along a chain of curves at one or more depths.
//!
//! A 2D Contour follows the selected chain and does not clean out an enclosed area. It is the
//! backbone of 2D milling, and its parameters are the pages a Mastercam user edits: the chained
//! geometry, the tool, the cut parameters (compensation, direction and stock to leave), the depth
//! cuts, the multiple passes across the wall, the lead in/out moves and the linking heights.
//!
//! The parameter groups mirror the help the blueprint points at:
//!
//! * `Cut Parameters` — compensation type, cutter side and stock to leave on walls and floors.
//! * `Depth Cuts` — maximum rough step, finish cuts and whether the tool stays down between them.
//! * `Multi Passes` — rough and finish passes across the wall, their spacing and spring passes.
//! * `Lead In/Out` — the entry and exit moves, as a line or an arc.
//! * `Linking Parameters` — clearance, retract, feed plane, top of stock and depth, absolute or
//!   incremental.
//!
//! Every length is millimetres, every angle degrees. The values validate rather than assume, so
//! a dialog can name the field that is wrong before an operation is stored.

use thiserror::Error;

use crate::stock::EntityId;
use crate::tool::CuttingTool;
use crate::toolpath::{Arc, MOVE_EPSILON, Point, Toolpath, ToolpathBuilder, distance};

/// Which side of the chain the cutter runs on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CutDirection {
    /// Climb milling: the cutter travels to the left of the chain.
    #[default]
    Left,
    /// Conventional milling: the cutter travels to the right of the chain.
    Right,
}

impl CutDirection {
    /// Both directions, in the order the dialog lists them.
    pub const ALL: [Self; 2] = [Self::Left, Self::Right];

    /// The direction as the dialog names it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "Left (climb)",
            Self::Right => "Right (conventional)",
        }
    }
}

/// How cutter compensation is applied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compensation {
    /// The toolpath is offset in the program; the control receives no compensation codes.
    #[default]
    Computer,
    /// The program names the chain and the machine's control applies the offset.
    Control,
    /// No compensation: the tool centre follows the chain.
    Off,
}

impl Compensation {
    /// Every type, in the order the dialog lists them.
    pub const ALL: [Self; 3] = [Self::Computer, Self::Control, Self::Off];

    /// The type as the dialog names it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Computer => "Computer",
            Self::Control => "Control",
            Self::Off => "Off",
        }
    }
}

/// The order the depth cuts are machined in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DepthOrder {
    /// From the top of the stock down to the final depth.
    #[default]
    TopDown,
    /// From the final depth back up to the top of the stock.
    BottomUp,
}

impl DepthOrder {
    /// Both orders, in the order the dialog lists them.
    pub const ALL: [Self; 2] = [Self::TopDown, Self::BottomUp];

    /// The order as the dialog names it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::TopDown => "Top down",
            Self::BottomUp => "Bottom up",
        }
    }
}

/// The `Cut Parameters` page: how the cutter meets the chain and what material it leaves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutParameters {
    /// How the cutter is compensated.
    pub compensation: Compensation,
    /// Which side of the chain the cutter runs on.
    pub direction: CutDirection,
    /// Material left on vertical walls, in millimetres. Negative removes more stock.
    pub stock_to_leave_walls: f64,
    /// Material left on horizontal floors, in millimetres. Negative removes more stock.
    pub stock_to_leave_floors: f64,
}

impl Default for CutParameters {
    fn default() -> Self {
        Self { compensation: Compensation::Computer, direction: CutDirection::Left, stock_to_leave_walls: 0.0, stock_to_leave_floors: 0.0 }
    }
}

/// The `Depth Cuts` page: how the total depth is divided into cuts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthCuts {
    /// The largest amount of material removed in Z by one rough cut, in millimetres.
    pub max_rough_step: f64,
    /// How many finish cuts are made after the rough cuts.
    pub finish_cuts: u32,
    /// The step between the finish cuts, in millimetres.
    pub finish_step: f64,
    /// Whether the tool stays down and moves straight to the next depth, instead of retracting.
    pub keep_tool_down: bool,
    /// Whether the depths run top down or bottom up.
    pub order: DepthOrder,
}

impl Default for DepthCuts {
    fn default() -> Self {
        Self { max_rough_step: 5.0, finish_cuts: 0, finish_step: 0.5, keep_tool_down: true, order: DepthOrder::TopDown }
    }
}

/// The `Multi Passes` page: extra passes across the wall, at the same depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MultiPasses {
    /// How many rough passes step out from the finish wall.
    pub rough_passes: u32,
    /// The distance between the rough passes, in millimetres.
    pub rough_spacing: f64,
    /// How many finish passes are made on the wall.
    pub finish_passes: u32,
    /// The distance between the finish passes, in millimetres.
    pub finish_spacing: f64,
    /// How many spring passes repeat the final pass without stepping over.
    pub spring_passes: u32,
    /// Whether the tool stays down between the passes.
    pub keep_tool_down: bool,
    /// Whether the finish passes on every chain wait until all roughing is done.
    pub finish_after_roughing: bool,
}

impl Default for MultiPasses {
    fn default() -> Self {
        Self {
            rough_passes: 0,
            rough_spacing: 1.0,
            finish_passes: 0,
            finish_spacing: 0.5,
            spring_passes: 0,
            keep_tool_down: true,
            finish_after_roughing: true,
        }
    }
}

/// The shape of a lead in or lead out move.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeadKind {
    /// No lead: the tool plunges on the chain itself.
    None,
    /// A straight line onto (or off) the chain.
    #[default]
    Line,
    /// An arc onto (or off) the chain.
    Arc,
}

impl LeadKind {
    /// Every kind, in the order the dialog lists them.
    pub const ALL: [Self; 3] = [Self::None, Self::Line, Self::Arc];

    /// The kind as the dialog names it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Line => "Line",
            Self::Arc => "Arc",
        }
    }
}

/// One lead move: how long it is when it is a line, and its radius and sweep when it is an arc.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeadMove {
    /// Whether the lead is absent, a line or an arc.
    pub kind: LeadKind,
    /// The line's length, in millimetres.
    pub line_length: f64,
    /// The arc's radius, in millimetres.
    pub arc_radius: f64,
    /// The arc's sweep, in degrees.
    pub arc_sweep_degrees: f64,
}

impl Default for LeadMove {
    fn default() -> Self {
        Self { kind: LeadKind::Line, line_length: 5.0, arc_radius: 5.0, arc_sweep_degrees: 90.0 }
    }
}

impl LeadMove {
    /// Checks the values the move's kind uses.
    pub fn validate(&self) -> Result<(), Contour2DError> {
        if !self.line_length.is_finite() || !self.arc_radius.is_finite() || !self.arc_sweep_degrees.is_finite() {
            return Err(Contour2DError::Lead);
        }
        match self.kind {
            LeadKind::None => Ok(()),
            LeadKind::Line => {
                if self.line_length > 0.0 {
                    Ok(())
                } else {
                    Err(Contour2DError::LeadLine)
                }
            }
            LeadKind::Arc => {
                if self.arc_radius > 0.0 && (1.0..=180.0).contains(&self.arc_sweep_degrees) {
                    Ok(())
                } else {
                    Err(Contour2DError::LeadArc)
                }
            }
        }
    }
}

/// The `Lead In/Out` page: how the tool enters and leaves the chain.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LeadInOut {
    /// The move onto the chain.
    pub entry: LeadMove,
    /// The move off the chain.
    pub exit: LeadMove,
    /// Whether a closed chain is entered and left at its midpoint rather than its start.
    pub entry_at_midpoint: bool,
    /// Whether only the last depth cut makes an exit move.
    pub exit_on_last_depth_cut_only: bool,
}

impl LeadInOut {
    /// Checks both lead moves.
    pub fn validate(&self) -> Result<(), Contour2DError> {
        self.entry.validate()?;
        self.exit.validate()
    }
}

/// Whether the linking heights are measured from the part origin or from the top of stock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkingMode {
    /// Every height is an absolute Z coordinate.
    #[default]
    Absolute,
    /// Every height is measured from the top of stock.
    Incremental,
}

impl LinkingMode {
    /// Both modes, in the order the dialog lists them.
    pub const ALL: [Self; 2] = [Self::Absolute, Self::Incremental];

    /// The mode as the dialog names it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Absolute => "Absolute",
            Self::Incremental => "Incremental",
        }
    }
}

/// The `Linking Parameters` page: the heights the tool moves between at the start, between
/// passes and at the end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Linking {
    /// The height the tool rapids to between operations, in millimetres.
    pub clearance: f64,
    /// The height the tool retracts to between passes, in millimetres.
    pub retract: f64,
    /// The height the tool feeds down to before entering the cut, in millimetres.
    pub feed_plane: f64,
    /// The top of the stock the depths are measured from, in millimetres.
    pub top_of_stock: f64,
    /// The final cutting depth, in millimetres.
    pub depth: f64,
    /// Whether the heights are absolute or incremental.
    pub mode: LinkingMode,
}

impl Default for Linking {
    fn default() -> Self {
        Self { clearance: 100.0, retract: 10.0, feed_plane: 5.0, top_of_stock: 0.0, depth: -10.0, mode: LinkingMode::Absolute }
    }
}

impl Linking {
    /// Checks that the heights are numbers in the order a machine can use them, and that the
    /// depth is below the top of stock.
    pub fn validate(&self) -> Result<(), Contour2DError> {
        let heights = [self.clearance, self.retract, self.feed_plane, self.top_of_stock, self.depth];
        if !heights.iter().all(|value| value.is_finite()) {
            return Err(Contour2DError::LinkingHeights);
        }
        if self.clearance < self.retract || self.retract < self.feed_plane || self.feed_plane < self.top_of_stock {
            return Err(Contour2DError::LinkingOrder);
        }
        if self.depth >= self.top_of_stock {
            return Err(Contour2DError::Depth);
        }
        Ok(())
    }

    /// The final cut depth as an absolute Z, for the given absolute top of stock: the depth
    /// itself in absolute mode, or the top of stock plus the incremental depth.
    pub fn absolute_depth(&self, top_of_stock: f64) -> f64 {
        match self.mode {
            LinkingMode::Absolute => self.depth,
            LinkingMode::Incremental => top_of_stock + self.depth,
        }
    }
}

/// Everything a 2D Contour operation is defined by, one field group per dialog page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contour2DParams {
    /// The wireframe entity ids the toolpath follows, in cutting order.
    pub chain: Vec<EntityId>,
    /// The id of the tool assembly in the tool library that cuts it.
    pub tool_id: usize,
    /// The `Cut Parameters` page.
    pub cut: CutParameters,
    /// The `Depth Cuts` page.
    pub depth_cuts: DepthCuts,
    /// The `Multi Passes` page.
    pub multipasses: MultiPasses,
    /// The `Lead In/Out` page.
    pub lead_in_out: LeadInOut,
    /// The `Linking Parameters` page.
    pub linking: Linking,
}

impl Contour2DParams {
    /// A 2D Contour over `chain`, cut by the tool assembly `tool_id`, with the default pages.
    pub fn new(chain: Vec<EntityId>, tool_id: usize) -> Self {
        Self { chain, tool_id, ..Self::default() }
    }

    /// How many entities the chain names.
    pub fn chain_len(&self) -> usize {
        self.chain.len()
    }

    /// Checks every page, naming the first value that cannot cut.
    pub fn validate(&self) -> Result<(), Contour2DError> {
        if self.chain.is_empty() {
            return Err(Contour2DError::EmptyChain);
        }
        if self.chain.contains(&0) {
            return Err(Contour2DError::ChainEntity);
        }
        if self.tool_id == 0 {
            return Err(Contour2DError::NoTool);
        }
        if !self.cut.stock_to_leave_walls.is_finite() || !self.cut.stock_to_leave_floors.is_finite() {
            return Err(Contour2DError::StockToLeave);
        }
        if !(self.depth_cuts.max_rough_step > 0.0) || !self.depth_cuts.max_rough_step.is_finite() {
            return Err(Contour2DError::RoughStep);
        }
        if self.depth_cuts.finish_cuts > 0 && (!(self.depth_cuts.finish_step > 0.0) || !self.depth_cuts.finish_step.is_finite()) {
            return Err(Contour2DError::FinishStep);
        }
        if !self.depth_cuts.finish_step.is_finite() {
            return Err(Contour2DError::FinishStep);
        }
        if !self.multipasses.rough_spacing.is_finite() || !self.multipasses.finish_spacing.is_finite() {
            return Err(Contour2DError::PassSpacing);
        }
        if (self.multipasses.rough_passes > 0 && !(self.multipasses.rough_spacing > 0.0))
            || (self.multipasses.finish_passes > 0 && !(self.multipasses.finish_spacing > 0.0))
        {
            return Err(Contour2DError::PassSpacing);
        }
        self.lead_in_out.validate()?;
        self.linking.validate()
    }

    /// The operation as the status bar and the toolpath tree name it, such as
    /// `2D Contour · 1 entity · depth -10.0 mm`.
    pub fn summary(&self) -> String {
        let count = self.chain.len();
        let noun = if count == 1 { "entity" } else { "entities" };
        format!("2D Contour · {count} {noun} · depth {:.1} mm", self.linking.depth)
    }
}

/// One curve of the geometry chain, as the calculation receives it: the entity the operation
/// names and the points that draw it, in cutting order.
///
/// The document hands its wireframe over in this form rather than the calculation reaching into
/// the sketch, so the operation's maths stays independent of where the geometry came from (a
/// sketch curve, a DXF import or a future feature).
#[derive(Clone, Debug, PartialEq)]
pub struct ChainCurve {
    /// The entity id the operation's chain names.
    pub id: EntityId,
    /// The curve's points, in the part's coordinates.
    pub points: Vec<Point>,
}

/// How many depth cuts one operation may make, so a hostile rough step cannot ask for millions.
const MAX_DEPTH_CUTS: usize = 1000;

/// How many wall passes one operation may make, for the same reason.
const MAX_WALL_PASSES: usize = 200;

/// Two points closer than this, in millimetres, are the same point: a chain's ends meet, and a
/// repeated point is not a segment.
const CHAIN_EPSILON: f64 = 0.1;

impl Contour2DParams {
    /// The depth cuts, in machining order and as absolute Z coordinates.
    ///
    /// The rough cuts divide the height from the top of stock down to the finish cuts into equal
    /// steps no larger than [`DepthCuts::max_rough_step`]; the finish cuts then step down to the
    /// final depth by [`DepthCuts::finish_step`]. [`DepthCuts::order`] decides whether they run
    /// top down (the default) or bottom up. An unusable depth gives no level at all, so a caller
    /// reports it instead of cutting nothing.
    pub fn depth_levels(&self) -> Vec<f64> {
        let top = self.linking.top_of_stock;
        let final_z = self.linking.absolute_depth(top);
        if ![top, final_z].iter().all(|value| value.is_finite()) || !(final_z < top) {
            return Vec::new();
        }
        let finish = usize::try_from(self.depth_cuts.finish_cuts).unwrap_or(0).min(MAX_DEPTH_CUTS);
        let finish_step = if self.depth_cuts.finish_step.is_finite() { self.depth_cuts.finish_step.max(0.0) } else { 0.0 };
        // Top down: the highest finish cut first, the final depth last.
        let finish_levels: Vec<f64> = (0..finish).rev().map(|index| final_z + index as f64 * finish_step).filter(|z| *z < top).collect();
        // The rough cuts stop at the highest finish cut, so its depth is not cut twice.
        let rough_bottom = finish_levels.first().copied().unwrap_or(final_z);
        let mut levels = Vec::new();
        let total = top - rough_bottom;
        let step = self.depth_cuts.max_rough_step;
        if total > MOVE_EPSILON && step.is_finite() && step > 0.0 {
            let count = ((total / step).ceil().max(1.0) as usize).min(MAX_DEPTH_CUTS);
            for index in 1..=count {
                levels.push(top - total * index as f64 / count as f64);
            }
        }
        // The first finish level is where the rough cuts stopped; the rest finish down to depth.
        let skip = usize::from(!levels.is_empty());
        levels.extend(finish_levels.iter().skip(skip).copied());
        if levels.is_empty() {
            levels.push(final_z);
        }
        if self.depth_cuts.order == DepthOrder::BottomUp {
            levels.reverse();
        }
        levels
    }

    /// The lateral offsets from the chain, in machining order and in millimetres: the rough
    /// passes step out from the finish wall, the finish passes step onto it, and the spring
    /// passes repeat the last one. A pass count of zero everywhere gives one pass at the wall.
    ///
    /// `radius` is the tool's cutting radius. The offsets include the stock to leave when the
    /// compensation is [`Compensation::Computer`]; control compensation and no compensation
    /// leave the offset to the control or to the chain itself.
    pub fn pass_offsets(&self, radius: f64) -> Vec<f64> {
        let base = self.wall_offset(radius);
        if !base.is_finite() {
            return vec![0.0];
        }
        let rough = usize::try_from(self.multipasses.rough_passes).unwrap_or(0).min(MAX_WALL_PASSES);
        let finish = usize::try_from(self.multipasses.finish_passes).unwrap_or(0).min(MAX_WALL_PASSES);
        let spring = usize::try_from(self.multipasses.spring_passes).unwrap_or(0).min(MAX_WALL_PASSES);
        let rough_spacing = finite_non_negative(self.multipasses.rough_spacing);
        let finish_spacing = finite_non_negative(self.multipasses.finish_spacing);
        let mut offsets = Vec::new();
        // Roughing from the outside in: the furthest pass first, so the wall is left for finishing.
        for index in (0..rough).rev() {
            offsets.push(base + (index + 1) as f64 * rough_spacing);
        }
        // Finishing ends on the wall itself.
        for index in 0..finish {
            offsets.push(base + (finish - 1 - index) as f64 * finish_spacing);
        }
        if finish == 0 {
            offsets.push(base);
        }
        for _ in 0..spring {
            offsets.push(base);
        }
        offsets.truncate(MAX_WALL_PASSES.max(1));
        if offsets.is_empty() {
            offsets.push(base);
        }
        offsets
    }

    /// The distance the tool centre runs from the chain when compensation is in the program:
    /// the cutting radius plus the stock to leave on the walls.
    fn wall_offset(&self, radius: f64) -> f64 {
        let stock = if self.cut.stock_to_leave_walls.is_finite() { self.cut.stock_to_leave_walls } else { 0.0 };
        match self.cut.compensation {
            Compensation::Computer => radius.max(0.0) + stock,
            Compensation::Control | Compensation::Off => 0.0,
        }
    }

    /// Calculates the operation's toolpath over `chain`, cut by `tool`.
    ///
    /// The calculation is a 2.5D contour: the chain is offset by the tool's radius to the side
    /// the cut direction names, dropped to each depth cut, and repeated for every wall pass. Each
    /// pass is entered through the lead in move, cut through the chain, left through the lead out
    /// move, and linked at the clearance, retract and feed planes. The offset is taken on the
    /// chain's polyline rather than by exact curve offsetting, which is the stub the blueprint
    /// allows for phase 2D; the depths, passes, leads and linking are calculated exactly.
    ///
    /// Returns the moves, or the first page value that cannot cut.
    pub fn toolpath(&self, chain: &[ChainCurve], tool: &CuttingTool) -> Result<Toolpath, Contour2DError> {
        self.validate()?;
        if !(tool.diameter > 0.0) || !tool.diameter.is_finite() {
            return Err(Contour2DError::ToolDiameter);
        }
        let points = ordered_points(chain);
        if points.len() < 2 {
            return Err(Contour2DError::ChainGeometry);
        }
        let depths = self.depth_levels();
        if depths.is_empty() {
            return Err(Contour2DError::Depth);
        }
        let closed = is_closed(&points);
        let up = chain_normal(&points);
        let radius = tool.diameter / 2.0;
        let offsets = self.pass_offsets(radius);
        let top = self.linking.top_of_stock;
        let clearance = self.absolute_height(self.linking.clearance, top);
        let retract = self.absolute_height(self.linking.retract, top);
        let feed_plane = self.absolute_height(self.linking.feed_plane, top);
        if ![clearance, retract, feed_plane].iter().all(|value| value.is_finite()) {
            return Err(Contour2DError::LinkingHeights);
        }
        let mut builder = ToolpathBuilder::new();
        let last_depth = depths.len().saturating_sub(1);
        for (depth_index, z) in depths.iter().enumerate() {
            for (pass_index, offset) in offsets.iter().enumerate() {
                let mut path = offset_chain(&points, *offset, self.cut.direction, up);
                if path.len() < 2 {
                    continue;
                }
                let midpoint = self.lead_in_out.entry_at_midpoint && closed;
                if midpoint {
                    path = start_at_midpoint(&path);
                }
                // The 2D contour cuts the chain at the depth; the chain's own Z is its plane's.
                for point in &mut path {
                    point[2] = *z;
                }
                let entry_tangent = tangent_at_start(&path, up);
                let (entry, entry_arc) = entry_lead(path[0], entry_tangent, up, &self.lead_in_out.entry);
                let keep_down = if depth_index == 0 && pass_index == 0 {
                    // The first pass has no previous move to stay down from.
                    false
                } else if pass_index == 0 {
                    self.depth_cuts.keep_tool_down
                } else {
                    self.multipasses.keep_tool_down
                };
                approach(&mut builder, entry, *z, clearance, retract, feed_plane, keep_down);
                match entry_arc {
                    Some(arc) => builder.arc_to(path[0], arc),
                    None => builder.feed_to(path[0]),
                }
                for point in path.iter().skip(1) {
                    builder.feed_to(*point);
                }
                let exit_allowed = !self.lead_in_out.exit_on_last_depth_cut_only || depth_index == last_depth;
                if exit_allowed && let Some(end) = path.last().copied() {
                    let exit_tangent = tangent_at_end(&path, up);
                    if let Some((to, arc)) = exit_lead(end, exit_tangent, up, &self.lead_in_out.exit) {
                        match arc {
                            Some(arc) => builder.arc_to(to, arc),
                            None => builder.feed_to(to),
                        }
                    }
                }
            }
        }
        if let Some(at) = builder.position() {
            builder.rapid_to([at[0], at[1], clearance]);
        }
        Ok(builder.finish())
    }

    /// `value` as an absolute Z: the coordinate itself in absolute mode, or the top of stock
    /// plus the value in incremental mode.
    fn absolute_height(&self, value: f64, top: f64) -> f64 {
        match self.linking.mode {
            LinkingMode::Absolute => value,
            LinkingMode::Incremental => top + value,
        }
    }
}

/// Moves the tool to the start of a pass at depth `z`: down the feed plane and into the cut when
/// it is clear, or straight across the cut when the tool stays down.
fn approach(builder: &mut ToolpathBuilder, entry: Point, z: f64, clearance: f64, retract: f64, feed_plane: f64, keep_down: bool) {
    match builder.position() {
        None => {
            // The program starts at the clearance plane above the first entry point.
            builder.place([entry[0], entry[1], clearance]);
            builder.rapid_to([entry[0], entry[1], feed_plane]);
        }
        Some(at) => {
            if keep_down && (at[2] - z).abs() <= MOVE_EPSILON {
                // Staying down: cross to the next entry at the cutting depth.
                builder.rapid_to([entry[0], entry[1], z]);
            } else {
                builder.rapid_to([at[0], at[1], retract]);
                builder.rapid_to([entry[0], entry[1], retract]);
                builder.rapid_to([entry[0], entry[1], feed_plane]);
            }
        }
    }
    if let Some(at) = builder.position()
        && at[2] > z + MOVE_EPSILON
    {
        builder.feed_to([entry[0], entry[1], z]);
    }
}

/// Where the entry lead plunges, and the arc it feeds along onto the chain at `start`.
fn entry_lead(start: Point, tangent: Point, up: Point, lead: &LeadMove) -> (Point, Option<Arc>) {
    match lead.kind {
        LeadKind::None => (start, None),
        LeadKind::Line => (subtract(start, scale(tangent, finite_non_negative(lead.line_length))), None),
        LeadKind::Arc => {
            let radius = lead.arc_radius;
            let sweep = lead.arc_sweep_degrees.to_radians();
            let Some(side) = left_normal(tangent, up) else {
                return (start, None);
            };
            if !(radius > 0.0) || !radius.is_finite() || !sweep.is_finite() {
                return (start, None);
            }
            let centre = add(start, scale(side, radius));
            let arm = subtract(start, centre);
            // The arc starts a sweep before the chain and turns onto it, tangent to the cut.
            (add(centre, rotate(arm, up, -sweep)), Some(Arc::new(centre, false)))
        }
    }
}

/// Where the exit lead ends, and the arc it leaves the chain along from `end`. `None` when the
/// exit is a plunge off the chain rather than a lead.
fn exit_lead(end: Point, tangent: Point, up: Point, lead: &LeadMove) -> Option<(Point, Option<Arc>)> {
    match lead.kind {
        LeadKind::None => None,
        LeadKind::Line => Some((add(end, scale(tangent, finite_non_negative(lead.line_length))), None)),
        LeadKind::Arc => {
            let radius = lead.arc_radius;
            let sweep = lead.arc_sweep_degrees.to_radians();
            let side = left_normal(tangent, up)?;
            if !(radius > 0.0) || !radius.is_finite() || !sweep.is_finite() {
                return None;
            }
            let centre = add(end, scale(side, radius));
            let arm = subtract(end, centre);
            Some((add(centre, rotate(arm, up, sweep)), Some(Arc::new(centre, false))))
        }
    }
}

/// The chain's curves as one polyline in cutting order: each curve is appended to the previous
/// one, reversed when its far end is the closer of the two, so a chain of four rectangle lines
/// becomes one closed loop.
fn ordered_points(chain: &[ChainCurve]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::new();
    for curve in chain {
        let points = distinct_points(&curve.points);
        let (Some(first), Some(last)) = (points.first().copied(), points.last().copied()) else {
            continue;
        };
        let Some(tail) = out.last().copied() else {
            out.extend(points);
            continue;
        };
        if distance(tail, first) > distance(tail, last) {
            out.extend(points.iter().rev().copied());
        } else {
            out.extend(points);
        }
    }
    out
}

/// The points of a curve with non-finite and repeated points left out.
fn distinct_points(points: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for point in points {
        if !point.iter().all(|value| value.is_finite()) {
            continue;
        }
        push_distinct(&mut out, *point);
    }
    out
}

/// Appends `point` unless it repeats the last one.
fn push_distinct(points: &mut Vec<Point>, point: Point) {
    if points.last().is_none_or(|last| distance(*last, point) > 1e-6) {
        points.push(point);
    }
}

/// Whether the chain returns to its start, so the tool can cut it as a closed loop.
fn is_closed(points: &[Point]) -> bool {
    match (points.first(), points.last()) {
        (Some(first), Some(last)) => distance(*first, *last) <= CHAIN_EPSILON,
        _ => false,
    }
}

/// The chain's plane normal, by Newell's method: `+Z` for a chain drawn on the top plane, and
/// the plane's own normal for one drawn on a construction plane such as the front.
fn chain_normal(points: &[Point]) -> Point {
    let mut normal = [0.0; 3];
    for (index, point) in points.iter().enumerate() {
        let next = points[(index + 1) % points.len()];
        normal[0] += (point[1] - next[1]) * (point[2] + next[2]);
        normal[1] += (point[2] - next[2]) * (point[0] + next[0]);
        normal[2] += (point[0] - next[0]) * (point[1] + next[1]);
    }
    normalize(normal).unwrap_or([0.0, 0.0, 1.0])
}

/// The chain offset sideways by `offset` in the cutter's direction: left of travel for
/// [`CutDirection::Left`], right for [`CutDirection::Right`]. Vertices are mitred, so a corner
/// keeps its shape instead of gaping.
fn offset_chain(points: &[Point], offset: f64, direction: CutDirection, up: Point) -> Vec<Point> {
    let sign = match direction {
        CutDirection::Left => 1.0,
        CutDirection::Right => -1.0,
    };
    let amount = sign * offset;
    if !amount.is_finite() || amount.abs() <= MOVE_EPSILON {
        return points.to_vec();
    }
    let closed = is_closed(points);
    let count = points.len();
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let previous = if index > 0 {
            Some(index - 1)
        } else if closed && count >= 2 {
            Some(count - 2)
        } else {
            None
        };
        let next = if index + 1 < count {
            Some(index + 1)
        } else if closed && count >= 3 {
            Some(1)
        } else {
            None
        };
        let incoming = previous.and_then(|at| segment_dir(points[at], points[index]));
        let outgoing = next.and_then(|at| segment_dir(points[index], points[at]));
        let normal_in = incoming.and_then(|dir| left_normal(dir, up));
        let normal_out = outgoing.and_then(|dir| left_normal(dir, up));
        let (normal, miter) = match (normal_in, normal_out) {
            (Some(before), Some(after)) => {
                let average = normalize(add(before, after)).unwrap_or(before);
                // A sharp corner would push the mitre out; cap the stretch at five radii.
                let cosine = dot(average, before).abs().max(0.2);
                (average, amount / cosine)
            }
            (Some(only), None) | (None, Some(only)) => (only, amount),
            (None, None) => {
                out.push(points[index]);
                continue;
            }
        };
        out.push(add(points[index], scale(normal, miter)));
    }
    out
}

/// Rotates a closed chain so it starts and ends at the point halfway along its length: where a
/// contour entered at the midpoint starts cutting.
fn start_at_midpoint(points: &[Point]) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let total: f64 = points.windows(2).map(|pair| distance(pair[0], pair[1])).sum();
    if !(total > MOVE_EPSILON) {
        return points.to_vec();
    }
    let half = total / 2.0;
    let mut walked = 0.0;
    for index in 0..points.len() - 1 {
        let segment = distance(points[index], points[index + 1]);
        if segment <= MOVE_EPSILON {
            continue;
        }
        if walked + segment >= half {
            let t = (half - walked) / segment;
            let mid = lerp(points[index], points[index + 1], t);
            let mut out = Vec::with_capacity(points.len() + 2);
            push_distinct(&mut out, mid);
            for point in &points[index + 1..] {
                push_distinct(&mut out, *point);
            }
            for point in &points[1..=index] {
                push_distinct(&mut out, *point);
            }
            push_distinct(&mut out, mid);
            return out;
        }
        walked += segment;
    }
    points.to_vec()
}

/// The direction of travel at the chain's start, unit length and in the chain's plane.
fn tangent_at_start(points: &[Point], up: Point) -> Point {
    for pair in points.windows(2) {
        if let Some(dir) = segment_dir(pair[0], pair[1]) {
            return dir;
        }
    }
    left_normal([1.0, 0.0, 0.0], up).unwrap_or([1.0, 0.0, 0.0])
}

/// The direction of travel at the chain's end, unit length and in the chain's plane.
fn tangent_at_end(points: &[Point], up: Point) -> Point {
    for pair in points.windows(2).rev() {
        if let Some(dir) = segment_dir(pair[0], pair[1]) {
            return dir;
        }
    }
    left_normal([1.0, 0.0, 0.0], up).unwrap_or([1.0, 0.0, 0.0])
}

/// The unit vector from `from` to `to`, or `None` when the two coincide.
fn segment_dir(from: Point, to: Point) -> Option<Point> {
    normalize(subtract(to, from))
}

/// The cutter's left of a travel direction, in the chain's plane: `None` when the direction lies
/// along the plane normal and has no left.
fn left_normal(direction: Point, up: Point) -> Option<Point> {
    normalize(cross(up, direction))
}

/// `value` when it is a usable number, zero otherwise: a spacing that cannot be read leaves the
/// passes on top of one another rather than in a hostile place.
fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() { value.max(0.0) } else { 0.0 }
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn subtract(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(vector: Point, factor: f64) -> Point {
    [vector[0] * factor, vector[1] * factor, vector[2] * factor]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Point, b: Point) -> Point {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(vector: Point) -> Option<Point> {
    let length = dot(vector, vector).sqrt();
    if length.is_finite() && length > 1e-12 { Some(scale(vector, 1.0 / length)) } else { None }
}

/// `linear` interpolated between `a` and `b`: 0 is `a`, 1 is `b`.
fn lerp(a: Point, b: Point, linear: f64) -> Point {
    add(a, scale(subtract(b, a), linear))
}

/// `vector` rotated about the unit `axis` by `angle` radians, counter-clockwise seen from the
/// axis' tip (Rodrigues' formula).
fn rotate(vector: Point, axis: Point, angle: f64) -> Point {
    let (sin, cos) = angle.sin_cos();
    add(add(scale(vector, cos), scale(cross(axis, vector), sin)), scale(axis, dot(axis, vector) * (1.0 - cos)))
}

/// Why a 2D Contour operation cannot be stored. Each rejection names the page and the value, so
/// the dialog can put it beside the field the user typed it in.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum Contour2DError {
    /// The operation follows no geometry.
    #[error("chain at least one wireframe entity for the toolpath")]
    EmptyChain,
    /// The chain names the reserved entity id zero.
    #[error("the geometry chain names entity 0, which is not an entity")]
    ChainEntity,
    /// The operation selects no tool.
    #[error("select a cutting tool for the operation")]
    NoTool,
    /// The selected tool has no cutting diameter.
    #[error("the tool's cutting diameter must be greater than zero")]
    ToolDiameter,
    /// The chain names no geometry, or the geometry the document holds has no length.
    #[error("the chain needs at least two points of wireframe to cut")]
    ChainGeometry,
    /// A stock-to-leave value is not a number.
    #[error("stock to leave must be a finite number")]
    StockToLeave,
    /// The rough depth of cut is zero, negative or not a number.
    #[error("the maximum rough depth of cut must be greater than zero")]
    RoughStep,
    /// The finish step cannot divide the finish cuts.
    #[error("the finish step must be greater than zero when finish cuts are used")]
    FinishStep,
    /// A pass has no usable spacing but is asked to be cut.
    #[error("rough and finish pass spacing must be greater than zero when the passes are used")]
    PassSpacing,
    /// A lead value is not a number.
    #[error("the lead in/out values must be finite numbers")]
    Lead,
    /// A lead line has no length.
    #[error("a lead line must have a length greater than zero")]
    LeadLine,
    /// A lead arc cannot sweep.
    #[error("a lead arc must have a radius greater than zero and a sweep between 1 and 180 degrees")]
    LeadArc,
    /// A linking height is not a number.
    #[error("the linking heights must be finite numbers")]
    LinkingHeights,
    /// The heights are out of the order a machine moves in.
    #[error("clearance, retract, feed plane and top of stock must be in that order")]
    LinkingOrder,
    /// The depth is not below the top of stock, so nothing is cut.
    #[error("the depth must be below the top of stock")]
    Depth,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolpath::MoveKind;

    /// A valid operation over one entity and the first tool.
    fn operation() -> Contour2DParams {
        Contour2DParams::new(vec![1, 2], 1)
    }

    #[test]
    fn the_default_pages_describe_a_usable_operation() {
        let params = operation();
        assert_eq!(params.validate(), Ok(()));
        assert_eq!(params.chain_len(), 2);
        assert_eq!(params.summary(), "2D Contour · 2 entities · depth -10.0 mm");
        assert_eq!(Contour2DParams::new(vec![7], 2).summary(), "2D Contour · 1 entity · depth -10.0 mm");
    }

    #[test]
    fn an_operation_must_chain_geometry_and_select_a_tool() {
        assert_eq!(Contour2DParams::new(Vec::new(), 1).validate(), Err(Contour2DError::EmptyChain));
        assert_eq!(Contour2DParams::new(vec![0], 1).validate(), Err(Contour2DError::ChainEntity));
        assert_eq!(Contour2DParams::new(vec![1], 0).validate(), Err(Contour2DError::NoTool));
    }

    #[test]
    fn stock_to_leave_must_be_a_number() {
        let mut params = operation();
        params.cut.stock_to_leave_walls = f64::NAN;
        assert_eq!(params.validate(), Err(Contour2DError::StockToLeave));
        params.cut.stock_to_leave_walls = -0.5;
        assert_eq!(params.validate(), Ok(()), "a negative stock to leave removes more stock");
    }

    #[test]
    fn depth_cuts_need_a_step_and_finish_cuts_need_a_step_of_their_own() {
        let mut params = operation();
        params.depth_cuts.max_rough_step = 0.0;
        assert_eq!(params.validate(), Err(Contour2DError::RoughStep));
        params.depth_cuts.max_rough_step = 5.0;
        params.depth_cuts.finish_cuts = 2;
        params.depth_cuts.finish_step = 0.0;
        assert_eq!(params.validate(), Err(Contour2DError::FinishStep));
        params.depth_cuts.finish_step = 0.5;
        assert_eq!(params.validate(), Ok(()));
    }

    #[test]
    fn extra_passes_need_a_spacing() {
        let mut params = operation();
        params.multipasses.rough_passes = 2;
        params.multipasses.rough_spacing = 0.0;
        assert_eq!(params.validate(), Err(Contour2DError::PassSpacing));
        params.multipasses.rough_spacing = 1.5;
        params.multipasses.finish_passes = 1;
        params.multipasses.finish_spacing = 0.0;
        assert_eq!(params.validate(), Err(Contour2DError::PassSpacing));
        params.multipasses.finish_spacing = 0.25;
        assert_eq!(params.validate(), Ok(()));
    }

    #[test]
    fn a_lead_move_matches_its_kind() {
        let mut params = operation();
        params.lead_in_out.entry = LeadMove { kind: LeadKind::Line, line_length: 0.0, ..LeadMove::default() };
        assert_eq!(params.validate(), Err(Contour2DError::LeadLine));
        params.lead_in_out.entry = LeadMove { kind: LeadKind::Arc, arc_radius: 4.0, arc_sweep_degrees: 90.0, ..LeadMove::default() };
        assert_eq!(params.validate(), Ok(()));
        params.lead_in_out.exit = LeadMove { kind: LeadKind::Arc, arc_radius: 4.0, arc_sweep_degrees: 200.0, ..LeadMove::default() };
        assert_eq!(params.validate(), Err(Contour2DError::LeadArc));
        params.lead_in_out.exit = LeadMove { kind: LeadKind::None, line_length: 0.0, arc_radius: 0.0, arc_sweep_degrees: 0.0 };
        assert_eq!(params.validate(), Ok(()), "no lead needs no size");
    }

    #[test]
    fn the_linking_heights_must_stack_and_reach_below_the_stock() {
        let mut params = operation();
        params.linking.retract = 200.0;
        assert_eq!(params.validate(), Err(Contour2DError::LinkingOrder), "retract above clearance");
        params.linking.retract = 10.0;
        params.linking.depth = 5.0;
        assert_eq!(params.validate(), Err(Contour2DError::Depth), "a depth above the stock cuts nothing");
        params.linking.depth = -10.0;
        params.linking.top_of_stock = f64::INFINITY;
        assert_eq!(params.validate(), Err(Contour2DError::LinkingHeights));
    }

    #[test]
    fn an_incremental_depth_is_measured_from_the_top_of_stock() {
        let mut params = operation();
        params.linking.mode = LinkingMode::Incremental;
        params.linking.depth = -6.0;
        assert_eq!(params.validate(), Ok(()));
        assert!((params.linking.absolute_depth(12.0) - 6.0).abs() < 1e-9);
        assert!((Linking::default().absolute_depth(12.0) + 10.0).abs() < 1e-9);
    }

    /// The four lines of a 40 × 30 rectangle at the origin, as a chain in cutting order.
    fn rectangle() -> Vec<ChainCurve> {
        vec![
            ChainCurve { id: 1, points: vec![[0.0, 0.0, 0.0], [40.0, 0.0, 0.0]] },
            ChainCurve { id: 2, points: vec![[40.0, 0.0, 0.0], [40.0, 30.0, 0.0]] },
            ChainCurve { id: 3, points: vec![[40.0, 30.0, 0.0], [0.0, 30.0, 0.0]] },
            ChainCurve { id: 4, points: vec![[0.0, 30.0, 0.0], [0.0, 0.0, 0.0]] },
        ]
    }

    /// A 6 mm flat endmill, the smallest preset.
    fn tool() -> CuttingTool {
        CuttingTool::new(crate::tool::ToolType::FlatEndmill, 6.0)
    }

    #[test]
    fn a_contour_generates_rapids_and_feeds_that_reach_the_depth() {
        let params = operation();
        let path = params.toolpath(&rectangle(), &tool()).unwrap_or_default();
        assert!(!path.is_empty());
        assert!(path.rapid_moves() > 0, "the path travels clear of the part");
        assert!(path.feed_moves() >= 4, "the four sides are cut");
        let (min, max) = path.bounds().unwrap_or_default();
        assert!((min[2] + 10.0).abs() < 1e-6, "the cut reaches the depth, got {min:?}");
        assert!((max[2] - 100.0).abs() < 1e-6, "and starts and ends at the clearance plane, got {max:?}");
        // The cutter runs on the left of the travel direction, so it is offset inside the
        // rectangle by its 3 mm radius: the cut spans 34 × 24 mm plus the lead lengths.
        assert!(min[0] < 3.0 + 5.0 && max[0] > 37.0 - 5.0, "the offset chain covers the rectangle, got {min:?}..{max:?}");
        assert_eq!(path.moves().first().map(|motion| motion.kind), Some(MoveKind::Rapid), "a program starts with a rapid");
        assert_eq!(path.moves().last().map(|motion| motion.kind), Some(MoveKind::Rapid), "and retracts at the end");
    }

    #[test]
    fn the_depth_cuts_divide_the_depth_into_equal_rough_cuts_then_finish() {
        let mut params = operation();
        params.depth_cuts.max_rough_step = 5.0;
        assert_eq!(params.depth_levels(), vec![-5.0, -10.0]);
        params.depth_cuts.finish_cuts = 2;
        params.depth_cuts.finish_step = 0.5;
        let levels = params.depth_levels();
        assert_eq!(levels.len(), 3, "two rough cuts and one finish cut below the second, got {levels:?}");
        assert!((levels[1] + 9.5).abs() < 1e-9);
        assert!((levels[2] + 10.0).abs() < 1e-9);
        params.depth_cuts.order = DepthOrder::BottomUp;
        let mut reversed = levels;
        reversed.reverse();
        assert_eq!(params.depth_levels(), reversed);
    }

    #[test]
    fn the_wall_passes_step_out_and_finish_on_the_wall() {
        let mut params = operation();
        assert_eq!(params.pass_offsets(3.0), vec![3.0], "one pass on the wall");
        params.multipasses.rough_passes = 2;
        params.multipasses.rough_spacing = 1.5;
        assert_eq!(params.pass_offsets(3.0), vec![6.0, 4.5, 3.0], "roughing from the outside in");
        params.multipasses.finish_passes = 2;
        params.multipasses.finish_spacing = 0.5;
        assert_eq!(params.pass_offsets(3.0), vec![6.0, 4.5, 3.5, 3.0]);
        params.multipasses.spring_passes = 1;
        assert_eq!(params.pass_offsets(3.0).last(), Some(&3.0), "a spring pass repeats the wall");
        params.cut.compensation = Compensation::Off;
        assert_eq!(params.pass_offsets(3.0), vec![3.0, 1.5, 0.5, 0.0, 0.0], "no compensation leaves the chain to the tool centre");
    }

    #[test]
    fn the_compensation_offsets_the_cut_and_the_stock_to_leave_widens_it() {
        let chain = vec![ChainCurve { id: 1, points: vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]] }];
        let mut params = operation();
        // A single straight line: the cutter centre runs 3 mm to the left of it (towards +Y).
        let path = params.toolpath(&chain, &tool()).unwrap_or_default();
        let (min, max) = path.bounds().unwrap_or_default();
        assert!(max[1] > 2.9 && max[1] < 3.1, "the cutter centre is one radius to the left, got {min:?}..{max:?}");
        params.cut.stock_to_leave_walls = 1.0;
        let wider = params.toolpath(&chain, &tool()).unwrap_or_default();
        let (_, wider_max) = wider.bounds().unwrap_or_default();
        assert!((wider_max[1] - 4.0).abs() < 1e-6, "one radius and 1 mm of stock, got {wider_max:?}");
        params.cut.direction = CutDirection::Right;
        let (right_min, right_max) = params.toolpath(&chain, &tool()).unwrap_or_default().bounds().unwrap_or_default();
        assert!(right_min[1] < -3.9 && right_max[1] < 1.0, "the other side of the chain, got {right_min:?}..{right_max:?}");
    }

    #[test]
    fn a_lead_arc_is_a_feed_arc_move() {
        let mut params = operation();
        params.lead_in_out.entry = LeadMove { kind: LeadKind::Arc, arc_radius: 4.0, arc_sweep_degrees: 90.0, ..LeadMove::default() };
        params.lead_in_out.exit = LeadMove { kind: LeadKind::None, ..LeadMove::default() };
        let path = params.toolpath(&rectangle(), &tool()).unwrap_or_default();
        let arcs: Vec<_> = path.moves().iter().filter(|motion| motion.arc.is_some()).collect();
        assert_eq!(arcs.len(), params.depth_levels().len(), "one entry arc per depth cut");
        for arc in &arcs {
            assert_eq!(arc.gcode(), "G3", "a counter-clockwise entry arc");
            let radius = arc.arc.map(|value| value.radius(arc.from)).unwrap_or_default();
            assert!((radius - 4.0).abs() < 1e-6, "the lead radius, got {radius}");
        }
    }

    #[test]
    fn a_lead_line_starts_the_cut_ahead_of_the_chain() {
        let mut params = operation();
        params.lead_in_out.entry = LeadMove { kind: LeadKind::Line, line_length: 8.0, ..LeadMove::default() };
        params.lead_in_out.exit = LeadMove { kind: LeadKind::Line, line_length: 8.0, ..LeadMove::default() };
        let path = params.toolpath(&rectangle(), &tool()).unwrap_or_default();
        let (min, max) = path.bounds().unwrap_or_default();
        // The cutter runs inside the rectangle, 3 mm in from the walls, and enters travelling
        // +X, so the entry lead reaches 8 mm before the corner at (3, 3).
        assert!(min[0] <= -5.0 + 1e-6, "the entry lead starts 8 mm before the cut, got {min:?}");
        assert!(max[0] >= 11.0 - 1e-6, "the exit lead leaves the closing corner, got {max:?}");
    }

    #[test]
    fn the_linking_heights_are_absolute_or_incremental() {
        let mut params = operation();
        params.linking.top_of_stock = 12.0;
        params.linking.clearance = 20.0;
        params.linking.retract = 15.0;
        params.linking.feed_plane = 13.0;
        params.linking.depth = -6.0;
        assert_eq!(params.validate(), Ok(()));
        let absolute = params.toolpath(&rectangle(), &tool()).unwrap_or_default();
        let (_, absolute_max) = absolute.bounds().unwrap_or_default();
        assert!((absolute_max[2] - 20.0).abs() < 1e-6, "absolute clearance, got {absolute_max:?}");
        params.linking.mode = LinkingMode::Incremental;
        let path = params.toolpath(&rectangle(), &tool()).unwrap_or_default();
        let (min, max) = path.bounds().unwrap_or_default();
        assert!((min[2] - 6.0).abs() < 1e-6, "12 - 6 mm, got {min:?}");
        assert!((max[2] - 32.0).abs() < 1e-6, "12 + 20 mm of clearance, got {max:?}");
    }

    #[test]
    fn keeping_the_tool_down_skips_the_retract_between_passes() {
        let mut params = operation();
        params.multipasses.rough_passes = 2;
        params.multipasses.rough_spacing = 1.0;
        params.depth_cuts.max_rough_step = 5.0;
        params.multipasses.keep_tool_down = true;
        params.depth_cuts.keep_tool_down = true;
        let down = params.toolpath(&rectangle(), &tool()).unwrap_or_default().rapid_length();
        params.multipasses.keep_tool_down = false;
        params.depth_cuts.keep_tool_down = false;
        let retracting = params.toolpath(&rectangle(), &tool()).unwrap_or_default().rapid_length();
        assert!(retracting > down, "retracting between passes travels further, {retracting} vs {down}");
    }

    #[test]
    fn a_chain_without_geometry_cannot_cut() {
        let params = operation();
        assert_eq!(params.toolpath(&[], &tool()), Err(Contour2DError::ChainGeometry));
        let flat = vec![ChainCurve { id: 1, points: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]] }];
        assert_eq!(params.toolpath(&flat, &tool()), Err(Contour2DError::ChainGeometry));
        let mut bad_tool = tool();
        bad_tool.diameter = 0.0;
        assert_eq!(params.toolpath(&rectangle(), &bad_tool), Err(Contour2DError::ToolDiameter));
    }
}

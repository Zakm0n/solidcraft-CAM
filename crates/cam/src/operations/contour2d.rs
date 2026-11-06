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
}

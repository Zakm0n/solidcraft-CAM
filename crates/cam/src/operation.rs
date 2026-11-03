//! Operations: the ordered machining steps of a job.
//!
//! An operation is one thing the machine does with one tool: face the top, rough a pocket,
//! drill a pattern, contour the outside. Order matters — it is the order the machine runs
//! them in — and a [`Program`] keeps that order explicit, with suppression and reordering
//! so a machinist can turn a step off without deleting it, exactly as a CAM operation tree
//! behaves.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::geometry::{GeometryId, GeometryStore};
use crate::reference::EntityRef;
use crate::routing::{Compensation, EntryStyle, PocketStrategy};

/// What an operation does. Each variant carries the parameters that only it needs; the
/// common ones (name, tool, depth) live on [`Operation`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OperationKind {
    /// Face the top of the stock flat.
    Face { stepover_mm: f64 },
    /// Follow the outside profile.
    Contour { finish_passes: u32 },
    /// Clear a closed pocket.
    Pocket { stepover_mm: f64 },
    /// Drill the target holes.
    Drill { peck_mm: Option<f64> },
    /// Remove bulk material in 3D.
    Rough { stepover_mm: f64 },
    /// Smooth the surface left by a roughing pass.
    Finish { stepover_mm: f64 },
    /// Mill a circular pocket from its centre and diameter.
    CircleMill { diameter_mm: f64, center: [f64; 2] },
    /// Mill around a closed slot of a given width.
    Slot { width_mm: f64 },
    /// Follow curves at engraving depth.
    Engrave,
    /// Constant-engagement trochoidal milling along a wall.
    DynamicContour,
    /// Constant-engagement clearing of a region.
    DynamicMill,
    /// Trochoidal slotting along a centreline.
    PeelMill { width_mm: f64 },
    /// Clear material left by previous operations, at constant engagement.
    AreaMill,
    /// Blend between two open chains.
    BlendMill { stepover_mm: f64 },
    /// Chamfer along selected edges.
    ModelChamfer { width_mm: f64, angle_deg: f64 },
    /// Helical bore from a centre point and diameter.
    HelixBore { diameter_mm: f64, pitch_mm: f64, center: [f64; 2] },
    /// Thread mill a bore at a pitch.
    ThreadMill { diameter_mm: f64, pitch_mm: f64, center: [f64; 2] },
}

impl OperationKind {
    /// A short label for the operation tree.
    pub fn label(&self) -> &'static str {
        match self {
            OperationKind::Face { .. } => "Face",
            OperationKind::Contour { .. } => "Contour",
            OperationKind::Pocket { .. } => "Pocket",
            OperationKind::Drill { .. } => "Drill",
            OperationKind::Rough { .. } => "3D Rough",
            OperationKind::Finish { .. } => "3D Finish",
            OperationKind::CircleMill { .. } => "Circle Mill",
            OperationKind::Slot { .. } => "Slot",
            OperationKind::Engrave => "Engrave",
            OperationKind::DynamicContour => "Dynamic Contour",
            OperationKind::DynamicMill => "Dynamic Mill",
            OperationKind::PeelMill { .. } => "Peel Mill",
            OperationKind::AreaMill => "Area Mill",
            OperationKind::BlendMill { .. } => "Blend Mill",
            OperationKind::ModelChamfer { .. } => "Model Chamfer",
            OperationKind::HelixBore { .. } => "Helix Bore",
            OperationKind::ThreadMill { .. } => "Thread Mill",
        }
    }

    /// The stepover this kind uses, when it has one.
    pub fn stepover_mm(&self) -> Option<f64> {
        match self {
            OperationKind::Face { stepover_mm }
            | OperationKind::Pocket { stepover_mm }
            | OperationKind::Rough { stepover_mm }
            | OperationKind::Finish { stepover_mm }
            | OperationKind::BlendMill { stepover_mm } => Some(*stepover_mm),
            OperationKind::CircleMill { .. }
            | OperationKind::Slot { .. }
            | OperationKind::Engrave
            | OperationKind::DynamicContour
            | OperationKind::DynamicMill
            | OperationKind::PeelMill { .. }
            | OperationKind::AreaMill
            | OperationKind::ModelChamfer { .. }
            | OperationKind::HelixBore { .. }
            | OperationKind::ThreadMill { .. }
            | OperationKind::Contour { .. }
            | OperationKind::Drill { .. } => None,
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum OperationError {
    #[error("operation name is empty")]
    EmptyName,
    #[error("operation has no tool")]
    EmptyTool,
    #[error("cut depth must be finite and not negative, got {0} mm")]
    BadDepth(f64),
    #[error("stepover must be finite and greater than zero, got {0} mm")]
    BadStepover(f64),
    #[error("peck depth must be finite and greater than zero, got {0} mm")]
    BadPeck(f64),
    #[error("the operation needs geometry to machine")]
    MissingGeometry,
    #[error("geometry {0:?} does not exist")]
    UnknownGeometry(GeometryId),
    #[error("geometry {0:?} is not the right shape for this operation")]
    WrongGeometry(GeometryId),
}

/// Where the tool may move relative to the selected containment boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Containment {
    #[default]
    Inside,
    Outside,
    Centers,
}

/// Constant-engagement (dynamic) parameters, in Mastercam's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EngagementOptions {
    /// Radial advance between trochoidal loops, mm.
    #[serde(default = "default_engagement_stepover")]
    pub stepover_mm: f64,
    /// Requested loop radius, mm.
    #[serde(default = "default_loop_radius")]
    pub loop_radius_mm: f64,
    /// Minimum toolpath radius (Mastercam's clamp), mm.
    #[serde(default = "default_min_toolpath_radius")]
    pub min_toolpath_radius_mm: f64,
    /// Micro lift on back moves, mm.
    #[serde(default = "default_micro_lift")]
    pub micro_lift_mm: f64,
    /// Back-move feed, mm/min; 0 uses 1.5× the cutting feed.
    #[serde(default)]
    pub back_feed_mm_min: f64,
    /// Target engagement angle in degrees; 0 derives it from the stepover.
    #[serde(default)]
    pub engagement_angle_deg: f64,
    /// Material left on walls/floors, mm.
    #[serde(default)]
    pub stock_to_leave_mm: f64,
    #[serde(default)]
    pub containment: Containment,
}

fn default_engagement_stepover() -> f64 {
    1.5
}

fn default_loop_radius() -> f64 {
    2.7
}

fn default_min_toolpath_radius() -> f64 {
    0.6
}

fn default_micro_lift() -> f64 {
    0.2
}

impl Default for EngagementOptions {
    fn default() -> Self {
        Self {
            stepover_mm: 1.5,
            loop_radius_mm: 2.7,
            min_toolpath_radius_mm: 0.6,
            micro_lift_mm: 0.2,
            back_feed_mm_min: 0.0,
            engagement_angle_deg: 0.0,
            stock_to_leave_mm: 0.0,
            containment: Containment::Inside,
        }
    }
}

/// Per-operation routing options, kept out of the operation kind so existing files load.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OperationOptions {
    #[serde(default)]
    pub compensation: Compensation,
    #[serde(default)]
    pub pocket_strategy: PocketStrategy,
    #[serde(default)]
    pub entry: EntryStyle,
    #[serde(default = "default_lead_in")]
    pub lead_in_mm: f64,
    #[serde(default = "default_true")]
    pub finish_pass: bool,
    /// Facing raster angle, degrees from +X.
    #[serde(default)]
    pub facing_angle_deg: f64,
    /// Ramp entry angle from horizontal, degrees.
    #[serde(default = "default_ramp_angle")]
    pub ramp_angle_deg: f64,
    /// Ramp depth per contour pass; None uses the operation's depth per pass.
    #[serde(default)]
    pub ramp_depth_per_pass_mm: Option<f64>,
    #[serde(default)]
    pub engagement: EngagementOptions,
}

fn default_lead_in() -> f64 {
    3.0
}

fn default_true() -> bool {
    true
}

fn default_ramp_angle() -> f64 {
    3.0
}

impl OperationOptions {
    /// Mastercam "compensation type": where the cutter centre sits relative to the chain.
    pub fn with_compensation(mut self, compensation: Compensation) -> Self {
        self.compensation = compensation;
        self
    }

    /// Mastercam "cutting method" for pockets.
    pub fn with_pocket_strategy(mut self, strategy: PocketStrategy) -> Self {
        self.pocket_strategy = strategy;
        self
    }

    /// Mastercam entry motion (plunge, ramp, helix).
    pub fn with_entry(mut self, entry: EntryStyle) -> Self {
        self.entry = entry;
        self
    }

    /// Mastercam "finish pass" switch.
    pub fn with_finish_pass(mut self, finish_pass: bool) -> Self {
        self.finish_pass = finish_pass;
        self
    }

    /// Mastercam face "cutting angle".
    pub fn with_facing_angle(mut self, degrees: f64) -> Self {
        self.facing_angle_deg = degrees;
        self
    }

    /// Mastercam ramp angle from horizontal.
    pub fn with_ramp_angle(mut self, degrees: f64) -> Self {
        self.ramp_angle_deg = degrees;
        self
    }

    /// Mastercam ramp depth per pass.
    pub fn with_ramp_depth_per_pass(mut self, depth_mm: f64) -> Self {
        self.ramp_depth_per_pass_mm = Some(depth_mm);
        self
    }

    /// Constant-engagement parameters for dynamic operations.
    pub fn with_engagement(mut self, engagement: EngagementOptions) -> Self {
        self.engagement = engagement;
        self
    }
}

impl Default for OperationOptions {
    fn default() -> Self {
        Self {
            compensation: Compensation::None,
            pocket_strategy: PocketStrategy::Spiral,
            entry: EntryStyle::Plunge,
            lead_in_mm: 3.0,
            finish_pass: true,
            facing_angle_deg: 0.0,
            ramp_angle_deg: 3.0,
            ramp_depth_per_pass_mm: None,
            engagement: EngagementOptions::default(),
        }
    }
}

/// One machining operation, in WCS millimetres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    name: String,
    kind: OperationKind,
    tool: String,
    depth_mm: f64,
    suppressed: bool,
    #[serde(default)]
    targets: Vec<EntityRef>,
    #[serde(default)]
    selection: Vec<GeometryId>,
    #[serde(default)]
    options: OperationOptions,
}

impl Operation {
    /// Build and validate an operation. `tool` is the name of an entry in the job's tool
    /// library (see the tool library next to this module).
    pub fn new(name: impl Into<String>, kind: OperationKind, tool: impl Into<String>, depth_mm: f64) -> Result<Self, OperationError> {
        let name = name.into();
        let tool = tool.into();
        if name.trim().is_empty() {
            return Err(OperationError::EmptyName);
        }
        if tool.trim().is_empty() {
            return Err(OperationError::EmptyTool);
        }
        if !depth_mm.is_finite() || depth_mm < 0.0 {
            return Err(OperationError::BadDepth(depth_mm));
        }
        if let Some(stepover) = kind.stepover_mm()
            && (!stepover.is_finite() || stepover <= 0.0)
        {
            return Err(OperationError::BadStepover(stepover));
        }
        if let OperationKind::Drill { peck_mm: Some(peck) } = kind
            && (!peck.is_finite() || peck <= 0.0)
        {
            return Err(OperationError::BadPeck(peck));
        }
        Ok(Self { name, kind, tool, depth_mm, suppressed: false, targets: Vec::new(), selection: Vec::new(), options: OperationOptions::default() })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> OperationKind {
        self.kind
    }

    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// Design entities this operation is tied to; a rebuild re-resolves them.
    pub fn targets(&self) -> &[EntityRef] {
        &self.targets
    }

    /// Attach the design entities this operation machines.
    pub fn with_targets(mut self, targets: Vec<EntityRef>) -> Self {
        self.targets = targets;
        self
    }

    /// The geometry this operation machines.
    pub fn selection(&self) -> &[GeometryId] {
        &self.selection
    }

    pub fn with_selection(mut self, selection: Vec<GeometryId>) -> Self {
        self.selection = selection;
        self
    }

    pub fn options(&self) -> OperationOptions {
        self.options
    }

    /// Change the cut depth (an edit in the operation tree).
    pub fn set_depth(&mut self, depth_mm: f64) -> Result<(), OperationError> {
        if !depth_mm.is_finite() || depth_mm < 0.0 {
            return Err(OperationError::BadDepth(depth_mm));
        }
        self.depth_mm = depth_mm;
        Ok(())
    }

    pub fn set_options(&mut self, options: OperationOptions) {
        self.options = options;
    }

    /// The parameter set a command schema or an agent receives for this operation.
    pub fn parameter_set(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    pub fn with_options(mut self, options: OperationOptions) -> Self {
        self.options = options;
        self
    }

    /// Check that the selected geometry exists and suits the operation.
    pub fn validate_geometry(&self, geometry: &GeometryStore) -> Result<(), OperationError> {
        if matches!(
            self.kind,
            OperationKind::Face { .. }
                | OperationKind::CircleMill { .. }
                | OperationKind::Drill { .. }
                | OperationKind::HelixBore { .. }
                | OperationKind::ThreadMill { .. }
        ) {
            return Ok(());
        }
        if matches!(self.kind, OperationKind::BlendMill { .. }) && self.selection.len() < 2 {
            return Err(OperationError::MissingGeometry);
        }
        if self.selection.is_empty() {
            return Err(OperationError::MissingGeometry);
        }
        for id in &self.selection {
            let item = geometry.get(*id).ok_or(OperationError::UnknownGeometry(*id))?;
            match self.kind {
                OperationKind::Pocket { .. } | OperationKind::Slot { .. } | OperationKind::DynamicMill | OperationKind::AreaMill => {
                    let region_ok = item.region().is_some();
                    let closed_ok = item.chain().map(|chain| chain.is_closed()).unwrap_or(false);
                    if !region_ok && !closed_ok {
                        return Err(OperationError::WrongGeometry(*id));
                    }
                }
                OperationKind::Contour { .. }
                | OperationKind::Engrave
                | OperationKind::DynamicContour
                | OperationKind::PeelMill { .. }
                | OperationKind::BlendMill { .. }
                | OperationKind::ModelChamfer { .. } => {}
                _ => {}
            }
        }
        Ok(())
    }

    /// Point the operation at another tool or assembly.
    pub fn set_tool(&mut self, tool: impl Into<String>) {
        self.tool = tool.into();
    }

    /// Point the operation at another tool, as a rebuild does when a hole changes diameter.
    pub(crate) fn retarget_tool(&mut self, tool: impl Into<String>) {
        self.tool = tool.into();
    }

    /// Depth of cut in mm, below the setup's Z zero.
    pub fn depth_mm(&self) -> f64 {
        self.depth_mm
    }

    /// A suppressed operation stays in the tree but does not run.
    pub fn is_suppressed(&self) -> bool {
        self.suppressed
    }

    pub fn suppress(&mut self) {
        self.suppressed = true;
    }

    pub fn unsuppress(&mut self) {
        self.suppressed = false;
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ProgramError {
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error("an operation named `{0}` already exists")]
    DuplicateName(String),
    #[error("operation index {index} is out of range (the program has {len})")]
    IndexOutOfRange { index: usize, len: usize },
}

/// The ordered operation list of a job.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Program {
    operations: Vec<Operation>,
}

impl Program {
    pub fn new() -> Self {
        Self::default()
    }

    /// The operations that actually run, in order.
    pub fn active(&self) -> impl Iterator<Item = &Operation> {
        self.operations.iter().filter(|op| !op.is_suppressed())
    }

    /// Every operation, including suppressed ones, in order.
    pub fn iter(&self) -> impl Iterator<Item = &Operation> {
        self.operations.iter()
    }

    /// Every operation, mutably, for a rebuild.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Operation> {
        self.operations.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.operations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Operation> {
        self.operations.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut Operation> {
        self.operations.get_mut(index)
    }

    /// Append an operation.
    pub fn push(&mut self, operation: Operation) -> Result<(), ProgramError> {
        self.check_name(&operation, None)?;
        self.operations.push(operation);
        Ok(())
    }

    /// Insert at `index`, shifting later operations down.
    pub fn insert(&mut self, index: usize, operation: Operation) -> Result<(), ProgramError> {
        if index > self.operations.len() {
            return Err(ProgramError::IndexOutOfRange { index, len: self.operations.len() });
        }
        self.check_name(&operation, None)?;
        self.operations.insert(index, operation);
        Ok(())
    }

    /// Remove and return the operation at `index`.
    pub fn remove(&mut self, index: usize) -> Result<Operation, ProgramError> {
        if index >= self.operations.len() {
            return Err(ProgramError::IndexOutOfRange { index, len: self.operations.len() });
        }
        Ok(self.operations.remove(index))
    }

    /// Move an operation to another position, e.g. to run a drill before a pocket.
    pub fn move_to(&mut self, from: usize, to: usize) -> Result<(), ProgramError> {
        let len = self.operations.len();
        if from >= len {
            return Err(ProgramError::IndexOutOfRange { index: from, len });
        }
        if to >= len {
            return Err(ProgramError::IndexOutOfRange { index: to, len });
        }
        if from == to {
            return Ok(());
        }
        let operation = self.operations.remove(from);
        self.operations.insert(to, operation);
        Ok(())
    }

    fn check_name(&self, operation: &Operation, skip: Option<usize>) -> Result<(), ProgramError> {
        let clash = self.operations.iter().enumerate().any(|(i, other)| Some(i) != skip && other.name == operation.name);
        if clash {
            return Err(ProgramError::DuplicateName(operation.name.clone()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face() -> Operation {
        Operation::new("Face top", OperationKind::Face { stepover_mm: 15.0 }, "6mm flat", 0.5).unwrap()
    }

    fn pocket() -> Operation {
        Operation::new("Rough pocket", OperationKind::Pocket { stepover_mm: 3.0 }, "6mm flat", 8.0).unwrap()
    }

    fn drill() -> Operation {
        Operation::new("Drill 6 holes", OperationKind::Drill { peck_mm: Some(2.0) }, "5mm drill", 10.0).unwrap()
    }

    #[test]
    fn mastercam_named_defaults_and_builders_agree_with_the_mapping() {
        let defaults = OperationOptions::default();
        assert_eq!(defaults.compensation, Compensation::None);
        assert_eq!(defaults.pocket_strategy, PocketStrategy::Spiral);
        assert_eq!(defaults.entry, EntryStyle::Plunge);
        assert_eq!(defaults.lead_in_mm, 3.0);
        assert!(defaults.finish_pass);
        assert_eq!(defaults.facing_angle_deg, 0.0);
        assert_eq!(defaults.ramp_angle_deg, 3.0);
        assert_eq!(defaults.ramp_depth_per_pass_mm, None);

        let custom = OperationOptions::default()
            .with_compensation(Compensation::Left)
            .with_pocket_strategy(PocketStrategy::ZigZag)
            .with_entry(EntryStyle::Helix)
            .with_finish_pass(false)
            .with_facing_angle(45.0)
            .with_ramp_angle(5.0)
            .with_ramp_depth_per_pass(0.4);
        assert_eq!(custom.compensation, Compensation::Left);
        assert_eq!(custom.pocket_strategy, PocketStrategy::ZigZag);
        assert_eq!(custom.entry, EntryStyle::Helix);
        assert!(!custom.finish_pass);
        assert_eq!(custom.facing_angle_deg, 45.0);
        assert_eq!(custom.ramp_angle_deg, 5.0);
        assert_eq!(custom.ramp_depth_per_pass_mm, Some(0.4));
    }

    #[test]
    fn operations_keep_their_order() {
        let mut program = Program::new();
        program.push(face()).unwrap();
        program.push(pocket()).unwrap();
        program.push(drill()).unwrap();
        let names: Vec<_> = program.iter().map(|op| op.name()).collect();
        assert_eq!(names, ["Face top", "Rough pocket", "Drill 6 holes"]);
    }

    #[test]
    fn a_suppressed_operation_stays_but_does_not_run() {
        let mut program = Program::new();
        program.push(face()).unwrap();
        program.push(pocket()).unwrap();
        program.get_mut(1).unwrap().suppress();
        let active: Vec<_> = program.active().map(|op| op.name()).collect();
        assert_eq!(active, ["Face top"]);
        assert_eq!(program.len(), 2);
    }

    #[test]
    fn move_to_reorders_the_program() {
        let mut program = Program::new();
        program.push(face()).unwrap();
        program.push(pocket()).unwrap();
        program.push(drill()).unwrap();
        program.move_to(2, 0).unwrap();
        let names: Vec<_> = program.iter().map(|op| op.name()).collect();
        assert_eq!(names, ["Drill 6 holes", "Face top", "Rough pocket"]);
    }

    #[test]
    fn insert_and_remove_keep_the_list_contiguous() {
        let mut program = Program::new();
        program.push(face()).unwrap();
        program.push(drill()).unwrap();
        program.insert(1, pocket()).unwrap();
        assert_eq!(program.get(1).unwrap().name(), "Rough pocket");
        let removed = program.remove(0).unwrap();
        assert_eq!(removed.name(), "Face top");
        assert_eq!(program.len(), 2);
        assert_eq!(program.get(0).unwrap().name(), "Rough pocket");
    }

    #[test]
    fn duplicate_names_and_bad_indices_are_rejected() {
        let mut program = Program::new();
        program.push(face()).unwrap();
        assert_eq!(program.push(face()), Err(ProgramError::DuplicateName("Face top".into())));
        assert_eq!(program.move_to(0, 5), Err(ProgramError::IndexOutOfRange { index: 5, len: 1 }));
        assert_eq!(program.remove(5), Err(ProgramError::IndexOutOfRange { index: 5, len: 1 }));
    }

    #[test]
    fn invalid_operations_are_rejected() {
        assert_eq!(Operation::new("", OperationKind::Face { stepover_mm: 1.0 }, "tool", 1.0), Err(OperationError::EmptyName));
        assert_eq!(Operation::new("op", OperationKind::Face { stepover_mm: 1.0 }, "", 1.0), Err(OperationError::EmptyTool));
        assert_eq!(Operation::new("op", OperationKind::Face { stepover_mm: 1.0 }, "tool", -1.0), Err(OperationError::BadDepth(-1.0)));
        assert_eq!(Operation::new("op", OperationKind::Pocket { stepover_mm: 0.0 }, "tool", 1.0), Err(OperationError::BadStepover(0.0)));
        assert_eq!(Operation::new("op", OperationKind::Drill { peck_mm: Some(0.0) }, "tool", 1.0), Err(OperationError::BadPeck(0.0)));
    }
}

//! Post-processing: turn a toolpath into machine G-code.
//!
//! The IR is a list of [`GcodeBlock`]s plus a [`ModalState`] that suppresses repeated modes,
//! feeds and unchanged coordinates. [`GrblPost`] is the first concrete post; it follows
//! `docs/grbl-scope.md`: absolute millimetres, G54, G17, G2/G3 with I/J, M3/M5 and optional
//! flood coolant. No tool changer, no tool-length compensation.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::generate::{GenerateError, generate};
use crate::hybrid::CamPlan;
use crate::operation::Operation;
use crate::tool::CuttingData;
use crate::toolpath::{Move, Toolpath};

/// Version of the GRBL post format. Cached G-code is only valid for the same version.
pub const GRBL_POST_VERSION: u32 = 1;

/// One line of the programme. `number` is 0 when line numbering is off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcodeBlock {
    pub number: u32,
    pub text: String,
}

/// Modal motion mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MotionMode {
    #[default]
    Rapid,
    Linear,
    ArcCw,
    ArcCcw,
}

impl MotionMode {
    fn word(self) -> &'static str {
        match self {
            MotionMode::Rapid => "G0",
            MotionMode::Linear => "G1",
            MotionMode::ArcCw => "G2",
            MotionMode::ArcCcw => "G3",
        }
    }
}

/// State that survives from one block to the next.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModalState {
    pub motion: MotionMode,
    pub units_mm: bool,
    pub absolute: bool,
    pub plane_xy: bool,
    pub feed: Option<f64>,
    pub spindle: Option<u32>,
    pub coolant: bool,
    pub position: Option<[f64; 3]>,
}

/// A complete programme.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GcodeProgram {
    pub blocks: Vec<GcodeBlock>,
    pub operations: Vec<String>,
}

impl GcodeProgram {
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|block| if block.number == 0 { block.text.clone() } else { format!("N{} {}", block.number, block.text) })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PostOptions {
    pub program_name: String,
    pub line_numbers: bool,
    pub coolant: bool,
    pub decimals: usize,
}

impl Default for PostOptions {
    fn default() -> Self {
        Self { program_name: "1001".into(), line_numbers: false, coolant: false, decimals: 3 }
    }
}

#[derive(Debug, Error)]
pub enum PostError {
    #[error("could not generate the operation's path: {0}")]
    Generate(#[from] GenerateError),
    #[error("operation `{0}` uses a tool that is not in the plan")]
    UnknownTool(String),
}

/// A post-processor turns one operation's path into a programme.
pub trait PostProcessor {
    fn post(&self, plan: &CamPlan, operation: &Operation, path: &Toolpath) -> Result<GcodeProgram, PostError>;
}

/// The GRBL post.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GrblPost {
    options: PostOptions,
}

impl GrblPost {
    pub fn new(options: PostOptions) -> Self {
        Self { options }
    }

    pub fn options(&self) -> &PostOptions {
        &self.options
    }

    /// Post every active operation into one programme with a single header and footer.
    pub fn post_all(&self, plan: &CamPlan) -> Result<GcodeProgram, PostError> {
        let mut emitter = Emitter::new(&self.options);
        emitter.header();
        let mut operations = Vec::new();
        for operation in plan.program().active() {
            let path = generate(plan, operation)?;
            emitter.operation(plan, operation)?;
            emitter.emit_path(&path);
            operations.push(operation.name().to_string());
        }
        emitter.footer();
        Ok(GcodeProgram { blocks: emitter.blocks, operations })
    }
}

impl PostProcessor for GrblPost {
    fn post(&self, plan: &CamPlan, operation: &Operation, path: &Toolpath) -> Result<GcodeProgram, PostError> {
        let mut emitter = Emitter::new(&self.options);
        emitter.header();
        emitter.operation(plan, operation)?;
        emitter.emit_path(path);
        emitter.footer();
        Ok(GcodeProgram { blocks: emitter.blocks, operations: vec![operation.name().to_string()] })
    }
}

/// Generate and post one operation.
pub fn post(plan: &CamPlan, operation: &Operation) -> Result<GcodeProgram, PostError> {
    let path = generate(plan, operation)?;
    GrblPost::default().post(plan, operation, &path)
}

/// Generate and post every active operation into one programme.
pub fn post_all(plan: &CamPlan) -> Result<GcodeProgram, PostError> {
    GrblPost::default().post_all(plan)
}

struct Emitter<'a> {
    options: &'a PostOptions,
    blocks: Vec<GcodeBlock>,
    number: u32,
    state: ModalState,
}

impl<'a> Emitter<'a> {
    fn new(options: &'a PostOptions) -> Self {
        Self { options, blocks: Vec::new(), number: 0, state: ModalState::default() }
    }

    fn push(&mut self, text: impl Into<String>) {
        self.number += 1;
        let number = if self.options.line_numbers { self.number } else { 0 };
        self.blocks.push(GcodeBlock { number, text: text.into() });
    }

    fn comment(&mut self, text: impl AsRef<str>) {
        self.push(format!("; {}", text.as_ref()));
    }

    fn header(&mut self) {
        self.comment("SolidCraft CAM");
        self.comment(format!("Program {}", self.options.program_name));
        self.push("G21");
        self.push("G90");
        self.push("G17");
        self.push("G54");
        self.state.units_mm = true;
        self.state.absolute = true;
        self.state.plane_xy = true;
        if self.options.coolant {
            self.push("M8");
            self.state.coolant = true;
        }
    }

    fn footer(&mut self) {
        self.push("M5");
        if self.state.coolant {
            self.push("M9");
        }
        self.push("M30");
        self.state.spindle = None;
    }

    fn operation(&mut self, plan: &CamPlan, operation: &Operation) -> Result<(), PostError> {
        let resolved = plan.resolve_tool(operation.tool()).ok_or_else(|| PostError::UnknownTool(operation.tool().to_string()))?;
        let tool = &resolved.tool;
        self.comment(format!("Operation: {}", operation.name()));
        self.comment(format!("Tool: {} D{}", tool.name(), format_number(tool.diameter_mm(), self.options.decimals)));
        let spindle = tool
            .cutting()
            .map(CuttingData::spindle_rpm)
            .unwrap_or_else(|| CuttingData::recommended(plan.stock().material(), tool).spindle_rpm())
            .round() as u32;
        if self.state.spindle != Some(spindle) {
            self.push(format!("M3 S{spindle}"));
            self.state.spindle = Some(spindle);
        }
        Ok(())
    }

    fn emit_path(&mut self, path: &Toolpath) {
        // Position the tool at the path's start before the first motion.
        self.emit_motion(MotionMode::Rapid, path.start(), None, None);
        for motion in path.moves() {
            match motion {
                Move::Rapid { to } => self.emit_motion(MotionMode::Rapid, *to, None, None),
                Move::Feed { to, feed_mm_min } => self.emit_motion(MotionMode::Linear, *to, Some(*feed_mm_min), None),
                Move::Arc { to, centre, clockwise, feed_mm_min } => {
                    let i = centre[0] - self.state.position.unwrap_or(*to)[0];
                    let j = centre[1] - self.state.position.unwrap_or(*to)[1];
                    let mode = if *clockwise { MotionMode::ArcCw } else { MotionMode::ArcCcw };
                    self.emit_motion(mode, *to, Some(*feed_mm_min), Some((i, j)));
                }
            }
        }
    }

    fn emit_motion(&mut self, mode: MotionMode, to: [f64; 3], feed: Option<f64>, arc: Option<(f64, f64)>) {
        let mut words = Vec::new();
        if mode != self.state.motion || self.state.position.is_none() {
            words.push(mode.word().to_string());
            self.state.motion = mode;
        }
        let previous = self.state.position;
        for (axis, letter) in ["X", "Y", "Z"].iter().enumerate() {
            let changed = previous.map(|p| (p[axis] - to[axis]).abs() > 1e-9).unwrap_or(true);
            if changed {
                words.push(format!("{letter}{}", format_number(to[axis], self.options.decimals)));
            }
        }
        if let Some((i, j)) = arc {
            words.push(format!("I{}", format_number(i, self.options.decimals)));
            words.push(format!("J{}", format_number(j, self.options.decimals)));
        }
        if let Some(feed) = feed
            && self.state.feed.map(|current| (current - feed).abs() > 1e-9).unwrap_or(true)
        {
            words.push(format!("F{}", format_number(feed, self.options.decimals)));
            self.state.feed = Some(feed);
        }
        if !words.is_empty() {
            self.push(words.join(" "));
        }
        self.state.position = Some(to);
    }
}

fn format_number(value: f64, decimals: usize) -> String {
    let mut text = format!("{:.*}", decimals, value);
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" { "0".to_string() } else { text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Curve2D, GeometryShape, Region};
    use crate::operation::OperationKind;
    use crate::reference::DesignRef;
    use crate::setup::Setup;
    use crate::stock::{Material, Stock};
    use crate::tool::{Cutter, Tool};

    fn stock() -> Stock {
        Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap()
    }

    fn square() -> crate::geometry::Chain {
        crate::geometry::Chain::closed(vec![Curve2D::polyline(vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]], true).unwrap()], 1e-6)
            .unwrap()
    }

    fn rounded() -> crate::geometry::Chain {
        let q = std::f64::consts::FRAC_PI_2;
        crate::geometry::Chain::closed(
            vec![
                Curve2D::line([5.0, 0.0], [25.0, 0.0]).unwrap(),
                Curve2D::arc([25.0, 5.0], 5.0, -q, q).unwrap(),
                Curve2D::line([30.0, 5.0], [30.0, 15.0]).unwrap(),
                Curve2D::arc([25.0, 15.0], 5.0, 0.0, q).unwrap(),
                Curve2D::line([25.0, 20.0], [5.0, 20.0]).unwrap(),
                Curve2D::arc([5.0, 15.0], 5.0, q, q).unwrap(),
                Curve2D::line([0.0, 15.0], [0.0, 5.0]).unwrap(),
                Curve2D::arc([5.0, 5.0], 5.0, 2.0 * q, q).unwrap(),
            ],
            1e-6,
        )
        .unwrap()
    }

    fn plan() -> CamPlan {
        let stock = stock();
        let setup = Setup::top_centre(&stock);
        let mut plan = CamPlan::new(DesignRef::new("bracket", 1).unwrap(), stock, setup);
        plan.add_tool(Tool::new("6mm flat", Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap()).unwrap();
        plan.geometry_mut().add_layer("Machining").unwrap();
        let square_id = plan.geometry_mut().add("Machining", GeometryShape::Chain(square())).unwrap();
        let rounded_id = plan.geometry_mut().add("Machining", GeometryShape::Chain(rounded())).unwrap();
        let region_id = plan.geometry_mut().add("Machining", GeometryShape::Region(Region::new(square(), vec![]).unwrap())).unwrap();
        plan.add_operation(
            Operation::new("Contour", OperationKind::Contour { finish_passes: 1 }, "6mm flat", 2.0).unwrap().with_selection(vec![square_id]),
        )
        .unwrap();
        plan.add_operation(
            Operation::new("Pocket", OperationKind::Pocket { stepover_mm: 4.0 }, "6mm flat", 2.0).unwrap().with_selection(vec![region_id]),
        )
        .unwrap();
        plan.add_operation(
            Operation::new("Arc Contour", OperationKind::Contour { finish_passes: 1 }, "6mm flat", 2.0).unwrap().with_selection(vec![rounded_id]),
        )
        .unwrap();
        plan.add_operation(Operation::new("Circle", OperationKind::CircleMill { diameter_mm: 20.0, center: [10.0, 10.0] }, "6mm flat", 2.0).unwrap())
            .unwrap();
        plan.add_operation(Operation::new("Slot", OperationKind::Slot { width_mm: 12.0 }, "6mm flat", 2.0).unwrap().with_selection(vec![square_id]))
            .unwrap();
        plan
    }

    #[test]
    fn the_programme_has_a_grbl_header_and_footer() {
        let programme = post(&plan(), plan().program().get(0).unwrap()).unwrap();
        let text = programme.text();
        for required in ["; SolidCraft CAM", "G21", "G90", "G17", "G54", "M3 S", "G0", "G1", "F", "M5", "M30"] {
            assert!(text.contains(required), "missing `{required}` in:\n{text}");
        }
    }

    #[test]
    fn arcs_carry_i_and_j() {
        let plan = plan();
        let operation = plan.program().iter().find(|op| op.name() == "Arc Contour").unwrap();
        let text = post(&plan, operation).unwrap().text();
        assert!(text.contains("G2") || text.contains("G3"), "{text}");
        assert!(text.contains("I") && text.contains("J"), "{text}");
    }

    #[test]
    fn modal_state_suppresses_repeated_modes_and_feeds() {
        let plan = plan();
        let operation = plan.program().get(0).unwrap();
        let path = generate(&plan, operation).unwrap();
        let feed_moves = path.moves().iter().filter(|m| m.feed_mm_min().is_some()).count();
        let programme = post(&plan, operation).unwrap();
        let text = programme.text();
        let g1_count = text.lines().filter(|line| line.contains("G1")).count();
        let f_count = text.lines().filter(|line| line.contains(" F")).count();
        assert!(g1_count < feed_moves, "G1 not suppressed: {g1_count} vs {feed_moves}");
        assert!(f_count < feed_moves, "F not suppressed: {f_count} vs {feed_moves}");
    }

    #[test]
    fn coolant_and_line_numbers_are_options() {
        let post = GrblPost::new(PostOptions { line_numbers: true, coolant: true, ..PostOptions::default() });
        let plan = plan();
        let programme = post.post(&plan, plan.program().get(0).unwrap(), &generate(&plan, plan.program().get(0).unwrap()).unwrap()).unwrap();
        let text = programme.text();
        assert!(text.contains("M8") && text.contains("M9"));
        assert!(text.lines().next().unwrap().starts_with("N1 "));
        assert!(text.lines().all(|line| line.starts_with('N')));
    }

    #[test]
    fn every_core_operation_posts_a_complete_programme() {
        let plan = plan();
        for operation in plan.program().iter() {
            let text = post(&plan, operation).unwrap().text();
            assert!(text.contains("M3 S"), "{} has no spindle", operation.name());
            assert!(text.contains("G0"), "{} has no rapid", operation.name());
            assert!(!text.contains("G43"), "{} posted tool length compensation", operation.name());
            if operation.name() == "Arc Contour" {
                assert!(text.contains("G2") || text.contains("G3"), "arcs missing:\n{text}");
                assert!(text.contains('I') && text.contains('J'), "I/J missing:\n{text}");
            }
        }
    }

    #[test]
    fn arc_direction_selects_g2_or_g3() {
        let plan = plan();
        let operation = plan.program().get(0).unwrap();
        let mut clockwise = Toolpath::new([0.0, 0.0, 0.0], crate::toolpath::Heights::default()).unwrap();
        clockwise.arc([10.0, 0.0, 0.0], [5.0, 0.0, 0.0], true, 100.0).unwrap();
        let text = GrblPost::default().post(&plan, operation, &clockwise).unwrap().text();
        assert!(text.contains("G2"), "clockwise arc did not post G2:\n{text}");

        let mut counter = Toolpath::new([0.0, 0.0, 0.0], crate::toolpath::Heights::default()).unwrap();
        counter.arc([10.0, 0.0, 0.0], [5.0, 0.0, 0.0], false, 100.0).unwrap();
        let text = GrblPost::default().post(&plan, operation, &counter).unwrap().text();
        assert!(text.contains("G3"), "counter-clockwise arc did not post G3:\n{text}");
    }

    #[test]
    fn post_all_writes_one_header_and_every_operation() {
        let plan = plan();
        let programme = post_all(&plan).unwrap();
        let text = programme.text();
        assert_eq!(text.matches("G21").count(), 1);
        assert_eq!(text.matches("M30").count(), 1);
        for operation in plan.program().iter() {
            assert!(text.contains(&format!("; Operation: {}", operation.name())));
        }
        assert_eq!(programme.operations.len(), plan.program().active().count());
    }
}

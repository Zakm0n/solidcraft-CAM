//! Simulation and verification: cut the stock with a toolpath, then check what is left.
//!
//! [`Simulation`] wraps a cutrail engine: tiled mesh stock, robust booleans, and per-cut
//! reports. Cutting a [`Toolpath`] with a [`Tool`] in a [`Setup`] yields a [`PathReport`] with
//! the removed volume, and [`Simulation::compare_with_design`] reports excess material (what a
//! real cut would have left standing) and gouges (where the tool cut into the design).

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use cutrail::{CutOutcome, DesignComparison, Engine, EngineOptions, ManifoldKernel, TriMesh};
use thiserror::Error;

use crate::{Setup, Stock, Tool, Toolpath};

#[derive(Debug, Error)]
pub enum SimulationError {
    #[error("simulation engine error: {0}")]
    Engine(#[from] cutrail::Error),
    #[error("stock tile size must be finite and greater than zero, got {0} mm")]
    BadTile(f64),
    #[error("tool `{0}` cannot be simulated: cutrail sweeps milling tools only, not drills")]
    UnsupportedTool(String),
    #[error("the toolpath has no cutting moves")]
    EmptyPath,
    #[error("could not write the simulated mesh: {0}")]
    Io(#[from] io::Error),
    #[error("could not generate the operation's path: {0}")]
    Generate(String),
    #[error("operation `{0}` uses a tool that is not in the plan")]
    UnknownTool(String),
}

/// Generate one operation of a plan and cut it into a fresh simulation of the plan's stock.
///
/// Returns the path as well as the report, so a caller can cache the path only after it cut
/// successfully.
pub fn simulate_operation(
    plan: &crate::hybrid::CamPlan,
    operation: &crate::operation::Operation,
    tile_mm: f64,
) -> Result<(crate::toolpath::Toolpath, PathReport), SimulationError> {
    let path = crate::generate::generate(plan, operation).map_err(|error| SimulationError::Generate(error.to_string()))?;
    let resolved = plan.resolve_tool(operation.tool()).ok_or_else(|| SimulationError::UnknownTool(operation.tool().to_string()))?;
    let cutter = match (&resolved.assembly, &resolved.holder) {
        (Some(assembly), holder) => {
            assembly.to_cutrail(&resolved.tool, holder.as_ref()).map_err(|error| SimulationError::Generate(error.to_string()))?
        }
        _ => resolved.tool.to_cutrail().ok_or_else(|| SimulationError::UnknownTool(resolved.tool.name().to_string()))?,
    };
    let mut simulation = Simulation::new(plan.stock(), tile_mm)?;
    let report = simulation.cut_path_cutrail(&cutter, &path, plan.setup())?;
    Ok((path, report))
}

/// What one toolpath did to the stock.
#[derive(Debug, Clone, PartialEq)]
pub struct PathReport {
    /// How many cutting segments were simulated.
    pub segments: usize,
    /// Total removed volume, mm³.
    pub removed_volume_mm3: f64,
    /// Segments that removed nothing (for example a second pass down the same slot).
    pub no_change_segments: usize,
    /// True when the stock was completely removed.
    pub exhausted: bool,
    /// Segments whose shank or holder hit the stock (only with collision checking enabled).
    pub collisions: usize,
    /// Time spent in the booleans.
    pub elapsed: Duration,
}

impl PathReport {
    /// Whether the path did nothing at all.
    pub fn is_no_op(&self) -> bool {
        self.no_change_segments == self.segments
    }
}

/// A stock block under the cutter.
pub struct Simulation {
    engine: Engine,
    initial_volume_mm3: f64,
}

impl Simulation {
    /// A tiled simulation of `stock`, with tiles `tile_mm` on a side.
    pub fn new(stock: &Stock, tile_mm: f64) -> Result<Self, SimulationError> {
        if !tile_mm.is_finite() || tile_mm <= 0.0 {
            return Err(SimulationError::BadTile(tile_mm));
        }
        let options = EngineOptions::default().with_tile_size(tile_mm).with_collision_check();
        Self::with_options(stock, options)
    }

    /// A simulation with explicit cutrail options (tolerance, tiling, collision checks).
    pub fn with_options(stock: &Stock, options: EngineOptions) -> Result<Self, SimulationError> {
        let engine = Engine::new(ManifoldKernel, &[stock.to_cutrail()], options)?;
        let initial_volume_mm3 = engine.total_volume();
        Ok(Self { engine, initial_volume_mm3 })
    }

    /// Cut the stock along `path` with `tool`, both interpreted in `setup`.
    pub fn cut_path(&mut self, tool: &Tool, path: &Toolpath, setup: &Setup) -> Result<PathReport, SimulationError> {
        let cutter = tool.to_cutrail().ok_or_else(|| SimulationError::UnsupportedTool(tool.name().to_string()))?;
        self.cut_path_cutrail(&cutter, path, setup)
    }

    /// Cut with an already assembled cutrail tool (cutter + shank + holder).
    pub fn cut_path_cutrail(&mut self, cutter: &cutrail::Tool, path: &Toolpath, setup: &Setup) -> Result<PathReport, SimulationError> {
        let segments = path.to_cutrail_cutting(setup);
        if segments.is_empty() {
            return Err(SimulationError::EmptyPath);
        }

        let mut report = PathReport {
            segments: segments.len(),
            removed_volume_mm3: 0.0,
            no_change_segments: 0,
            exhausted: false,
            collisions: 0,
            elapsed: Duration::ZERO,
        };
        for segment in &segments {
            let cut = self.engine.cut(cutter, segment)?;
            report.removed_volume_mm3 += cut.removed_volume;
            report.elapsed += cut.elapsed;
            if cut.outcome == CutOutcome::NoChange {
                report.no_change_segments += 1;
            }
            if cut.outcome == CutOutcome::Exhausted {
                report.exhausted = true;
            }
            if cut.collision.is_some() {
                report.collisions += 1;
            }
        }
        Ok(report)
    }

    /// Cut one motion segment directly (used by the G-code simulator, which owns its own
    /// move list rather than building a `Toolpath`).
    pub fn cut_segment(&mut self, tool: &Tool, segment: &cutrail::MotionSegment) -> Result<cutrail::CutReport, SimulationError> {
        let cutter = tool.to_cutrail().ok_or_else(|| SimulationError::UnsupportedTool(tool.name().to_string()))?;
        Ok(self.engine.cut(&cutter, segment)?)
    }

    /// Volume of the stock that is still standing, mm³.
    pub fn remaining_volume_mm3(&self) -> f64 {
        self.engine.total_volume()
    }

    /// Volume removed since the simulation started, mm³.
    pub fn removed_volume_mm3(&self) -> f64 {
        (self.initial_volume_mm3 - self.remaining_volume_mm3()).max(0.0)
    }

    /// The machined stock as one mesh.
    pub fn mesh(&self) -> Result<TriMesh, SimulationError> {
        Ok(self.engine.merged_mesh()?)
    }

    /// Compare the machined stock with the design: excess material and gouges.
    pub fn compare_with_design(&self, design: &TriMesh, tolerance_mm: f64) -> Result<DesignComparison, SimulationError> {
        Ok(self.engine.compare_with_design(design, tolerance_mm)?)
    }

    /// Write the machined stock as STL, OBJ, glTF or 3MF, chosen by the file extension.
    pub fn write_mesh_file(&self, path: impl AsRef<Path>) -> Result<(), SimulationError> {
        Ok(self.engine.write_mesh_file(path)?)
    }

    /// Write the machined stock as binary STL.
    pub fn write_stl(&self, writer: impl Write) -> Result<(), SimulationError> {
        Ok(self.engine.write_stl(writer)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stock::Material;
    use crate::toolpath::Heights;
    use crate::{Cutter, Setup};

    fn stock() -> Stock {
        Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap()
    }

    fn setup() -> Setup {
        Setup::top_centre(&stock())
    }

    fn flat() -> Tool {
        Tool::new("6mm flat", Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap()
    }

    fn slot() -> Toolpath {
        let mut path = Toolpath::new([-20.0, 0.0, -1.0], Heights::default()).unwrap();
        path.feed([20.0, 0.0, -1.0], 300.0).unwrap();
        path
    }

    #[test]
    fn a_slot_removes_a_slot_shaped_volume() {
        let mut sim = Simulation::new(&stock(), 10.0).unwrap();
        assert!((sim.remaining_volume_mm3() - 48_000.0).abs() < 1.0);
        let report = sim.cut_path(&flat(), &slot(), &setup()).unwrap();
        assert_eq!(report.segments, 1);
        assert!(!report.is_no_op());
        // 40 mm long, 6 mm wide, 1 mm deep, with rounded ends.
        assert!(report.removed_volume_mm3 > 200.0, "removed {}", report.removed_volume_mm3);
        assert!(report.removed_volume_mm3 < 400.0, "removed {}", report.removed_volume_mm3);
        assert!((sim.removed_volume_mm3() - report.removed_volume_mm3).abs() < 1.0);
        assert!((sim.remaining_volume_mm3() - (48_000.0 - report.removed_volume_mm3)).abs() < 1.0);
    }

    #[test]
    fn cutting_the_same_slot_again_removes_nothing() {
        let mut sim = Simulation::new(&stock(), 10.0).unwrap();
        sim.cut_path(&flat(), &slot(), &setup()).unwrap();
        let again = sim.cut_path(&flat(), &slot(), &setup()).unwrap();
        assert_eq!(again.no_change_segments, again.segments);
        assert!(again.is_no_op());
        assert!(again.removed_volume_mm3.abs() < 1.0);
    }

    #[test]
    fn a_path_with_only_rapids_has_nothing_to_cut() {
        let mut sim = Simulation::new(&stock(), 10.0).unwrap();
        let mut path = Toolpath::new([0.0, 0.0, 10.0], Heights::default()).unwrap();
        path.rapid([10.0, 0.0, 10.0]).unwrap();
        assert!(matches!(sim.cut_path(&flat(), &path, &setup()), Err(SimulationError::EmptyPath)));
    }

    #[test]
    fn drills_are_reported_as_unsupported() {
        let mut sim = Simulation::new(&stock(), 10.0).unwrap();
        let drill = Tool::new("5mm drill", Cutter::Drill { point_angle_deg: 118.0 }, 5.0, 2, 20.0, 30.0).unwrap();
        assert!(matches!(
            sim.cut_path(&drill, &slot(), &setup()),
            Err(SimulationError::UnsupportedTool(name)) if name == "5mm drill"
        ));
    }

    #[test]
    fn a_bad_tile_size_is_rejected() {
        assert!(matches!(Simulation::new(&stock(), 0.0), Err(SimulationError::BadTile(0.0))));
    }

    #[test]
    fn the_machined_stock_can_be_written_as_stl() {
        let mut sim = Simulation::new(&stock(), 10.0).unwrap();
        sim.cut_path(&flat(), &slot(), &setup()).unwrap();
        let mut bytes = Vec::new();
        sim.write_stl(&mut bytes).unwrap();
        assert!(bytes.len() > 84, "STL has only {} bytes", bytes.len());
    }

    #[test]
    fn the_machined_stock_compares_with_itself_within_tolerance() {
        let mut sim = Simulation::new(&stock(), 10.0).unwrap();
        sim.cut_path(&flat(), &slot(), &setup()).unwrap();
        let design = sim.mesh().unwrap();
        let comparison = sim.compare_with_design(&design, 0.05).unwrap();
        assert!(comparison.is_within_tolerance());
        assert!(comparison.excess_volume <= 0.0);
        assert!(comparison.gouge_volume <= 0.0);
    }
}

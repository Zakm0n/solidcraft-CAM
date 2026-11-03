//! A CAM job: the design it machines, the stock, the setup, the tools and the operation list.
//!
//! A job is parametric in the same sense the design is. Operations carry [`EntityRef`]s to the
//! features they machine, so [`CamJob::rebuild`] can follow a design change — a hole that grows
//! from 6 mm to 8 mm re-targets its drilling operation to the matching tool instead of being
//! rebuilt by hand. References that no longer exist are reported, never silently dropped.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::operation::{Operation, OperationError, OperationKind, Program, ProgramError};
use crate::reference::{DesignRef, DesignSnapshot, EntityRef};
use crate::setup::Setup;
use crate::stock::Stock;
use crate::tool::{Cutter, Tool, ToolError, ToolLibrary};
use crate::toolpath::Toolpath;

#[derive(Debug, Error, PartialEq)]
pub enum JobError {
    #[error("job name is empty")]
    EmptyName,
    #[error("operation `{operation}` uses unknown tool `{tool}`")]
    UnknownTool { operation: String, tool: String },
    #[error("the job was built for design `{0}`, not `{1}`")]
    ForeignDesign(String, String),
    #[error("no operation named `{0}`")]
    UnknownOperation(String),
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error(transparent)]
    Program(#[from] ProgramError),
    #[error(transparent)]
    Tool(#[from] ToolError),
}

/// What a rebuild had to redo, and what it could not resolve.
#[derive(Debug, Clone, PartialEq)]
pub struct RebuildReport {
    /// The design revision the job was built against.
    pub previous: DesignRef,
    /// The design revision it is now built against.
    pub design: DesignRef,
    /// Operations whose toolpath must be regenerated.
    pub recomputed: Vec<String>,
    /// References or tools a rebuild could not resolve.
    pub issues: Vec<RebuildIssue>,
}

impl RebuildReport {
    /// Nothing was stale and nothing is missing.
    pub fn is_current(&self) -> bool {
        self.recomputed.is_empty() && self.issues.is_empty()
    }

    pub fn has_missing_references(&self) -> bool {
        self.issues.iter().any(|issue| matches!(issue, RebuildIssue::MissingReference { .. }))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RebuildIssue {
    /// The design no longer has the feature an operation machines.
    MissingReference { operation: String, reference: EntityRef },
    /// The hole changed diameter but the tool library has no cutter that size.
    NoMatchingTool { operation: String, diameter_mm: f64 },
}

/// A generated toolpath stored on the job, so one file carries the paths too.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredPath {
    operation: String,
    path: Toolpath,
}

impl StoredPath {
    pub fn operation(&self) -> &str {
        &self.operation
    }

    pub fn path(&self) -> &Toolpath {
        &self.path
    }
}

/// A read-only view of how a job relates to a design snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncReport {
    /// The design moved on since the job was built.
    pub stale: bool,
    /// References the design no longer has.
    pub missing: Vec<RebuildIssue>,
}

impl SyncReport {
    /// The job needs a rebuild or a decision from the machinist.
    pub fn needs_work(&self) -> bool {
        self.stale || !self.missing.is_empty()
    }
}

/// One machining job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CamJob {
    name: String,
    design: DesignRef,
    stock: Stock,
    setup: Setup,
    tools: ToolLibrary,
    program: Program,
    #[serde(default)]
    paths: Vec<StoredPath>,
}

impl CamJob {
    pub fn new(name: impl Into<String>, design: DesignRef, stock: Stock, setup: Setup) -> Result<Self, JobError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(JobError::EmptyName);
        }
        Ok(Self { name, design, stock, setup, tools: ToolLibrary::new(), program: Program::new(), paths: Vec::new() })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn design(&self) -> &DesignRef {
        &self.design
    }

    pub fn stock(&self) -> &Stock {
        &self.stock
    }

    pub fn setup(&self) -> &Setup {
        &self.setup
    }

    pub fn tools(&self) -> &ToolLibrary {
        &self.tools
    }

    pub fn program(&self) -> &Program {
        &self.program
    }

    /// Add a cutter to the job's library.
    pub fn add_tool(&mut self, tool: Tool) -> Result<(), JobError> {
        self.tools.add(tool)?;
        Ok(())
    }

    /// The generated toolpaths stored on the job.
    pub fn toolpaths(&self) -> &[StoredPath] {
        &self.paths
    }

    /// The toolpath generated for `operation`, if it is stored.
    pub fn toolpath(&self, operation: &str) -> Option<&Toolpath> {
        self.paths.iter().find(|entry| entry.operation == operation).map(StoredPath::path)
    }

    /// Store the toolpath generated for an operation, replacing an older one.
    pub fn add_toolpath(&mut self, operation: impl Into<String>, path: Toolpath) -> Result<(), JobError> {
        let operation = operation.into();
        if self.program.iter().all(|candidate| candidate.name() != operation) {
            return Err(JobError::UnknownOperation(operation));
        }
        self.paths.retain(|entry| entry.operation != operation);
        self.paths.push(StoredPath { operation, path });
        Ok(())
    }

    /// References the design no longer has, checked without changing the job.
    ///
    /// This is what a machinist sees when a face or hole was deleted or renamed: the job stays
    /// as it was, and every operation that pointed at the missing entity is named.
    pub fn missing_references(&self, snapshot: &DesignSnapshot) -> Vec<RebuildIssue> {
        let mut issues = Vec::new();
        for operation in self.program.iter().filter(|operation| !operation.is_suppressed()) {
            for reference in operation.targets() {
                if snapshot.get(reference).is_none() {
                    issues.push(RebuildIssue::MissingReference { operation: operation.name().to_string(), reference: reference.clone() });
                }
            }
        }
        issues
    }

    /// True when the design moved on since this job was built.
    pub fn is_invalidated_by(&self, design: &DesignRef) -> bool {
        &self.design != design
    }

    /// Check the job against a design snapshot without changing anything.
    pub fn sync(&self, snapshot: &DesignSnapshot) -> SyncReport {
        SyncReport { stale: self.is_invalidated_by(snapshot.design()), missing: self.missing_references(snapshot) }
    }

    /// Add an operation, refusing one that names a tool the library does not hold.
    pub fn add_operation(&mut self, operation: Operation) -> Result<(), JobError> {
        if self.tools.find(operation.tool()).is_none() {
            return Err(JobError::UnknownTool { operation: operation.name().to_string(), tool: operation.tool().to_string() });
        }
        self.program.push(operation)?;
        Ok(())
    }

    /// Re-resolve the job against a new design state.
    ///
    /// Drilling operations follow their hole's diameter: when the hole changes, the operation
    /// is pointed at the tool of that size. Every other referenced operation is marked for
    /// regeneration (the CAM layer does not generate paths yet). A reference the design no
    /// longer has is reported as [`RebuildIssue::MissingReference`].
    pub fn rebuild(&mut self, snapshot: &DesignSnapshot) -> Result<RebuildReport, JobError> {
        if !self.design.is_same_design(snapshot.design()) {
            return Err(JobError::ForeignDesign(self.design.design_id().to_string(), snapshot.design().design_id().to_string()));
        }
        let previous = self.design.clone();
        if &self.design == snapshot.design() {
            return Ok(RebuildReport { previous: previous.clone(), design: previous, recomputed: Vec::new(), issues: Vec::new() });
        }

        let mut recomputed = Vec::new();
        let mut issues = Vec::new();
        for operation in self.program.iter_mut() {
            if operation.is_suppressed() {
                continue;
            }
            let mut touched = false;
            let targets = operation.targets().to_vec();
            for reference in &targets {
                let Some(entity) = snapshot.get(reference) else {
                    issues.push(RebuildIssue::MissingReference { operation: operation.name().to_string(), reference: reference.clone() });
                    continue;
                };
                if let (OperationKind::Drill { .. }, Some(diameter)) = (operation.kind(), entity.diameter_mm()) {
                    let desired = diameter;
                    let candidate = self
                        .tools
                        .iter()
                        .find(|tool| matches!(tool.cutter(), Cutter::Drill { .. }) && (tool.diameter_mm() - desired).abs() < 1e-6)
                        .or_else(|| self.tools.iter().find(|tool| (tool.diameter_mm() - desired).abs() < 1e-6));
                    match candidate {
                        Some(tool) => {
                            if tool.name() != operation.tool() {
                                operation.retarget_tool(tool.name());
                            }
                            touched = true;
                        }
                        None => issues.push(RebuildIssue::NoMatchingTool { operation: operation.name().to_string(), diameter_mm: desired }),
                    }
                } else {
                    touched = true;
                }
            }
            if touched {
                recomputed.push(operation.name().to_string());
            }
        }

        self.design = snapshot.design().clone();
        Ok(RebuildReport { previous, design: self.design.clone(), recomputed, issues })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::Operation;
    use crate::reference::{Entity, EntityKind};
    use crate::stock::Material;

    fn design(revision: u64) -> DesignRef {
        DesignRef::new("bracket", revision).unwrap()
    }

    fn stock() -> Stock {
        Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap()
    }

    fn setup() -> Setup {
        Setup::top_centre(&stock())
    }

    fn drill(name: &str, diameter: f64) -> Tool {
        Tool::new(name, Cutter::Drill { point_angle_deg: 118.0 }, diameter, 2, 20.0, 30.0).unwrap()
    }

    fn job_with_drill() -> (CamJob, EntityRef) {
        let hole = EntityRef::new(EntityKind::Hole, "hole.42").unwrap();
        let mut job = CamJob::new("Setup 1", design(1), stock(), setup()).unwrap();
        job.add_tool(drill("5mm drill", 5.0)).unwrap();
        job.add_tool(drill("6mm drill", 6.0)).unwrap();
        job.add_tool(drill("8mm drill", 8.0)).unwrap();
        let operation =
            Operation::new("Drill 6 holes", OperationKind::Drill { peck_mm: Some(2.0) }, "6mm drill", 10.0).unwrap().with_targets(vec![hole.clone()]);
        job.add_operation(operation).unwrap();
        (job, hole)
    }

    fn snapshot(revision: u64, hole: &EntityRef, diameter: f64) -> DesignSnapshot {
        let mut snapshot = DesignSnapshot::new(design(revision));
        snapshot.insert(Entity::hole(hole.clone(), diameter));
        snapshot
    }

    #[test]
    fn a_job_needs_a_name_and_known_tools() {
        assert!(matches!(CamJob::new("  ", design(1), stock(), setup()), Err(JobError::EmptyName)));
        let mut job = CamJob::new("Setup 1", design(1), stock(), setup()).unwrap();
        let operation = Operation::new("Face", OperationKind::Face { stepover_mm: 5.0 }, "missing", 1.0).unwrap();
        assert_eq!(job.add_operation(operation), Err(JobError::UnknownTool { operation: "Face".into(), tool: "missing".into() }));
    }

    #[test]
    fn a_hole_growing_from_6_to_8mm_retargets_its_drill() {
        let (mut job, hole) = job_with_drill();
        let report = job.rebuild(&snapshot(2, &hole, 8.0)).unwrap();
        assert_eq!(report.previous.revision(), 1);
        assert_eq!(report.design.revision(), 2);
        assert_eq!(report.recomputed, ["Drill 6 holes"]);
        assert!(report.issues.is_empty());
        assert!(!report.is_current());
        assert_eq!(job.program().get(0).unwrap().tool(), "8mm drill");
        assert_eq!(job.design().revision(), 2);
    }

    #[test]
    fn an_unchanged_design_needs_no_work() {
        let (mut job, hole) = job_with_drill();
        let report = job.rebuild(&snapshot(1, &hole, 6.0)).unwrap();
        assert!(report.is_current());
        assert_eq!(job.program().get(0).unwrap().tool(), "6mm drill");
    }

    #[test]
    fn a_deleted_hole_is_reported_not_swallowed() {
        let (mut job, hole) = job_with_drill();
        let empty = DesignSnapshot::new(design(2));
        let report = job.rebuild(&empty).unwrap();
        assert!(report.has_missing_references());
        assert_eq!(report.issues, [RebuildIssue::MissingReference { operation: "Drill 6 holes".into(), reference: hole }]);
        assert!(report.recomputed.is_empty());
    }

    #[test]
    fn a_hole_size_without_a_tool_is_reported() {
        let (mut job, hole) = job_with_drill();
        let report = job.rebuild(&snapshot(2, &hole, 12.0)).unwrap();
        assert_eq!(report.issues, [RebuildIssue::NoMatchingTool { operation: "Drill 6 holes".into(), diameter_mm: 12.0 }]);
    }

    #[test]
    fn a_missing_face_is_named_without_changing_the_job() {
        let (job, hole) = job_with_drill();
        let empty = DesignSnapshot::new(design(2));
        let missing = job.missing_references(&empty);
        assert_eq!(missing, [RebuildIssue::MissingReference { operation: "Drill 6 holes".into(), reference: hole }]);
        // The check is read-only: the job still points at the old revision and tool.
        assert_eq!(job.design().revision(), 1);
        assert_eq!(job.program().get(0).unwrap().tool(), "6mm drill");
    }

    #[test]
    fn a_design_change_invalidates_the_job() {
        let (job, _) = job_with_drill();
        assert!(job.is_invalidated_by(&design(2)));
        assert!(!job.is_invalidated_by(&design(1)));
    }

    #[test]
    fn sync_detects_change_and_missing_references_without_mutating() {
        let (job, hole) = job_with_drill();
        let current = job.sync(&snapshot(1, &hole, 6.0));
        assert!(!current.stale);
        assert!(!current.needs_work());
        let changed = job.sync(&snapshot(2, &hole, 8.0));
        assert!(changed.stale);
        assert!(changed.missing.is_empty());
        assert!(changed.needs_work());
        let deleted = job.sync(&DesignSnapshot::new(design(2)));
        assert!(deleted.stale);
        assert!(deleted.needs_work());
        assert_eq!(job.design().revision(), 1);
    }

    #[test]
    fn a_job_cannot_be_rebuilt_against_another_design() {
        let (mut job, _) = job_with_drill();
        let mut other = DesignSnapshot::new(DesignRef::new("other", 2).unwrap());
        other.insert(Entity::hole(EntityRef::new(EntityKind::Hole, "hole.42").unwrap(), 6.0));
        assert_eq!(job.rebuild(&other), Err(JobError::ForeignDesign("bracket".into(), "other".into())));
    }
}

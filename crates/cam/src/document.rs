//! CAM documents: how a job is written down and read back.
//!
//! A job file carries the design reference, the stock, the setup, the tool library, the
//! operation list and any generated toolpaths — one file to open, back up and hand off. It
//! deliberately does **not** carry meshes, simulation results or images: those are derived
//! data and are regenerated, which keeps the file small even for a heavily machined part.
//!
//! The format is versioned. A file written by a newer build is refused with
//! [`DocumentError::UnsupportedVersion`] rather than silently misread.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::job::{CamJob, JobError, RebuildReport};
use crate::reference::{DesignRef, DesignSnapshot};

/// The CAM document format this build writes and reads.
pub const CAM_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct CamFile {
    format_version: u32,
    job: CamJob,
}

#[derive(Debug, Error)]
pub enum DocumentError {
    #[error("CAM file version {found} is not supported by this build (it reads version {expected})")]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("invalid CAM file: {0}")]
    Json(#[from] serde_json::Error),
    #[error("could not read or write the CAM file: {0}")]
    Io(#[from] std::io::Error),
}

fn check_version(file: CamFile) -> Result<CamJob, DocumentError> {
    if file.format_version != CAM_FORMAT_VERSION {
        return Err(DocumentError::UnsupportedVersion { found: file.format_version, expected: CAM_FORMAT_VERSION });
    }
    Ok(file.job)
}

/// The CAM section to embed in a design document under one key (for example `"cam"`).
///
/// Keeping the whole job in one namespaced object means the design file gains a single key
/// rather than a scatter of manufacturing fields, and a reader that does not care about CAM
/// can ignore it wholesale.
pub fn cam_section(job: &CamJob) -> Result<serde_json::Value, DocumentError> {
    Ok(serde_json::to_value(CamFile { format_version: CAM_FORMAT_VERSION, job: job.clone() })?)
}

/// Read a job back from a [`cam_section`] value.
pub fn job_from_cam_section(section: &serde_json::Value) -> Result<CamJob, DocumentError> {
    check_version(serde_json::from_value(section.clone())?)
}

/// Serialize one job as a self-contained CAM file.
pub fn job_to_json(job: &CamJob) -> Result<String, DocumentError> {
    Ok(serde_json::to_string_pretty(&CamFile { format_version: CAM_FORMAT_VERSION, job: job.clone() })?)
}

/// Read a job from a CAM file's text.
pub fn job_from_json(text: &str) -> Result<CamJob, DocumentError> {
    check_version(serde_json::from_str(text)?)
}

/// Write one job to `path`.
pub fn save_job(job: &CamJob, path: impl AsRef<Path>) -> Result<(), DocumentError> {
    fs::write(path, job_to_json(job)?)?;
    Ok(())
}

/// Read one job from `path`.
pub fn load_job(path: impl AsRef<Path>) -> Result<CamJob, DocumentError> {
    job_from_json(&fs::read_to_string(path)?)
}

/// A named set of machining jobs for one design: one per setup, machine or customer.
///
/// The set lives outside the CAD file, so machining plans never touch design history, and each
/// job can also be written to its own file with [`CamJobSet::save_job`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CamJobSet {
    design: DesignRef,
    jobs: Vec<CamJob>,
}

/// Why a job could not join a set.
#[derive(Debug, Error, PartialEq)]
pub enum SetError {
    #[error("a job named `{0}` already exists in this set")]
    DuplicateJob(String),
    #[error("job `{job}` is for design `{found}`, but the set is for `{expected}`")]
    ForeignDesign { job: String, expected: String, found: String },
}

/// What a refresh did to one job.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRefresh {
    pub job: String,
    pub report: RebuildReport,
}

#[derive(Debug, Serialize, Deserialize)]
struct CamJobSetFile {
    format_version: u32,
    set: CamJobSet,
}

impl CamJobSet {
    pub fn new(design: DesignRef) -> Self {
        Self { design, jobs: Vec::new() }
    }

    pub fn design(&self) -> &DesignRef {
        &self.design
    }

    pub fn jobs(&self) -> &[CamJob] {
        &self.jobs
    }

    pub fn job(&self, name: &str) -> Option<&CamJob> {
        self.jobs.iter().find(|job| job.name() == name)
    }

    pub fn job_mut(&mut self, name: &str) -> Option<&mut CamJob> {
        self.jobs.iter_mut().find(|job| job.name() == name)
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Add a job; its name must be new and its design must be the set's design.
    pub fn add_job(&mut self, job: CamJob) -> Result<(), SetError> {
        if self.job(job.name()).is_some() {
            return Err(SetError::DuplicateJob(job.name().to_string()));
        }
        if !self.design.is_same_design(job.design()) {
            return Err(SetError::ForeignDesign {
                job: job.name().to_string(),
                expected: self.design.design_id().to_string(),
                found: job.design().design_id().to_string(),
            });
        }
        self.jobs.push(job);
        Ok(())
    }

    /// Names of the jobs built against an older design revision.
    pub fn stale(&self, design: &DesignRef) -> Vec<&str> {
        self.jobs.iter().filter(|job| job.is_invalidated_by(design)).map(CamJob::name).collect()
    }

    /// Re-resolve every job against a new design state.
    pub fn refresh(&mut self, snapshot: &DesignSnapshot) -> Result<Vec<JobRefresh>, JobError> {
        if !self.design.is_same_design(snapshot.design()) {
            return Err(JobError::ForeignDesign(self.design.design_id().to_string(), snapshot.design().design_id().to_string()));
        }
        let mut out = Vec::with_capacity(self.jobs.len());
        for job in &mut self.jobs {
            let report = job.rebuild(snapshot)?;
            out.push(JobRefresh { job: job.name().to_string(), report });
        }
        self.design = snapshot.design().clone();
        Ok(out)
    }

    /// Write every job to its own `<job-name>.cam` file in `dir`, for archiving or emailing.
    pub fn archive_to_dir(&self, dir: impl AsRef<Path>) -> Result<Vec<PathBuf>, DocumentError> {
        let dir = dir.as_ref();
        let mut written = Vec::with_capacity(self.jobs.len());
        for job in &self.jobs {
            let path = dir.join(format!("{}.cam", safe_file_stem(job.name())));
            save_job(job, &path)?;
            written.push(path);
        }
        Ok(written)
    }

    /// Write this job of the set to its own CAM file.
    pub fn save_job(&self, name: &str, path: impl AsRef<Path>) -> Result<(), DocumentError> {
        let job = self
            .job(name)
            .ok_or_else(|| DocumentError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, format!("no job named `{name}` in this set"))))?;
        save_job(job, path)
    }
}

/// A file-system-safe stem for a job name: letters, digits, `.`, `_` and `-` survive, runs of
/// anything else become one `-`.
fn safe_file_stem(name: &str) -> String {
    let mut stem = String::with_capacity(name.len());
    let mut last_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
            stem.push(c);
            last_dash = false;
        } else if !last_dash {
            stem.push('-');
            last_dash = true;
        }
    }
    let trimmed = stem.trim_matches('-').to_string();
    if trimmed.is_empty() { "job".to_string() } else { trimmed }
}

/// Serialize a job set as one versioned file.
pub fn set_to_json(set: &CamJobSet) -> Result<String, DocumentError> {
    Ok(serde_json::to_string_pretty(&CamJobSetFile { format_version: CAM_FORMAT_VERSION, set: set.clone() })?)
}

/// Read a job set from text.
pub fn set_from_json(text: &str) -> Result<CamJobSet, DocumentError> {
    let file: CamJobSetFile = serde_json::from_str(text)?;
    if file.format_version != CAM_FORMAT_VERSION {
        return Err(DocumentError::UnsupportedVersion { found: file.format_version, expected: CAM_FORMAT_VERSION });
    }
    Ok(file.set)
}

pub fn save_set(set: &CamJobSet, path: impl AsRef<Path>) -> Result<(), DocumentError> {
    fs::write(path, set_to_json(set)?)?;
    Ok(())
}

pub fn load_set(path: impl AsRef<Path>) -> Result<CamJobSet, DocumentError> {
    set_from_json(&fs::read_to_string(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::CamJob;
    use crate::operation::{Operation, OperationKind};
    use crate::reference::{DesignRef, DesignSnapshot, Entity, EntityKind, EntityRef};
    use crate::setup::Setup;
    use crate::stock::{Material, Stock};
    use crate::toolpath::Heights;
    use crate::{Cutter, Tool, Toolpath};

    fn sample_job() -> CamJob {
        let stock = Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap();
        let setup = Setup::top_centre(&stock);
        let mut job = CamJob::new("Setup 1", DesignRef::new("bracket", 3).unwrap(), stock, setup).unwrap();
        job.add_tool(Tool::new("6mm flat", Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap()).unwrap();
        job.add_operation(Operation::new("Face top", OperationKind::Face { stepover_mm: 15.0 }, "6mm flat", 0.5).unwrap()).unwrap();
        let mut path = Toolpath::new([-20.0, 0.0, -1.0], Heights::default()).unwrap();
        path.feed([20.0, 0.0, -1.0], 300.0).unwrap();
        job.add_toolpath("Face top", path).unwrap();
        job
    }

    #[test]
    fn a_job_round_trips_with_its_toolpaths() {
        let original = sample_job();
        let text = job_to_json(&original).unwrap();
        assert!(text.contains("\"format_version\": 1"));
        let loaded = job_from_json(&text).unwrap();
        assert_eq!(loaded, original);
        assert_eq!(loaded.name(), "Setup 1");
        assert_eq!(loaded.design().revision(), 3);
        assert_eq!(loaded.tools().len(), 1);
        assert_eq!(loaded.program().len(), 1);
        assert_eq!(loaded.toolpaths().len(), 1);
        assert_eq!(loaded.toolpath("Face top").unwrap().moves().len(), 1);
    }

    #[test]
    fn the_file_carries_no_meshes_or_simulation_results() {
        let text = job_to_json(&sample_job()).unwrap();
        for derived in ["triangles", "vertices", "removed_volume", "mesh"] {
            assert!(!text.contains(derived), "job file unexpectedly contains `{derived}`");
        }
    }

    #[test]
    fn the_embedded_section_is_one_namespaced_key_and_round_trips() {
        let original = sample_job();
        let section = cam_section(&original).unwrap();
        let keys: Vec<_> = section.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["format_version", "job"]);
        let loaded = job_from_cam_section(&section).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn the_embedded_section_stays_small() {
        let section = cam_section(&sample_job()).unwrap();
        let text = serde_json::to_string(&section).unwrap();
        assert!(text.len() < 10_000, "CAM section is {} bytes", text.len());
    }

    #[test]
    fn a_newer_format_is_refused() {
        let text = job_to_json(&sample_job()).unwrap().replace("\"format_version\": 1", "\"format_version\": 99");
        assert!(matches!(job_from_json(&text), Err(DocumentError::UnsupportedVersion { found: 99, expected: 1 })));
    }

    #[test]
    fn a_job_saves_to_and_loads_from_a_file() {
        let path = std::env::temp_dir().join(format!("solidcraft-cam-{}.cam", std::process::id()));
        save_job(&sample_job(), &path).unwrap();
        let loaded = load_job(&path).unwrap();
        assert_eq!(loaded, sample_job());
        fs::remove_file(path).ok();
    }

    fn set_with_jobs() -> CamJobSet {
        let design = DesignRef::new("bracket", 1).unwrap();
        let mut set = CamJobSet::new(design.clone());
        for name in ["Setup 1 - vise", "Setup 2 - flipped", "Customer A - Haas"] {
            let mut job = CamJob::new(name, design.clone(), sample_job().stock().clone(), sample_job().setup().clone()).unwrap();
            job.add_tool(sample_job().tools().iter().next().unwrap().clone()).unwrap();
            set.add_job(job).unwrap();
        }
        set
    }

    #[test]
    fn one_design_can_have_several_machining_jobs() {
        let set = set_with_jobs();
        assert_eq!(set.len(), 3);
        assert!(set.job("Setup 1 - vise").is_some());
        assert!(set.job("Customer A - Haas").is_some());
        let text = set_to_json(&set).unwrap();
        let loaded = set_from_json(&text).unwrap();
        assert_eq!(loaded, set);
        assert_eq!(loaded.design().design_id(), "bracket");
    }

    #[test]
    fn a_set_rejects_duplicate_names_and_other_designs() {
        let mut set = set_with_jobs();
        let design = DesignRef::new("bracket", 1).unwrap();
        let duplicate = CamJob::new("Setup 1 - vise", design.clone(), sample_job().stock().clone(), sample_job().setup().clone()).unwrap();
        assert_eq!(set.add_job(duplicate), Err(SetError::DuplicateJob("Setup 1 - vise".into())));
        let foreign =
            CamJob::new("Other part", DesignRef::new("widget", 1).unwrap(), sample_job().stock().clone(), sample_job().setup().clone()).unwrap();
        assert_eq!(
            set.add_job(foreign),
            Err(SetError::ForeignDesign { job: "Other part".into(), expected: "bracket".into(), found: "widget".into() })
        );
    }

    #[test]
    fn the_set_file_holds_only_design_reference_and_jobs() {
        let text = set_to_json(&set_with_jobs()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let top: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(top, ["format_version", "set"]);
        let inside: Vec<_> = value["set"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(inside, ["design", "jobs"]);
    }

    #[test]
    fn each_job_can_be_written_and_read_on_its_own() {
        let set = set_with_jobs();
        let dir = std::env::temp_dir().join(format!("solidcraft-cam-set-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let files = set.archive_to_dir(&dir).unwrap();
        assert_eq!(files.len(), 3);
        for (file, job) in files.iter().zip(set.jobs()) {
            assert!(file.exists(), "missing {}", file.display());
            assert_eq!(&load_job(file).unwrap(), job);
        }
        assert_eq!(files[2].file_name().unwrap(), "Customer-A-Haas.cam");
        assert!(set.save_job("no such job", dir.join("nope.cam")).is_err());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn cam_edits_do_not_touch_the_design_history() {
        let set = set_with_jobs();
        let design = set.design().clone();
        let mut job = set.job("Setup 1 - vise").unwrap().clone();
        job.add_operation(Operation::new("Pocket", OperationKind::Pocket { stepover_mm: 2.0 }, "6mm flat", 5.0).unwrap()).unwrap();
        assert_eq!(job.design(), &design);
        assert_eq!(set.design(), &design);
        assert_eq!(job.design().revision(), 1);
        let text = set_to_json(&set).unwrap();
        assert!(!text.contains("\"bodies\""), "the CAM file must not carry design geometry");
        assert!(!text.contains("\"features\""));
    }

    fn drill_job(design: &DesignRef, name: &str, hole: &EntityRef) -> CamJob {
        let mut job = CamJob::new(name, design.clone(), sample_job().stock().clone(), sample_job().setup().clone()).unwrap();
        for diameter in [5.0, 6.0, 8.0] {
            job.add_tool(Tool::new(format!("{diameter}mm drill"), Cutter::Drill { point_angle_deg: 118.0 }, diameter, 2, 20.0, 30.0).unwrap())
                .unwrap();
        }
        let operation =
            Operation::new("Drill 6 holes", OperationKind::Drill { peck_mm: Some(2.0) }, "6mm drill", 10.0).unwrap().with_targets(vec![hole.clone()]);
        job.add_operation(operation).unwrap();
        job
    }

    fn set_with_drill() -> (CamJobSet, EntityRef) {
        let design = DesignRef::new("bracket", 1).unwrap();
        let hole = EntityRef::new(EntityKind::Hole, "hole.42").unwrap();
        let mut set = CamJobSet::new(design.clone());
        set.add_job(drill_job(&design, "Setup 1", &hole)).unwrap();
        set.add_job(drill_job(&design, "Setup 2", &hole)).unwrap();
        (set, hole)
    }

    #[test]
    fn a_design_edit_marks_jobs_stale_and_refresh_regenerates_them() {
        let (mut set, hole) = set_with_drill();
        let mut snapshot = DesignSnapshot::new(DesignRef::new("bracket", 2).unwrap());
        snapshot.insert(Entity::hole(hole.clone(), 8.0));
        assert_eq!(set.stale(snapshot.design()), ["Setup 1", "Setup 2"]);

        let reports = set.refresh(&snapshot).unwrap();
        assert_eq!(reports.len(), 2);
        for refresh in &reports {
            assert_eq!(refresh.report.recomputed, ["Drill 6 holes"]);
            assert!(refresh.report.issues.is_empty());
        }
        assert_eq!(set.job("Setup 1").unwrap().program().get(0).unwrap().tool(), "8mm drill");
        assert_eq!(set.job("Setup 2").unwrap().program().get(0).unwrap().tool(), "8mm drill");
        assert_eq!(set.design().revision(), 2);
        assert!(set.stale(snapshot.design()).is_empty());
    }

    #[test]
    fn refresh_reports_a_deleted_hole_and_leaves_the_job_usable() {
        let (mut set, hole) = set_with_drill();
        let empty = DesignSnapshot::new(DesignRef::new("bracket", 2).unwrap());
        let reports = set.refresh(&empty).unwrap();
        assert!(reports.iter().all(|refresh| refresh.report.has_missing_references()));
        assert_eq!(reports[0].report.issues, [crate::job::RebuildIssue::MissingReference { operation: "Drill 6 holes".into(), reference: hole }]);
        // The job still names its old tool; the machinist decides what to do.
        assert_eq!(set.job("Setup 1").unwrap().program().get(0).unwrap().tool(), "6mm drill");
    }

    #[test]
    fn refresh_refuses_a_snapshot_of_another_design() {
        let (mut set, _) = set_with_drill();
        let other = DesignSnapshot::new(DesignRef::new("widget", 2).unwrap());
        assert!(matches!(set.refresh(&other), Err(JobError::ForeignDesign(_, _))));
    }

    #[test]
    fn an_up_to_date_set_needs_no_regeneration() {
        let (mut set, hole) = set_with_drill();
        let mut snapshot = DesignSnapshot::new(DesignRef::new("bracket", 1).unwrap());
        snapshot.insert(Entity::hole(hole, 6.0));
        assert!(set.stale(snapshot.design()).is_empty());
        let reports = set.refresh(&snapshot).unwrap();
        assert!(reports.iter().all(|refresh| refresh.report.is_current()));
    }
}

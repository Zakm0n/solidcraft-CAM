//! Hybrid CAM storage: operations live with the design, generated work lives beside it.
//!
//! A [`CamPlan`] is the part of a job that belongs in the design document — stock, setup, tool
//! library, operation list and the design references that tie operations to features. It is
//! small and parameter-driven, so a model edit can rebuild it.
//!
//! Anything expensive and derived — generated toolpaths, simulation summaries, cached meshes —
//! goes in a sidecar [`PathCache`], keyed by a fingerprint of the operation and its inputs.
//! Editing a hole invalidates that hole's paths and leaves the rest alone; changing stock or a
//! tool invalidates everything that depends on it. Nothing in the sidecar can change the model.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::catalog::ToolCatalog;
use crate::document::CAM_FORMAT_VERSION;
use crate::geometry::{GeometryIssue, GeometryItem, GeometryStore};
use crate::job::{CamJob, JobError, RebuildReport};
use crate::operation::{Operation, Program};
use crate::post::GRBL_POST_VERSION;
use crate::reference::{DesignRef, DesignSnapshot};
use crate::setup::Setup;
use crate::stock::Stock;
use crate::tool::{Tool, ToolLibrary};
use crate::tooling::{Holder, ToolAssembly};
use crate::toolpath::Toolpath;

#[derive(Debug, Error)]
pub enum HybridError {
    #[error(transparent)]
    Job(#[from] JobError),
    #[error("invalid CAM data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("CAM data version {found} is not supported (this build reads {expected})")]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("no operation named `{0}` in this plan")]
    UnknownOperation(String),
    #[error("the tool `{0}` is not in this plan")]
    UnknownTool(String),
    #[error("operation `{0}` has no cached toolpath to attach a result to")]
    NoCachedToolpath(String),
    #[error("could not read or write the CAM data: {0}")]
    Io(#[from] std::io::Error),
}

/// The part of a machining job that belongs in the design document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CamPlan {
    design: DesignRef,
    stock: Stock,
    setup: Setup,
    tools: ToolLibrary,
    program: Program,
    /// Layers of machining geometry: owned wireframe and links to design edges/faces.
    #[serde(default)]
    geometry: GeometryStore,
    /// Reusable tools, holders and assemblies for this plan.
    #[serde(default)]
    catalog: ToolCatalog,
}

impl CamPlan {
    pub fn new(design: DesignRef, stock: Stock, setup: Setup) -> Self {
        Self { design, stock, setup, tools: ToolLibrary::new(), program: Program::new(), geometry: GeometryStore::new(), catalog: ToolCatalog::new() }
    }

    /// The plan of an existing job, dropping any generated toolpaths it carries.
    pub fn from_job(job: &CamJob) -> Self {
        Self {
            design: job.design().clone(),
            stock: job.stock().clone(),
            setup: job.setup().clone(),
            tools: job.tools().clone(),
            program: job.program().clone(),
            // A job carries no geometry store; a plan built from one starts empty.
            geometry: GeometryStore::new(),
            catalog: ToolCatalog::new(),
        }
    }

    /// A full job built from the plan, with no generated paths.
    pub fn to_job(&self) -> Result<CamJob, JobError> {
        let mut job = CamJob::new("CAM plan", self.design.clone(), self.stock.clone(), self.setup.clone())?;
        for tool in self.tools.iter() {
            job.add_tool(tool.clone())?;
        }
        for operation in self.program.iter() {
            job.add_operation(operation.clone())?;
        }
        Ok(job)
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

    /// One operation, mutably, for an edit in the CAM project.
    pub fn operation_mut(&mut self, name: &str) -> Option<&mut Operation> {
        self.program.iter_mut().find(|operation| operation.name() == name)
    }

    /// The plan's machining geometry, layer by layer.
    pub fn geometry(&self) -> &GeometryStore {
        &self.geometry
    }

    pub fn geometry_mut(&mut self) -> &mut GeometryStore {
        &mut self.geometry
    }

    /// The reusable tool/holder/assembly catalog.
    pub fn catalog(&self) -> &ToolCatalog {
        &self.catalog
    }

    pub fn catalog_mut(&mut self) -> &mut ToolCatalog {
        &mut self.catalog
    }

    /// Resolve an operation's tool name: an assembly first, then a catalog tool, then the
    /// legacy tool library.
    pub fn resolve_tool(&self, name: &str) -> Option<ResolvedTool> {
        if let Some(assembly) = self.catalog.assembly_by_name(name) {
            let tool = self.catalog.tool(assembly.tool_id().as_str())?;
            let effective = assembly.effective_tool(tool).ok()?;
            let holder = assembly.holder_id().and_then(|id| self.catalog.holder(id)).cloned();
            return Some(ResolvedTool { tool: effective, holder, assembly: Some(assembly.clone()) });
        }
        if let Some(tool) = self.catalog.tool_by_name(name) {
            return Some(ResolvedTool { tool: tool.clone(), holder: None, assembly: None });
        }
        self.tools.find(name).cloned().map(|tool| ResolvedTool { tool, holder: None, assembly: None })
    }

    /// Refresh design-linked geometry; missing references are reported by name.
    pub fn refresh_geometry(&mut self, resolve: impl Fn(&crate::reference::EntityRef) -> Option<crate::geometry::Chain>) -> Vec<GeometryIssue> {
        self.geometry.refresh_linked(resolve)
    }

    /// Add a cutter to the plan's library.
    pub fn add_tool(&mut self, tool: Tool) -> Result<(), JobError> {
        self.tools.add(tool)?;
        Ok(())
    }

    /// Add an operation, refusing one that names a tool or assembly the plan does not hold.
    pub fn add_operation(&mut self, operation: Operation) -> Result<(), JobError> {
        if self.tools.find(operation.tool()).is_none() && self.resolve_tool(operation.tool()).is_none() {
            return Err(JobError::UnknownTool { operation: operation.name().to_string(), tool: operation.tool().to_string() });
        }
        self.program.push(operation)?;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PlanFile {
    format_version: u32,
    plan: CamPlan,
}

/// The plan as the value to store under one key in a design document.
pub fn plan_section(plan: &CamPlan) -> Result<serde_json::Value, HybridError> {
    Ok(serde_json::to_value(PlanFile { format_version: CAM_FORMAT_VERSION, plan: plan.clone() })?)
}

/// Read a plan back from a design document section.
pub fn plan_from_section(section: &serde_json::Value) -> Result<CamPlan, HybridError> {
    let file: PlanFile = serde_json::from_value(section.clone())?;
    if file.format_version != CAM_FORMAT_VERSION {
        return Err(HybridError::UnsupportedVersion { found: file.format_version, expected: CAM_FORMAT_VERSION });
    }
    Ok(file.plan)
}

/// The sidecar cache format this build writes and reads.
pub const CAM_CACHE_VERSION: u32 = 1;

/// A simulation result kept beside the toolpath it belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulationSummary {
    pub removed_volume_mm3: f64,
    pub remaining_volume_mm3: f64,
    pub collisions: usize,
    /// Name of a cached mesh written beside the cache file, if one was kept.
    #[serde(default)]
    pub mesh_file: Option<String>,
}

/// Posted G-code cached beside its toolpath, with the post format version that produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredGcode {
    pub post_version: u32,
    pub text: String,
    #[serde(default)]
    pub simulation: Option<SimulationSummary>,
}

impl StoredGcode {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn simulation(&self) -> Option<&SimulationSummary> {
        self.simulation.as_ref()
    }
}

/// One cached result: a toolpath, its simulation summary and posted G-code, valid for one
/// fingerprint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheEntry {
    operation: String,
    design_revision: u64,
    plan_key: u64,
    path: Toolpath,
    #[serde(default)]
    simulation: Option<SimulationSummary>,
    #[serde(default)]
    gcode: Option<StoredGcode>,
}

impl CacheEntry {
    pub fn operation(&self) -> &str {
        &self.operation
    }

    /// The design revision the entry was produced against (diagnostic only; validity is the key).
    pub fn design_revision(&self) -> u64 {
        self.design_revision
    }

    pub fn plan_key(&self) -> u64 {
        self.plan_key
    }

    pub fn path(&self) -> &Toolpath {
        &self.path
    }

    pub fn simulation(&self) -> Option<&SimulationSummary> {
        self.simulation.as_ref()
    }
}

/// Derived work for a plan, stored beside the design.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PathCache {
    #[serde(default)]
    entries: Vec<CacheEntry>,
}

impl PathCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> &[CacheEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn find(&self, operation: &str, key: u64) -> Option<&CacheEntry> {
        self.entries.iter().find(|entry| entry.operation == operation && entry.plan_key == key)
    }

    fn find_mut(&mut self, operation: &str, key: u64) -> Option<&mut CacheEntry> {
        self.entries.iter_mut().find(|entry| entry.operation == operation && entry.plan_key == key)
    }

    fn put(&mut self, entry: CacheEntry) {
        self.entries.retain(|existing| existing.operation != entry.operation);
        self.entries.push(entry);
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    format_version: u32,
    cache: PathCache,
}

pub fn cache_to_json(cache: &PathCache) -> Result<String, HybridError> {
    Ok(serde_json::to_string_pretty(&CacheFile { format_version: CAM_CACHE_VERSION, cache: cache.clone() })?)
}

pub fn cache_from_json(text: &str) -> Result<PathCache, HybridError> {
    let file: CacheFile = serde_json::from_str(text)?;
    if file.format_version != CAM_CACHE_VERSION {
        return Err(HybridError::UnsupportedVersion { found: file.format_version, expected: CAM_CACHE_VERSION });
    }
    Ok(file.cache)
}

pub fn save_cache(cache: &PathCache, path: impl AsRef<Path>) -> Result<(), HybridError> {
    fs::write(path, cache_to_json(cache)?)?;
    Ok(())
}

pub fn load_cache(path: impl AsRef<Path>) -> Result<PathCache, HybridError> {
    cache_from_json(&fs::read_to_string(path)?)
}

/// An operation's tool resolved from an assembly, catalog tool or legacy library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTool {
    pub tool: Tool,
    pub holder: Option<Holder>,
    pub assembly: Option<ToolAssembly>,
}

/// A plan with its sidecar cache: the hybrid store.
#[derive(Debug, Clone, PartialEq)]
pub struct HybridCam {
    plan: CamPlan,
    cache: PathCache,
}

impl HybridCam {
    pub fn new(plan: CamPlan) -> Self {
        Self { plan, cache: PathCache::new() }
    }

    pub fn plan(&self) -> &CamPlan {
        &self.plan
    }

    pub fn cache(&self) -> &PathCache {
        &self.cache
    }

    /// Replace the plan (an edit in the design document).
    pub fn set_plan(&mut self, plan: CamPlan) {
        self.plan = plan;
    }

    /// The `cam` section to write into the design document.
    pub fn to_design_section(&self) -> Result<serde_json::Value, HybridError> {
        plan_section(&self.plan)
    }

    /// Read the plan from a design document; the cache starts empty.
    pub fn from_design_section(section: &serde_json::Value) -> Result<Self, HybridError> {
        Ok(Self::new(plan_from_section(section)?))
    }

    /// The cached toolpath for an operation, if it still matches the plan.
    pub fn toolpath(&self, operation: &str) -> Option<&Toolpath> {
        let key = plan_key_for(&self.plan, operation).ok()?;
        self.cache.find(operation, key).map(CacheEntry::path)
    }

    /// The cached simulation summary for an operation, if it still matches the plan.
    pub fn simulation(&self, operation: &str) -> Option<&SimulationSummary> {
        let key = plan_key_for(&self.plan, operation).ok()?;
        self.cache.find(operation, key).and_then(CacheEntry::simulation)
    }

    /// Cache a generated toolpath against the current plan.
    pub fn store_toolpath(&mut self, operation: &str, path: Toolpath) -> Result<(), HybridError> {
        let key = plan_key_for(&self.plan, operation)?;
        self.cache.put(CacheEntry {
            operation: operation.to_string(),
            design_revision: self.plan.design().revision(),
            plan_key: key,
            path,
            simulation: None,
            gcode: None,
        });
        Ok(())
    }

    /// Attach a simulation result to an already cached toolpath.
    pub fn store_simulation(&mut self, operation: &str, summary: SimulationSummary) -> Result<(), HybridError> {
        let key = plan_key_for(&self.plan, operation)?;
        let entry = self.cache.find_mut(operation, key).ok_or_else(|| HybridError::NoCachedToolpath(operation.to_string()))?;
        entry.simulation = Some(summary);
        Ok(())
    }

    /// Cache the posted G-code and its simulation for an operation.
    ///
    /// The toolpath must already be cached; the G-code is keyed by the same plan fingerprint
    /// and stamped with the post format version.
    pub fn store_gcode(&mut self, operation: &str, text: impl Into<String>, simulation: Option<SimulationSummary>) -> Result<(), HybridError> {
        let key = plan_key_for(&self.plan, operation)?;
        let entry = self.cache.find_mut(operation, key).ok_or_else(|| HybridError::NoCachedToolpath(operation.to_string()))?;
        entry.gcode = Some(StoredGcode { post_version: GRBL_POST_VERSION, text: text.into(), simulation });
        Ok(())
    }

    /// Cached G-code for an operation, if it matches the plan and post version.
    pub fn gcode(&self, operation: &str) -> Option<&StoredGcode> {
        let key = plan_key_for(&self.plan, operation).ok()?;
        self.cache.find(operation, key).and_then(|entry| entry.gcode.as_ref()).filter(|gcode| gcode.post_version == GRBL_POST_VERSION)
    }

    /// Cached post-gcode simulation summary for an operation.
    pub fn gcode_simulation(&self, operation: &str) -> Option<&SimulationSummary> {
        self.gcode(operation).and_then(StoredGcode::simulation)
    }

    /// Rebuild the plan against a new design state and drop the paths that changed.
    ///
    /// Operations the rebuild did not touch keep their cached toolpaths: their fingerprint
    /// still matches, so no work is thrown away beyond what the edit invalidated.
    pub fn rebuild(&mut self, snapshot: &DesignSnapshot) -> Result<RebuildReport, JobError> {
        let mut job = self.plan.to_job()?;
        let report = job.rebuild(snapshot)?;
        self.plan = CamPlan::from_job(&job);
        self.cache.entries.retain(|entry| !report.recomputed.contains(&entry.operation));
        Ok(report)
    }

    /// Drop cache entries whose fingerprint no longer matches the plan; returns how many.
    pub fn prune(&mut self) -> usize {
        let before = self.cache.entries.len();
        self.cache.entries.retain(|entry| plan_key_for(&self.plan, &entry.operation).map(|key| key == entry.plan_key).unwrap_or(false));
        before - self.cache.entries.len()
    }

    /// How many operations have a usable cached toolpath.
    pub fn cached_count(&self) -> usize {
        self.plan.program().iter().filter(|operation| self.toolpath(operation.name()).is_some()).count()
    }

    pub fn save_cache(&self, path: impl AsRef<Path>) -> Result<(), HybridError> {
        save_cache(&self.cache, path)
    }

    pub fn load_cache(&mut self, path: impl AsRef<Path>) -> Result<(), HybridError> {
        self.cache = load_cache(path)?;
        Ok(())
    }
}

fn plan_key_for(plan: &CamPlan, operation: &str) -> Result<u64, HybridError> {
    let operation =
        plan.program().iter().find(|candidate| candidate.name() == operation).ok_or_else(|| HybridError::UnknownOperation(operation.to_string()))?;
    let resolved = plan.resolve_tool(operation.tool()).ok_or_else(|| HybridError::UnknownTool(operation.tool().to_string()))?;
    let referenced: Vec<&GeometryItem> = operation.selection().iter().filter_map(|id| plan.geometry().get(*id)).collect();
    #[derive(Serialize)]
    struct KeyInput<'a> {
        stock: &'a Stock,
        setup: &'a Setup,
        tool: &'a Tool,
        operation: &'a Operation,
        geometry: Vec<&'a GeometryItem>,
        assembly: Option<&'a ToolAssembly>,
        holder: Option<&'a Holder>,
    }
    Ok(fingerprint(&KeyInput {
        stock: plan.stock(),
        setup: plan.setup(),
        tool: &resolved.tool,
        operation,
        geometry: referenced,
        assembly: resolved.assembly.as_ref(),
        holder: resolved.holder.as_ref(),
    }))
}

/// FNV-1a over the JSON of the inputs: stable for a build, no dependency, and good enough to
/// decide whether cached work still applies.
fn fingerprint<T: Serialize>(value: &T) -> u64 {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::OperationKind;
    use crate::stock::Material;
    use crate::tool::Cutter;

    pub(crate) fn design(revision: u64) -> DesignRef {
        DesignRef::new("bracket", revision).unwrap()
    }

    pub(crate) fn stock() -> Stock {
        Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap()
    }

    fn setup() -> Setup {
        Setup::top_centre(&stock())
    }

    pub(crate) fn plan() -> CamPlan {
        let mut plan = CamPlan::new(design(1), stock(), setup());
        plan.add_tool(crate::tool::Tool::new("6mm flat", Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap()).unwrap();
        plan.add_operation(Operation::new("Face top", OperationKind::Face { stepover_mm: 15.0 }, "6mm flat", 0.5).unwrap()).unwrap();
        plan
    }

    #[test]
    fn the_plan_section_carries_operations_but_no_generated_work() {
        let section = plan_section(&plan()).unwrap();
        let keys: Vec<_> = section.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["format_version", "plan"]);
        let text = serde_json::to_string(&section).unwrap();
        for derived in ["\"moves\"", "\"toolpath\"", "\"removed_volume\"", "\"triangles\""] {
            assert!(!text.contains(derived), "plan section unexpectedly contains `{derived}`");
        }
        assert!(text.contains("Face top"));
    }

    #[test]
    fn geometry_layers_and_links_round_trip_with_the_plan() {
        use crate::geometry::{Curve2D, GeometryShape};
        use crate::reference::{EntityKind, EntityRef};
        let mut plan = plan();
        {
            let geometry = plan.geometry_mut();
            geometry.add_layer("Machining").unwrap();
            geometry.add("Machining", GeometryShape::Curve(Curve2D::line([0.0, 0.0], [10.0, 0.0]).unwrap())).unwrap();
            geometry
                .add_linked(
                    "Machining",
                    GeometryShape::Curve(Curve2D::line([0.0, 5.0], [10.0, 5.0]).unwrap()),
                    EntityRef::new(EntityKind::Edge, "edge.9").unwrap(),
                )
                .unwrap();
        }
        let section = plan_section(&plan).unwrap();
        let loaded = plan_from_section(&section).unwrap();
        assert_eq!(loaded.geometry().layers().len(), 1);
        assert_eq!(loaded.geometry().len(), 2);
        assert_eq!(loaded.geometry().linked_count(), 1);
        assert_eq!(loaded, plan);
    }

    #[test]
    fn the_plan_section_round_trips() {
        let original = plan();
        let loaded = plan_from_section(&plan_section(&original).unwrap()).unwrap();
        assert_eq!(loaded, original);
        assert_eq!(loaded.design().revision(), 1);
        assert_eq!(loaded.tools().len(), 1);
        assert_eq!(loaded.program().len(), 1);
    }

    #[test]
    fn a_newer_plan_section_is_refused() {
        let mut section = plan_section(&plan()).unwrap();
        section["format_version"] = serde_json::json!(99);
        assert!(matches!(plan_from_section(&section), Err(HybridError::UnsupportedVersion { found: 99, expected: 1 })));
    }

    #[test]
    fn a_plan_converts_to_a_job_and_back() {
        let original = plan();
        let job = original.to_job().unwrap();
        assert_eq!(job.design(), original.design());
        assert_eq!(job.toolpaths().len(), 0);
        assert_eq!(CamPlan::from_job(&job), original);
    }

    fn path() -> Toolpath {
        let mut path = Toolpath::new([0.0, 0.0, -1.0], crate::toolpath::Heights::default()).unwrap();
        path.feed([10.0, 0.0, -1.0], 300.0).unwrap();
        path
    }

    fn hybrid() -> (HybridCam, crate::reference::EntityRef) {
        use crate::reference::{EntityKind, EntityRef};
        let hole = EntityRef::new(EntityKind::Hole, "hole.42").unwrap();
        let mut plan = CamPlan::new(design(1), stock(), setup());
        plan.add_tool(Tool::new("6mm flat", Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap()).unwrap();
        for diameter in [6.0, 8.0] {
            plan.add_tool(Tool::new(format!("{diameter}mm drill"), Cutter::Drill { point_angle_deg: 118.0 }, diameter, 2, 20.0, 30.0).unwrap())
                .unwrap();
        }
        plan.add_operation(Operation::new("Face top", OperationKind::Face { stepover_mm: 15.0 }, "6mm flat", 0.5).unwrap()).unwrap();
        plan.add_operation(
            Operation::new("Drill holes", OperationKind::Drill { peck_mm: Some(2.0) }, "6mm drill", 10.0).unwrap().with_targets(vec![hole.clone()]),
        )
        .unwrap();
        let mut hybrid = HybridCam::new(plan);
        hybrid.store_toolpath("Face top", path()).unwrap();
        hybrid.store_toolpath("Drill holes", path()).unwrap();
        (hybrid, hole)
    }

    #[test]
    fn toolpaths_and_simulation_round_trip_through_the_cache() {
        let (mut hybrid, _) = hybrid();
        hybrid
            .store_simulation(
                "Face top",
                SimulationSummary {
                    removed_volume_mm3: 120.0,
                    remaining_volume_mm3: 47_880.0,
                    collisions: 0,
                    mesh_file: Some("bracket.mesh.stl".into()),
                },
            )
            .unwrap();
        let dir = std::env::temp_dir().join(format!("solidcraft-hybrid-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bracket.cam-cache.json");
        hybrid.save_cache(&file).unwrap();

        let mut loaded = HybridCam::from_design_section(&hybrid.to_design_section().unwrap()).unwrap();
        loaded.load_cache(&file).unwrap();
        assert_eq!(loaded.toolpath("Face top"), Some(&path()));
        assert_eq!(loaded.simulation("Face top").unwrap().removed_volume_mm3, 120.0);
        assert_eq!(loaded.toolpath("Drill holes"), Some(&path()));
        assert_eq!(loaded.cached_count(), 2);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_design_edit_invalidates_only_the_affected_operation() {
        let (mut hybrid, hole) = hybrid();
        let mut snapshot = DesignSnapshot::new(design(2));
        snapshot.insert(crate::reference::Entity::hole(hole, 8.0));
        let report = hybrid.rebuild(&snapshot).unwrap();
        assert_eq!(report.recomputed, ["Drill holes"]);
        assert_eq!(hybrid.plan().design().revision(), 2);
        assert_eq!(hybrid.plan().program().iter().find(|op| op.name() == "Drill holes").unwrap().tool(), "8mm drill");
        assert!(hybrid.toolpath("Drill holes").is_none(), "changed operation must lose its cache");
        assert!(hybrid.toolpath("Face top").is_some(), "untouched operation must keep its cache");
        assert_eq!(hybrid.cached_count(), 1);
    }

    #[test]
    fn editing_an_operation_invalidates_its_cached_path() {
        let (mut hybrid, _) = hybrid();
        let mut edited = CamPlan::new(design(1), stock(), setup());
        for tool in hybrid.plan().tools().iter() {
            edited.add_tool(tool.clone()).unwrap();
        }
        for operation in hybrid.plan().program().iter() {
            let operation = if operation.name() == "Face top" {
                Operation::new("Face top", OperationKind::Face { stepover_mm: 10.0 }, "6mm flat", 0.5).unwrap()
            } else {
                operation.clone()
            };
            edited.add_operation(operation).unwrap();
        }
        hybrid.set_plan(edited);
        assert!(hybrid.toolpath("Face top").is_none());
        assert!(hybrid.toolpath("Drill holes").is_some());
        assert_eq!(hybrid.prune(), 1);
        assert_eq!(hybrid.cached_count(), 1);
    }

    #[test]
    fn posted_gcode_and_its_simulation_round_trip_with_the_cache() {
        let (mut hybrid, _) = hybrid();
        let summary =
            SimulationSummary { removed_volume_mm3: 42.0, remaining_volume_mm3: 47_958.0, collisions: 0, mesh_file: Some("face.stl".into()) };
        hybrid.store_gcode("Face top", "G21\nG0 X0\nM30", Some(summary.clone())).unwrap();
        assert_eq!(hybrid.gcode("Face top").unwrap().text(), "G21\nG0 X0\nM30");
        assert_eq!(hybrid.gcode_simulation("Face top"), Some(&summary));

        let dir = std::env::temp_dir().join(format!("solidcraft-gcode-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bracket.cam-cache.json");
        hybrid.save_cache(&file).unwrap();
        let mut loaded = HybridCam::from_design_section(&hybrid.to_design_section().unwrap()).unwrap();
        loaded.load_cache(&file).unwrap();
        assert_eq!(loaded.gcode("Face top").unwrap().text(), "G21\nG0 X0\nM30");
        assert_eq!(loaded.gcode_simulation("Face top"), Some(&summary));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn editing_the_plan_drops_cached_gcode() {
        let (mut hybrid, _) = hybrid();
        hybrid.store_gcode("Face top", "G21\nM30", None).unwrap();
        let mut edited = hybrid.plan().clone();
        edited.operation_mut("Face top").unwrap().set_depth(1.0).unwrap();
        hybrid.set_plan(edited);
        assert!(hybrid.gcode("Face top").is_none());
    }

    #[test]
    fn gcode_from_another_post_version_is_ignored() {
        let (mut hybrid, _) = hybrid();
        hybrid.store_gcode("Face top", "G21\nM30", None).unwrap();
        let dir = std::env::temp_dir().join(format!("solidcraft-gcode-ver-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cache.json");
        hybrid.save_cache(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap().replace("\"post_version\": 1", "\"post_version\": 99");
        fs::write(&file, text).unwrap();
        let mut loaded = HybridCam::from_design_section(&hybrid.to_design_section().unwrap()).unwrap();
        loaded.load_cache(&file).unwrap();
        assert!(loaded.gcode("Face top").is_none());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_result_needs_a_cached_toolpath_to_attach_to() {
        let (mut hybrid, _) = hybrid();
        let summary = SimulationSummary { removed_volume_mm3: 1.0, remaining_volume_mm3: 1.0, collisions: 0, mesh_file: None };
        assert!(matches!(hybrid.store_simulation("Missing op", summary.clone()), Err(HybridError::UnknownOperation(_))));
        let _ = summary;
    }

    #[test]
    fn a_newer_cache_format_is_refused() {
        let (hybrid, _) = hybrid();
        let text = cache_to_json(hybrid.cache()).unwrap().replace("\"format_version\": 1", "\"format_version\": 99");
        assert!(matches!(cache_from_json(&text), Err(HybridError::UnsupportedVersion { found: 99, expected: 1 })));
    }

    #[test]
    fn the_design_section_stays_free_of_cached_work() {
        let (hybrid, _) = hybrid();
        let text = serde_json::to_string(&hybrid.to_design_section().unwrap()).unwrap();
        assert!(!text.contains("moves"));
        assert!(!text.contains("plan_key"));
        assert!(!text.contains("removed_volume"));
    }
}

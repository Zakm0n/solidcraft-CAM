//! CAM for SolidCraft.
//!
//! The crate is deliberately split from the CAD kernel: it describes a machining *job*
//! (stock, setup, tools, operations, toolpaths) and hands the cutting to
//! [`cutrail`](https://crates.io/crates/cutrail), a mesh-based material-removal engine.
//! SolidCraft's command registry can call into this crate later; nothing here depends on
//! the UI or on a particular storage model, so the job can live in the design document or
//! in a separate CAM file without changing these types.

pub mod catalog;
pub mod document;
pub mod engagement;
pub mod gcode;
pub mod generate;
pub mod geometry;
pub mod hybrid;
pub mod job;
pub mod mastercam;
pub mod offset;
pub mod operation;
pub mod ops_extra;
pub mod post;
pub mod reference;
pub mod routing;
pub mod setup;
pub mod simulate;
pub mod stock;
pub mod surface3d;
pub mod tool;
pub mod tooling;
pub mod toolpath;

pub use catalog::{CAM_CATALOG_VERSION, CatalogError, ToolCatalog};
pub use document::{
    CAM_FORMAT_VERSION, CamJobSet, DocumentError, JobRefresh, SetError, cam_section, job_from_cam_section, job_from_json, job_to_json, load_job,
    load_set, save_job, save_set, set_from_json, set_to_json,
};
pub use engagement::{
    EngagementError, EngagementParams, dynamic_contour, engagement_angle_deg, minimum_radius_from_percent, stepover_for_angle, trochoidal,
};
pub use gcode::{
    GcodeComparison, GcodeComparisonIssue, GcodeIssue, GcodeMove, GcodeSimulationReport, OperationStats, ParsedGcode, compare_gcode,
    compare_gcode_with_design, parse_gcode, simulate_gcode, simulate_gcode_with_tool,
};
pub use generate::{GenerateError, generate, generate_named, wcs_bounds};
pub use geometry::{Chain, Curve2D, GeometryError, GeometryId, GeometryItem, GeometryShape, GeometrySource, GeometryStore, Layer, Point2, Region};
pub use hybrid::ResolvedTool;
pub use hybrid::{
    CAM_CACHE_VERSION, CacheEntry, CamPlan, HybridCam, HybridError, PathCache, SimulationSummary, cache_from_json, cache_to_json, load_cache,
    plan_from_section, plan_section, save_cache,
};
pub use job::{CamJob, JobError, RebuildIssue, RebuildReport, StoredPath, SyncReport};
pub use mastercam::{MAPPING, MappingSource, MappingStatus, ParameterMap, counts, for_operation};
pub use offset::{
    JoinStyle, OffsetError, OffsetOptions, boolean_difference, boolean_union, clip_to_stock, minimum_inward_radius, offset_chain, offset_region,
};
pub use operation::{Containment, EngagementOptions, Operation, OperationError, OperationKind, OperationOptions, Program, ProgramError};
pub use ops_extra::{ExtraError, blend_mill, helix_bore, model_chamfer, thread_mill};
pub use post::{GcodeBlock, GcodeProgram, GrblPost, ModalState, MotionMode, PostError, PostOptions, PostProcessor, post, post_all};
pub use reference::{DesignRef, DesignSnapshot, Entity, EntityKind, EntityRef, ReferenceError};
pub use routing::{
    Compensation, EntryStyle, PocketStrategy, RoutingError, RoutingParams, circle_mill, contour, depth_levels, engrave, facing, pocket, slot_mill,
};
pub use setup::{Direction, Fixture, Setup, SetupError};
pub use simulate::{PathReport, Simulation, SimulationError};
pub use stock::{Material, Stock, StockError};
pub use surface3d::{HeightMap, SurfaceError, SurfaceMesh, chord_error, cusp_height, stepover_for_cusp, z_levels};
pub use tool::{Cutter, CuttingData, Tool, ToolError, ToolLibrary};
pub use tooling::{Holder, ProfileSegment, Shank, ToolAssembly, ToolOverrides, ToolingError};
pub use toolpath::{Heights, LeadIn, Move, Tab, Toolpath, ToolpathError};

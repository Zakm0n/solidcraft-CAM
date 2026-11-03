//! Generation: turn a CAM operation into a toolpath.
//!
//! One dispatch point maps each core operation to a routing primitive, resolves the tool and
//! its cutting data, applies the operation's options, and resolves the selected geometry.
//! Operations that this milestone does not generate yet fail with a named error instead of
//! producing a wrong path.

use thiserror::Error;

use crate::engagement::{EngagementError, EngagementParams, area_mill, dynamic_contour, dynamic_mill, peel_mill, stepover_for_angle};
use crate::geometry::{Chain, GeometryStore, Region};
use crate::hybrid::CamPlan;
use crate::operation::{Operation, OperationError, OperationKind};
use crate::ops_extra::{ExtraError, blend_mill, helix_bore, model_chamfer, thread_mill};
use crate::routing::{RoutingError, RoutingParams, circle_mill, contour, engrave, facing, pocket, slot_mill};
use crate::setup::Setup;
use crate::stock::Stock;
use crate::tool::CuttingData;
use crate::toolpath::Toolpath;

#[derive(Debug, Error)]
pub enum GenerateError {
    #[error(transparent)]
    Routing(#[from] RoutingError),
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error(transparent)]
    Geometry(#[from] crate::geometry::GeometryError),
    #[error("tool `{0}` is not in the plan")]
    UnknownTool(String),
    #[error("operation `{0}` has no geometry selected")]
    NoGeometry(String),
    #[error("operation `{0}` is not generated yet")]
    Unsupported(String),
    #[error(transparent)]
    Engagement(#[from] EngagementError),
    #[error(transparent)]
    Extra(#[from] ExtraError),
}

/// Generate the toolpath for one operation of a plan.
pub fn generate(plan: &CamPlan, operation: &Operation) -> Result<Toolpath, GenerateError> {
    operation.validate_geometry(plan.geometry())?;
    let resolved = plan.resolve_tool(operation.tool()).ok_or_else(|| GenerateError::UnknownTool(operation.tool().to_string()))?;
    let tool = &resolved.tool;
    let cutting = tool.cutting().copied().unwrap_or_else(|| CuttingData::recommended(plan.stock().material(), tool));
    let mut params = RoutingParams::new(tool, &cutting, operation.depth_mm());
    let options = operation.options();
    params.compensation = options.compensation;
    params.entry = options.entry;
    params.lead_in_mm = options.lead_in_mm;
    params.finish_pass = options.finish_pass;
    params.facing_angle_deg = options.facing_angle_deg;
    params.ramp_angle_deg = options.ramp_angle_deg;
    params.ramp_depth_per_pass_mm = options.ramp_depth_per_pass_mm;

    match operation.kind() {
        OperationKind::Face { stepover_mm } => {
            if stepover_mm > 0.0 {
                params.stepover_mm = stepover_mm;
            }
            let (min, max) = wcs_bounds(plan.stock(), plan.setup());
            Ok(facing(min, max, &params, tool)?)
        }
        OperationKind::Contour { .. } => {
            let chain = first_chain(plan.geometry(), operation)?;
            Ok(contour(&chain, &params, tool)?)
        }
        OperationKind::Pocket { stepover_mm } => {
            if stepover_mm > 0.0 {
                params.stepover_mm = stepover_mm;
            }
            let region = selected_region(plan.geometry(), operation)?;
            Ok(pocket(&region, options.pocket_strategy, &params, tool)?)
        }
        OperationKind::CircleMill { diameter_mm, center } => Ok(circle_mill(center, diameter_mm, &params, tool)?),
        OperationKind::Slot { width_mm } => {
            let chain = first_chain(plan.geometry(), operation)?;
            Ok(slot_mill(&chain, width_mm, &params, tool)?)
        }
        OperationKind::Engrave => {
            let chain = first_chain(plan.geometry(), operation)?;
            Ok(engrave(&chain, &params)?)
        }
        OperationKind::DynamicContour => {
            let chain = first_chain(plan.geometry(), operation)?;
            let engagement = engagement_params(tool, &cutting, operation);
            Ok(dynamic_contour(&chain, options.compensation, &engagement, tool)?)
        }
        OperationKind::DynamicMill => {
            let region = selected_region(plan.geometry(), operation)?;
            let engagement = engagement_params(tool, &cutting, operation);
            Ok(dynamic_mill(&region, &engagement, tool)?)
        }
        OperationKind::PeelMill { width_mm } => {
            let chain = first_chain(plan.geometry(), operation)?;
            let engagement = engagement_params(tool, &cutting, operation);
            Ok(peel_mill(&chain, width_mm, &engagement, tool)?)
        }
        OperationKind::AreaMill => {
            let region = selected_region(plan.geometry(), operation)?;
            let engagement = engagement_params(tool, &cutting, operation);
            Ok(area_mill(&region, None, &engagement, tool)?)
        }
        OperationKind::BlendMill { stepover_mm } => {
            let first = chain_at(plan.geometry(), operation, 0)?;
            let second = chain_at(plan.geometry(), operation, 1)?;
            Ok(blend_mill(&first, &second, stepover_mm, &params, tool)?)
        }
        OperationKind::ModelChamfer { width_mm, angle_deg } => {
            let chain = first_chain(plan.geometry(), operation)?;
            Ok(model_chamfer(&chain, width_mm, angle_deg, &params, tool)?)
        }
        OperationKind::HelixBore { diameter_mm, pitch_mm, center } => Ok(helix_bore(center, diameter_mm, pitch_mm, &params, tool)?),
        OperationKind::ThreadMill { diameter_mm, pitch_mm, center } => Ok(thread_mill(center, diameter_mm, pitch_mm, &params, tool)?),
        other => Err(GenerateError::Unsupported(other.label().to_string())),
    }
}

/// Generate by operation name.
pub fn generate_named(plan: &CamPlan, operation: &str) -> Result<Toolpath, GenerateError> {
    let operation =
        plan.program().iter().find(|candidate| candidate.name() == operation).ok_or_else(|| GenerateError::NoGeometry(operation.to_string()))?;
    generate(plan, operation)
}

/// The WCS window the stock occupies, from its eight corners.
pub fn wcs_bounds(stock: &Stock, setup: &Setup) -> ([f64; 2], [f64; 2]) {
    let (min, max) = (stock.min(), stock.max());
    let mut bounds = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for x in [min[0], max[0]] {
        for y in [min[1], max[1]] {
            for z in [min[2], max[2]] {
                let wcs = setup.to_wcs([x, y, z]);
                for (axis, value) in wcs.iter().take(2).enumerate() {
                    bounds.0[axis] = bounds.0[axis].min(*value);
                    bounds.1[axis] = bounds.1[axis].max(*value);
                }
            }
        }
    }
    bounds
}

fn engagement_params(tool: &crate::tool::Tool, cutting: &CuttingData, operation: &Operation) -> EngagementParams {
    let options = operation.options().engagement;
    let mut engagement = EngagementParams::new(tool, cutting, operation.depth_mm());
    if options.stepover_mm > 0.0 {
        engagement.stepover_mm = options.stepover_mm;
    }
    if options.engagement_angle_deg > 0.0 {
        engagement.stepover_mm = stepover_for_angle(tool.diameter_mm() / 2.0, options.engagement_angle_deg);
    }
    if options.loop_radius_mm > 0.0 {
        engagement.loop_radius_mm = options.loop_radius_mm;
    }
    if options.min_toolpath_radius_mm >= 0.0 {
        engagement.min_toolpath_radius_mm = options.min_toolpath_radius_mm;
    }
    engagement.micro_lift_mm = options.micro_lift_mm.max(0.0);
    if options.back_feed_mm_min > 0.0 {
        engagement.back_feed_mm_min = options.back_feed_mm_min;
    }
    engagement
}

fn first_chain(geometry: &GeometryStore, operation: &Operation) -> Result<Chain, GenerateError> {
    chain_at(geometry, operation, 0)
}

fn chain_at(geometry: &GeometryStore, operation: &Operation, index: usize) -> Result<Chain, GenerateError> {
    let id = operation.selection().get(index).ok_or_else(|| GenerateError::NoGeometry(operation.name().to_string()))?;
    let item = geometry.get(*id).ok_or(OperationError::UnknownGeometry(*id))?;
    match item.shape() {
        crate::geometry::GeometryShape::Curve(curve) => Ok(Chain::open(vec![curve.clone()], 1e-6)?),
        crate::geometry::GeometryShape::Chain(chain) => Ok(chain.clone()),
        crate::geometry::GeometryShape::Region(region) => Ok(region.outer().clone()),
    }
}

fn selected_region(geometry: &GeometryStore, operation: &Operation) -> Result<Region, GenerateError> {
    let mut chains = Vec::new();
    for id in operation.selection() {
        let item = geometry.get(*id).ok_or(OperationError::UnknownGeometry(*id))?;
        match item.shape() {
            crate::geometry::GeometryShape::Region(region) => return Ok(region.clone()),
            crate::geometry::GeometryShape::Chain(chain) if chain.is_closed() => chains.push(chain.clone()),
            _ => return Err(OperationError::WrongGeometry(*id).into()),
        }
    }
    if chains.is_empty() {
        return Err(GenerateError::NoGeometry(operation.name().to_string()));
    }
    Ok(Region::nest(chains)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Curve2D, GeometryShape};
    use crate::hybrid::HybridCam;
    use crate::operation::{Operation, OperationOptions};
    use crate::reference::DesignRef;
    use crate::routing::{Compensation, EntryStyle, PocketStrategy};
    use crate::stock::Material;
    use crate::tool::{Cutter, Tool};
    use crate::toolpath::Heights;

    fn stock() -> Stock {
        Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap()
    }

    fn setup() -> Setup {
        Setup::top_centre(&stock())
    }

    fn square() -> Chain {
        Chain::closed(vec![Curve2D::polyline(vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]], true).unwrap()], 1e-6).unwrap()
    }

    fn line() -> Chain {
        Chain::open(vec![Curve2D::line([0.0, 0.0], [10.0, 0.0]).unwrap()], 1e-6).unwrap()
    }

    struct Fixture {
        plan: CamPlan,
        square: crate::geometry::GeometryId,
        region: crate::geometry::GeometryId,
        line: crate::geometry::GeometryId,
    }

    fn plan_with_geometry() -> Fixture {
        let mut plan = CamPlan::new(DesignRef::new("bracket", 1).unwrap(), stock(), setup());
        plan.add_tool(Tool::new("6mm flat", Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap()).unwrap();
        plan.add_tool(Tool::new("3mm flat", Cutter::Flat, 3.0, 2, 15.0, 25.0).unwrap()).unwrap();
        let geometry = plan.geometry_mut();
        geometry.add_layer("Machining").unwrap();
        let square_id = geometry.add("Machining", GeometryShape::Chain(square())).unwrap();
        let region = geometry.add("Machining", GeometryShape::Region(Region::new(square(), vec![]).unwrap())).unwrap();
        let line = geometry.add("Machining", GeometryShape::Chain(line())).unwrap();
        Fixture { plan, square: square_id, region, line }
    }

    fn add(plan: &mut CamPlan, name: &str, kind: OperationKind, tool: &str, depth: f64, selection: Vec<crate::geometry::GeometryId>) {
        plan.add_operation(Operation::new(name, kind, tool, depth).unwrap().with_selection(selection)).unwrap();
    }

    fn full_plan() -> Fixture {
        let mut fixture = plan_with_geometry();
        let plan = &mut fixture.plan;
        add(plan, "Face", OperationKind::Face { stepover_mm: 6.0 }, "6mm flat", 1.0, vec![]);
        add(plan, "Contour", OperationKind::Contour { finish_passes: 1 }, "6mm flat", 2.0, vec![fixture.square]);
        add(plan, "Pocket", OperationKind::Pocket { stepover_mm: 4.0 }, "6mm flat", 2.0, vec![fixture.region]);
        add(plan, "Circle", OperationKind::CircleMill { diameter_mm: 20.0, center: [10.0, 10.0] }, "6mm flat", 2.0, vec![]);
        add(plan, "Slot", OperationKind::Slot { width_mm: 12.0 }, "6mm flat", 2.0, vec![fixture.square]);
        add(plan, "Engrave", OperationKind::Engrave, "3mm flat", 0.5, vec![fixture.line]);
        fixture
    }

    #[test]
    fn every_core_operation_generates_a_path() {
        let fixture = full_plan();
        for operation in fixture.plan.program().iter() {
            let path = generate(&fixture.plan, operation).unwrap_or_else(|error| panic!("{} did not generate: {error}", operation.name()));
            assert!(path.cutting_length_mm() > 0.5, "{} produced no cutting moves", operation.name());
        }
        assert!(generate_named(&fixture.plan, "Pocket").is_ok());
    }

    #[test]
    fn a_face_operation_mills_the_whole_stock_window() {
        let fixture = full_plan();
        let path = generate_named(&fixture.plan, "Face").unwrap();
        let (min, max) = path.bounds();
        assert!(min[0] <= -29.9 && max[0] >= 29.9);
        assert!(min[1] <= -19.9 && max[1] >= 19.9);
    }

    #[test]
    fn options_reach_the_generated_path() {
        let mut fixture = plan_with_geometry();
        let options = OperationOptions {
            compensation: Compensation::Left,
            entry: EntryStyle::Ramp,
            pocket_strategy: PocketStrategy::ZigZag,
            ..OperationOptions::default()
        };
        let operation = Operation::new("Contour", OperationKind::Contour { finish_passes: 1 }, "6mm flat", 2.0)
            .unwrap()
            .with_selection(vec![fixture.square])
            .with_options(options);
        fixture.plan.add_operation(operation).unwrap();
        let path = generate_named(&fixture.plan, "Contour").unwrap();
        let (min, _max) = path.bounds();
        assert!(min[0] > 2.5, "compensation did not apply, min x {}", min[0]);
    }

    #[test]
    fn missing_and_wrong_geometry_are_rejected() {
        let mut fixture = plan_with_geometry();
        let no_selection = Operation::new("Contour", OperationKind::Contour { finish_passes: 1 }, "6mm flat", 2.0).unwrap();
        assert!(matches!(generate(&fixture.plan, &no_selection), Err(GenerateError::Operation(OperationError::MissingGeometry))));
        let open_pocket =
            Operation::new("Pocket", OperationKind::Pocket { stepover_mm: 4.0 }, "6mm flat", 2.0).unwrap().with_selection(vec![fixture.line]);
        assert!(matches!(generate(&fixture.plan, &open_pocket), Err(GenerateError::Operation(OperationError::WrongGeometry(_)))));
        let missing = Operation::new("Contour", OperationKind::Contour { finish_passes: 1 }, "6mm flat", 2.0)
            .unwrap()
            .with_selection(vec![crate::geometry::GeometryId(999)]);
        assert!(matches!(generate(&fixture.plan, &missing), Err(GenerateError::Operation(OperationError::UnknownGeometry(_)))));
        let _ = &mut fixture;
    }

    #[test]
    fn parameter_sets_expose_kind_options_and_selection() {
        let fixture = full_plan();
        let contour = fixture.plan.program().iter().find(|op| op.name() == "Contour").unwrap();
        let value = contour.parameter_set();
        assert_eq!(value["name"], "Contour");
        assert!(value["kind"].is_object() || value["kind"].is_string());
        assert_eq!(value["depth_mm"], 2.0);
        assert!(value["options"].is_object());
        assert!(value["selection"].is_array());
    }

    #[test]
    fn editing_selected_geometry_invalidates_only_its_cached_paths() {
        let fixture = full_plan();
        let mut hybrid = HybridCam::new(fixture.plan.clone());
        for name in ["Face", "Contour", "Pocket"] {
            let path = generate_named(&fixture.plan, name).unwrap();
            hybrid.store_toolpath(name, path).unwrap();
        }
        assert_eq!(hybrid.cached_count(), 3);

        // Edit the square geometry used by Contour and Slot.
        let mut replacement = square();
        replacement = replacement.reversed();
        hybrid.plan().geometry().get(fixture.square).expect("square exists");
        let mut edited = hybrid.plan().clone();
        edited.geometry_mut().set_shape(fixture.square, GeometryShape::Chain(replacement)).unwrap();
        hybrid.set_plan(edited);

        assert!(hybrid.toolpath("Face").is_some(), "unrelated operation lost its cache");
        assert!(hybrid.toolpath("Contour").is_none(), "edited geometry kept a stale path");
        assert!(hybrid.toolpath("Pocket").is_some(), "unrelated region lost its cache");
        assert_eq!(hybrid.prune(), 1);
    }

    #[test]
    fn generated_pocket_cuts_remove_material_without_gouging() {
        use crate::simulate::{Simulation, simulate_operation};
        let fixture = full_plan();
        let pocket = fixture.plan.program().iter().find(|op| op.name() == "Pocket").unwrap();
        let (path, report) = simulate_operation(&fixture.plan, pocket, 5.0).unwrap();
        assert!(path.cutting_length_mm() > 100.0);
        assert!(report.removed_volume_mm3 > 200.0, "removed {}", report.removed_volume_mm3);
        assert!(report.removed_volume_mm3 < 1200.0, "removed {}", report.removed_volume_mm3);
        assert_eq!(report.collisions, 0);

        let mut simulation = Simulation::new(&stock(), 5.0).unwrap();
        simulation.cut_path(fixture.plan.tools().find(pocket.tool()).unwrap(), &path, &setup()).unwrap();
        let machined = simulation.mesh().unwrap();
        let comparison = simulation.compare_with_design(&machined, 0.05).unwrap();
        assert!(comparison.is_within_tolerance());
    }

    #[test]
    fn editing_an_operation_parameter_invalidates_its_cache() {
        let fixture = full_plan();
        let mut hybrid = HybridCam::new(fixture.plan.clone());
        for name in ["Face", "Contour"] {
            let path = generate_named(&fixture.plan, name).unwrap();
            hybrid.store_toolpath(name, path).unwrap();
        }
        let mut edited = hybrid.plan().clone();
        edited.operation_mut("Contour").unwrap().set_depth(3.0).unwrap();
        hybrid.set_plan(edited);
        assert!(hybrid.toolpath("Contour").is_none(), "edited depth kept a stale path");
        assert!(hybrid.toolpath("Face").is_some(), "unrelated operation lost its cache");
    }

    #[test]
    fn every_core_operation_simulates_and_removes_material() {
        use crate::simulate::simulate_operation;
        let fixture = full_plan();
        for operation in fixture.plan.program().iter() {
            let (_path, report) =
                simulate_operation(&fixture.plan, operation, 5.0).unwrap_or_else(|error| panic!("{} failed to simulate: {error}", operation.name()));
            assert!(report.removed_volume_mm3 > 0.0, "{} removed nothing", operation.name());
            assert_eq!(report.collisions, 0, "{} collided", operation.name());
        }
    }

    fn add_dynamic(
        plan: &mut CamPlan,
        name: &str,
        kind: OperationKind,
        selection: Vec<crate::geometry::GeometryId>,
        engagement: crate::operation::EngagementOptions,
    ) {
        let operation = Operation::new(name, kind, "6mm flat", 1.0)
            .unwrap()
            .with_selection(selection)
            .with_options(OperationOptions::default().with_engagement(engagement));
        plan.add_operation(operation).unwrap();
    }

    fn dynamic_plan() -> Fixture {
        let mut fixture = plan_with_geometry();
        let square = fixture.square;
        let region = fixture.region;
        add_dynamic(&mut fixture.plan, "Dynamic Contour", OperationKind::DynamicContour, vec![square], Default::default());
        add_dynamic(&mut fixture.plan, "Dynamic Mill", OperationKind::DynamicMill, vec![region], Default::default());
        add_dynamic(&mut fixture.plan, "Peel Mill", OperationKind::PeelMill { width_mm: 12.0 }, vec![square], Default::default());
        add_dynamic(&mut fixture.plan, "Area Mill", OperationKind::AreaMill, vec![region], Default::default());
        fixture
    }

    #[test]
    fn every_dynamic_operation_generates_and_simulates() {
        use crate::simulate::simulate_operation;
        let fixture = dynamic_plan();
        for operation in fixture.plan.program().iter() {
            let path = generate(&fixture.plan, operation).unwrap_or_else(|error| panic!("{}: {error}", operation.name()));
            assert!(path.cutting_length_mm() > 0.0, "{} empty path", operation.name());
            let (_path, report) =
                simulate_operation(&fixture.plan, operation, 5.0).unwrap_or_else(|error| panic!("{} simulate: {error}", operation.name()));
            assert!(report.removed_volume_mm3 > 0.0, "{} removed nothing", operation.name());
        }
    }

    #[test]
    fn dynamic_minimum_radius_and_back_feed_reach_the_path() {
        let mut fixture = plan_with_geometry();
        let engagement = crate::operation::EngagementOptions {
            loop_radius_mm: 0.5,
            min_toolpath_radius_mm: 3.0,
            back_feed_mm_min: 2_000.0,
            micro_lift_mm: 0.3,
            stepover_mm: 3.0,
            ..crate::operation::EngagementOptions::default()
        };
        add_dynamic(&mut fixture.plan, "Dynamic Contour", OperationKind::DynamicContour, vec![fixture.square], engagement);
        let path = generate_named(&fixture.plan, "Dynamic Contour").unwrap();
        let mut radii = Vec::new();
        let mut back_feeds = 0;
        for motion in path.moves() {
            if let crate::toolpath::Move::Arc { to, feed_mm_min, centre, .. } = motion {
                radii.push(((to[0] - centre[0]).powi(2) + (to[1] - centre[1]).powi(2)).sqrt());
                if (*feed_mm_min - 2_000.0).abs() < 1e-6 {
                    back_feeds += 1;
                }
            }
        }
        assert!(!radii.is_empty());
        assert!(radii.iter().all(|radius| *radius >= 3.0 - 1e-6), "{radii:?}");
        assert!(back_feeds > 0);
    }

    #[test]
    fn dynamic_programmes_round_trip_through_post_and_parser() {
        let fixture = dynamic_plan();
        let programme = crate::post::post_all(&fixture.plan).unwrap();
        let parsed = crate::gcode::parse_gcode(&programme.text());
        assert!(parsed.issues.is_empty(), "issues: {:?}", parsed.issues);
        assert!(parsed.moves.iter().any(|movement| matches!(movement.motion, crate::toolpath::Move::Arc { .. })));
        let comparison = crate::gcode::compare_gcode(&fixture.plan, &programme.text(), 0.1).unwrap();
        assert!(comparison.post.removed_volume_mm3 > 0.0);
    }

    #[test]
    fn remaining_operations_simulate_and_post() {
        use crate::simulate::simulate_operation;
        let mut fixture = plan_with_geometry();
        let second = fixture
            .plan
            .geometry_mut()
            .add("Machining", GeometryShape::Chain(Chain::open(vec![Curve2D::line([0.0, 8.0], [10.0, 8.0]).unwrap()], 1e-6).unwrap()))
            .unwrap();
        add(&mut fixture.plan, "Blend", OperationKind::BlendMill { stepover_mm: 2.0 }, "6mm flat", 1.0, vec![fixture.line, second]);
        add(&mut fixture.plan, "Chamfer", OperationKind::ModelChamfer { width_mm: 2.0, angle_deg: 45.0 }, "6mm flat", 1.0, vec![fixture.square]);
        add(
            &mut fixture.plan,
            "Helix Bore",
            OperationKind::HelixBore { diameter_mm: 20.0, pitch_mm: 2.0, center: [10.0, 10.0] },
            "6mm flat",
            2.0,
            vec![],
        );
        add(
            &mut fixture.plan,
            "Thread",
            OperationKind::ThreadMill { diameter_mm: 20.0, pitch_mm: 1.5, center: [10.0, 10.0] },
            "6mm flat",
            3.0,
            vec![],
        );

        for operation in fixture.plan.program().active().filter(|op| matches!(op.name(), "Blend" | "Chamfer" | "Helix Bore" | "Thread")) {
            let path = generate(&fixture.plan, operation).unwrap_or_else(|error| panic!("{}: {error}", operation.name()));
            assert!(path.cutting_length_mm() > 0.0, "{}", operation.name());
            let (_path, report) =
                simulate_operation(&fixture.plan, operation, 5.0).unwrap_or_else(|error| panic!("{} simulate: {error}", operation.name()));
            assert!(report.removed_volume_mm3 > 0.0, "{} removed nothing", operation.name());
        }

        let programme = crate::post::post_all(&fixture.plan).unwrap();
        let parsed = crate::gcode::parse_gcode(&programme.text());
        assert!(parsed.issues.is_empty(), "issues: {:?}", parsed.issues);
        assert!(parsed.moves.iter().any(|movement| matches!(movement.motion, crate::toolpath::Move::Arc { .. })));
    }

    fn with_assembly(
        plan: &mut CamPlan,
        name: &str,
        tool: crate::tool::Tool,
        holder: crate::tooling::Holder,
        overrides: crate::tooling::ToolOverrides,
    ) {
        let holder_id = holder.id().to_string();
        let catalog = plan.catalog_mut();
        let tool_id = catalog.add_tool(tool).unwrap();
        catalog.add_holder(holder).unwrap();
        let assembly =
            crate::tooling::ToolAssembly::new(format!("{name}-asm"), name, tool_id, Some(holder_id), 80.0).unwrap().with_overrides(overrides);
        catalog.add_assembly(assembly).unwrap();
    }

    #[test]
    fn an_operation_resolves_its_assembly_and_overrides() {
        let mut fixture = plan_with_geometry();
        let tool = crate::tool::Tool::new("Catalog 6mm", crate::tool::Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap();
        let holder = crate::tooling::Holder::new("h1", "Holder", vec![crate::tooling::ProfileSegment::new(20.0, 60.0, 60.0).unwrap()], 80.0).unwrap();
        with_assembly(
            &mut fixture.plan,
            "Assembly 10mm",
            tool,
            holder,
            crate::tooling::ToolOverrides { diameter_mm: Some(10.0), ..Default::default() },
        );
        assert!(fixture.plan.catalog().assembly_by_name("Assembly 10mm").is_some(), "assembly missing");
        assert!(fixture.plan.resolve_tool("Assembly 10mm").is_some(), "assembly does not resolve");
        fixture
            .plan
            .add_operation(
                Operation::new("Assembly Contour", OperationKind::Contour { finish_passes: 1 }, "Assembly 10mm", 2.0)
                    .unwrap()
                    .with_selection(vec![fixture.square])
                    .with_options(OperationOptions::default().with_compensation(crate::routing::Compensation::Left)),
            )
            .unwrap();
        assert!(fixture.plan.resolve_tool("Assembly 10mm").is_some(), "resolution lost after add");
        assert_eq!(fixture.plan.program().iter().last().unwrap().tool(), "Assembly 10mm");
        let path = generate_named(&fixture.plan, "Assembly Contour").unwrap();
        let (min, max) = path.bounds();
        // Compensation uses the overridden 10 mm diameter: 5 mm inside the 20 mm wall.
        assert!((min[0] - 5.0).abs() < 0.3, "min x {}", min[0]);
        assert!((max[0] - 15.0).abs() < 0.3, "max x {}", max[0]);

        let operation = fixture.plan.program().iter().find(|op| op.name() == "Assembly Contour").unwrap();
        let (_path, report) = crate::simulate::simulate_operation(&fixture.plan, operation, 5.0).unwrap();
        assert!(report.removed_volume_mm3 > 0.0);
        let programme = crate::post::post(&fixture.plan, operation).unwrap();
        assert!(crate::gcode::parse_gcode(&programme.text()).issues.is_empty());
    }

    #[test]
    fn assembly_edits_invalidate_only_that_operations_cache() {
        let mut fixture = plan_with_geometry();
        let tool = crate::tool::Tool::new("Catalog 6mm", crate::tool::Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap();
        let holder = crate::tooling::Holder::new("h2", "Holder", vec![crate::tooling::ProfileSegment::new(20.0, 40.0, 40.0).unwrap()], 80.0).unwrap();
        with_assembly(&mut fixture.plan, "Assembly 8mm", tool, holder, Default::default());
        assert!(fixture.plan.catalog().assembly_by_name("Assembly 8mm").is_some(), "assembly missing");
        assert!(fixture.plan.resolve_tool("Assembly 8mm").is_some(), "assembly does not resolve");
        add(&mut fixture.plan, "Assembly Contour", OperationKind::Contour { finish_passes: 1 }, "Assembly 8mm", 2.0, vec![fixture.square]);
        add(&mut fixture.plan, "Legacy Contour", OperationKind::Contour { finish_passes: 1 }, "6mm flat", 2.0, vec![fixture.square]);

        let mut hybrid = crate::hybrid::HybridCam::new(fixture.plan.clone());
        for name in ["Assembly Contour", "Legacy Contour"] {
            let path = generate_named(&fixture.plan, name).unwrap();
            hybrid.store_toolpath(name, path).unwrap();
        }
        let mut edited = fixture.plan.clone();
        edited
            .catalog_mut()
            .assembly_by_name_mut("Assembly 8mm")
            .unwrap()
            .set_overrides(crate::tooling::ToolOverrides { diameter_mm: Some(12.0), ..Default::default() });
        hybrid.set_plan(edited);
        assert!(hybrid.toolpath("Assembly Contour").is_none(), "edited assembly kept a stale path");
        assert!(hybrid.toolpath("Legacy Contour").is_some(), "unrelated operation lost its cache");
    }

    #[test]
    fn an_assembled_holder_reports_collisions() {
        let mut fixture = plan_with_geometry();
        let tool = crate::tool::Tool::new("Short 6mm", crate::tool::Cutter::Flat, 6.0, 3, 4.0, 5.0).unwrap();
        let holder =
            crate::tooling::Holder::new("huge", "Huge holder", vec![crate::tooling::ProfileSegment::new(20.0, 50.0, 50.0).unwrap()], 80.0).unwrap();
        with_assembly(&mut fixture.plan, "Short assembly", tool, holder, Default::default());
        add(&mut fixture.plan, "Deep Contour", OperationKind::Contour { finish_passes: 1 }, "Short assembly", 8.0, vec![fixture.square]);
        let operation = fixture.plan.program().iter().find(|op| op.name() == "Deep Contour").unwrap();
        let (_path, report) = crate::simulate::simulate_operation(&fixture.plan, operation, 5.0).unwrap();
        assert!(report.collisions > 0, "oversized holder did not collide");
    }

    #[test]
    fn generated_paths_are_still_shared_toolpaths() {
        let fixture = full_plan();
        let path = generate_named(&fixture.plan, "Contour").unwrap();
        assert_eq!(path.heights(), Heights::default());
        assert!(!path.moves().is_empty());
    }
}

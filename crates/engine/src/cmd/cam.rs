//! CAM commands: the machining pipeline as engine commands, so the desktop app, CLI scripts,
//! the control channel and MCP all drive the same code.
//!
//! The commands are stateless: each takes the CAM plan as JSON and returns the updated plan or
//! the generated artefact. That keeps CAM out of the design document's undo history while an
//! agent builds a plan step by step.

use serde_json::{Value, json};

use solidcraft_cam::{
    CamPlan, Chain, Curve2D, Cutter, DesignRef, GeometryId, GeometryShape, Holder, Material, Operation, OperationKind, OperationOptions,
    ProfileSegment, Region, Setup, Stock, Tool, ToolAssembly, ToolCatalog, ToolOverrides, compare_gcode, generate, post_all, simulate_gcode,
};

use super::CommandSpec;
use crate::params::bad;
use crate::{Result, Session};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cam.plan.new", "New CAM Plan", plan_new).noundo().doc_only().params(
        "stock_min: [x,y,z], stock_max: [x,y,z], material?: aluminum|steel|brass|plastic|text, design_id?: text (default part), revision?: integer → the plan JSON",
    ),
    CommandSpec::new("cam.plan.add_layer", "Add CAM Layer", add_layer_cmd).noundo().doc_only().params(
        "plan: the plan JSON, name: layer name → plan",
    ),
    CommandSpec::new("cam.plan.add_tool", "Add CAM Tool", add_tool_cmd).noundo().doc_only().params(
        "plan, name, diameter_mm, flutes?: integer (3), flute_length_mm?: (20), stickout_mm?: (30), cutter?: flat|ball|bull_nose|drill, corner_radius_mm?, point_angle_deg? → plan",
    ),
    CommandSpec::new("cam.plan.add_geometry", "Add CAM Geometry", add_geometry_cmd).noundo().doc_only().params(
        "plan, layer, points: [[x,y],…], closed?: true, islands?: [[[x,y],…],…] → {plan, geometry_id}",
    ),
    CommandSpec::new("cam.operation.add", "Add CAM Operation", add_operation_cmd).noundo().doc_only().params(
        "plan, name, kind: face|contour|pocket|circle_mill|slot|engrave, tool, depth_mm, selection?: [geometry_id], \
         stepover_mm?, finish_passes?, diameter_mm?, center?: [x,y], width_mm?, compensation?: none|left|right, \
         pocket_strategy?: spiral|zig_zag, entry?: plunge|ramp|helix, lead_in_mm?, finish_pass?, facing_angle_deg?, \
         ramp_angle_deg?, ramp_depth_per_pass_mm? → plan",
    ),
    CommandSpec::new("cam.generate", "Generate CAM Toolpath", generate_cmd).noundo().doc_only().params(
        "plan, operation: name → the toolpath JSON",
    ),
    CommandSpec::new("cam.post", "Post CAM G-code", post_cmd).noundo().doc_only().params(
        "plan, operation: name → {gcode, blocks, operations}",
    ),
    CommandSpec::new("cam.post_all", "Post All CAM G-code", post_all_cmd).noundo().doc_only().params(
        "plan → {gcode, blocks, operations}",
    ),
    CommandSpec::new("cam.simulate_gcode", "Simulate CAM G-code", simulate_cmd).noundo().doc_only().params(
        "plan, gcode: text, tile_mm?: 5 → removed volume, collisions, parse errors, lengths, per-operation stats",
    ),
    CommandSpec::new("cam.compare_gcode", "Compare CAM G-code", compare_cmd).noundo().doc_only().params(
        "plan, gcode: text, tolerance_mm?: 0.1 → removal delta, excess/gouge, pre/post and design comparisons, deviation, line issues",
    ),
    CommandSpec::new("cam.mapping", "Mastercam Mapping", mapping_cmd).noundo().doc_only().params(
        "no parameters → mapping counts and covered operations",
    ),
    CommandSpec::new("cam.catalog.list", "List Tool Catalog", catalog_list_cmd).noundo().doc_only().params(
        "plan → tools, holders and assemblies (id, name, summary)",
    ),
    CommandSpec::new("cam.catalog.get", "Get Catalog Entry", catalog_get_cmd).noundo().doc_only().params(
        "plan, kind: tool|holder|assembly, name: id or name → the entry",
    ),
    CommandSpec::new("cam.catalog.add_tool", "Add Catalog Tool", catalog_add_tool_cmd).noundo().doc_only().params(
        "plan, name, diameter_mm, cutter?: flat|ball|bull_nose|drill|chamfer|face_mill|thread_mill|tapered|dovetail|slot, flutes?, flute_length_mm?, stickout_mm?, tool_number? → plan",
    ),
    CommandSpec::new("cam.catalog.add_holder", "Add Catalog Holder", catalog_add_holder_cmd).noundo().doc_only().params(
        "plan, id, name, gauge_length_mm, segments: [{length_mm, top_diameter_mm, bottom_diameter_mm}] → plan",
    ),
    CommandSpec::new("cam.catalog.add_assembly", "Add Tool Assembly", catalog_add_assembly_cmd).noundo().doc_only().params(
        "plan, id, name, tool: id or name, holder?: id or name, gauge_length_mm, tool_number?, diameter_mm?, stickout_mm?, flute_length_mm? → plan",
    ),
    CommandSpec::new("cam.catalog.remove", "Remove Catalog Entry", catalog_remove_cmd).noundo().doc_only().params(
        "plan, kind: tool|holder|assembly, name: id or name → plan",
    ),
    CommandSpec::new("cam.catalog.export", "Export Tool Catalog", catalog_export_cmd).noundo().doc_only().params(
        "plan → the catalog JSON",
    ),
    CommandSpec::new("cam.catalog.import", "Import Tool Catalog", catalog_import_cmd).noundo().doc_only().params(
        "plan, catalog: catalog JSON → plan",
    ),
    CommandSpec::new("cam.catalog.assign", "Assign Assembly", catalog_assign_cmd).noundo().doc_only().params(
        "plan, operation: name, assembly: name → plan",
    ),
];

fn plan_new(_session: &mut Session, params: &Value) -> Result<Value> {
    let plan = create_plan(params)?;
    to_json(&plan, "cam.plan.new")
}

fn add_layer_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let mut plan = parse_plan(params, "cam.plan.add_layer")?;
    add_layer(&mut plan, params)?;
    to_json(&plan, "cam.plan.add_layer")
}

fn add_tool_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let mut plan = parse_plan(params, "cam.plan.add_tool")?;
    add_tool(&mut plan, params)?;
    to_json(&plan, "cam.plan.add_tool")
}

fn add_geometry_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let mut plan = parse_plan(params, "cam.plan.add_geometry")?;
    let id = add_geometry(&mut plan, params)?;
    Ok(json!({ "plan": plan, "geometry_id": id.0 }))
}

fn add_operation_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let mut plan = parse_plan(params, "cam.operation.add")?;
    add_operation(&mut plan, params)?;
    to_json(&plan, "cam.operation.add")
}

fn generate_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let plan = parse_plan(params, "cam.generate")?;
    let operation = find_operation(&plan, params, "cam.generate")?;
    let path = generate(&plan, operation).map_err(|error| bad("cam.generate", error.to_string()))?;
    to_json(&path, "cam.generate")
}

fn post_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let plan = parse_plan(params, "cam.post")?;
    let operation = find_operation(&plan, params, "cam.post")?;
    let program = solidcraft_cam::post(&plan, operation).map_err(|error| bad("cam.post", error.to_string()))?;
    Ok(json!({
        "gcode": program.text(),
        "blocks": program.len(),
        "operations": program.operations,
    }))
}

fn post_all_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let plan = parse_plan(params, "cam.post_all")?;
    let program = post_all(&plan).map_err(|error| bad("cam.post_all", error.to_string()))?;
    Ok(json!({
        "gcode": program.text(),
        "blocks": program.len(),
        "operations": program.operations,
    }))
}

fn simulate_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let plan = parse_plan(params, "cam.simulate_gcode")?;
    let gcode = text(params, "gcode", "cam.simulate_gcode")?;
    let tile = number(params, "tile_mm").unwrap_or(5.0);
    let report = simulate_gcode(&plan, &gcode, tile).map_err(|error| bad("cam.simulate_gcode", error.to_string()))?;
    to_json(&report, "cam.simulate_gcode")
}

fn compare_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let plan = parse_plan(params, "cam.compare_gcode")?;
    let gcode = text(params, "gcode", "cam.compare_gcode")?;
    let tolerance = number(params, "tolerance_mm").unwrap_or(0.1);
    let comparison = compare_gcode(&plan, &gcode, tolerance).map_err(|error| bad("cam.compare_gcode", error.to_string()))?;
    to_json(&comparison, "cam.compare_gcode")
}

fn mapping_cmd(_session: &mut Session, _params: &Value) -> Result<Value> {
    let (mapped, partial, deferred, unsupported) = solidcraft_cam::mastercam::counts();
    Ok(json!({
        "mapped": mapped,
        "partial": partial,
        "deferred": deferred,
        "unsupported": unsupported,
        "operations": solidcraft_cam::mastercam::OPERATIONS,
    }))
}

fn catalog_list_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let plan = parse_plan(params, "cam.catalog.list")?;
    catalog_list(&plan)
}

fn catalog_get_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let plan = parse_plan(params, "cam.catalog.get")?;
    catalog_get(&plan, params)
}

fn catalog_add_tool_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let mut plan = parse_plan(params, "cam.catalog.add_tool")?;
    catalog_add_tool(&mut plan, params)?;
    to_json(&plan, "cam.catalog.add_tool")
}

fn catalog_add_holder_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let mut plan = parse_plan(params, "cam.catalog.add_holder")?;
    catalog_add_holder(&mut plan, params)?;
    to_json(&plan, "cam.catalog.add_holder")
}

fn catalog_add_assembly_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let mut plan = parse_plan(params, "cam.catalog.add_assembly")?;
    catalog_add_assembly(&mut plan, params)?;
    to_json(&plan, "cam.catalog.add_assembly")
}

fn catalog_remove_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let mut plan = parse_plan(params, "cam.catalog.remove")?;
    catalog_remove(&mut plan, params)?;
    to_json(&plan, "cam.catalog.remove")
}

fn catalog_export_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let plan = parse_plan(params, "cam.catalog.export")?;
    to_json(plan.catalog(), "cam.catalog.export")
}

fn catalog_import_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let mut plan = parse_plan(params, "cam.catalog.import")?;
    let catalog: ToolCatalog = serde_json::from_value(params.get("catalog").ok_or_else(|| bad("cam.catalog.import", "catalog is required"))?.clone())
        .map_err(|error| bad("cam.catalog.import", error.to_string()))?;
    *plan.catalog_mut() = catalog;
    to_json(&plan, "cam.catalog.import")
}

fn catalog_assign_cmd(_session: &mut Session, params: &Value) -> Result<Value> {
    let mut plan = parse_plan(params, "cam.catalog.assign")?;
    catalog_assign(&mut plan, params)?;
    to_json(&plan, "cam.catalog.assign")
}

fn catalog_list(plan: &CamPlan) -> Result<Value> {
    let tools: Vec<Value> = plan
        .catalog()
        .tools()
        .map(|tool| {
            json!({
                "id": tool.id().as_str(),
                "name": tool.name(),
                "diameter_mm": tool.diameter_mm(),
                "cutter": tool.cutter().label(),
                "tool_number": tool.tool_number(),
            })
        })
        .collect();
    let holders: Vec<Value> = plan
        .catalog()
        .holders()
        .map(|holder| {
            json!({
                "id": holder.id(),
                "name": holder.name(),
                "length_mm": holder.length_mm(),
                "max_diameter_mm": holder.max_diameter_mm(),
            })
        })
        .collect();
    let assemblies: Vec<Value> = plan
        .catalog()
        .assemblies()
        .map(|assembly| {
            json!({
                "id": assembly.id(),
                "name": assembly.name(),
                "tool_id": assembly.tool_id().as_str(),
                "holder_id": assembly.holder_id(),
                "tool_number": assembly.tool_number(),
            })
        })
        .collect();
    Ok(json!({ "tools": tools, "holders": holders, "assemblies": assemblies }))
}

fn catalog_get(plan: &CamPlan, params: &Value) -> Result<Value> {
    let cmd = "cam.catalog.get";
    let kind = text(params, "kind", cmd)?;
    let name = text(params, "name", cmd)?;
    match kind.as_str() {
        "tool" => {
            let tool =
                plan.catalog().tool(&name).or_else(|| plan.catalog().tool_by_name(&name)).ok_or_else(|| bad(cmd, format!("no tool `{name}`")))?;
            to_json(tool, cmd)
        }
        "holder" => {
            let holder = plan
                .catalog()
                .holder(&name)
                .or_else(|| plan.catalog().holder_by_name(&name))
                .ok_or_else(|| bad(cmd, format!("no holder `{name}`")))?;
            to_json(holder, cmd)
        }
        "assembly" => {
            let assembly = plan
                .catalog()
                .assembly(&name)
                .or_else(|| plan.catalog().assembly_by_name(&name))
                .ok_or_else(|| bad(cmd, format!("no assembly `{name}`")))?;
            to_json(assembly, cmd)
        }
        other => Err(bad(cmd, format!("unknown kind `{other}` (want tool, holder or assembly)"))),
    }
}

fn catalog_add_tool(plan: &mut CamPlan, params: &Value) -> Result<()> {
    let cmd = "cam.catalog.add_tool";
    let name = text(params, "name", cmd)?;
    let cutter = parse_cutter(params, cmd);
    let tool = Tool::new(
        name,
        cutter,
        number(params, "diameter_mm").ok_or_else(|| bad(cmd, "diameter_mm is required"))?,
        number(params, "flutes").unwrap_or(3.0) as u32,
        number(params, "flute_length_mm").unwrap_or(20.0),
        number(params, "stickout_mm").unwrap_or(30.0),
    )
    .map_err(|error| bad(cmd, error.to_string()))?;
    let tool = if let Some(number) = number(params, "tool_number") { tool.with_tool_number(number as u32) } else { tool };
    plan.catalog_mut().add_tool(tool).map_err(|error| bad(cmd, error.to_string()))?;
    Ok(())
}

fn catalog_add_holder(plan: &mut CamPlan, params: &Value) -> Result<()> {
    let cmd = "cam.catalog.add_holder";
    let id = text(params, "id", cmd)?;
    let name = text(params, "name", cmd)?;
    let gauge = number(params, "gauge_length_mm").ok_or_else(|| bad(cmd, "gauge_length_mm is required"))?;
    let segments = params
        .get("segments")
        .and_then(Value::as_array)
        .ok_or_else(|| bad(cmd, "segments is required"))?
        .iter()
        .map(|segment| {
            ProfileSegment::new(
                number(segment, "length_mm").ok_or_else(|| bad(cmd, "segment length_mm is required"))?,
                number(segment, "top_diameter_mm").ok_or_else(|| bad(cmd, "segment top_diameter_mm is required"))?,
                number(segment, "bottom_diameter_mm").ok_or_else(|| bad(cmd, "segment bottom_diameter_mm is required"))?,
            )
            .map_err(|error| bad(cmd, error.to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    let holder = Holder::new(id, name, segments, gauge).map_err(|error| bad(cmd, error.to_string()))?;
    plan.catalog_mut().add_holder(holder).map_err(|error| bad(cmd, error.to_string()))?;
    Ok(())
}

fn catalog_add_assembly(plan: &mut CamPlan, params: &Value) -> Result<()> {
    let cmd = "cam.catalog.add_assembly";
    let id = text(params, "id", cmd)?;
    let name = text(params, "name", cmd)?;
    let tool_name = text(params, "tool", cmd)?;
    let tool = plan
        .catalog()
        .tool(&tool_name)
        .or_else(|| plan.catalog().tool_by_name(&tool_name))
        .ok_or_else(|| bad(cmd, format!("no tool `{tool_name}`")))?
        .id()
        .clone();
    let holder_id = params.get("holder").and_then(Value::as_str).map(|holder| {
        plan.catalog()
            .holder(holder)
            .or_else(|| plan.catalog().holder_by_name(holder))
            .map(|entry| entry.id().to_string())
            .unwrap_or_else(|| holder.to_string())
    });
    let gauge = number(params, "gauge_length_mm").ok_or_else(|| bad(cmd, "gauge_length_mm is required"))?;
    let mut assembly = ToolAssembly::new(id, name, tool, holder_id, gauge).map_err(|error| bad(cmd, error.to_string()))?;
    let overrides = ToolOverrides {
        diameter_mm: number(params, "diameter_mm"),
        stickout_mm: number(params, "stickout_mm"),
        flute_length_mm: number(params, "flute_length_mm"),
    };
    assembly = assembly.with_overrides(overrides);
    if let Some(number) = number(params, "tool_number") {
        assembly = assembly.with_tool_number(number as u32);
    }
    plan.catalog_mut().add_assembly(assembly).map_err(|error| bad(cmd, error.to_string()))?;
    Ok(())
}

fn catalog_remove(plan: &mut CamPlan, params: &Value) -> Result<()> {
    let cmd = "cam.catalog.remove";
    let kind = text(params, "kind", cmd)?;
    let name = text(params, "name", cmd)?;
    match kind.as_str() {
        "tool" => {
            let id = plan
                .catalog()
                .tool(&name)
                .or_else(|| plan.catalog().tool_by_name(&name))
                .map(|tool| tool.id().as_str().to_string())
                .ok_or_else(|| bad(cmd, format!("no tool `{name}`")))?;
            plan.catalog_mut().remove_tool(&id);
        }
        "holder" => {
            let id = plan
                .catalog()
                .holder(&name)
                .or_else(|| plan.catalog().holder_by_name(&name))
                .map(|holder| holder.id().to_string())
                .ok_or_else(|| bad(cmd, format!("no holder `{name}`")))?;
            plan.catalog_mut().remove_holder(&id);
        }
        "assembly" => {
            let id = plan
                .catalog()
                .assembly(&name)
                .or_else(|| plan.catalog().assembly_by_name(&name))
                .map(|assembly| assembly.id().to_string())
                .ok_or_else(|| bad(cmd, format!("no assembly `{name}`")))?;
            plan.catalog_mut().remove_assembly(&id);
        }
        other => return Err(bad(cmd, format!("unknown kind `{other}`"))),
    }
    Ok(())
}

fn catalog_assign(plan: &mut CamPlan, params: &Value) -> Result<()> {
    let cmd = "cam.catalog.assign";
    let operation_name = text(params, "operation", cmd)?;
    let assembly = text(params, "assembly", cmd)?;
    if plan.catalog().assembly_by_name(&assembly).is_none() {
        return Err(bad(cmd, format!("no assembly `{assembly}`")));
    }
    plan.operation_mut(&operation_name).ok_or_else(|| bad(cmd, format!("no operation `{operation_name}`")))?.set_tool(assembly);
    Ok(())
}

fn parse_cutter(params: &Value, _cmd: &str) -> Cutter {
    match params.get("cutter").and_then(Value::as_str).unwrap_or("flat") {
        "ball" => Cutter::Ball,
        "bull_nose" => Cutter::BullNose { corner_radius_mm: number(params, "corner_radius_mm").unwrap_or(1.0) },
        "drill" => Cutter::Drill { point_angle_deg: number(params, "point_angle_deg").unwrap_or(118.0) },
        "chamfer" => Cutter::Chamfer {
            tip_angle_deg: number(params, "tip_angle_deg").unwrap_or(90.0),
            tip_diameter_mm: number(params, "tip_diameter_mm").unwrap_or(0.0),
        },
        "face_mill" => Cutter::FaceMill { lead_angle_deg: number(params, "lead_angle_deg").unwrap_or(45.0) },
        "thread_mill" => Cutter::ThreadMill { thread_angle_deg: number(params, "thread_angle_deg").unwrap_or(60.0) },
        "tapered" => Cutter::Tapered {
            half_angle_deg: number(params, "half_angle_deg").unwrap_or(5.0),
            tip_diameter_mm: number(params, "tip_diameter_mm").unwrap_or(0.0),
        },
        "dovetail" => Cutter::Dovetail { undercut_angle_deg: number(params, "undercut_angle_deg").unwrap_or(10.0) },
        "slot" => Cutter::Slot { thickness_mm: number(params, "thickness_mm").unwrap_or(3.0) },
        _ => Cutter::Flat,
    }
    .clone()
}

fn create_plan(params: &Value) -> Result<CamPlan> {
    let cmd = "cam.plan.new";
    let stock = Stock::new(arr3(params, "stock_min", cmd)?, arr3(params, "stock_max", cmd)?, material(params))
        .map_err(|error| bad(cmd, error.to_string()))?;
    let setup = Setup::top_centre(&stock);
    let design = DesignRef::new(
        params.get("design_id").and_then(Value::as_str).unwrap_or("part"),
        params.get("revision").and_then(Value::as_u64).unwrap_or(1),
    )
    .map_err(|error| bad(cmd, error.to_string()))?;
    Ok(CamPlan::new(design, stock, setup))
}

fn add_layer(plan: &mut CamPlan, params: &Value) -> Result<()> {
    let name = text(params, "name", "cam.plan.add_layer")?;
    plan.geometry_mut().add_layer(name).map_err(|error| bad("cam.plan.add_layer", error.to_string()))
}

fn add_tool(plan: &mut CamPlan, params: &Value) -> Result<()> {
    let cmd = "cam.plan.add_tool";
    let name = text(params, "name", cmd)?;
    let cutter = match params.get("cutter").and_then(Value::as_str).unwrap_or("flat") {
        "ball" => Cutter::Ball,
        "bull_nose" => Cutter::BullNose { corner_radius_mm: number(params, "corner_radius_mm").unwrap_or(1.0) },
        "drill" => Cutter::Drill { point_angle_deg: number(params, "point_angle_deg").unwrap_or(118.0) },
        _ => Cutter::Flat,
    };
    let tool = Tool::new(
        name,
        cutter,
        number(params, "diameter_mm").ok_or_else(|| bad(cmd, "diameter_mm is required"))?,
        number(params, "flutes").unwrap_or(3.0) as u32,
        number(params, "flute_length_mm").unwrap_or(20.0),
        number(params, "stickout_mm").unwrap_or(30.0),
    )
    .map_err(|error| bad(cmd, error.to_string()))?;
    plan.add_tool(tool).map_err(|error| bad(cmd, error.to_string()))
}

fn add_geometry(plan: &mut CamPlan, params: &Value) -> Result<GeometryId> {
    let cmd = "cam.plan.add_geometry";
    let layer = text(params, "layer", cmd)?;
    let closed = params.get("closed").and_then(Value::as_bool).unwrap_or(true);
    let points = points(params.get("points").ok_or_else(|| bad(cmd, "points is required"))?, cmd)?;
    let outer = chain(points, closed, cmd)?;
    let shape = match params.get("islands") {
        Some(Value::Array(islands)) if !islands.is_empty() => {
            if !closed {
                return Err(bad(cmd, "an island region needs a closed outer chain"));
            }
            let mut islands_chains = Vec::with_capacity(islands.len());
            for island in islands {
                islands_chains.push(chain(points_of(island, cmd)?, true, cmd)?);
            }
            let region = Region::new(outer, islands_chains).map_err(|error| bad(cmd, error.to_string()))?;
            GeometryShape::Region(region)
        }
        _ => GeometryShape::Chain(outer),
    };
    plan.geometry_mut().add(&layer, shape).map_err(|error| bad(cmd, error.to_string()))
}

fn add_operation(plan: &mut CamPlan, params: &Value) -> Result<()> {
    let cmd = "cam.operation.add";
    let name = text(params, "name", cmd)?;
    let tool = text(params, "tool", cmd)?;
    let depth = number(params, "depth_mm").ok_or_else(|| bad(cmd, "depth_mm is required"))?;
    let kind = match params.get("kind").and_then(Value::as_str).unwrap_or("contour") {
        "face" => OperationKind::Face { stepover_mm: number(params, "stepover_mm").unwrap_or(6.0) },
        "contour" => OperationKind::Contour { finish_passes: number(params, "finish_passes").unwrap_or(1.0) as u32 },
        "pocket" => OperationKind::Pocket { stepover_mm: number(params, "stepover_mm").unwrap_or(3.0) },
        "circle_mill" => OperationKind::CircleMill {
            diameter_mm: number(params, "diameter_mm").ok_or_else(|| bad(cmd, "diameter_mm is required for circle_mill"))?,
            center: arr2(params.get("center").ok_or_else(|| bad(cmd, "center is required for circle_mill"))?, cmd)?,
        },
        "slot" => OperationKind::Slot { width_mm: number(params, "width_mm").ok_or_else(|| bad(cmd, "width_mm is required for slot"))? },
        "engrave" => OperationKind::Engrave,
        "dynamic_contour" => OperationKind::DynamicContour,
        "dynamic_mill" => OperationKind::DynamicMill,
        "peel_mill" => {
            OperationKind::PeelMill { width_mm: number(params, "width_mm").ok_or_else(|| bad(cmd, "width_mm is required for peel_mill"))? }
        }
        "area_mill" => OperationKind::AreaMill,
        "blend_mill" => OperationKind::BlendMill { stepover_mm: number(params, "stepover_mm").unwrap_or(3.0) },
        "model_chamfer" => OperationKind::ModelChamfer {
            width_mm: number(params, "width_mm").ok_or_else(|| bad(cmd, "width_mm is required for model_chamfer"))?,
            angle_deg: number(params, "angle_deg").unwrap_or(45.0),
        },
        "helix_bore" => OperationKind::HelixBore {
            diameter_mm: number(params, "diameter_mm").ok_or_else(|| bad(cmd, "diameter_mm is required for helix_bore"))?,
            pitch_mm: number(params, "pitch_mm").unwrap_or(1.0),
            center: arr2(params.get("center").ok_or_else(|| bad(cmd, "center is required for helix_bore"))?, cmd)?,
        },
        "thread_mill" => OperationKind::ThreadMill {
            diameter_mm: number(params, "diameter_mm").ok_or_else(|| bad(cmd, "diameter_mm is required for thread_mill"))?,
            pitch_mm: number(params, "pitch_mm").ok_or_else(|| bad(cmd, "pitch_mm is required for thread_mill"))?,
            center: arr2(params.get("center").ok_or_else(|| bad(cmd, "center is required for thread_mill"))?, cmd)?,
        },
        other => return Err(bad(cmd, format!("unknown kind `{other}`"))),
    };
    let mut operation = Operation::new(name, kind, tool, depth).map_err(|error| bad(cmd, error.to_string()))?;
    if let Some(selection) = params.get("selection").and_then(Value::as_array) {
        let ids = selection.iter().filter_map(Value::as_u64).map(GeometryId).collect();
        operation = operation.with_selection(ids);
    }
    let mut options = OperationOptions::default();
    if let Some(value) = params.get("compensation").and_then(Value::as_str) {
        options.compensation = match value {
            "left" => solidcraft_cam::Compensation::Left,
            "right" => solidcraft_cam::Compensation::Right,
            _ => solidcraft_cam::Compensation::None,
        };
    }
    if let Some(value) = params.get("pocket_strategy").and_then(Value::as_str) {
        options.pocket_strategy = match value {
            "zig_zag" | "zigzag" => solidcraft_cam::PocketStrategy::ZigZag,
            _ => solidcraft_cam::PocketStrategy::Spiral,
        };
    }
    if let Some(value) = params.get("entry").and_then(Value::as_str) {
        options.entry = match value {
            "ramp" => solidcraft_cam::EntryStyle::Ramp,
            "helix" => solidcraft_cam::EntryStyle::Helix,
            _ => solidcraft_cam::EntryStyle::Plunge,
        };
    }
    if let Some(value) = number(params, "lead_in_mm") {
        options.lead_in_mm = value;
    }
    if let Some(value) = params.get("finish_pass").and_then(Value::as_bool) {
        options.finish_pass = value;
    }
    if let Some(value) = number(params, "facing_angle_deg") {
        options.facing_angle_deg = value;
    }
    if let Some(value) = number(params, "ramp_angle_deg") {
        options.ramp_angle_deg = value;
    }
    if let Some(value) = number(params, "ramp_depth_per_pass_mm") {
        options.ramp_depth_per_pass_mm = Some(value);
    }
    if let Some(value) = number(params, "engagement_angle_deg") {
        options.engagement.engagement_angle_deg = value;
    }
    if let Some(value) = number(params, "loop_radius_mm") {
        options.engagement.loop_radius_mm = value;
    }
    if let Some(value) = number(params, "min_toolpath_radius_mm") {
        options.engagement.min_toolpath_radius_mm = value;
    }
    if let Some(value) = number(params, "micro_lift_mm") {
        options.engagement.micro_lift_mm = value;
    }
    if let Some(value) = number(params, "back_feed_mm_min") {
        options.engagement.back_feed_mm_min = value;
    }
    if let Some(value) = number(params, "stock_to_leave_mm") {
        options.engagement.stock_to_leave_mm = value;
    }
    if let Some(value) = params.get("containment").and_then(Value::as_str) {
        options.engagement.containment = match value {
            "outside" => solidcraft_cam::Containment::Outside,
            "centers" => solidcraft_cam::Containment::Centers,
            _ => solidcraft_cam::Containment::Inside,
        };
    }
    operation = operation.with_options(options);
    plan.add_operation(operation).map_err(|error| bad(cmd, error.to_string()))
}

fn parse_plan(params: &Value, cmd: &str) -> Result<CamPlan> {
    let value = params.get("plan").ok_or_else(|| bad(cmd, "plan is required (the JSON returned by cam.plan.new)"))?;
    serde_json::from_value(value.clone()).map_err(|error| bad(cmd, format!("plan: {error}")))
}

fn find_operation<'a>(plan: &'a CamPlan, params: &Value, cmd: &str) -> Result<&'a Operation> {
    let name = text(params, "operation", cmd)?;
    plan.program().iter().find(|operation| operation.name() == name).ok_or_else(|| bad(cmd, format!("no operation named `{name}`")))
}

fn material(params: &Value) -> Material {
    match params.get("material").and_then(Value::as_str) {
        Some("steel") => Material::Steel,
        Some("brass") => Material::Brass,
        Some("plastic") => Material::Plastic,
        Some(other) if !other.eq_ignore_ascii_case("aluminum") => Material::Other(other.to_string()),
        _ => Material::Aluminum,
    }
}

fn chain(points: Vec<[f64; 2]>, closed: bool, cmd: &str) -> Result<Chain> {
    let curve = Curve2D::polyline(points, closed).map_err(|error| bad(cmd, error.to_string()))?;
    if closed { Chain::closed(vec![curve], 1e-6) } else { Chain::open(vec![curve], 1e-6) }.map_err(|error| bad(cmd, error.to_string()))
}

fn points(value: &Value, cmd: &str) -> Result<Vec<[f64; 2]>> {
    points_of(value, cmd)
}

fn points_of(value: &Value, cmd: &str) -> Result<Vec<[f64; 2]>> {
    let array = value.as_array().ok_or_else(|| bad(cmd, "points must be an array of [x, y]"))?;
    array.iter().map(|point| arr2(point, cmd)).collect::<Result<Vec<_>>>()
}

fn arr2(value: &Value, cmd: &str) -> Result<[f64; 2]> {
    let array = value.as_array().ok_or_else(|| bad(cmd, "expected [x, y]"))?;
    if array.len() != 2 {
        return Err(bad(cmd, "expected [x, y]"));
    }
    Ok([array[0].as_f64().unwrap_or(0.0), array[1].as_f64().unwrap_or(0.0)])
}

fn arr3(params: &Value, key: &str, cmd: &str) -> Result<[f64; 3]> {
    let array = params.get(key).and_then(Value::as_array).ok_or_else(|| bad(cmd, format!("{key} must be [x, y, z]")))?;
    if array.len() != 3 {
        return Err(bad(cmd, format!("{key} must be [x, y, z]")));
    }
    Ok([array[0].as_f64().unwrap_or(0.0), array[1].as_f64().unwrap_or(0.0), array[2].as_f64().unwrap_or(0.0)])
}

fn number(params: &Value, key: &str) -> Option<f64> {
    params.get(key).and_then(Value::as_f64)
}

fn text(params: &Value, key: &str, cmd: &str) -> Result<String> {
    params.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| bad(cmd, format!("{key} is required")))
}

fn to_json<T: serde::Serialize>(value: &T, cmd: &str) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| bad(cmd, error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cam_commands_are_registered() {
        let specs = crate::cmd::command_specs();
        for id in [
            "cam.plan.new",
            "cam.plan.add_layer",
            "cam.plan.add_tool",
            "cam.plan.add_geometry",
            "cam.operation.add",
            "cam.generate",
            "cam.post",
            "cam.post_all",
            "cam.simulate_gcode",
            "cam.compare_gcode",
            "cam.mapping",
            "cam.catalog.list",
            "cam.catalog.get",
            "cam.catalog.add_tool",
            "cam.catalog.add_holder",
            "cam.catalog.add_assembly",
            "cam.catalog.remove",
            "cam.catalog.export",
            "cam.catalog.import",
            "cam.catalog.assign",
        ] {
            assert!(specs.iter().any(|spec| spec.id == id), "{id} not registered");
        }
    }

    #[test]
    fn catalog_commands_add_assign_and_round_trip() {
        let mut plan = create_plan(&json!({
            "stock_min": [0.0, 0.0, 0.0],
            "stock_max": [60.0, 40.0, 20.0]
        }))
        .unwrap();
        catalog_add_tool(&mut plan, &json!({ "name": "Catalog 6mm", "diameter_mm": 6.0, "flutes": 3 })).unwrap();
        catalog_add_holder(
            &mut plan,
            &json!({
                "id": "er",
                "name": "ER collet",
                "gauge_length_mm": 50.0,
                "segments": [{ "length_mm": 20.0, "top_diameter_mm": 30.0, "bottom_diameter_mm": 25.0 }]
            }),
        )
        .unwrap();
        catalog_add_assembly(
            &mut plan,
            &json!({
                "id": "asm",
                "name": "Assembly",
                "tool": "Catalog 6mm",
                "holder": "er",
                "gauge_length_mm": 60.0
            }),
        )
        .unwrap();

        add_layer(&mut plan, &json!({ "name": "Machining" })).unwrap();
        let geometry = add_geometry(
            &mut plan,
            &json!({
                "layer": "Machining",
                "points": [[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]]
            }),
        )
        .unwrap();
        add_tool(&mut plan, &json!({ "name": "6mm flat", "diameter_mm": 6.0 })).unwrap();
        add_operation(
            &mut plan,
            &json!({
                "name": "Contour",
                "kind": "contour",
                "tool": "6mm flat",
                "depth_mm": 2.0,
                "selection": [geometry.0]
            }),
        )
        .unwrap();
        catalog_assign(&mut plan, &json!({ "operation": "Contour", "assembly": "Assembly" })).unwrap();
        assert_eq!(plan.program().get(0).unwrap().tool(), "Assembly");

        let list = catalog_list(&plan).unwrap();
        assert_eq!(list["tools"].as_array().unwrap().len(), 1);
        assert_eq!(list["holders"].as_array().unwrap().len(), 1);
        assert_eq!(list["assemblies"].as_array().unwrap().len(), 1);

        let exported = to_json(plan.catalog(), "cam.catalog.export").unwrap();
        let imported: ToolCatalog = serde_json::from_value(exported).unwrap();
        assert_eq!(&imported, plan.catalog());
        let path = generate(&plan, plan.program().get(0).unwrap()).unwrap();
        assert!(path.cutting_length_mm() > 0.0);
    }

    #[test]
    fn the_helpers_build_a_plan_and_run_the_pipeline() {
        let mut plan = create_plan(&json!({
            "stock_min": [0.0, 0.0, 0.0],
            "stock_max": [60.0, 40.0, 20.0],
            "design_id": "bracket"
        }))
        .unwrap();
        add_layer(&mut plan, &json!({ "name": "Machining" })).unwrap();
        let geometry = add_geometry(
            &mut plan,
            &json!({
                "layer": "Machining",
                "points": [[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]],
                "closed": true
            }),
        )
        .unwrap();
        add_tool(
            &mut plan,
            &json!({
                "name": "6mm flat",
                "diameter_mm": 6.0,
                "flutes": 3,
                "flute_length_mm": 20.0,
                "stickout_mm": 30.0
            }),
        )
        .unwrap();
        add_operation(
            &mut plan,
            &json!({
                "name": "Contour",
                "kind": "contour",
                "tool": "6mm flat",
                "depth_mm": 2.0,
                "selection": [geometry.0]
            }),
        )
        .unwrap();

        let path = generate(&plan, plan.program().get(0).unwrap()).unwrap();
        assert!(path.cutting_length_mm() > 0.0);
        let program = post_all(&plan).unwrap();
        assert!(program.text().contains("G21"));
        let report = simulate_gcode(&plan, &program.text(), 5.0).unwrap();
        assert!(report.removed_volume_mm3 > 0.0);
        let comparison = compare_gcode(&plan, &program.text(), 0.1).unwrap();
        assert!(comparison.issues.is_empty(), "issues: {:?}", comparison.issues);
    }
}

//! Reading GRBL back into motions, simulating a programme, and comparing it with the CAM
//! toolpath that produced it.
//!
//! The reader implements the subset in `docs/grbl-scope.md`: modal motion, feed, spindle and
//! coolant state, absolute millimetres, arcs with I/J. Unsupported or malformed blocks become
//! [`GcodeIssue`]s with their line number; the rest of the programme still runs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::generate::generate;
use crate::hybrid::CamPlan;
use crate::post::MotionMode;
use crate::simulate::{Simulation, SimulationError};
use crate::tool::Tool;
use crate::toolpath::Move;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcodeIssue {
    pub line: usize,
    pub text: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GcodeMove {
    pub line: usize,
    pub operation: Option<String>,
    pub motion: Move,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParsedGcode {
    pub moves: Vec<GcodeMove>,
    pub issues: Vec<GcodeIssue>,
    pub operations: Vec<String>,
}

/// One operation's share of a G-code programme.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationStats {
    pub operation: Option<String>,
    pub removed_volume_mm3: f64,
    pub first_line: usize,
    pub last_line: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GcodeSimulationReport {
    pub removed_volume_mm3: f64,
    pub remaining_volume_mm3: f64,
    pub collisions: usize,
    pub moves: usize,
    pub rapid_moves: usize,
    pub cutting_length_mm: f64,
    pub travel_length_mm: f64,
    pub parse_errors: Vec<GcodeIssue>,
    pub operations: Vec<OperationStats>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GcodeComparisonIssue {
    pub operation: Option<String>,
    pub first_line: usize,
    pub last_line: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GcodeComparison {
    pub removed_volume_delta_mm3: f64,
    /// Post-gcode result against the reference (design when supplied, otherwise pre-gcode).
    pub excess_volume_mm3: f64,
    pub gouge_volume_mm3: f64,
    /// Post-gcode result against the pre-gcode result.
    pub pre_post_excess_volume_mm3: f64,
    pub pre_post_gouge_volume_mm3: f64,
    /// Post-gcode result against an explicit design mesh, when one was supplied.
    pub design_excess_volume_mm3: Option<f64>,
    pub design_gouge_volume_mm3: Option<f64>,
    /// Pre-gcode result against the same design mesh.
    pub pre_design_excess_volume_mm3: Option<f64>,
    pub pre_design_gouge_volume_mm3: Option<f64>,
    pub max_deviation_mm: f64,
    pub mean_deviation_mm: f64,
    pub issues: Vec<GcodeComparisonIssue>,
    pub post: GcodeSimulationReport,
}

struct State {
    motion: Option<MotionMode>,
    position: [f64; 3],
    feed: Option<f64>,
    scale: f64,
    absolute: bool,
    operation: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self { motion: None, position: [0.0; 3], feed: None, scale: 1.0, absolute: true, operation: None }
    }
}

/// Parse a GRBL programme into motions in the setup's WCS.
pub fn parse_gcode(text: &str) -> ParsedGcode {
    let mut out = ParsedGcode::default();
    let mut state = State::default();

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let mut body = raw.to_string();
        if let Some(pos) = body.find("; Operation:") {
            let name = body[pos + "; Operation:".len()..].trim().to_string();
            if !name.is_empty() {
                if !out.operations.contains(&name) {
                    out.operations.push(name.clone());
                }
                state.operation = Some(name);
            }
        }
        // Parenthesised and semicolon comments.
        while let Some(start) = body.find('(') {
            match body[start..].find(')') {
                Some(end) => body.replace_range(start..start + end + 1, ""),
                None => {
                    body.truncate(start);
                    break;
                }
            }
        }
        if let Some(pos) = body.find(';') {
            body.truncate(pos);
        }
        let body = body.trim();
        if body.is_empty() {
            continue;
        }
        let words = tokenize(body);
        if words.is_empty() {
            continue;
        }

        let mut problem: Option<String> = None;
        let mut motion_word = false;
        // Codes first: units and modes affect the coordinate values in this block.
        for (letter, value) in &words {
            match *letter {
                'G' => {
                    let code = value.unwrap_or(0.0) as i32;
                    match code {
                        0 => {
                            state.motion = Some(MotionMode::Rapid);
                            motion_word = true;
                        }
                        1 => {
                            state.motion = Some(MotionMode::Linear);
                            motion_word = true;
                        }
                        2 => {
                            state.motion = Some(MotionMode::ArcCw);
                            motion_word = true;
                        }
                        3 => {
                            state.motion = Some(MotionMode::ArcCcw);
                            motion_word = true;
                        }
                        17 => {}
                        18 | 19 => problem = Some(format!("G{code}: arcs outside the XY plane are not supported")),
                        20 => state.scale = 25.4,
                        21 => state.scale = 1.0,
                        54 => {}
                        90 => state.absolute = true,
                        91 => problem = Some("G91: incremental distance mode is not supported".into()),
                        43 | 49 => problem = Some(format!("G{code}: tool length compensation is not supported")),
                        41 | 42 => problem = Some(format!("G{code}: cutter compensation must be applied in CAM")),
                        28 | 30 => problem = Some(format!("G{code}: machine home moves are not supported")),
                        81..=89 => problem = Some(format!("G{code}: canned cycles are not supported")),
                        _ => problem = Some(format!("G{code}: unsupported G-code")),
                    }
                }
                'M' => {
                    let code = value.unwrap_or(0.0) as i32;
                    match code {
                        3 | 5 | 8 | 9 | 30 => {}
                        6 | 98 | 99 => problem = Some(format!("M{code}: tool change/subprogram is not supported")),
                        _ => {}
                    }
                }
                'T' => problem = Some("T: no automatic tool changer".into()),
                'N' => {}
                _ => {}
            }
        }
        if let Some(reason) = problem {
            out.issues.push(GcodeIssue { line, text: raw.to_string(), reason });
            continue;
        }

        let mut coords: [Option<f64>; 3] = [None; 3];
        let mut ij: [Option<f64>; 2] = [None; 2];
        let mut feed = None;
        for (letter, value) in &words {
            let Some(value) = *value else { continue };
            match *letter {
                'X' => coords[0] = Some(value * state.scale),
                'Y' => coords[1] = Some(value * state.scale),
                'Z' => coords[2] = Some(value * state.scale),
                'I' => ij[0] = Some(value * state.scale),
                'J' => ij[1] = Some(value * state.scale),
                'F' => feed = Some(value * state.scale),
                _ => {}
            }
        }
        if let Some(feed) = feed {
            state.feed = Some(feed);
        }

        let any_coord = coords.iter().any(Option::is_some);
        if !any_coord && !motion_word {
            continue;
        }
        if !state.absolute {
            continue;
        }
        let motion = state.motion.unwrap_or(MotionMode::Rapid);
        let to = [coords[0].unwrap_or(state.position[0]), coords[1].unwrap_or(state.position[1]), coords[2].unwrap_or(state.position[2])];
        let move_ = match motion {
            MotionMode::Rapid => Move::Rapid { to },
            MotionMode::Linear => Move::Feed { to, feed_mm_min: state.feed.unwrap_or(300.0) },
            MotionMode::ArcCw | MotionMode::ArcCcw => {
                if ij[0].is_none() && ij[1].is_none() {
                    out.issues.push(GcodeIssue { line, text: raw.to_string(), reason: "arc without I/J centre".into() });
                    continue;
                }
                let centre = [state.position[0] + ij[0].unwrap_or(0.0), state.position[1] + ij[1].unwrap_or(0.0), state.position[2]];
                Move::Arc { to, centre, clockwise: motion == MotionMode::ArcCw, feed_mm_min: state.feed.unwrap_or(300.0) }
            }
        };
        out.moves.push(GcodeMove { line, operation: state.operation.clone(), motion: move_ });
        state.position = to;
    }
    out
}

fn tokenize(line: &str) -> Vec<(char, Option<f64>)> {
    let bytes = line.as_bytes();
    let mut words = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let c = bytes[index] as char;
        if c.is_ascii_alphabetic() {
            let letter = c.to_ascii_uppercase();
            index += 1;
            let start = index;
            while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.' || bytes[index] == b'-' || bytes[index] == b'+') {
                index += 1;
            }
            let value = if index > start { std::str::from_utf8(&bytes[start..index]).ok().and_then(|text| text.parse().ok()) } else { None };
            words.push((letter, value));
        } else {
            index += 1;
        }
    }
    words
}

/// Simulate a programme with the plan's first tool.
pub fn simulate_gcode(plan: &CamPlan, gcode: &str, tile_mm: f64) -> Result<GcodeSimulationReport, SimulationError> {
    let tool = plan.tools().iter().next().ok_or_else(|| SimulationError::UnknownTool("(plan has no tool)".into()))?;
    simulate_gcode_with_tool(plan, gcode, tool, tile_mm).map(|(_, report)| report)
}

/// Simulate a programme with an explicit tool.
pub fn simulate_gcode_with_tool(
    plan: &CamPlan,
    gcode: &str,
    tool: &Tool,
    tile_mm: f64,
) -> Result<(Simulation, GcodeSimulationReport), SimulationError> {
    let parsed = parse_gcode(gcode);
    let (simulation, report) = simulate_parsed(plan, &parsed, tool, tile_mm)?;
    Ok((simulation, report))
}

fn simulate_parsed(plan: &CamPlan, parsed: &ParsedGcode, tool: &Tool, tile_mm: f64) -> Result<(Simulation, GcodeSimulationReport), SimulationError> {
    let mut simulation = Simulation::new(plan.stock(), tile_mm)?;
    let axis = plan.setup().z_axis().vector();
    let mut current = [0.0; 3];
    let mut report = GcodeSimulationReport {
        removed_volume_mm3: 0.0,
        remaining_volume_mm3: 0.0,
        collisions: 0,
        moves: parsed.moves.len(),
        rapid_moves: 0,
        cutting_length_mm: 0.0,
        travel_length_mm: 0.0,
        parse_errors: parsed.issues.clone(),
        operations: Vec::new(),
    };
    let mut stats: BTreeMap<String, OperationStats> = BTreeMap::new();

    for movement in &parsed.moves {
        let to = movement.motion.end();
        match movement.motion {
            Move::Rapid { .. } => {
                report.rapid_moves += 1;
                report.travel_length_mm += movement.motion.length_from(current);
            }
            Move::Feed { .. } => {
                let segment = cutrail::MotionSegment::linear(
                    cutrail::Pose::new(plan.setup().to_model(current), axis),
                    cutrail::Pose::new(plan.setup().to_model(to), axis),
                );
                let cut = simulation.cut_segment(tool, &segment)?;
                report.removed_volume_mm3 += cut.removed_volume;
                report.collisions += usize::from(cut.collision.is_some());
                let length = movement.motion.length_from(current);
                report.cutting_length_mm += length;
                let key = movement.operation.clone().unwrap_or_else(|| "(untagged)".into());
                let entry = stats.entry(key.clone()).or_insert(OperationStats {
                    operation: movement.operation.clone(),
                    removed_volume_mm3: 0.0,
                    first_line: movement.line,
                    last_line: movement.line,
                });
                entry.removed_volume_mm3 += cut.removed_volume;
                entry.first_line = entry.first_line.min(movement.line);
                entry.last_line = entry.last_line.max(movement.line);
            }
            Move::Arc { centre, clockwise, .. } => {
                let normal = if clockwise { [-axis[0], -axis[1], -axis[2]] } else { axis };
                let segment = cutrail::MotionSegment::arc(
                    cutrail::Pose::new(plan.setup().to_model(current), axis),
                    cutrail::Pose::new(plan.setup().to_model(to), axis),
                    plan.setup().to_model(centre),
                    normal,
                );
                let cut = simulation.cut_segment(tool, &segment)?;
                report.removed_volume_mm3 += cut.removed_volume;
                report.collisions += usize::from(cut.collision.is_some());
                let length = movement.motion.length_from(current);
                report.cutting_length_mm += length;
                let key = movement.operation.clone().unwrap_or_else(|| "(untagged)".into());
                let entry = stats.entry(key.clone()).or_insert(OperationStats {
                    operation: movement.operation.clone(),
                    removed_volume_mm3: 0.0,
                    first_line: movement.line,
                    last_line: movement.line,
                });
                entry.removed_volume_mm3 += cut.removed_volume;
                entry.first_line = entry.first_line.min(movement.line);
                entry.last_line = entry.last_line.max(movement.line);
            }
        }
        current = to;
    }
    report.remaining_volume_mm3 = simulation.remaining_volume_mm3();
    report.operations = stats.into_values().collect();
    Ok((simulation, report))
}

/// Compare a posted programme with the CAM toolpaths that produced it.
///
/// The pre-gcode simulation is the reference (the design, when one is supplied). The report
/// carries volume/excess/gouge and sampled vertex deviation, plus issues attributed to the
/// operation and line range whose removal differs.
pub fn compare_gcode(plan: &CamPlan, gcode: &str, tolerance_mm: f64) -> Result<GcodeComparison, SimulationError> {
    compare_gcode_inner(plan, gcode, None, tolerance_mm)
}

/// Compare a posted programme against an explicit design mesh as well as the pre-gcode cut.
pub fn compare_gcode_with_design(
    plan: &CamPlan,
    gcode: &str,
    design: &cutrail::TriMesh,
    tolerance_mm: f64,
) -> Result<GcodeComparison, SimulationError> {
    compare_gcode_inner(plan, gcode, Some(design), tolerance_mm)
}

fn compare_gcode_inner(
    plan: &CamPlan,
    gcode: &str,
    design: Option<&cutrail::TriMesh>,
    tolerance_mm: f64,
) -> Result<GcodeComparison, SimulationError> {
    let tool = plan.tools().iter().next().ok_or_else(|| SimulationError::UnknownTool("(plan has no tool)".into()))?;
    let parsed = parse_gcode(gcode);

    let mut pre = Simulation::new(plan.stock(), 5.0)?;
    let mut pre_operations = Vec::new();
    let mut pre_total = 0.0;
    for operation in plan.program().active() {
        let tool = plan.tools().find(operation.tool()).ok_or_else(|| SimulationError::UnknownTool(operation.tool().to_string()))?;
        let path = generate(plan, operation).map_err(|error| SimulationError::Generate(error.to_string()))?;
        let report = pre.cut_path(tool, &path, plan.setup())?;
        pre_total += report.removed_volume_mm3;
        pre_operations.push((operation.name().to_string(), report.removed_volume_mm3));
    }
    let pre_mesh = pre.mesh()?;
    let (post, report) = simulate_parsed(plan, &parsed, tool, 5.0)?;
    let post_mesh = post.mesh()?;
    let pre_post = post.compare_with_design(&pre_mesh, tolerance_mm)?;
    let reference = design.unwrap_or(&pre_mesh);
    let comparison = post.compare_with_design(reference, tolerance_mm)?;
    let (design_excess, design_gouge) = match design {
        Some(mesh) => {
            let cmp = post.compare_with_design(mesh, tolerance_mm)?;
            (Some(cmp.excess_volume), Some(cmp.gouge_volume))
        }
        None => (None, None),
    };
    let (pre_design_excess, pre_design_gouge) = match design {
        Some(mesh) => {
            let cmp = pre.compare_with_design(mesh, tolerance_mm)?;
            (Some(cmp.excess_volume), Some(cmp.gouge_volume))
        }
        None => (None, None),
    };
    let (mean_deviation_mm, max_deviation_mm) = mesh_deviation(&post_mesh, reference, 400);

    let mut issues = Vec::new();
    for error in &parsed.issues {
        issues.push(GcodeComparisonIssue { operation: None, first_line: error.line, last_line: error.line, reason: error.reason.clone() });
    }
    for (name, before) in pre_operations {
        let after = report.operations.iter().find(|stats| stats.operation.as_deref() == Some(name.as_str()));
        let removed = after.map(|stats| stats.removed_volume_mm3).unwrap_or(0.0);
        let delta = before - removed;
        if delta.abs() > 0.5 {
            let (first_line, last_line) = after.map(|stats| (stats.first_line, stats.last_line)).unwrap_or((1, 1));
            issues.push(GcodeComparisonIssue {
                operation: Some(name.clone()),
                first_line,
                last_line,
                reason: format!("removed volume differs by {delta:.3} mm³ (toolpath {before:.3}, G-code {removed:.3})"),
            });
        }
    }

    Ok(GcodeComparison {
        removed_volume_delta_mm3: pre_total - report.removed_volume_mm3,
        excess_volume_mm3: comparison.excess_volume,
        gouge_volume_mm3: comparison.gouge_volume,
        pre_post_excess_volume_mm3: pre_post.excess_volume,
        pre_post_gouge_volume_mm3: pre_post.gouge_volume,
        design_excess_volume_mm3: design_excess,
        design_gouge_volume_mm3: design_gouge,
        pre_design_excess_volume_mm3: pre_design_excess,
        pre_design_gouge_volume_mm3: pre_design_gouge,
        max_deviation_mm,
        mean_deviation_mm,
        issues,
        post: report,
    })
}

fn mesh_deviation(a: &cutrail::TriMesh, b: &cutrail::TriMesh, cap: usize) -> (f64, f64) {
    let sa = sample(&a.positions, cap);
    let sb = sample(&b.positions, cap);
    if sa.is_empty() || sb.is_empty() {
        return (0.0, 0.0);
    }
    let mut sum = 0.0;
    let mut max: f64 = 0.0;
    for point in &sa {
        let mut nearest = f64::INFINITY;
        for other in &sb {
            let distance = ((point[0] - other[0]).powi(2) + (point[1] - other[1]).powi(2) + (point[2] - other[2]).powi(2)).sqrt();
            nearest = nearest.min(distance);
        }
        sum += nearest;
        max = max.max(nearest);
    }
    (sum / sa.len() as f64, max)
}

fn sample(points: &[[f64; 3]], cap: usize) -> Vec<[f64; 3]> {
    if points.len() <= cap {
        return points.to_vec();
    }
    let step = points.len().div_ceil(cap);
    points.iter().step_by(step).copied().take(cap).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Chain, Curve2D, GeometryShape};
    use crate::operation::{Operation, OperationKind};
    use crate::post::{post, post_all};
    use crate::reference::DesignRef;
    use crate::setup::Setup;
    use crate::stock::{Material, Stock};
    use crate::tool::Cutter;

    fn stock() -> Stock {
        Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap()
    }

    fn square() -> Chain {
        Chain::closed(vec![Curve2D::polyline(vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]], true).unwrap()], 1e-6).unwrap()
    }

    fn plan() -> CamPlan {
        let stock = stock();
        let setup = Setup::top_centre(&stock);
        let mut plan = CamPlan::new(DesignRef::new("bracket", 1).unwrap(), stock, setup);
        plan.add_tool(crate::tool::Tool::new("6mm flat", Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap()).unwrap();
        plan.geometry_mut().add_layer("Machining").unwrap();
        let id = plan.geometry_mut().add("Machining", GeometryShape::Chain(square())).unwrap();
        plan.add_operation(Operation::new("Contour", OperationKind::Contour { finish_passes: 1 }, "6mm flat", 2.0).unwrap().with_selection(vec![id]))
            .unwrap();
        plan
    }

    #[test]
    fn posted_programmes_round_trip_through_the_reader() {
        let plan = plan();
        let text = post(&plan, plan.program().get(0).unwrap()).unwrap().text();
        let parsed = parse_gcode(&text);
        assert!(parsed.issues.is_empty(), "issues: {:?}", parsed.issues);
        assert!(parsed.moves.len() >= plan.program().len());
        assert!(parsed.operations.contains(&"Contour".to_string()));
        assert!(parsed.moves.iter().any(|movement| matches!(movement.motion, Move::Feed { .. })));
    }

    #[test]
    fn unsupported_codes_are_reported_by_line() {
        let parsed = parse_gcode("G43 H1\nG1 X10 F300\nT2 M6\nG91 X5\n");
        assert_eq!(parsed.issues.len(), 3);
        assert_eq!(parsed.issues[0].line, 1);
        assert!(parsed.issues[0].reason.contains("tool length"));
        assert!(parsed.issues.iter().any(|issue| issue.reason.contains("incremental")));
        assert_eq!(parsed.moves.len(), 1);
    }

    #[test]
    fn gcode_simulation_removes_material() {
        let plan = plan();
        let text = post(&plan, plan.program().get(0).unwrap()).unwrap().text();
        let report = simulate_gcode(&plan, &text, 5.0).unwrap();
        assert!(report.removed_volume_mm3 > 100.0, "removed {}", report.removed_volume_mm3);
        assert_eq!(report.parse_errors.len(), 0);
        assert_eq!(report.collisions, 0);
        assert!(report.cutting_length_mm > 50.0);
        assert!(report.travel_length_mm > 0.0);
    }

    #[test]
    fn an_identical_programme_has_zero_deviation() {
        let plan = plan();
        let text = post(&plan, plan.program().get(0).unwrap()).unwrap().text();
        let comparison = compare_gcode(&plan, &text, 0.1).unwrap();
        assert!(comparison.excess_volume_mm3 < 0.5, "excess {}", comparison.excess_volume_mm3);
        assert!(comparison.gouge_volume_mm3 < 0.5, "gouge {}", comparison.gouge_volume_mm3);
        assert!(comparison.max_deviation_mm < 0.1, "max {}", comparison.max_deviation_mm);
        assert!(comparison.issues.is_empty(), "issues: {:?}", comparison.issues);
    }

    #[test]
    fn an_omitted_move_is_attributed_to_its_operation_and_lines() {
        let plan = plan();
        let text = post(&plan, plan.program().get(0).unwrap()).unwrap().text();
        let mut lines: Vec<&str> = text.lines().collect();
        let remove = lines.iter().position(|line| line.trim_start().starts_with("G1 ")).expect("a feed line");
        lines.remove(remove);
        let broken = lines.join("\n");
        let comparison = compare_gcode(&plan, &broken, 0.1).unwrap();
        assert!(!comparison.issues.is_empty(), "omitted move was not reported");
        assert!(comparison.removed_volume_delta_mm3.abs() > 0.5, "volume delta {}", comparison.removed_volume_delta_mm3);
        assert!(comparison.pre_post_excess_volume_mm3 + comparison.pre_post_gouge_volume_mm3 > 0.0, "pre/post comparison found no deviation");
        assert!(comparison.issues.iter().any(|issue| issue.first_line > 0));
        assert!(comparison.issues.iter().any(|issue| issue.operation.is_some()), "issue not attributed to an operation");
    }

    #[test]
    fn post_all_round_trips_through_comparison() {
        let plan = plan();
        let text = post_all(&plan).unwrap().text();
        let comparison = compare_gcode(&plan, &text, 0.1).unwrap();
        assert!(comparison.issues.is_empty(), "issues: {:?}", comparison.issues);
        assert!(comparison.post.removed_volume_mm3 > 100.0);
    }
}

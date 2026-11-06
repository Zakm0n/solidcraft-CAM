//! The CAM document: the levels, the wireframe drawn on them and the solid bodies.
//!
//! Geometry is never anonymous in a CAM program. Everything a user draws, models or imports
//! belongs to a level, and the level decides whether it is drawn and whether a toolpath may pick
//! it. This module is where that rule lives: [`CamDocument`] owns the levels, one sketch of
//! wireframe per level ([`solidcraft_sketch::Sketch`]) and the solid bodies built from those
//! sketches ([`solidcraft_kernel`]).
//!
//! The modeling calls are the same SolidCraft core the full CAD application uses: wireframe
//! tools add curves to a sketch, solid tools find the sketch's closed profiles and hand them to
//! the kernel, and imports read DXF entities and STEP solids into the same structures. Whatever
//! creates geometry, it is added to the *active* level, so the manager and the viewport agree on
//! what belongs where.

use std::sync::Arc;

use solidcore_record::{Plane, Region2, Vec2, Vec3};
use solidcraft_cam::stock::StockDef;
use solidcraft_geom::Mesh;
use solidcraft_geom::PlaneGeom;
use solidcraft_kernel::{Body, BoolOp, MeshTolerance};
use solidcraft_sketch::{CurveKind, Sketch, find_profiles};

/// The standard construction planes, in the order the manager lists them.
pub const STANDARD_PLANES: [&str; 6] = ["TOP", "FRONT", "RIGHT", "BACK", "LEFT", "BOTTOM"];

/// Depth of an extrusion made from the ribbon before parameter dialogs exist, in millimetres.
pub const DEFAULT_EXTRUDE_DEPTH: f64 = 12.0;

/// Angle of a revolution made from the ribbon, in degrees.
pub const DEFAULT_REVOLVE_ANGLE: f64 = 360.0;

/// Radius of a fillet made from the ribbon, in millimetres.
pub const DEFAULT_FILLET_RADIUS: f64 = 2.0;

/// Size of a chamfer made from the ribbon, in millimetres.
pub const DEFAULT_CHAMFER_DISTANCE: f64 = 1.0;

/// Most edges one ribbon fillet or chamfer rounds at once: a whole imported body is a selection
/// made elsewhere, and a blend over thousands of edges would block the window.
pub const MAX_BLEND_EDGES: usize = 64;

/// Distance between the cells new wireframe is laid out in, in millimetres: the tools draw at
/// known sizes, so successive shapes sit side by side instead of on top of each other.
const SHAPE_SPACING: f64 = 60.0;

/// Columns of the wireframe layout grid.
const SHAPE_COLUMNS: usize = 3;

/// Half-length of the cross that draws an unattached sketch point, in millimetres.
const POINT_ARM: f64 = 1.5;

/// The construction plane named by an entry of [`STANDARD_PLANES`].
pub fn standard_plane(index: usize) -> Plane {
    match index {
        1 => Plane::XZ,
        2 => Plane::YZ,
        // The opposing planes: the same frame seen from behind.
        3 => Plane { origin: Vec3::ZERO, x: -Vec3::X, y: Vec3::Z },
        4 => Plane { origin: Vec3::ZERO, x: -Vec3::Y, y: Vec3::Z },
        5 => Plane { origin: Vec3::ZERO, x: Vec3::X, y: -Vec3::Y },
        _ => Plane::XY,
    }
}

/// One level (layer): a named group of geometry with its own visibility and selectability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    /// The level's number, also its identity: numbers are never reused.
    pub id: usize,
    /// The level's name as the manager shows it.
    pub name: String,
    /// Whether the level's geometry is drawn.
    pub visible: bool,
    /// Whether the level's geometry can be picked.
    pub selectable: bool,
    /// How many entities (curves, points, bodies) the level holds.
    pub entity_count: usize,
}

impl Level {
    /// A level that starts out visible, selectable and empty.
    pub fn new(id: usize, name: impl Into<String>) -> Self {
        Self { id, name: name.into(), visible: true, selectable: true, entity_count: 0 }
    }
}

/// The wireframe tools the `Wireframe` ribbon tab offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireTool {
    /// A single sketch point.
    Point,
    /// A straight line.
    Line,
    /// A closed rectangle of four lines.
    Rectangle,
    /// A full circle.
    Circle,
    /// A quarter arc.
    Arc,
    /// A free-form spline through fit points.
    Spline,
}

impl WireTool {
    /// The tools in ribbon order.
    pub const ALL: [Self; 6] = [Self::Point, Self::Line, Self::Rectangle, Self::Circle, Self::Arc, Self::Spline];

    /// The tool's text on its ribbon button.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Point => "Point",
            Self::Line => "Line",
            Self::Rectangle => "Rectangle",
            Self::Circle => "Circle",
            Self::Arc => "Arc",
            Self::Spline => "Spline",
        }
    }

    /// How many entities one use of the tool adds to a level.
    pub const fn entity_count(self) -> usize {
        match self {
            Self::Rectangle => 4,
            _ => 1,
        }
    }
}

/// The solid modeling tools the `Solids` ribbon tab offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolidTool {
    /// Sweep the level's closed profiles along the plane normal.
    Extrude,
    /// Revolve the level's closed profiles about the sketch Y axis.
    Revolve,
    /// Subtract the last body from the one before it.
    BooleanCut,
    /// Union the last two bodies.
    BooleanUnion,
    /// Round the edges of the last body.
    Fillet,
    /// Bevel the edges of the last body.
    Chamfer,
}

impl SolidTool {
    /// The tools in ribbon order.
    pub const ALL: [Self; 6] = [Self::Extrude, Self::Revolve, Self::BooleanCut, Self::BooleanUnion, Self::Fillet, Self::Chamfer];

    /// The tool's text on its ribbon button.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Extrude => "Extrude",
            Self::Revolve => "Revolve",
            Self::BooleanCut => "Boolean Cut",
            Self::BooleanUnion => "Boolean Union",
            Self::Fillet => "Fillet",
            Self::Chamfer => "Chamfer",
        }
    }
}

/// One solid body in the document, with the display mesh the viewport draws.
#[derive(Clone, Debug)]
pub struct SolidBody {
    /// The body's identity: unique for the life of the document.
    pub id: usize,
    /// The name the `Solids` view lists, such as `Extrude 1` or an imported part's name.
    pub name: String,
    /// The level the body belongs to.
    pub level: usize,
    /// The exact B-rep the tools edit.
    pub body: Body,
    /// The body's display mesh, made once and shared with the renderer.
    pub mesh: Arc<Mesh>,
    /// Whether the body is drawn.
    pub visible: bool,
}

/// One polyline of wireframe, in world coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct WirePolyline {
    /// The level the polyline belongs to.
    pub level: usize,
    /// The polyline's points, in world coordinates.
    pub points: Vec<Vec3>,
}

/// What an import brought in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Entities added to the active level.
    pub added: usize,
    /// Entities the reader skipped (text, unsupported records).
    pub skipped: usize,
}

/// Why a CAM command did not happen. Every command reports one of these instead of failing
/// silently or crashing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CamError {
    /// A sketch refused the geometry.
    Sketch(String),
    /// The kernel refused the operation.
    Kernel(String),
    /// A file could not be read as the format it claimed.
    Import(String),
    /// The command cannot run with the document as it stands.
    Invalid(String),
}

impl std::fmt::Display for CamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sketch(message) => write!(f, "sketch: {message}"),
            Self::Kernel(message) => write!(f, "kernel: {message}"),
            Self::Import(message) => write!(f, "import: {message}"),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CamError {}

/// One level's wireframe, with the construction plane it was started on.
#[derive(Clone, Debug)]
struct LevelSketch {
    /// The level the sketch belongs to.
    level: usize,
    /// The construction plane the level's wireframe lies on: the plane active when the level
    /// first received geometry.
    plane: Plane,
    /// The exact sketch curves.
    sketch: Sketch,
    /// How many tools have drawn into the sketch, which decides where the next shape lands.
    shapes: usize,
}

/// The CAM document: levels, their wireframe and their solids.
#[derive(Debug)]
pub struct CamDocument {
    /// The levels, in number order.
    levels: Vec<Level>,
    /// The id of the level new geometry is created on.
    active_level: usize,
    /// The index into [`STANDARD_PLANES`] of the construction plane new levels draw on.
    active_plane: usize,
    /// One wireframe sketch per level that has geometry.
    sketches: Vec<LevelSketch>,
    /// The solid bodies, in creation order.
    solids: Vec<SolidBody>,
    /// The raw material the operations cut, once the user has set it up.
    stock: Option<StockDef>,
    /// Bumped on every change that alters what the viewport draws.
    revision: u64,
    /// The number the next body is named with.
    next_solid: usize,
}

impl Default for CamDocument {
    /// A new part starts on level 1, named `X`, with TOP as the construction plane.
    fn default() -> Self {
        Self {
            levels: vec![Level::new(1, "X")],
            active_level: 1,
            active_plane: 0,
            sketches: Vec::new(),
            solids: Vec::new(),
            stock: None,
            revision: 0,
            next_solid: 1,
        }
    }
}

impl CamDocument {
    /// The levels, in number order.
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// One level by number.
    pub fn level(&self, id: usize) -> Option<&Level> {
        self.levels.iter().find(|level| level.id == id)
    }

    /// Whether a level's geometry is drawn.
    pub fn level_visible(&self, id: usize) -> bool {
        self.level(id).is_some_and(|level| level.visible)
    }

    /// The id of the level new geometry is created on.
    pub fn active_level(&self) -> usize {
        self.active_level
    }

    /// The active level's name, for the status bar.
    pub fn active_level_name(&self) -> &str {
        self.level(self.active_level).map_or("", |level| level.name.as_str())
    }

    /// Makes `id` the level new geometry is created on. Unknown levels are ignored.
    pub fn set_active_level(&mut self, id: usize) {
        if self.level(id).is_some() {
            self.active_level = id;
            self.revision += 1;
        }
    }

    /// Adds a level and makes it the active creation level. Returns the new level's number.
    pub fn add_level(&mut self) -> usize {
        let id = self.levels.iter().map(|level| level.id).max().unwrap_or(0) + 1;
        self.levels.push(Level::new(id, format!("Level {id}")));
        self.active_level = id;
        self.revision += 1;
        id
    }

    /// Shows or hides a level's geometry everywhere it is drawn.
    pub fn set_level_visible(&mut self, id: usize, visible: bool) {
        if let Some(level) = self.levels.iter_mut().find(|level| level.id == id) {
            level.visible = visible;
            self.revision += 1;
        }
    }

    /// Allows or refuses picking a level's geometry.
    pub fn set_level_selectable(&mut self, id: usize, selectable: bool) {
        if let Some(level) = self.levels.iter_mut().find(|level| level.id == id) {
            level.selectable = selectable;
            self.revision += 1;
        }
    }

    /// The index into [`STANDARD_PLANES`] of the selected construction plane.
    pub fn active_plane(&self) -> usize {
        self.active_plane
    }

    /// The selected construction plane's name.
    pub fn active_plane_name(&self) -> &'static str {
        STANDARD_PLANES.get(self.active_plane).copied().unwrap_or(STANDARD_PLANES[0])
    }

    /// Selects a construction plane for the next level that receives geometry.
    pub fn set_active_plane(&mut self, index: usize) {
        if index < STANDARD_PLANES.len() {
            self.active_plane = index;
            self.revision += 1;
        }
    }

    /// The solid bodies, in creation order.
    pub fn solids(&self) -> &[SolidBody] {
        &self.solids
    }

    /// One solid body by id.
    pub fn solid_by_id(&self, id: usize) -> Option<&SolidBody> {
        self.solids.iter().find(|solid| solid.id == id)
    }

    /// The box around every solid in the document, as `(min, max)` corners: the geometry a
    /// bounding-box stock is sized from. `None` until the document holds a body.
    pub fn part_bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        self.solids.iter().filter_map(|solid| bounds_of(&solid.body)).reduce(|(min, max), (other_min, other_max)| {
            (
                [min[0].min(other_min[0]), min[1].min(other_min[1]), min[2].min(other_min[2])],
                [max[0].max(other_max[0]), max[1].max(other_max[1]), max[2].max(other_max[2])],
            )
        })
    }

    /// The stock the setup defines, if the user has set any up.
    pub fn stock(&self) -> Option<StockDef> {
        self.stock
    }

    /// Stores the stock definition, or removes it with `None`.
    pub fn set_stock(&mut self, stock: Option<StockDef>) {
        if self.stock != stock {
            self.stock = stock;
            self.revision += 1;
        }
    }

    /// The box the stock occupies in the viewport, as `[min x, min y, min z, max x, max y,
    /// max z]`: a bounding box's own extents, or the selected body's box.
    pub fn stock_extents(&self) -> Option<[f64; 6]> {
        let stock = self.stock?;
        match stock {
            StockDef::BoundingBox { .. } => stock.extents(),
            StockDef::Solid(id) => {
                let (min, max) = bounds_of(&self.solid_by_id(id)?.body)?;
                Some([min[0], min[1], min[2], max[0], max[1], max[2]])
            }
        }
    }
    /// Shows or hides a solid body.
    pub fn set_solid_visible(&mut self, id: usize, visible: bool) {
        if let Some(solid) = self.solids.iter_mut().find(|solid| solid.id == id) {
            solid.visible = visible;
            self.revision += 1;
        }
    }

    /// The document's revision: what the viewport keys its rendered picture on.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Draws one wireframe shape with `tool` on the active level. Returns the entities added.
    pub fn draw(&mut self, tool: WireTool) -> Result<usize, CamError> {
        let level = self.active_level;
        let index = self.sketch_index(level);
        let origin = shape_origin(self.sketches[index].shapes);
        draw_shape(&mut self.sketches[index].sketch, tool, origin)?;
        self.sketches[index].shapes += 1;
        let entities = tool.entity_count();
        self.count_entities(level, entities);
        self.revision += 1;
        Ok(entities)
    }

    /// Runs one solid modeling tool on the document.
    pub fn solid(&mut self, tool: SolidTool) -> Result<usize, CamError> {
        match tool {
            SolidTool::Extrude => self.extrude(DEFAULT_EXTRUDE_DEPTH),
            SolidTool::Revolve => self.revolve(DEFAULT_REVOLVE_ANGLE),
            SolidTool::BooleanCut => self.boolean(BoolOp::Cut),
            SolidTool::BooleanUnion => self.boolean(BoolOp::Union),
            SolidTool::Fillet => self.fillet(DEFAULT_FILLET_RADIUS),
            SolidTool::Chamfer => self.chamfer(DEFAULT_CHAMFER_DISTANCE),
        }
    }

    /// Extrudes every closed profile on the active level along its plane normal, from the plane
    /// up to `depth`. Returns the number of bodies made.
    pub fn extrude(&mut self, depth: f64) -> Result<usize, CamError> {
        if !(depth.is_finite() && depth.abs() > 1e-6 && depth.abs() < 1e6) {
            return Err(CamError::Invalid(format!("an extrusion of {depth} mm is not a usable depth")));
        }
        let level = self.active_level;
        let plane = self.level_plane(level);
        let regions = self.level_regions(level)?;
        let bodies = solidcraft_kernel::extrude(&plane, &regions, 0.0, depth).map_err(kernel_error)?;
        let count = bodies.len();
        for body in bodies {
            self.push_solid(body, level, "Extrude")?;
        }
        Ok(count)
    }

    /// Revolves every closed profile on the active level about the sketch Y axis through the
    /// plane origin, through `angle_degrees`. Returns the number of bodies made.
    pub fn revolve(&mut self, angle_degrees: f64) -> Result<usize, CamError> {
        if !(angle_degrees.is_finite() && angle_degrees.abs() > 1e-3 && angle_degrees.abs() <= 360.0) {
            return Err(CamError::Invalid(format!("a revolution of {angle_degrees} degrees is not usable")));
        }
        let level = self.active_level;
        let plane = self.level_plane(level);
        let regions = self.level_regions(level)?;
        let bodies =
            solidcraft_kernel::revolve(&plane, &regions, Vec2::ZERO, Vec2::new(0.0, 1.0), angle_degrees.to_radians()).map_err(kernel_error)?;
        let count = bodies.len();
        for body in bodies {
            self.push_solid(body, level, "Revolve")?;
        }
        Ok(count)
    }

    /// Combines the last two bodies with a boolean. The earlier body survives, renamed by the
    /// name it already had; the later one becomes its tool and leaves the document.
    pub fn boolean(&mut self, op: BoolOp) -> Result<usize, CamError> {
        if self.solids.len() < 2 {
            return Err(CamError::Invalid("a boolean needs two solids: create or import another body first".into()));
        }
        let tool_index = self.solids.len() - 1;
        let target_index = tool_index - 1;
        let result = {
            let (target, tool) = (&self.solids[target_index], &self.solids[tool_index]);
            solidcraft_kernel::boolean(&target.body, &tool.body, op).map_err(kernel_error)?
        };
        let Some(body) = result else {
            return Err(CamError::Invalid("the boolean leaves no body".into()));
        };
        let mesh = display_mesh(&body)?;
        let target = &mut self.solids[target_index];
        target.body = body;
        target.mesh = mesh;
        let removed = self.solids.remove(tool_index);
        self.release_entities(removed.level, 1);
        self.revision += 1;
        Ok(1)
    }

    /// Rounds every edge of the last body with `radius`. Returns the number of bodies edited.
    pub fn fillet(&mut self, radius: f64) -> Result<usize, CamError> {
        self.blend(radius, false)
    }

    /// Bevels every edge of the last body by `distance`. Returns the number of bodies edited.
    pub fn chamfer(&mut self, distance: f64) -> Result<usize, CamError> {
        self.blend(distance, true)
    }

    /// Imports ASCII DXF entities onto the active level. Returns how many were added and skipped.
    pub fn import_dxf(&mut self, text: &str, name: &str) -> Result<ImportSummary, CamError> {
        let geoms = solidcraft_io::read_dxf(text).map_err(|error| CamError::Import(error.to_string()))?;
        if geoms.is_empty() {
            return Err(CamError::Import(format!("{name} holds no entities")));
        }
        let level = self.active_level;
        let index = self.sketch_index(level);
        let mut summary = ImportSummary::default();
        for geom in geoms {
            if add_geom(&mut self.sketches[index].sketch, geom)? {
                summary.added += 1;
            } else {
                summary.skipped += 1;
            }
        }
        if summary.added == 0 {
            return Err(CamError::Import(format!("{name} holds nothing this reader can draw")));
        }
        self.count_entities(level, summary.added);
        self.revision += 1;
        Ok(summary)
    }

    /// Imports the solids of a STEP file onto the active level. Returns how many were added.
    pub fn import_step(&mut self, text: &str, name: &str) -> Result<ImportSummary, CamError> {
        let import = solidcraft_kernel::step_import(text).map_err(|error| CamError::Import(error.to_string()))?;
        if import.bodies.is_empty() {
            return Err(CamError::Import(format!("{name} holds no solids")));
        }
        let level = self.active_level;
        let mut summary = ImportSummary::default();
        for imported in import.bodies {
            let base = if imported.name.is_empty() { name.to_string() } else { imported.name.clone() };
            self.push_solid(imported.body, level, &base)?;
            summary.added += 1;
        }
        Ok(summary)
    }

    /// The wireframe of every visible level, in world coordinates.
    pub fn wireframe_lines(&self) -> Vec<WirePolyline> {
        let mut out = Vec::new();
        for level_sketch in &self.sketches {
            if !self.level_visible(level_sketch.level) {
                continue;
            }
            let plane = level_sketch.plane;
            for (index, curve) in level_sketch.sketch.curves.iter().enumerate() {
                if curve.construction {
                    continue;
                }
                let points: Vec<Vec3> = level_sketch.sketch.polyline(index).iter().map(|point| plane.to_world(*point)).collect();
                if points.len() >= 2 {
                    out.push(WirePolyline { level: level_sketch.level, points });
                }
            }
            // Points no curve uses are drawn as a small cross on the plane.
            for (index, point) in level_sketch.sketch.points.iter().enumerate() {
                let used = level_sketch.sketch.curves.iter().any(|curve| curve.kind.uses(index));
                if index == 0 || used {
                    continue;
                }
                let at = plane.to_world(point.pos);
                for arm in [plane.x * POINT_ARM, plane.y * POINT_ARM] {
                    out.push(WirePolyline { level: level_sketch.level, points: vec![at - arm, at + arm] });
                }
            }
        }
        out
    }

    /// The plane a level's wireframe lies on: the plane it was started on, or the selected
    /// construction plane before the level has geometry.
    fn level_plane(&self, level: usize) -> Plane {
        self.sketches.iter().find(|sketch| sketch.level == level).map_or_else(|| standard_plane(self.active_plane), |sketch| sketch.plane)
    }

    /// The index of a level's sketch, made on first use with the selected construction plane.
    fn sketch_index(&mut self, level: usize) -> usize {
        if let Some(index) = self.sketches.iter().position(|sketch| sketch.level == level) {
            return index;
        }
        let plane = standard_plane(self.active_plane);
        self.sketches.push(LevelSketch { level, plane, sketch: Sketch::new(), shapes: 0 });
        self.sketches.len() - 1
    }

    /// The closed profiles of a level's sketch, as kernel regions.
    fn level_regions(&self, level: usize) -> Result<Vec<Region2>, CamError> {
        let Some(level_sketch) = self.sketches.iter().find(|sketch| sketch.level == level) else {
            return Err(CamError::Invalid(format!("level {level} has no wireframe to build from")));
        };
        let profiles = find_profiles(&level_sketch.sketch);
        if profiles.is_empty() {
            return Err(CamError::Invalid(format!("level {level} has no closed profile to build from")));
        }
        Ok(profiles.into_iter().map(|profile| profile.region).collect())
    }

    /// Adds a body to the document and its level, with a name numbered by the document.
    fn push_solid(&mut self, body: Body, level: usize, base: &str) -> Result<usize, CamError> {
        let mesh = display_mesh(&body)?;
        let id = self.next_solid;
        self.next_solid += 1;
        let name = format!("{base} {id}");
        self.solids.push(SolidBody { id, name, level, body, mesh, visible: true });
        self.count_entities(level, 1);
        self.revision += 1;
        Ok(id)
    }

    /// Adds `count` to a level's entity count.
    fn count_entities(&mut self, id: usize, count: usize) {
        if let Some(level) = self.levels.iter_mut().find(|level| level.id == id) {
            level.entity_count = level.entity_count.saturating_add(count);
        }
    }

    /// Takes `count` entities away from a level's entity count.
    fn release_entities(&mut self, id: usize, count: usize) {
        if let Some(level) = self.levels.iter_mut().find(|level| level.id == id) {
            level.entity_count = level.entity_count.saturating_sub(count);
        }
    }

    /// Rounds (`chamfer` false) or bevels (`chamfer` true) every edge of the last body.
    fn blend(&mut self, size: f64, chamfer: bool) -> Result<usize, CamError> {
        if !(size.is_finite() && size > 1e-6 && size < 1e3) {
            return Err(CamError::Invalid(format!("a {size} mm blend is not a usable size")));
        }
        let Some(last) = self.solids.last() else {
            return Err(CamError::Invalid("there is no solid to blend: create or import a body first".into()));
        };
        let body = last.body.clone();
        let edges = body.edges(0.01).map_err(kernel_error)?;
        if edges.is_empty() {
            return Err(CamError::Invalid("the body has no edges to blend".into()));
        }
        if edges.len() > MAX_BLEND_EDGES {
            return Err(CamError::Invalid(format!("the body has {} edges; a ribbon blend takes at most {MAX_BLEND_EDGES}", edges.len())));
        }
        let picks: Vec<Vec3> = edges.iter().map(|edge| edge.mid).collect();
        let result = if chamfer { solidcraft_kernel::chamfer(&body, &picks, size) } else { solidcraft_kernel::fillet(&body, &picks, size) };
        let blended = result.map_err(kernel_error)?;
        let mesh = display_mesh(&blended)?;
        if let Some(last) = self.solids.last_mut() {
            last.body = blended;
            last.mesh = mesh;
        }
        self.revision += 1;
        Ok(1)
    }
}

/// A solid's display mesh at the display tolerance.
fn display_mesh(body: &Body) -> Result<Arc<Mesh>, CamError> {
    let tolerance = MeshTolerance::display_of(body.size().max(1.0));
    body.mesh(&tolerance).map(Arc::new).map_err(kernel_error)
}

/// The box around a body's exact vertices and edge hull, as `(min, max)` corners. `None` for a
/// body with no finite geometry, so callers never size stock from an empty box.
fn bounds_of(body: &Body) -> Option<([f64; 3], [f64; 3])> {
    let bounds = body.outline_bounds();
    if bounds.is_empty() {
        return None;
    }
    let (min, max) = (bounds.min, bounds.max);
    if ![min.x, min.y, min.z, max.x, max.y, max.z].iter().all(|value| value.is_finite()) {
        return None;
    }
    Some(([min.x, min.y, min.z], [max.x, max.y, max.z]))
}

/// A kernel failure as a command error.
fn kernel_error(error: solidcraft_kernel::KernelError) -> CamError {
    CamError::Kernel(error.to_string())
}

/// Where a new wireframe shape lands: cells of the sketch's layout grid, so shapes do not sit
/// on top of one another.
fn shape_origin(index: usize) -> Vec2 {
    let column = index % SHAPE_COLUMNS;
    let row = index / SHAPE_COLUMNS;
    Vec2::new(10.0 + SHAPE_SPACING * column as f64, 10.0 + SHAPE_SPACING * row as f64)
}

/// Draws one wireframe shape into `sketch` at `origin`.
fn draw_shape(sketch: &mut Sketch, tool: WireTool, origin: Vec2) -> Result<(), CamError> {
    let refused = |error: solidcraft_sketch::SketchError| CamError::Sketch(error.to_string());
    match tool {
        WireTool::Point => {
            sketch.add_point(origin, None).map_err(refused)?;
        }
        WireTool::Line => {
            sketch.add_line(origin, origin + Vec2::new(40.0, 0.0), None, None, None).map_err(refused)?;
        }
        WireTool::Rectangle => {
            let corners = [origin, origin + Vec2::new(40.0, 0.0), origin + Vec2::new(40.0, 30.0), origin + Vec2::new(0.0, 30.0)];
            for (from, to) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
                sketch.add_line(corners[from], corners[to], None, None, None).map_err(refused)?;
            }
        }
        WireTool::Circle => {
            sketch.add_circle(origin + Vec2::new(20.0, 15.0), 15.0, None, None).map_err(refused)?;
        }
        WireTool::Arc => {
            let centre = origin + Vec2::new(20.0, 15.0);
            sketch.add_arc(centre, centre + Vec2::new(15.0, 0.0), centre + Vec2::new(0.0, 15.0), [None; 3], None).map_err(refused)?;
        }
        WireTool::Spline => {
            let offsets = [(0.0, 0.0), (10.0, 12.0), (20.0, -6.0), (30.0, 10.0), (40.0, 0.0)];
            let mut points = Vec::with_capacity(offsets.len());
            for (x, y) in offsets {
                points.push(sketch.add_point(origin + Vec2::new(x, y), None).map_err(refused)?);
            }
            let kind = CurveKind::Spline { pts: points, control: false, degree: 3, periodic: false, nurbs: None };
            sketch.add_curve(kind, None).map_err(refused)?;
        }
    }
    Ok(())
}

/// Adds one imported 2D entity to `sketch`. Returns whether it was drawn: text is read but has
/// nowhere to go in a wireframe sketch yet.
fn add_geom(sketch: &mut Sketch, geom: solidcraft_io::Geom2) -> Result<bool, CamError> {
    use solidcraft_io::Geom2;

    let refused = |error: solidcraft_sketch::SketchError| CamError::Sketch(error.to_string());
    match geom {
        Geom2::Point(point) => {
            sketch.add_point(point, None).map_err(refused)?;
        }
        Geom2::Line(from, to) => {
            sketch.add_line(from, to, None, None, None).map_err(refused)?;
        }
        Geom2::Circle(centre, radius) => {
            sketch.add_circle(centre, radius, None, None).map_err(refused)?;
        }
        Geom2::Arc { c, a, b } => {
            sketch.add_arc(c, a, b, [None; 3], None).map_err(refused)?;
        }
        Geom2::Ellipse { c, major, minor } => {
            let centre = sketch.add_point(c, None).map_err(refused)?;
            let end = sketch.add_point(c + major, None).map_err(refused)?;
            sketch.add_curve(CurveKind::Ellipse { c: centre, m: end, r: minor }, None).map_err(refused)?;
        }
        Geom2::Spline { pts, control, degree } => {
            let mut points = Vec::with_capacity(pts.len());
            for point in pts {
                points.push(sketch.add_point(point, None).map_err(refused)?);
            }
            let kind = CurveKind::Spline { pts: points, control, degree, periodic: false, nurbs: None };
            sketch.add_curve(kind, None).map_err(refused)?;
        }
        Geom2::Conic { a, apex, b, rho } => {
            let start = sketch.add_point(a, None).map_err(refused)?;
            let shoulder = sketch.add_point(apex, None).map_err(refused)?;
            let end = sketch.add_point(b, None).map_err(refused)?;
            sketch.add_curve(CurveKind::Conic { a: start, b: end, apex: shoulder, rho }, None).map_err(refused)?;
        }
        Geom2::Text { .. } => return Ok(false),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small DXF drawing: one line and one circle.
    const DXF: &str = "0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n40\n21\n0\n0\nCIRCLE\n10\n20\n20\n15\n40\n10\n0\nENDSEC\n0\nEOF\n";

    #[test]
    fn a_new_document_starts_on_level_one_named_x() {
        let document = CamDocument::default();
        assert_eq!(document.levels(), &[Level::new(1, "X")]);
        assert_eq!(document.active_level(), 1);
        assert_eq!(document.active_plane_name(), "TOP");
        assert!(document.solids().is_empty());
        assert_eq!(document.revision(), 0);
        assert!(document.levels()[0].visible && document.levels()[0].selectable);
    }

    #[test]
    fn a_new_level_is_numbered_next_and_becomes_the_creation_level() {
        let mut document = CamDocument::default();
        assert_eq!(document.add_level(), 2);
        assert_eq!(document.add_level(), 3);
        assert_eq!(document.levels().iter().map(|level| level.id).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(document.levels()[2].name, "Level 3");
        assert_eq!(document.active_level(), 3);
    }

    #[test]
    fn wireframe_tools_draw_on_the_active_level_and_change_the_revision() {
        let mut document = CamDocument::default();
        assert_eq!(document.draw(WireTool::Rectangle), Ok(4));
        assert_eq!(document.draw(WireTool::Circle), Ok(1));
        assert_eq!(document.level(1).map(|level| level.entity_count), Some(5));
        assert_eq!(document.wireframe_lines().len(), 5, "four rectangle lines and one circle, each a polyline");
        assert!(document.revision() > 0);

        document.add_level();
        assert_eq!(document.draw(WireTool::Line), Ok(1));
        assert_eq!(document.level(1).map(|level| level.entity_count), Some(5));
        assert_eq!(document.level(2).map(|level| level.entity_count), Some(1));
    }

    #[test]
    fn hiding_a_level_hides_its_wireframe() {
        let mut document = CamDocument::default();
        document.draw(WireTool::Rectangle).unwrap_or_default();
        assert!(!document.wireframe_lines().is_empty());
        document.set_level_visible(1, false);
        assert!(document.wireframe_lines().is_empty());
        document.set_level_visible(1, true);
        assert_eq!(document.wireframe_lines().len(), 4);
    }

    #[test]
    fn extruding_a_profiles_sketch_adds_a_solid_to_the_active_level() {
        let mut document = CamDocument::default();
        document.draw(WireTool::Rectangle).unwrap_or_default();
        assert_eq!(document.extrude(10.0), Ok(1));
        let solids = document.solids();
        assert_eq!(solids.len(), 1);
        assert_eq!(solids[0].level, 1);
        assert_eq!(solids[0].name, "Extrude 1");
        assert!(solids[0].visible);
        assert!(!solids[0].mesh.triangles.is_empty(), "the body has a display mesh");
        assert_eq!(document.level(1).map(|level| level.entity_count), Some(5), "four curves and one body");
    }

    #[test]
    fn a_second_level_builds_only_its_own_wireframe() {
        let mut document = CamDocument::default();
        document.draw(WireTool::Rectangle).unwrap_or_default();
        document.extrude(10.0).unwrap_or_default();
        document.add_level();
        assert_eq!(document.extrude(10.0), Err(CamError::Invalid("level 2 has no wireframe to build from".into())));
        document.draw(WireTool::Circle).unwrap_or_default();
        assert_eq!(document.extrude(8.0), Ok(1));
        assert_eq!(document.solids().iter().map(|solid| solid.level).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn a_revolution_makes_a_solid_from_a_profile_off_the_axis() {
        let mut document = CamDocument::default();
        document.draw(WireTool::Circle).unwrap_or_default();
        assert_eq!(document.revolve(360.0), Ok(1));
        assert_eq!(document.solids().len(), 1);
        assert_eq!(document.solids()[0].name, "Revolve 1");
    }

    #[test]
    fn a_boolean_needs_two_bodies_and_leaves_one() {
        let mut document = CamDocument::default();
        document.draw(WireTool::Rectangle).unwrap_or_default();
        document.extrude(10.0).unwrap_or_default();
        assert_eq!(document.boolean(BoolOp::Cut), Err(CamError::Invalid("a boolean needs two solids: create or import another body first".into())));
        // A round boss on its own level overlaps the plate, so the cut leaves the plate.
        document.add_level();
        document.draw(WireTool::Circle).unwrap_or_default();
        document.extrude(10.0).unwrap_or_default();
        assert_eq!(document.solids().len(), 2);
        assert_eq!(document.boolean(BoolOp::Cut), Ok(1));
        assert_eq!(document.solids().len(), 1);
        assert_eq!(document.solids()[0].name, "Extrude 1", "the target keeps its name");
        assert_eq!(document.solids()[0].level, 1);
        assert_eq!(document.level(2).map(|level| level.entity_count), Some(1), "the tool body left level 2");
    }

    #[test]
    fn a_dxf_import_lands_on_the_active_level() {
        let mut document = CamDocument::default();
        document.add_level();
        let summary = document.import_dxf(DXF, "plate.dxf").unwrap_or_default();
        assert_eq!(summary, ImportSummary { added: 2, skipped: 0 });
        assert_eq!(document.level(2).map(|level| level.entity_count), Some(2));
        assert_eq!(document.level(1).map(|level| level.entity_count), Some(0));
        assert_eq!(document.wireframe_lines().len(), 2, "the level's line and circle are drawn");
        assert_eq!(document.wireframe_lines()[0].level, 2);
    }

    #[test]
    fn a_dxf_import_without_entities_is_an_error() {
        let mut document = CamDocument::default();
        let error = document.import_dxf("0\nSECTION\n2\nENTITIES\n0\nENDSEC\n0\nEOF\n", "empty.dxf");
        assert!(error.is_err());
    }

    #[test]
    fn construction_planes_follow_the_manager_list() {
        assert_eq!(standard_plane(0), Plane::XY);
        assert_eq!(standard_plane(1), Plane::XZ);
        assert_eq!(standard_plane(2), Plane::YZ);
        assert_eq!(standard_plane(3).y, Vec3::Z);
        assert_eq!(standard_plane(5).y, -Vec3::Y);
        let mut document = CamDocument::default();
        document.set_active_plane(1);
        assert_eq!(document.active_plane_name(), "FRONT");
        document.draw(WireTool::Rectangle).unwrap_or_default();
        assert!(document.wireframe_lines().iter().all(|line| line.points.iter().all(|point| point.y == 0.0)), "the rectangle lies on FRONT");
    }

    #[test]
    fn part_bounds_cover_every_body() {
        let mut document = CamDocument::default();
        assert_eq!(document.part_bounds(), None, "an empty document has no part to size stock from");
        document.draw(WireTool::Rectangle).unwrap_or_default();
        document.extrude(10.0).unwrap_or_default();
        let (min, max) = document.part_bounds().unwrap_or_default();
        assert!((min[0] - 10.0).abs() < 1e-6 && (min[1] - 10.0).abs() < 1e-6, "the rectangle starts at (10, 10), got {min:?}");
        assert!((max[0] - 50.0).abs() < 1e-6 && (max[1] - 40.0).abs() < 1e-6, "and is 40 by 30, got {max:?}");
        assert!((max[2] - 10.0).abs() < 1e-6, "the extrusion is 10 mm high");
    }

    #[test]
    fn stock_is_stored_and_its_box_follows_the_definition() {
        let mut document = CamDocument::default();
        assert_eq!(document.stock(), None);
        document.draw(WireTool::Rectangle).unwrap_or_default();
        document.extrude(10.0).unwrap_or_default();
        let (min, max) = document.part_bounds().unwrap_or_default();
        let boxed = StockDef::from_bounds(min, max, solidcraft_cam::stock::StockExcess::new(2.0, 2.0, 1.0));
        let revision = document.revision();
        document.set_stock(Some(boxed));
        assert_eq!(document.stock(), Some(boxed));
        assert!(document.revision() > revision, "setting stock redraws the viewport");
        let extents = document.stock_extents().unwrap_or_default();
        assert!((extents[0] - (min[0] - 2.0)).abs() < 1e-4 && (extents[5] - (max[2] + 1.0)).abs() < 1e-4);
        document.set_stock(None);
        assert_eq!(document.stock(), None);
        assert_eq!(document.stock_extents(), None);
    }

    #[test]
    fn a_solid_stock_takes_its_box_from_that_body() {
        let mut document = CamDocument::default();
        // The second rectangle lands in the next cell of the layout grid, beside the first, and
        // the one extrusion makes a body of each closed profile.
        document.draw(WireTool::Rectangle).unwrap_or_default();
        document.draw(WireTool::Rectangle).unwrap_or_default();
        assert_eq!(document.extrude(10.0), Ok(2));
        assert_eq!(document.solids().len(), 2);
        let first = document.solids()[0].id;
        document.set_stock(Some(StockDef::Solid(first)));
        let extents = document.stock_extents().unwrap_or_default();
        assert!((extents[0] - 10.0).abs() < 1e-4 && (extents[3] - 50.0).abs() < 1e-4, "the first body's box, got {extents:?}");
        assert!((extents[5] - 10.0).abs() < 1e-4);
        let (_, part_max) = document.part_bounds().unwrap_or_default();
        assert!(part_max[0] > 100.0, "the whole part reaches past the first body, got {part_max:?}");
        document.set_stock(Some(StockDef::Solid(0)));
        assert_eq!(document.stock_extents(), None, "an unknown body has no box");
    }
}

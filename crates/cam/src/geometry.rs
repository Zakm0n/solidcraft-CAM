//! Machining geometry: the curves, chains and regions a CAM project machines.
//!
//! This is CAM-owned geometry, not the CAD model: a project keeps its own wireframe on layers,
//! and may also hold a *linked* copy of design geometry so it can follow the model. Curves are
//! exact line/arc segments (a polyline is a convenience curve); chains are ordered curves that
//! meet end to start; regions are a closed outer chain minus closed islands.
//!
//! Coordinates are 2D in the setup plane, millimetres.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type Point2 = [f64; 2];

const JOIN_TOL: f64 = 1e-7;

#[derive(Debug, Error, PartialEq)]
pub enum GeometryError {
    #[error("curve coordinate is not finite")]
    NotFinite,
    #[error("arc radius must be finite and greater than zero, got {0}")]
    BadRadius(f64),
    #[error("arc sweep must be finite and non-zero, got {0}")]
    BadSweep(f64),
    #[error("a polyline needs at least two points")]
    EmptyPolyline,
    #[error("a chain needs at least one curve")]
    EmptyChain,
    #[error("curve {index} starts {gap} mm from the previous curve's end (tolerance {tolerance} mm)")]
    Gap { index: usize, gap: f64, tolerance: f64 },
    #[error("a closed chain must end where it starts")]
    NotClosed,
    #[error("a region's outer chain must be closed")]
    RegionNotClosed,
    #[error("layer `{0}` already exists")]
    DuplicateLayer(String),
    #[error("layer `{0}` does not exist")]
    UnknownLayer(String),
    #[error("geometry {0:?} does not exist")]
    UnknownGeometry(GeometryId),
    #[error("geometry {0:?} is not a chain or curve")]
    NotAChain(GeometryId),
    #[error("geometry {0:?} is not a single curve")]
    NotASingleCurve(GeometryId),
    #[error("an island is not inside the outer chain")]
    IslandNotInside,
    #[error("a region needs at least one closed chain")]
    EmptyRegion,
}

/// One curve in the setup plane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Curve2D {
    Line {
        from: Point2,
        to: Point2,
    },
    /// A circular arc from `start_angle`, turning `sweep` radians (positive is counter-clockwise).
    Arc {
        center: Point2,
        radius: f64,
        start_angle: f64,
        sweep: f64,
    },
    /// A convenience polyline; closed means the last point joins the first.
    Polyline {
        points: Vec<Point2>,
        #[serde(default)]
        closed: bool,
    },
}

impl Curve2D {
    pub fn line(from: Point2, to: Point2) -> Result<Self, GeometryError> {
        finite(from)?;
        finite(to)?;
        Ok(Curve2D::Line { from, to })
    }

    pub fn arc(center: Point2, radius: f64, start_angle: f64, sweep: f64) -> Result<Self, GeometryError> {
        finite(center)?;
        if !radius.is_finite() || radius <= 0.0 {
            return Err(GeometryError::BadRadius(radius));
        }
        if !start_angle.is_finite() || !sweep.is_finite() || sweep.abs() < 1e-12 {
            return Err(GeometryError::BadSweep(sweep));
        }
        Ok(Curve2D::Arc { center, radius, start_angle, sweep })
    }

    pub fn polyline(points: Vec<Point2>, closed: bool) -> Result<Self, GeometryError> {
        if points.len() < 2 {
            return Err(GeometryError::EmptyPolyline);
        }
        for point in &points {
            finite(*point)?;
        }
        Ok(Curve2D::Polyline { points, closed })
    }

    pub fn start(&self) -> Point2 {
        match self {
            Curve2D::Line { from, .. } => *from,
            Curve2D::Arc { center, radius, start_angle, .. } => [center[0] + radius * start_angle.cos(), center[1] + radius * start_angle.sin()],
            Curve2D::Polyline { points, .. } => points[0],
        }
    }

    pub fn end(&self) -> Point2 {
        match self {
            Curve2D::Line { to, .. } => *to,
            Curve2D::Arc { center, radius, start_angle, sweep } => {
                let a = start_angle + sweep;
                [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
            }
            Curve2D::Polyline { points, .. } => *points.last().unwrap_or(&points[0]),
        }
    }

    pub fn length(&self) -> f64 {
        match self {
            Curve2D::Line { from, to } => distance(*from, *to),
            Curve2D::Arc { radius, sweep, .. } => radius * sweep.abs(),
            Curve2D::Polyline { points, closed } => {
                let mut total = 0.0;
                for pair in points.windows(2) {
                    total += distance(pair[0], pair[1]);
                }
                if *closed && let (Some(first), Some(last)) = (points.first(), points.last()) {
                    total += distance(*last, *first);
                }
                total
            }
        }
    }

    pub fn is_closed(&self) -> bool {
        match self {
            Curve2D::Polyline { closed, .. } => *closed,
            Curve2D::Arc { sweep, .. } => (sweep.abs() - std::f64::consts::TAU).abs() < 1e-9,
            Curve2D::Line { from, to } => distance(*from, *to) < JOIN_TOL,
        }
    }

    pub fn reversed(&self) -> Self {
        match self {
            Curve2D::Line { from, to } => Curve2D::Line { from: *to, to: *from },
            Curve2D::Arc { center, radius, start_angle, sweep } => {
                Curve2D::Arc { center: *center, radius: *radius, start_angle: start_angle + sweep, sweep: -sweep }
            }
            Curve2D::Polyline { points, closed } => {
                let mut points = points.clone();
                points.reverse();
                Curve2D::Polyline { points, closed: *closed }
            }
        }
    }

    /// Points along the curve, first to last, within `tol` chord error (arcs only).
    pub fn tessellate(&self, tol: f64) -> Vec<Point2> {
        match self {
            Curve2D::Line { from, to } => vec![*from, *to],
            Curve2D::Polyline { points, .. } => points.clone(),
            Curve2D::Arc { center, radius, start_angle, sweep } => {
                let step = arc_step(*radius, tol);
                let count = ((sweep.abs() / step).ceil() as usize).max(1);
                let mut points = Vec::with_capacity(count + 1);
                for i in 0..=count {
                    let a = start_angle + sweep * (i as f64 / count as f64);
                    points.push([center[0] + radius * a.cos(), center[1] + radius * a.sin()]);
                }
                if (sweep.abs() - std::f64::consts::TAU).abs() < 1e-9
                    && let (Some(first), Some(last)) = (points.first().copied(), points.last().copied())
                    && distance(first, last) > JOIN_TOL
                {
                    points.push(first);
                }
                points
            }
        }
    }
}

fn arc_step(radius: f64, tol: f64) -> f64 {
    let tol = tol.max(1e-6);
    if radius <= tol {
        return std::f64::consts::FRAC_PI_8;
    }
    let step = 2.0 * (1.0 - tol / radius).clamp(-1.0, 1.0).acos();
    step.clamp(1e-4, std::f64::consts::FRAC_PI_8)
}

/// An ordered run of curves: open, or closed when it ends where it starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chain {
    curves: Vec<Curve2D>,
    closed: bool,
}

impl Chain {
    /// Join curves in order, requiring end-to-start continuity within `gap_tol`.
    pub fn join(curves: Vec<Curve2D>, gap_tol: f64) -> Result<Self, GeometryError> {
        if curves.is_empty() {
            return Err(GeometryError::EmptyChain);
        }
        for (index, pair) in curves.windows(2).enumerate() {
            let gap = distance(pair[0].end(), pair[1].start());
            if gap > gap_tol {
                return Err(GeometryError::Gap { index: index + 1, gap, tolerance: gap_tol });
            }
        }
        let closed = curves.len() == 1 && curves[0].is_closed() || distance(curves[0].start(), curves[curves.len() - 1].end()) <= gap_tol;
        Ok(Self { curves, closed })
    }

    /// Join curves in any order, flipping each one whose end meets the growing chain, so a
    /// selection of individually drawn or picked curves becomes one path.
    pub fn orient_and_join(mut curves: Vec<Curve2D>, gap_tol: f64) -> Result<Self, GeometryError> {
        if curves.is_empty() {
            return Err(GeometryError::EmptyChain);
        }
        let mut ordered = vec![curves.remove(0)];
        while !curves.is_empty() {
            let tail = ordered[ordered.len() - 1].end();
            let head = ordered[0].start();
            if let Some((index, reverse)) = best_match(&curves, tail, gap_tol) {
                let curve = curves.remove(index);
                ordered.push(if reverse { curve.reversed() } else { curve });
                continue;
            }
            if let Some((index, reverse)) = best_match(&curves, head, gap_tol) {
                let curve = curves.remove(index);
                let curve = if reverse { curve.reversed() } else { curve };
                ordered.insert(0, curve);
                continue;
            }
            let gap = curves.iter().map(|curve| distance(curve.start(), tail).min(distance(curve.end(), tail))).fold(f64::INFINITY, f64::min);
            return Err(GeometryError::Gap { index: ordered.len(), gap, tolerance: gap_tol });
        }
        let closed = distance(ordered[0].start(), ordered[ordered.len() - 1].end()) <= gap_tol;
        Ok(Self { curves: ordered, closed })
    }

    /// A chain that must be closed.
    pub fn closed(curves: Vec<Curve2D>, gap_tol: f64) -> Result<Self, GeometryError> {
        let chain = Self::join(curves, gap_tol)?;
        if !chain.closed {
            return Err(GeometryError::NotClosed);
        }
        Ok(chain)
    }

    /// An open chain, even if it happens to close.
    pub fn open(curves: Vec<Curve2D>, gap_tol: f64) -> Result<Self, GeometryError> {
        let mut chain = Self::join(curves, gap_tol)?;
        chain.closed = false;
        Ok(chain)
    }

    /// An open chain from a 3D edge polyline, projected onto the setup plane.
    pub fn from_edge_points(points: &[solidcraft_geom::Vec3], setup: &crate::setup::Setup) -> Result<Self, GeometryError> {
        if points.len() < 2 {
            return Err(GeometryError::EmptyPolyline);
        }
        let projected = project_points(points, setup);
        Self::open(vec![Curve2D::polyline(projected, false)?], 1e-6)
    }

    /// A closed chain from a planar face's outer boundary, projected onto the setup plane.
    pub fn from_face_boundary(points: &[solidcraft_geom::Vec3], setup: &crate::setup::Setup) -> Result<Self, GeometryError> {
        if points.len() < 3 {
            return Err(GeometryError::EmptyPolyline);
        }
        let projected = project_points(points, setup);
        Self::closed(vec![Curve2D::polyline(projected, true)?], 1e-6)
    }

    pub fn from_loop2(loop2: &solidcraft_geom::Loop2) -> Self {
        use solidcraft_geom::Seg2;

        let curves = loop2
            .segs
            .iter()
            .map(|seg| match seg {
                Seg2::Line { a, b } => Curve2D::Line { from: [a.x, a.y], to: [b.x, b.y] },
                Seg2::Arc { center, radius, start, sweep } => {
                    Curve2D::Arc { center: [center.x, center.y], radius: *radius, start_angle: *start, sweep: *sweep }
                }
                // Curves without an exact line/arc form are kept as their polyline approximation.
                other => Curve2D::Polyline { points: other.polyline(0.01).into_iter().map(|point| [point.x, point.y]).collect(), closed: false },
            })
            .collect();
        Self { curves, closed: true }
    }

    pub fn curves(&self) -> &[Curve2D] {
        &self.curves
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn start(&self) -> Point2 {
        self.curves[0].start()
    }

    pub fn end(&self) -> Point2 {
        self.curves[self.curves.len() - 1].end()
    }

    pub fn length(&self) -> f64 {
        self.curves.iter().map(Curve2D::length).sum()
    }

    pub fn reversed(&self) -> Chain {
        Chain { curves: self.curves.iter().rev().map(Curve2D::reversed).collect(), closed: self.closed }
    }

    pub fn segment(&self, index: usize) -> Option<&Curve2D> {
        self.curves.get(index)
    }

    /// Points along the chain with duplicate joints removed.
    pub fn tessellate(&self, tol: f64) -> Vec<Point2> {
        let mut points = Vec::new();
        for curve in &self.curves {
            for point in curve.tessellate(tol) {
                if points.last().map(|last| distance(*last, point) > JOIN_TOL).unwrap_or(true) {
                    points.push(point);
                }
            }
        }
        points
    }

    pub fn bounds(&self) -> (Point2, Point2) {
        let points = self.tessellate(0.01);
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for point in points {
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        (min, max)
    }

    /// Enclosed area (absolute), from the polyline approximation.
    pub fn area(&self, tol: f64) -> f64 {
        self.signed_area(tol).abs()
    }

    /// Signed area: positive when the chain runs counter-clockwise. Clipper returns islands as
    /// reversed loops, so a region's area is the signed sum, not the sum of absolute areas.
    pub fn signed_area(&self, tol: f64) -> f64 {
        let points = self.tessellate(tol);
        if points.len() < 3 {
            return 0.0;
        }
        let mut sum = 0.0;
        for i in 0..points.len() {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            sum += a[0] * b[1] - b[0] * a[1];
        }
        sum / 2.0
    }

    /// Even-odd containment of a point, on the closed chain.
    pub fn contains(&self, point: Point2) -> bool {
        let points = self.tessellate(0.01);
        let mut inside = false;
        let n = points.len();
        for i in 0..n {
            let a = points[i];
            let b = points[(i + 1) % n];
            if (a[1] > point[1]) != (b[1] > point[1]) {
                let t = (point[1] - a[1]) / (b[1] - a[1]);
                if point[0] < a[0] + t * (b[0] - a[0]) {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

/// A linked geometry item the design could not refresh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeometryIssue {
    pub id: GeometryId,
    pub reference: crate::reference::EntityRef,
}

/// A closed outer chain with closed islands cut out of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    outer: Chain,
    #[serde(default)]
    islands: Vec<Chain>,
}

impl Region {
    /// Nest closed chains: the largest becomes the outer boundary, the rest islands. Every
    /// island must lie inside the outer chain.
    pub fn nest(chains: Vec<Chain>) -> Result<Self, GeometryError> {
        let mut chains = chains;
        if chains.is_empty() {
            return Err(GeometryError::EmptyRegion);
        }
        if chains.iter().any(|chain| !chain.is_closed()) {
            return Err(GeometryError::RegionNotClosed);
        }
        chains.sort_by(|a, b| b.area(0.01).partial_cmp(&a.area(0.01)).unwrap_or(std::cmp::Ordering::Equal));
        let outer = chains.remove(0);
        for island in &chains {
            let probe = island.tessellate(0.01).into_iter().next().unwrap_or([0.0, 0.0]);
            if !outer.contains(probe) {
                return Err(GeometryError::IslandNotInside);
            }
        }
        Ok(Self { outer, islands: chains })
    }

    /// A region from a SolidCraft planar region (sketch profile or face outline).
    pub fn from_region2(region: &solidcraft_geom::Region2) -> Self {
        Self { outer: Chain::from_loop2(&region.outer), islands: region.holes.iter().map(Chain::from_loop2).collect() }
    }

    /// A region from a sketch profile (its `region` is the exact loop set).
    pub fn from_profile(profile: &solidcraft_sketch::Profile) -> Self {
        Self::from_region2(&profile.region)
    }

    /// A region from a planar face's outer boundary and its inner loops, projected onto the
    /// setup plane.
    pub fn from_face_loops(
        outer: &[solidcraft_geom::Vec3],
        islands: &[Vec<solidcraft_geom::Vec3>],
        setup: &crate::setup::Setup,
    ) -> Result<Self, GeometryError> {
        let outer = Chain::from_face_boundary(outer, setup)?;
        let islands = islands.iter().map(|points| Chain::from_face_boundary(points, setup)).collect::<Result<Vec<_>, _>>()?;
        Self::new(outer, islands)
    }

    pub fn new(outer: Chain, islands: Vec<Chain>) -> Result<Self, GeometryError> {
        if !outer.is_closed() {
            return Err(GeometryError::RegionNotClosed);
        }
        if islands.iter().any(|island| !island.is_closed()) {
            return Err(GeometryError::RegionNotClosed);
        }
        Ok(Self { outer, islands })
    }

    pub fn outer(&self) -> &Chain {
        &self.outer
    }

    pub fn islands(&self) -> &[Chain] {
        &self.islands
    }

    pub fn area(&self, tol: f64) -> f64 {
        (self.outer.area(tol) - self.islands.iter().map(|island| island.area(tol)).sum::<f64>()).max(0.0)
    }

    pub fn contains(&self, point: Point2) -> bool {
        self.outer.contains(point) && !self.islands.iter().any(|island| island.contains(point))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GeometryId(pub u64);

/// How a piece of CAM geometry came to be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeometrySource {
    /// Drawn or imported into CAM and independent of the design.
    Owned,
    /// Linked to a design entity and refreshed when the design changes.
    Linked(crate::reference::EntityRef),
}

/// The shape a geometry item holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GeometryShape {
    Curve(Curve2D),
    Chain(Chain),
    Region(Region),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeometryItem {
    id: GeometryId,
    source: GeometrySource,
    shape: GeometryShape,
}

impl GeometrySource {
    pub fn is_linked(&self) -> bool {
        matches!(self, GeometrySource::Linked(_))
    }

    pub fn reference(&self) -> Option<&crate::reference::EntityRef> {
        match self {
            GeometrySource::Linked(reference) => Some(reference),
            GeometrySource::Owned => None,
        }
    }
}

impl GeometryItem {
    pub fn id(&self) -> GeometryId {
        self.id
    }

    pub fn source(&self) -> &GeometrySource {
        &self.source
    }

    pub fn shape(&self) -> &GeometryShape {
        &self.shape
    }

    pub fn chain(&self) -> Option<&Chain> {
        match &self.shape {
            GeometryShape::Chain(chain) => Some(chain),
            GeometryShape::Region(region) => Some(region.outer()),
            GeometryShape::Curve(_) => None,
        }
    }

    pub fn region(&self) -> Option<&Region> {
        match &self.shape {
            GeometryShape::Region(region) => Some(region),
            _ => None,
        }
    }
}

/// A named group of geometry items, shown or hidden together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    name: String,
    visible: bool,
    items: Vec<GeometryItem>,
}

impl Layer {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn items(&self) -> &[GeometryItem] {
        &self.items
    }
}

/// The project's machining geometry, organised in layers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GeometryStore {
    #[serde(default)]
    layers: Vec<Layer>,
    #[serde(default)]
    next_id: u64,
}

impl GeometryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_layer(&mut self, name: impl Into<String>) -> Result<(), GeometryError> {
        let name = name.into();
        if self.layers.iter().any(|layer| layer.name == name) {
            return Err(GeometryError::DuplicateLayer(name));
        }
        self.layers.push(Layer { name, visible: true, items: Vec::new() });
        Ok(())
    }

    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.name == name)
    }

    pub fn layer_mut(&mut self, name: &str) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|layer| layer.name == name)
    }

    /// Add owned geometry to a layer and return its id.
    pub fn add(&mut self, layer: &str, shape: GeometryShape) -> Result<GeometryId, GeometryError> {
        self.add_from(layer, shape, GeometrySource::Owned)
    }

    /// Import a SolidCraft edge polyline as a linked CAM chain.
    pub fn import_edge(
        &mut self,
        layer: &str,
        points: &[solidcraft_geom::Vec3],
        setup: &crate::setup::Setup,
        reference: crate::reference::EntityRef,
    ) -> Result<GeometryId, GeometryError> {
        let chain = Chain::from_edge_points(points, setup)?;
        self.add_linked(layer, GeometryShape::Chain(chain), reference)
    }

    /// Import a planar face's outer boundary and inner loops as a linked CAM region.
    pub fn import_face(
        &mut self,
        layer: &str,
        outer: &[solidcraft_geom::Vec3],
        islands: &[Vec<solidcraft_geom::Vec3>],
        setup: &crate::setup::Setup,
        reference: crate::reference::EntityRef,
    ) -> Result<GeometryId, GeometryError> {
        let region = Region::from_face_loops(outer, islands, setup)?;
        self.add_linked(layer, GeometryShape::Region(region), reference)
    }

    /// Add a copy of design geometry, linked so a design change can refresh it.
    pub fn add_linked(&mut self, layer: &str, shape: GeometryShape, reference: crate::reference::EntityRef) -> Result<GeometryId, GeometryError> {
        self.add_from(layer, shape, GeometrySource::Linked(reference))
    }

    /// Every item linked to the design.
    pub fn linked(&self) -> impl Iterator<Item = &GeometryItem> {
        self.iter().filter(|item| item.source().is_linked())
    }

    /// Add geometry with an explicit source (owned, or linked to a design entity).
    pub fn add_from(&mut self, layer: &str, shape: GeometryShape, source: GeometrySource) -> Result<GeometryId, GeometryError> {
        let id = GeometryId(self.next_id);
        let target = self.layer_mut(layer).ok_or_else(|| GeometryError::UnknownLayer(layer.to_string()))?;
        target.items.push(GeometryItem { id, source, shape });
        self.next_id += 1;
        Ok(id)
    }

    /// A single curve by id (a one-curve chain counts).
    pub fn select_curve(&self, id: GeometryId) -> Result<&Curve2D, GeometryError> {
        let item = self.get(id).ok_or(GeometryError::UnknownGeometry(id))?;
        match item.shape() {
            GeometryShape::Curve(curve) => Ok(curve),
            GeometryShape::Chain(chain) if chain.curves().len() == 1 => Ok(&chain.curves()[0]),
            _ => Err(GeometryError::NotASingleCurve(id)),
        }
    }

    /// Join the selected geometry into one chain, in any order and direction.
    pub fn select_chain(&self, ids: &[GeometryId], gap_tol: f64) -> Result<Chain, GeometryError> {
        let mut curves = Vec::new();
        for id in ids {
            let item = self.get(*id).ok_or(GeometryError::UnknownGeometry(*id))?;
            match item.shape() {
                GeometryShape::Curve(curve) => curves.push(curve.clone()),
                GeometryShape::Chain(chain) => curves.extend(chain.curves().iter().cloned()),
                GeometryShape::Region(_) => return Err(GeometryError::NotAChain(*id)),
            }
        }
        Chain::orient_and_join(curves, gap_tol)
    }

    /// A closed chain that must be closed.
    pub fn select_closed_chain(&self, ids: &[GeometryId], gap_tol: f64) -> Result<Chain, GeometryError> {
        let chain = self.select_chain(ids, gap_tol)?;
        if !chain.is_closed() {
            return Err(GeometryError::NotClosed);
        }
        Ok(chain)
    }

    /// Select chains and nest them into a region (largest outer, the rest islands).
    pub fn select_region(&self, ids: &[GeometryId], gap_tol: f64) -> Result<Region, GeometryError> {
        let mut chains = Vec::with_capacity(ids.len());
        for id in ids {
            chains.push(self.select_closed_chain(&[*id], gap_tol)?);
        }
        Region::nest(chains)
    }

    /// Refresh every linked item from the design. `resolve` returns the new chain for a
    /// reference, or `None` when the design no longer has that face or edge.
    ///
    /// Owned geometry is untouched. Missing links are reported, never deleted: the machinist
    /// decides whether to re-pick, unlink or remove the operation using them.
    pub fn refresh_linked(&mut self, resolve: impl Fn(&crate::reference::EntityRef) -> Option<Chain>) -> Vec<GeometryIssue> {
        let mut issues = Vec::new();
        for layer in &mut self.layers {
            for item in &mut layer.items {
                let Some(reference) = item.source.reference().cloned() else { continue };
                match resolve(&reference) {
                    Some(chain) => item.shape = GeometryShape::Chain(chain),
                    None => issues.push(GeometryIssue { id: item.id, reference }),
                }
            }
        }
        issues
    }

    pub fn linked_count(&self) -> usize {
        self.linked().count()
    }

    pub fn get(&self, id: GeometryId) -> Option<&GeometryItem> {
        self.layers.iter().flat_map(|layer| &layer.items).find(|item| item.id == id)
    }

    pub fn get_mut(&mut self, id: GeometryId) -> Option<&mut GeometryItem> {
        self.layers.iter_mut().flat_map(|layer| &mut layer.items).find(|item| item.id == id)
    }

    /// Replace a geometry item's shape (an edit in the CAM project).
    pub fn set_shape(&mut self, id: GeometryId, shape: GeometryShape) -> Result<(), GeometryError> {
        let item = self.get_mut(id).ok_or(GeometryError::UnknownGeometry(id))?;
        item.shape = shape;
        Ok(())
    }

    pub fn remove(&mut self, id: GeometryId) -> Option<GeometryItem> {
        for layer in &mut self.layers {
            if let Some(index) = layer.items.iter().position(|item| item.id == id) {
                return Some(layer.items.remove(index));
            }
        }
        None
    }

    pub fn iter(&self) -> impl Iterator<Item = &GeometryItem> {
        self.layers.iter().flat_map(|layer| layer.items.iter())
    }

    pub fn len(&self) -> usize {
        self.layers.iter().map(|layer| layer.items.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn best_match(curves: &[Curve2D], point: Point2, tol: f64) -> Option<(usize, bool)> {
    let mut best: Option<(usize, bool, f64)> = None;
    for (index, curve) in curves.iter().enumerate() {
        for (reverse, candidate) in [(false, curve.start()), (true, curve.end())] {
            let gap = distance(candidate, point);
            if gap <= tol && best.map(|(_, _, current)| gap < current).unwrap_or(true) {
                best = Some((index, reverse, gap));
            }
        }
    }
    best.map(|(index, reverse, _)| (index, reverse))
}

fn project_points(points: &[solidcraft_geom::Vec3], setup: &crate::setup::Setup) -> Vec<Point2> {
    points
        .iter()
        .map(|point| {
            let wcs = setup.to_wcs([point.x, point.y, point.z]);
            [wcs[0], wcs[1]]
        })
        .collect()
}

fn finite(point: Point2) -> Result<(), GeometryError> {
    if point.iter().all(|coordinate| coordinate.is_finite()) { Ok(()) } else { Err(GeometryError::NotFinite) }
}

fn distance(a: Point2, b: Point2) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_and_arc_lengths() {
        let line = Curve2D::line([0.0, 0.0], [3.0, 4.0]).unwrap();
        assert!((line.length() - 5.0).abs() < 1e-12);
        let arc = Curve2D::arc([0.0, 0.0], 10.0, 0.0, std::f64::consts::FRAC_PI_2).unwrap();
        assert!((arc.length() - std::f64::consts::FRAC_PI_2 * 10.0).abs() < 1e-9);
    }

    #[test]
    fn arc_tessellation_preserves_endpoints() {
        let arc = Curve2D::arc([5.0, 5.0], 2.0, 0.0, std::f64::consts::PI).unwrap();
        let points = arc.tessellate(0.01);
        assert!(points.len() > 8);
        assert!((points[0][0] - 7.0).abs() < 1e-9);
        assert!((points[points.len() - 1][0] - 3.0).abs() < 1e-9);
        assert!((points[points.len() - 1][1] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn a_square_chain_is_closed_and_has_the_right_area() {
        let curves = vec![
            Curve2D::line([0.0, 0.0], [10.0, 0.0]).unwrap(),
            Curve2D::line([10.0, 0.0], [10.0, 10.0]).unwrap(),
            Curve2D::line([10.0, 10.0], [0.0, 10.0]).unwrap(),
            Curve2D::line([0.0, 10.0], [0.0, 0.0]).unwrap(),
        ];
        let chain = Chain::closed(curves, 1e-6).unwrap();
        assert!(chain.is_closed());
        assert!((chain.length() - 40.0).abs() < 1e-9);
        assert!((chain.area(0.01) - 100.0).abs() < 1e-6);
        assert!(chain.contains([5.0, 5.0]));
        assert!(!chain.contains([15.0, 5.0]));
        let (min, max) = chain.bounds();
        assert_eq!(min, [0.0, 0.0]);
        assert_eq!(max, [10.0, 10.0]);
    }

    #[test]
    fn a_chain_reverses_end_for_end() {
        let curves = vec![Curve2D::line([0.0, 0.0], [10.0, 0.0]).unwrap(), Curve2D::line([10.0, 0.0], [10.0, 10.0]).unwrap()];
        let chain = Chain::open(curves, 1e-6).unwrap();
        let reversed = chain.reversed();
        assert_eq!(reversed.start(), chain.end());
        assert_eq!(reversed.end(), chain.start());
        assert!((reversed.length() - chain.length()).abs() < 1e-9);
    }

    #[test]
    fn a_gap_larger_than_the_tolerance_is_reported() {
        let curves = vec![Curve2D::line([0.0, 0.0], [10.0, 0.0]).unwrap(), Curve2D::line([11.0, 0.0], [20.0, 0.0]).unwrap()];
        let error = Chain::join(curves, 0.5).unwrap_err();
        assert!(matches!(error, GeometryError::Gap { index: 1, .. }));
    }

    #[test]
    fn a_region_subtracts_its_islands() {
        let outer = Chain::closed(
            vec![
                Curve2D::line([0.0, 0.0], [20.0, 0.0]).unwrap(),
                Curve2D::line([20.0, 0.0], [20.0, 20.0]).unwrap(),
                Curve2D::line([20.0, 20.0], [0.0, 20.0]).unwrap(),
                Curve2D::line([0.0, 20.0], [0.0, 0.0]).unwrap(),
            ],
            1e-6,
        )
        .unwrap();
        let island = Chain::closed(
            vec![
                Curve2D::line([5.0, 5.0], [10.0, 5.0]).unwrap(),
                Curve2D::line([10.0, 5.0], [10.0, 10.0]).unwrap(),
                Curve2D::line([10.0, 10.0], [5.0, 10.0]).unwrap(),
                Curve2D::line([5.0, 10.0], [5.0, 5.0]).unwrap(),
            ],
            1e-6,
        )
        .unwrap();
        let region = Region::new(outer, vec![island]).unwrap();
        assert!((region.area(0.01) - 375.0).abs() < 1e-6);
        assert!(region.contains([2.0, 2.0]));
        assert!(!region.contains([7.0, 7.0]));
    }

    #[test]
    fn imported_edges_and_faces_keep_stable_references() {
        use crate::reference::{EntityKind, EntityRef};
        use crate::setup::Setup;
        use crate::stock::{Material, Stock};
        use solidcraft_geom::Vec3;

        let stock = Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap();
        let setup = Setup::top_centre(&stock);
        let mut store = GeometryStore::new();
        store.add_layer("Design").unwrap();

        let edge = EntityRef::new(EntityKind::Edge, "edge.42").unwrap();
        let id = store.import_edge("Design", &[Vec3::new(25.0, 20.0, 20.0), Vec3::new(35.0, 20.0, 20.0)], &setup, edge.clone()).unwrap();
        assert_eq!(store.get(id).unwrap().source(), &GeometrySource::Linked(edge.clone()));
        assert_eq!(store.get(id).unwrap().chain().unwrap().length(), 10.0);

        let face = EntityRef::new(EntityKind::Face, "face.7").unwrap();
        let outer = [Vec3::new(25.0, 15.0, 20.0), Vec3::new(35.0, 15.0, 20.0), Vec3::new(35.0, 25.0, 20.0), Vec3::new(25.0, 25.0, 20.0)];
        let face_id = store.import_face("Design", &outer, &[], &setup, face.clone()).unwrap();
        assert_eq!(store.get(face_id).unwrap().source(), &GeometrySource::Linked(face.clone()));
        assert!((store.get(face_id).unwrap().region().unwrap().area(0.01) - 100.0).abs() < 0.1);

        let issues = store.refresh_linked(|_| None);
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().any(|issue| issue.reference == edge));
        assert!(issues.iter().any(|issue| issue.reference == face));
    }

    #[test]
    fn linked_geometry_refreshes_and_reports_missing_design_entities() {
        use crate::reference::{EntityKind, EntityRef};
        let curve = || Curve2D::line([0.0, 0.0], [10.0, 0.0]).unwrap();
        let mut store = GeometryStore::new();
        store.add_layer("Linked").unwrap();
        let kept = EntityRef::new(EntityKind::Edge, "edge.1").unwrap();
        let gone = EntityRef::new(EntityKind::Edge, "edge.2").unwrap();
        let kept_id = store.add_linked("Linked", GeometryShape::Curve(curve()), kept.clone()).unwrap();
        let gone_id = store.add_linked("Linked", GeometryShape::Curve(curve()), gone.clone()).unwrap();
        store.add("Linked", GeometryShape::Curve(curve())).unwrap();

        let replacement = Chain::open(vec![Curve2D::line([0.0, 0.0], [20.0, 0.0]).unwrap()], 1e-6).unwrap();
        let issues = store.refresh_linked(|reference| if reference == &kept { Some(replacement.clone()) } else { None });
        assert_eq!(issues, vec![GeometryIssue { id: gone_id, reference: gone.clone() }]);
        assert_eq!(store.get(kept_id).unwrap().chain().unwrap().length(), 20.0);
        assert!(store.get(gone_id).is_some(), "a missing link stays in place for the machinist");
        assert_eq!(store.linked_count(), 2);
    }

    #[test]
    fn a_solidcraft_loop_keeps_its_arcs() {
        use solidcraft_geom::{Loop2, Seg2, Vec2};
        let loop2 = Loop2 {
            segs: vec![
                Seg2::Line { a: Vec2::new(0.0, 0.0), b: Vec2::new(10.0, 0.0) },
                Seg2::Arc { center: Vec2::new(10.0, 10.0), radius: 10.0, start: -std::f64::consts::FRAC_PI_2, sweep: std::f64::consts::FRAC_PI_2 },
                Seg2::Line { a: Vec2::new(0.0, 10.0), b: Vec2::new(0.0, 0.0) },
            ],
        };
        let chain = Chain::from_loop2(&loop2);
        assert!(chain.is_closed());
        assert!(matches!(chain.segment(1), Some(Curve2D::Arc { .. })));
        assert_eq!(chain.curves().len(), 3);
        let region = Region::from_region2(&solidcraft_geom::Region2 { outer: loop2, holes: vec![] });
        assert!(region.outer().is_closed());
        assert!(region.area(0.01) > 0.0);
    }

    #[test]
    fn an_edge_polyline_projects_into_the_setup_plane() {
        use solidcraft_geom::Vec3;
        let stock = crate::stock::Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], crate::stock::Material::Aluminum).unwrap();
        let setup = crate::setup::Setup::top_centre(&stock);
        let points = vec![Vec3::new(25.0, 20.0, 20.0), Vec3::new(35.0, 20.0, 20.0)];
        let chain = Chain::from_edge_points(&points, &setup).unwrap();
        assert!(!chain.is_closed());
        assert_eq!(chain.start(), [-5.0, 0.0]);
        assert_eq!(chain.end(), [5.0, 0.0]);
        assert!((chain.length() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn selection_joins_curves_in_any_order_and_direction() {
        let mut store = GeometryStore::new();
        store.add_layer("Picked").unwrap();
        let bottom = Curve2D::line([0.0, 0.0], [10.0, 0.0]).unwrap();
        let right = Curve2D::line([10.0, 10.0], [10.0, 0.0]).unwrap(); // drawn backwards
        let top = Curve2D::line([10.0, 10.0], [0.0, 10.0]).unwrap();
        let left = Curve2D::line([0.0, 0.0], [0.0, 10.0]).unwrap(); // drawn backwards
        let ids: Vec<_> = [left, bottom, right, top].into_iter().map(|curve| store.add("Picked", GeometryShape::Curve(curve)).unwrap()).collect();
        let chain = store.select_closed_chain(&ids, 1e-6).unwrap();
        assert!(chain.is_closed());
        assert!((chain.length() - 40.0).abs() < 1e-9);
        assert!((chain.area(0.01) - 100.0).abs() < 1e-6);
        let single = store.select_curve(ids[1]).unwrap();
        assert!((single.length() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn selection_nests_an_island_in_the_largest_chain() {
        let square = |x: f64, y: f64, size: f64| {
            Chain::closed(
                vec![
                    Curve2D::line([x, y], [x + size, y]).unwrap(),
                    Curve2D::line([x + size, y], [x + size, y + size]).unwrap(),
                    Curve2D::line([x + size, y + size], [x, y + size]).unwrap(),
                    Curve2D::line([x, y + size], [x, y]).unwrap(),
                ],
                1e-6,
            )
            .unwrap()
        };
        let mut store = GeometryStore::new();
        store.add_layer("Picked").unwrap();
        let outer = store.add("Picked", GeometryShape::Chain(square(0.0, 0.0, 20.0))).unwrap();
        let island = store.add("Picked", GeometryShape::Chain(square(5.0, 5.0, 5.0))).unwrap();
        let region = store.select_region(&[island, outer], 1e-6).unwrap();
        assert!((region.area(0.01) - 375.0).abs() < 1e-6);
        assert_eq!(region.islands().len(), 1);
        // Two disjoint chains cannot be nested.
        let far = store.add("Picked", GeometryShape::Chain(square(100.0, 100.0, 5.0))).unwrap();
        assert!(matches!(store.select_region(&[outer, island, far], 1e-6), Err(GeometryError::IslandNotInside)));
    }

    #[test]
    fn geometry_can_be_owned_or_linked_to_the_design() {
        use crate::reference::{EntityKind, EntityRef};
        let mut store = GeometryStore::new();
        store.add_layer("From design").unwrap();
        let reference = EntityRef::new(EntityKind::Edge, "edge.7").unwrap();
        let linked =
            store.add_linked("From design", GeometryShape::Curve(Curve2D::line([0.0, 0.0], [5.0, 0.0]).unwrap()), reference.clone()).unwrap();
        let owned = store.add("From design", GeometryShape::Curve(Curve2D::line([0.0, 1.0], [5.0, 1.0]).unwrap())).unwrap();
        assert_eq!(store.get(linked).unwrap().source(), &GeometrySource::Linked(reference.clone()));
        assert_eq!(store.get(owned).unwrap().source(), &GeometrySource::Owned);
        assert_eq!(store.linked().count(), 1);
        assert_eq!(store.get(linked).unwrap().source().reference(), Some(&reference));
        let json = serde_json::to_string(&store).unwrap();
        assert_eq!(serde_json::from_str::<GeometryStore>(&json).unwrap(), store);
    }

    #[test]
    fn layers_hold_geometry_and_round_trip() {
        let mut store = GeometryStore::new();
        store.add_layer("Machining").unwrap();
        store.add_layer("Notes").unwrap();
        assert_eq!(store.add_layer("Machining"), Err(GeometryError::DuplicateLayer("Machining".into())));
        let id = store.add("Machining", GeometryShape::Curve(Curve2D::line([0.0, 0.0], [1.0, 0.0]).unwrap())).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(id).unwrap().source(), &GeometrySource::Owned);
        assert!(matches!(
            store.add("missing", GeometryShape::Curve(Curve2D::line([0.0, 0.0], [1.0, 0.0]).unwrap())),
            Err(GeometryError::UnknownLayer(_))
        ));
        let json = serde_json::to_string(&store).unwrap();
        assert_eq!(serde_json::from_str::<GeometryStore>(&json).unwrap(), store);
        assert!(store.remove(id).is_some());
        assert!(store.is_empty());
    }
}

//! 3D foundation: meshes in the setup frame, sampled height maps and z-levels.
//!
//! This is the geometry layer 3D toolpaths will stand on. It takes SolidCraft's tessellated
//! bodies (a cutrail `TriMesh`) and the active `Setup`, transforms the vertices into WCS, and
//! samples the top surface into a regular height map. Gouge and scallop decisions are made on
//! that map; cutrail remains the verifier.

use cutrail::TriMesh;
use thiserror::Error;

use crate::setup::Setup;

#[derive(Debug, Error)]
pub enum SurfaceError {
    #[error("the mesh has no triangles")]
    Empty,
    #[error("cell size must be finite and greater than zero, got {0}")]
    BadCell(f64),
    #[error("z-level step must be finite and greater than zero, got {0}")]
    BadStep(f64),
    #[error("the mesh needs at least three vertices")]
    BadMesh,
}

/// A triangle mesh transformed into the setup's WCS.
#[derive(Debug, Clone)]
pub struct SurfaceMesh {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[usize; 3]>,
    min: [f64; 2],
    max: [f64; 2],
}

impl SurfaceMesh {
    /// Transform a SolidCraft/cutrail mesh into WCS coordinates.
    pub fn from_tri_mesh(mesh: &TriMesh, setup: &Setup) -> Result<Self, SurfaceError> {
        if mesh.triangles.is_empty() || mesh.positions.len() < 3 {
            return Err(SurfaceError::BadMesh);
        }
        let vertices: Vec<[f64; 3]> = mesh.positions.iter().map(|point| setup.to_wcs([point[0], point[1], point[2]])).collect();
        let triangles: Vec<[usize; 3]> =
            mesh.triangles.iter().map(|triangle| [triangle[0] as usize, triangle[1] as usize, triangle[2] as usize]).collect();
        Ok(Self::from_parts(vertices, triangles))
    }

    pub fn from_parts(vertices: Vec<[f64; 3]>, triangles: Vec<[usize; 3]>) -> Self {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for vertex in &vertices {
            for axis in 0..2 {
                min[axis] = min[axis].min(vertex[axis]);
                max[axis] = max[axis].max(vertex[axis]);
            }
        }
        Self { vertices, triangles, min, max }
    }

    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        (self.min, self.max)
    }

    /// Highest surface point at (x, y), or `None` outside every triangle.
    pub fn height_at(&self, x: f64, y: f64) -> Option<f64> {
        let mut best: Option<f64> = None;
        for triangle in &self.triangles {
            let (Some(a), Some(b), Some(c)) = (self.vertices.get(triangle[0]), self.vertices.get(triangle[1]), self.vertices.get(triangle[2])) else {
                continue;
            };
            let det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
            if det.abs() < 1e-12 {
                continue;
            }
            let l1 = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / det;
            let l2 = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / det;
            let l3 = 1.0 - l1 - l2;
            if l1 < -1e-9 || l2 < -1e-9 || l3 < -1e-9 {
                continue;
            }
            let z = l1 * a[2] + l2 * b[2] + l3 * c[2];
            best = Some(best.map(|current| current.max(z)).unwrap_or(z));
        }
        best
    }
}

/// A regular grid of top-surface heights.
#[derive(Debug, Clone)]
pub struct HeightMap {
    pub min: [f64; 2],
    pub cell_size: f64,
    pub cols: usize,
    pub rows: usize,
    heights: Vec<Option<f64>>,
}

impl HeightMap {
    pub fn from_tri_mesh(mesh: &TriMesh, setup: &Setup, cell_size: f64) -> Result<Self, SurfaceError> {
        let surface = SurfaceMesh::from_tri_mesh(mesh, setup)?;
        Self::from_surface(&surface, cell_size)
    }

    pub fn from_surface(surface: &SurfaceMesh, cell_size: f64) -> Result<Self, SurfaceError> {
        if !cell_size.is_finite() || cell_size <= 0.0 {
            return Err(SurfaceError::BadCell(cell_size));
        }
        let (min, max) = surface.bounds();
        let cols = (((max[0] - min[0]) / cell_size).ceil() as usize).max(1);
        let rows = (((max[1] - min[1]) / cell_size).ceil() as usize).max(1);
        let mut heights = Vec::with_capacity(cols * rows);
        for row in 0..rows {
            for col in 0..cols {
                let x = min[0] + (col as f64 + 0.5) * cell_size;
                let y = min[1] + (row as f64 + 0.5) * cell_size;
                heights.push(surface.height_at(x, y));
            }
        }
        Ok(Self { min, cell_size, cols, rows, heights })
    }

    pub fn height_at(&self, x: f64, y: f64) -> Option<f64> {
        let col = ((x - self.min[0]) / self.cell_size).floor();
        let row = ((y - self.min[1]) / self.cell_size).floor();
        if col < 0.0 || row < 0.0 {
            return None;
        }
        let (col, row) = (col as usize, row as usize);
        if col >= self.cols || row >= self.rows {
            return None;
        }
        self.heights[row * self.cols + col]
    }

    pub fn heights(&self) -> &[Option<f64>] {
        &self.heights
    }

    pub fn top(&self) -> Option<f64> {
        self.heights.iter().flatten().copied().fold(None, |current, value| Some(current.map(|z: f64| z.max(value)).unwrap_or(value)))
    }

    pub fn bottom(&self) -> Option<f64> {
        self.heights.iter().flatten().copied().fold(None, |current, value| Some(current.map(|z: f64| z.min(value)).unwrap_or(value)))
    }

    /// Z levels from the top surface down to the bottom at `step`.
    pub fn z_levels(&self, step: f64) -> Result<Vec<f64>, SurfaceError> {
        match (self.top(), self.bottom()) {
            (Some(top), Some(bottom)) => z_levels(top, bottom, step),
            _ => Ok(Vec::new()),
        }
    }
}

/// Z levels from `top` down to `bottom`, including the bottom.
pub fn z_levels(top: f64, bottom: f64, step: f64) -> Result<Vec<f64>, SurfaceError> {
    if !step.is_finite() || step <= 0.0 {
        return Err(SurfaceError::BadStep(step));
    }
    if !(top.is_finite() && bottom.is_finite()) {
        return Err(SurfaceError::BadStep(step));
    }
    if top < bottom {
        return Ok(Vec::new());
    }
    let mut levels = Vec::new();
    let mut z = top;
    while z > bottom + 1e-9 {
        levels.push(z);
        z -= step;
    }
    levels.push(bottom);
    Ok(levels)
}

/// Scallop (cusp) height left between two parallel passes.
pub fn cusp_height(tool_radius: f64, stepover: f64) -> f64 {
    if tool_radius <= 0.0 || stepover <= 0.0 {
        return 0.0;
    }
    let half = stepover / 2.0;
    if half >= tool_radius {
        return tool_radius;
    }
    tool_radius - (tool_radius * tool_radius - half * half).sqrt()
}

/// Stepover that leaves a given scallop height.
pub fn stepover_for_cusp(tool_radius: f64, cusp: f64) -> f64 {
    if tool_radius <= 0.0 || cusp <= 0.0 || cusp >= tool_radius {
        return 0.0;
    }
    let remaining = tool_radius - cusp;
    2.0 * (tool_radius * tool_radius - remaining * remaining).sqrt()
}

/// Chord error of a polyline approximating a radius with the given angular step.
pub fn chord_error(radius: f64, step_radians: f64) -> f64 {
    if radius <= 0.0 || step_radians <= 0.0 {
        return 0.0;
    }
    radius * (1.0 - (step_radians / 2.0).cos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::{Direction, Fixture};

    fn setup() -> Setup {
        Setup::new("top", [0.0, 0.0, 0.0], Direction::PosZ, Direction::PosX, Fixture::None).unwrap()
    }

    fn plane(z: f64) -> TriMesh {
        TriMesh { positions: vec![[0.0, 0.0, z], [10.0, 0.0, z], [10.0, 10.0, z], [0.0, 10.0, z]], triangles: vec![[0, 1, 2], [0, 2, 3]] }
    }

    #[test]
    fn a_plane_samples_to_its_height() {
        let map = HeightMap::from_tri_mesh(&plane(5.0), &setup(), 2.0).unwrap();
        assert!(map.heights().iter().all(|height| (height.unwrap() - 5.0).abs() < 1e-9));
        assert_eq!(map.top(), Some(5.0));
        assert_eq!(map.bottom(), Some(5.0));
        assert_eq!(map.height_at(1.0, 1.0), Some(5.0));
        assert_eq!(map.height_at(50.0, 1.0), None);
    }

    #[test]
    fn the_setup_transform_is_applied() {
        let mesh = plane(5.0);
        let shifted = Setup::new("shifted", [0.0, 0.0, 2.0], Direction::PosZ, Direction::PosX, Fixture::None).unwrap();
        let map = HeightMap::from_tri_mesh(&mesh, &shifted, 2.0).unwrap();
        assert!((map.top().unwrap() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_sloped_surface_interpolates() {
        let mesh = TriMesh { positions: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 10.0], [0.0, 10.0, 0.0]], triangles: vec![[0, 1, 2]] };
        let surface = SurfaceMesh::from_tri_mesh(&mesh, &setup()).unwrap();
        assert!((surface.height_at(5.0, 2.5).unwrap() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn z_levels_include_the_bottom() {
        assert_eq!(z_levels(0.0, -5.0, 2.0).unwrap(), [0.0, -2.0, -4.0, -5.0]);
        assert!(z_levels(0.0, -5.0, 0.0).is_err());
    }

    #[test]
    fn scallop_and_chord_tolerances_are_consistent() {
        let cusp = cusp_height(3.0, 2.0);
        assert!((cusp - (3.0 - 8.0f64.sqrt())).abs() < 1e-9);
        let stepover = stepover_for_cusp(3.0, cusp);
        assert!((stepover - 2.0).abs() < 1e-9);
        let error = chord_error(10.0, std::f64::consts::FRAC_PI_8);
        assert!(error > 0.0 && error < 0.2);
    }
}

//! Stock: the raw block a job starts from (size, position in model space, material).

use cutrail::StockShape;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The material of the stock. Feeds and speeds will be derived from it later; for now it is
/// part of the job so a setup sheet can print it and a tool library can look it up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Material {
    Aluminum,
    Steel,
    Brass,
    Plastic,
    Other(String),
}

impl Material {
    /// A short, stable label as it appears on a setup sheet.
    pub fn label(&self) -> &str {
        match self {
            Material::Aluminum => "Aluminum",
            Material::Steel => "Steel",
            Material::Brass => "Brass",
            Material::Plastic => "Plastic",
            Material::Other(name) => name,
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum StockError {
    #[error("stock coordinate is not finite")]
    NotFinite,
    #[error("stock has no thickness on the {axis} axis (min == max)")]
    Empty { axis: &'static str },
    #[error("stock bounds are inverted on the {axis} axis (min > max)")]
    Inverted { axis: &'static str },
}

/// A rectangular block of raw material, in model coordinates (millimetres, Z up).
///
/// The block's position matters: it is where the raw material sits relative to the part.
/// Which corner is the work origin is a [`Setup`](crate::setup::Setup) concern, not a stock one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stock {
    min: [f64; 3],
    max: [f64; 3],
    material: Material,
}

impl Stock {
    /// A block from two opposite corners.
    pub fn new(min: [f64; 3], max: [f64; 3], material: Material) -> Result<Self, StockError> {
        for (axis, (lo, hi)) in ["X", "Y", "Z"].iter().zip(min.iter().zip(max.iter())) {
            if !lo.is_finite() || !hi.is_finite() {
                return Err(StockError::NotFinite);
            }
            if lo == hi {
                return Err(StockError::Empty { axis });
            }
            if lo > hi {
                return Err(StockError::Inverted { axis });
            }
        }
        Ok(Self { min, max, material })
    }

    /// A block of `size` growing from a minimum corner.
    pub fn from_size(origin: [f64; 3], size: [f64; 3], material: Material) -> Result<Self, StockError> {
        let max = [origin[0] + size[0], origin[1] + size[1], origin[2] + size[2]];
        Self::new(origin, max, material)
    }

    pub fn min(&self) -> [f64; 3] {
        self.min
    }

    pub fn max(&self) -> [f64; 3] {
        self.max
    }

    pub fn size(&self) -> [f64; 3] {
        [self.max[0] - self.min[0], self.max[1] - self.min[1], self.max[2] - self.min[2]]
    }

    /// The centre of the block.
    pub fn centre(&self) -> [f64; 3] {
        let s = self.size();
        [self.min[0] + s[0] / 2.0, self.min[1] + s[1] / 2.0, self.min[2] + s[2] / 2.0]
    }

    /// Volume in mm³.
    pub fn volume(&self) -> f64 {
        let s = self.size();
        s[0] * s[1] * s[2]
    }

    pub fn material(&self) -> &Material {
        &self.material
    }

    /// True when `point` lies inside or on the surface of the block.
    pub fn contains(&self, point: [f64; 3]) -> bool {
        (0..3).all(|i| point[i] >= self.min[i] && point[i] <= self.max[i])
    }

    /// The stock as cutrail sees it.
    pub fn to_cutrail(&self) -> StockShape {
        StockShape::Box { min: self.min, max: self.max }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block() -> Stock {
        Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap()
    }

    #[test]
    fn size_volume_and_centre() {
        let stock = block();
        assert_eq!(stock.size(), [60.0, 40.0, 20.0]);
        assert_eq!(stock.volume(), 48_000.0);
        assert_eq!(stock.centre(), [30.0, 20.0, 10.0]);
    }

    #[test]
    fn from_size_grows_from_the_origin_corner() {
        let stock = Stock::from_size([10.0, -5.0, 0.0], [20.0, 10.0, 5.0], Material::Plastic).unwrap();
        assert_eq!(stock.min(), [10.0, -5.0, 0.0]);
        assert_eq!(stock.max(), [30.0, 5.0, 5.0]);
    }

    #[test]
    fn inverted_and_empty_bounds_are_rejected() {
        assert_eq!(Stock::new([0.0, 0.0, 0.0], [-1.0, 10.0, 10.0], Material::Steel), Err(StockError::Inverted { axis: "X" }));
        assert_eq!(Stock::new([0.0, 0.0, 0.0], [10.0, 10.0, 0.0], Material::Steel), Err(StockError::Empty { axis: "Z" }));
    }

    #[test]
    fn non_finite_coordinates_are_rejected() {
        assert_eq!(Stock::new([0.0, 0.0, 0.0], [f64::NAN, 10.0, 10.0], Material::Steel), Err(StockError::NotFinite));
    }

    #[test]
    fn contains_is_inclusive_on_the_surface() {
        let stock = block();
        assert!(stock.contains([0.0, 20.0, 20.0]));
        assert!(!stock.contains([60.1, 20.0, 10.0]));
    }

    #[test]
    fn converts_to_cutrail_stock() {
        let shape = block().to_cutrail();
        assert_eq!(shape, StockShape::Box { min: [0.0, 0.0, 0.0], max: [60.0, 40.0, 20.0] });
    }
}

//! Setup: the work coordinate system (WCS) a job is machined in.
//!
//! A [`Setup`] says where the machine's origin sits relative to the model, which way Z points
//! (the tool axis, usually up), which way X points, and how the part is held. Operations are
//! written in WCS coordinates; the model and the stock stay in model coordinates, and
//! [`Setup::to_model`] / [`Setup::to_wcs`] translate between them.

use cutrail::Pose;
use thiserror::Error;

use crate::stock::Stock;
use serde::{Deserialize, Serialize};

/// One of the six signed axis directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Direction {
    /// The unit vector this direction points along.
    pub fn vector(self) -> [f64; 3] {
        match self {
            Direction::PosX => [1.0, 0.0, 0.0],
            Direction::NegX => [-1.0, 0.0, 0.0],
            Direction::PosY => [0.0, 1.0, 0.0],
            Direction::NegY => [0.0, -1.0, 0.0],
            Direction::PosZ => [0.0, 0.0, 1.0],
            Direction::NegZ => [0.0, 0.0, -1.0],
        }
    }

    /// The opposite direction, for flipping a setup over.
    pub fn flipped(self) -> Self {
        match self {
            Direction::PosX => Direction::NegX,
            Direction::NegX => Direction::PosX,
            Direction::PosY => Direction::NegY,
            Direction::NegY => Direction::PosY,
            Direction::PosZ => Direction::NegZ,
            Direction::NegZ => Direction::PosZ,
        }
    }

    /// 0 = X, 1 = Y, 2 = Z; the sign is the direction.
    pub fn axis(self) -> usize {
        match self {
            Direction::PosX | Direction::NegX => 0,
            Direction::PosY | Direction::NegY => 1,
            Direction::PosZ | Direction::NegZ => 2,
        }
    }
}

/// How the part is held for this setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fixture {
    /// Held in a machine vise.
    Vise,
    /// Held with clamps and toe clamps.
    Clamps,
    /// Held in a chuck (turning).
    Chuck,
    /// Not restrained: for checking, or a part held by its own geometry.
    None,
    /// Anything else, named.
    Other(String),
}

#[derive(Debug, Error, PartialEq)]
pub enum SetupError {
    #[error("the Z and X axes must be perpendicular, not `{0:?}` and `{1:?}`")]
    AxesNotPerpendicular(Direction, Direction),
    #[error("the setup origin is not finite")]
    OriginNotFinite,
}

/// A machining setup: work origin, axes and fixture, in model coordinates (mm, Z up).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    name: String,
    origin: [f64; 3],
    z: Direction,
    x: Direction,
    fixture: Fixture,
}

impl Setup {
    /// A setup with an explicit origin and axes. Z is the tool axis; X completes the frame
    /// (Y is the cross product).
    pub fn new(name: impl Into<String>, origin: [f64; 3], z: Direction, x: Direction, fixture: Fixture) -> Result<Self, SetupError> {
        if !origin.iter().all(|c| c.is_finite()) {
            return Err(SetupError::OriginNotFinite);
        }
        if z.axis() == x.axis() {
            return Err(SetupError::AxesNotPerpendicular(z, x));
        }
        Ok(Self { name: name.into(), origin, z, x, fixture })
    }

    /// The usual first setup: origin at the top centre of the stock, Z up, X to the right,
    /// with the part in a vise.
    pub fn top_centre(stock: &Stock) -> Self {
        let centre = stock.centre();
        Self {
            name: "Setup 1".into(),
            origin: [centre[0], centre[1], stock.max()[2]],
            z: Direction::PosZ,
            x: Direction::PosX,
            fixture: Fixture::Vise,
        }
    }

    /// The same setup flipped over, as a second-op setup does.
    pub fn flipped(&self) -> Self {
        let mut flipped = self.clone();
        flipped.name = format!("{} (flipped)", self.name);
        flipped.z = self.z.flipped();
        flipped
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn origin(&self) -> [f64; 3] {
        self.origin
    }

    pub fn z_axis(&self) -> Direction {
        self.z
    }

    pub fn x_axis(&self) -> Direction {
        self.x
    }

    pub fn fixture(&self) -> &Fixture {
        &self.fixture
    }

    /// The three unit axes of the setup frame, in the order X, Y, Z.
    pub fn axes(&self) -> [[f64; 3]; 3] {
        let x = self.x.vector();
        let z = self.z.vector();
        let y = cross(z, x);
        [x, y, z]
    }

    /// A point given in setup (WCS) coordinates, in model coordinates.
    pub fn to_model(&self, point: [f64; 3]) -> [f64; 3] {
        let [x, y, z] = self.axes();
        [
            self.origin[0] + point[0] * x[0] + point[1] * y[0] + point[2] * z[0],
            self.origin[1] + point[0] * x[1] + point[1] * y[1] + point[2] * z[1],
            self.origin[2] + point[0] * x[2] + point[1] * y[2] + point[2] * z[2],
        ]
    }

    /// A model point, in setup (WCS) coordinates.
    pub fn to_wcs(&self, point: [f64; 3]) -> [f64; 3] {
        let d = [point[0] - self.origin[0], point[1] - self.origin[1], point[2] - self.origin[2]];
        let [x, y, z] = self.axes();
        [dot(d, x), dot(d, y), dot(d, z)]
    }

    /// A tool pose given in WCS coordinates, as cutrail expects it.
    pub fn pose(&self, tip: [f64; 3]) -> Pose {
        let model = self.to_model(tip);
        Pose { tip: model, axis: self.z.vector() }
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stock::Material;

    fn stock() -> Stock {
        Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap()
    }

    #[test]
    fn perpendicular_axes_are_required() {
        assert_eq!(
            Setup::new("bad", [0.0; 3], Direction::PosZ, Direction::NegZ, Fixture::Vise),
            Err(SetupError::AxesNotPerpendicular(Direction::PosZ, Direction::NegZ))
        );
    }

    #[test]
    fn top_centre_sits_on_the_stock_top() {
        let setup = Setup::top_centre(&stock());
        assert_eq!(setup.origin(), [30.0, 20.0, 20.0]);
        assert_eq!(setup.z_axis(), Direction::PosZ);
        assert_eq!(setup.fixture(), &Fixture::Vise);
    }

    #[test]
    fn wcs_round_trips_through_model_coordinates() {
        let setup = Setup::top_centre(&stock());
        let local = [5.0, -10.0, 0.0];
        let model = setup.to_model(local);
        assert_eq!(model, [35.0, 10.0, 20.0]);
        assert_eq!(setup.to_wcs(model), local);
    }

    #[test]
    fn a_flipped_setup_points_z_down() {
        let setup = Setup::top_centre(&stock()).flipped();
        assert_eq!(setup.z_axis(), Direction::NegZ);
        assert_eq!(setup.to_model([0.0, 0.0, 5.0]), [30.0, 20.0, 15.0]);
    }

    #[test]
    fn pose_carries_the_setup_axis() {
        let setup = Setup::top_centre(&stock());
        let pose = setup.pose([0.0, 0.0, 2.0]);
        assert_eq!(pose.axis, [0.0, 0.0, 1.0]);
        assert_eq!(pose.tip, [30.0, 20.0, 22.0]);
    }
}

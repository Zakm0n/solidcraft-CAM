//! The stock definition: the raw material a program cuts the part out of.
//!
//! A CAM programmer defines the stock once per setup. It is either a block sized from the part's
//! geometry plus an excess on each side — the usual way to leave room for clamping and facing —
//! or an existing solid body that is already the raw material (an imported rough casting, for
//! instance).
//!
//! The definition stores what the viewport draws, not how it was entered: a bounding box carries
//! its extents, a solid its body. The extents are ordered `[min x, min y, min z, max x, max y,
//! max z]` and are in millimetres, as every other length in this crate is.

use thiserror::Error;

/// The identity of a document entity, such as the solid body a stock definition selects.
///
/// Ids are assigned by the document that owns the entity and are stable for its life.
pub type EntityId = usize;

/// Why a stock definition cannot be used. The dialog shows the reason beside the field it comes
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum StockError {
    /// The extents are not finite, or do not describe a box with a positive size.
    #[error("the stock's extents must describe a box with a positive size")]
    Extents,
    /// A bounding box was asked for but the document holds no solid geometry to size it from.
    #[error("there is no solid geometry to size the stock from")]
    NoGeometry,
    /// The definition names no solid body, or a body the document does not hold.
    #[error("the stock must name an existing solid body")]
    Body,
}

/// The material left around the part on each side, in millimetres: a positive value grows the
/// stock, a negative one trims it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StockExcess {
    /// The excess on the X axis, on both sides.
    pub x: f64,
    /// The excess on the Y axis, on both sides.
    pub y: f64,
    /// The excess on the Z axis: the facing allowance on the top, and usually on the bottom too.
    pub z: f64,
}

impl StockExcess {
    /// An excess of `x`, `y` and `z` millimetres.
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Whether every axis is a usable number.
    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// The excess as the three-array the stock's bounds use.
    const fn axes(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
}

/// How the stock is defined.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StockDef {
    /// A box around the part geometry, grown by the excess on each side.
    BoundingBox {
        /// `[min x, min y, min z, max x, max y, max z]`, in millimetres.
        extents: [f32; 6],
    },
    /// An existing solid body used as the raw material.
    Solid(EntityId),
}

impl StockDef {
    /// A bounding box from `min` and `max` part corners, grown by `excess` on each side.
    ///
    /// The result is not validated: the dialog calls [`StockDef::validate`] before it stores one,
    /// so an excess that swallows the part is reported instead of silently inverted.
    pub fn from_bounds(min: [f64; 3], max: [f64; 3], excess: StockExcess) -> Self {
        let grow = excess.axes();
        let mut extents = [0.0f32; 6];
        for axis in 0..3 {
            extents[axis] = (min[axis] - grow[axis]) as f32;
            extents[axis + 3] = (max[axis] + grow[axis]) as f32;
        }
        Self::BoundingBox { extents }
    }

    /// The box's extents as `[min x, min y, min z, max x, max y, max z]`, if this is a bounding
    /// box. A solid's box is the document's to work out, so it is not known here.
    pub fn extents(self) -> Option<[f64; 6]> {
        match self {
            Self::BoundingBox { extents } => Some([
                f64::from(extents[0]),
                f64::from(extents[1]),
                f64::from(extents[2]),
                f64::from(extents[3]),
                f64::from(extents[4]),
                f64::from(extents[5]),
            ]),
            Self::Solid(_) => None,
        }
    }

    /// The box's size along X, Y and Z in millimetres, if this is a bounding box.
    pub fn size(self) -> Option<[f64; 3]> {
        let extents = self.extents()?;
        Some([extents[3] - extents[0], extents[4] - extents[1], extents[5] - extents[2]])
    }

    /// Checks that the definition describes material: a box with a positive size on every axis,
    /// or a body id that could name one.
    pub fn validate(self) -> Result<(), StockError> {
        match self {
            Self::BoundingBox { extents } => {
                if !extents.iter().all(|value| value.is_finite()) {
                    return Err(StockError::Extents);
                }
                let size = [extents[3] - extents[0], extents[4] - extents[1], extents[5] - extents[2]];
                if size.iter().all(|value| *value > 0.0) { Ok(()) } else { Err(StockError::Extents) }
            }
            Self::Solid(id) => {
                if id == 0 {
                    Err(StockError::Body)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// The stock as the status bar names it, such as `60.0 × 40.0 × 12.0 mm` for a box.
    pub fn summary(self) -> String {
        match self.size() {
            Some([x, y, z]) => format!("{x:.1} × {y:.1} × {z:.1} mm"),
            None => match self {
                Self::Solid(id) => format!("solid {id}"),
                Self::BoundingBox { .. } => "stock".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bounding_box_grows_by_the_excess_on_every_axis() {
        let stock = StockDef::from_bounds([0.0, 0.0, 0.0], [40.0, 30.0, 12.0], StockExcess::new(5.0, 2.0, 1.0));
        assert_eq!(stock.extents(), Some([-5.0, -2.0, -1.0, 45.0, 32.0, 13.0]));
        assert_eq!(stock.size(), Some([50.0, 34.0, 14.0]));
        assert_eq!(stock.validate(), Ok(()));
    }

    #[test]
    fn a_box_with_no_height_is_refused() {
        let stock = StockDef::from_bounds([0.0, 0.0, 0.0], [40.0, 30.0, 12.0], StockExcess::new(0.0, 0.0, -6.0));
        assert_eq!(stock.validate(), Err(StockError::Extents), "negative excess swallowed the height");
        let nan = StockDef::BoundingBox { extents: [0.0, 0.0, 0.0, f32::NAN, 1.0, 1.0] };
        assert_eq!(nan.validate(), Err(StockError::Extents));
    }

    #[test]
    fn a_solid_stock_names_a_body() {
        assert_eq!(StockDef::Solid(1).validate(), Ok(()));
        assert_eq!(StockDef::Solid(0).validate(), Err(StockError::Body));
        assert_eq!(StockDef::Solid(1).extents(), None, "the document works out a solid's box");
    }

    #[test]
    fn a_stock_summarises_its_size() {
        let stock = StockDef::from_bounds([0.0, 0.0, 0.0], [60.0, 40.0, 12.0], StockExcess::default());
        assert_eq!(stock.summary(), "60.0 × 40.0 × 12.0 mm");
        assert_eq!(StockDef::Solid(3).summary(), "solid 3");
    }

    #[test]
    fn a_non_finite_excess_makes_an_unusable_box() {
        let stock = StockDef::from_bounds([0.0, 0.0, 0.0], [40.0, 30.0, 12.0], StockExcess::new(f64::INFINITY, 0.0, 0.0));
        assert_eq!(stock.validate(), Err(StockError::Extents));
        assert!(!StockExcess::new(f64::NAN, 0.0, 0.0).is_finite());
    }
}

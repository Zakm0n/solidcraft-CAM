//! The machining operations a program runs, one module per operation type.
//!
//! An operation is the unit a CAM programmer edits: it selects geometry, selects a tool and
//! carries the parameter pages the dialog shows. [`Operation`] pairs those parameters with the
//! toolpath calculated from them, so the manager can list the operation, toggle whether the
//! viewport draws it, and regenerate it after the geometry or the pages change. The definition
//! lives in the operation's own module ([`contour2d`]) and the moves in [`crate::toolpath`], so a
//! post processor and the viewport read exactly what the dialog stored.
//!
//! Phase 2C of the Z-CAM blueprint adds the 2D Contour operation ([`contour2d`]); phase 2D adds
//! its calculation and its place in the program ([`Operation`]).

use crate::tool::CuttingTool;
use crate::toolpath::Toolpath;

pub mod contour2d;

pub use contour2d::{ChainCurve, Contour2DError, Contour2DParams};

/// One stored operation: its parameter pages and the toolpath last calculated from them.
///
/// The toolpath is kept beside the parameters rather than recalculated every frame: the viewport
/// draws it, the toolpath tree lists it, and a regeneration is an explicit command. `revision`
/// changes whenever the shown path does, which is what lets the viewport skip a redraw.
#[derive(Clone, Debug, PartialEq)]
pub struct Operation {
    /// The parameter pages the dialog edits.
    pub params: Contour2DParams,
    /// The calculated moves; empty until [`Operation::generate`] has run.
    pub toolpath: Toolpath,
    /// Whether the viewport draws the toolpath.
    pub visible: bool,
    /// Bumped whenever the path or its visibility changes.
    pub revision: u64,
}

impl Default for Operation {
    /// An operation with the default pages, no path and its path shown: what a test or a new
    /// program starts from.
    fn default() -> Self {
        Self::new(Contour2DParams::default())
    }
}

impl Operation {
    /// An operation with its parameters, no calculated path yet and its path shown.
    pub fn new(params: Contour2DParams) -> Self {
        Self { params, toolpath: Toolpath::new(), visible: true, revision: 0 }
    }
    /// An operation whose path is calculated straight away, or the reason it cannot be.
    pub fn generated(params: Contour2DParams, chain: &[ChainCurve], tool: &CuttingTool) -> Result<Self, Contour2DError> {
        let mut operation = Self::new(params);
        operation.generate(chain, tool)?;
        Ok(operation)
    }

    /// Recalculates the toolpath from the parameters and `chain`.
    ///
    /// A refused calculation leaves the previous path in place and stores nothing, so a chain
    /// whose geometry has gone does not silently empty a verified program.
    pub fn generate(&mut self, chain: &[ChainCurve], tool: &CuttingTool) -> Result<(), Contour2DError> {
        let path = self.params.toolpath(chain, tool)?;
        self.toolpath = path;
        self.revision += 1;
        Ok(())
    }

    /// Whether the path has been calculated.
    pub fn is_generated(&self) -> bool {
        !self.toolpath.is_empty()
    }

    /// Shows or hides the path in the viewport.
    pub fn set_visible(&mut self, visible: bool) {
        if self.visible != visible {
            self.visible = visible;
            self.revision += 1;
        }
    }

    /// The operation as the toolpath tree lists it, such as
    /// `2D Contour · 4 entities · depth -10.0 mm`.
    pub fn summary(&self) -> String {
        self.params.summary()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{CuttingTool, ToolType};

    /// The four lines of a 40 × 30 rectangle, as a closed chain.
    fn rectangle() -> Vec<ChainCurve> {
        vec![
            ChainCurve { id: 1, points: vec![[0.0, 0.0, 0.0], [40.0, 0.0, 0.0]] },
            ChainCurve { id: 2, points: vec![[40.0, 0.0, 0.0], [40.0, 30.0, 0.0]] },
            ChainCurve { id: 3, points: vec![[40.0, 30.0, 0.0], [0.0, 30.0, 0.0]] },
            ChainCurve { id: 4, points: vec![[0.0, 30.0, 0.0], [0.0, 0.0, 0.0]] },
        ]
    }

    fn tool() -> CuttingTool {
        CuttingTool::new(ToolType::FlatEndmill, 6.0)
    }

    #[test]
    fn a_generated_operation_holds_its_path() {
        let params = Contour2DParams::new(vec![1, 2, 3, 4], 1);
        let generated = Operation::generated(params.clone(), &rectangle(), &tool());
        assert!(generated.is_ok(), "the rectangle generates: {generated:?}");
        let operation = generated.unwrap_or_default();
        assert!(operation.is_generated());
        assert!(operation.visible);
        assert_eq!(operation.params, params);
        assert_eq!(operation.revision, 1);
        assert!(operation.summary().starts_with("2D Contour · 4 entities"));
    }

    #[test]
    fn visibility_and_regeneration_bump_the_revision() {
        let params = Contour2DParams::new(vec![1, 2, 3, 4], 1);
        let mut operation = Operation::generated(params, &rectangle(), &tool()).unwrap_or_default();
        let generated = operation.revision;
        operation.generate(&rectangle(), &tool()).unwrap_or_default();
        assert_eq!(operation.revision, generated + 1, "regenerating redraws the path");
        operation.set_visible(false);
        assert!(!operation.visible);
        assert_eq!(operation.revision, generated + 2);
        operation.set_visible(false);
        assert_eq!(operation.revision, generated + 2, "setting the same visibility changes nothing");
    }

    #[test]
    fn a_refused_regeneration_keeps_the_previous_path() {
        let params = Contour2DParams::new(vec![1, 2, 3, 4], 1);
        let mut operation = Operation::generated(params, &rectangle(), &tool()).unwrap_or_default();
        let kept = operation.toolpath.clone();
        let refused = operation.generate(&[], &tool());
        assert_eq!(refused, Err(Contour2DError::ChainGeometry));
        assert_eq!(operation.toolpath, kept, "the verified path stays");
    }
}

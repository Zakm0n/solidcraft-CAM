//! The machining operations a program runs, one module per operation type.
//!
//! An operation is the unit a CAM programmer edits: it selects geometry, selects a tool and
//! carries the parameter pages the dialog shows. The operation computes its moves in a later
//! step; this module is where its definition lives, so the dialog, the toolpath calculation and
//! a future post processor all read the same values.
//!
//! Phase 2C of the Z-CAM blueprint adds the 2D Contour operation ([`contour2d`]).

pub mod contour2d;

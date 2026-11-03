//! Mastercam parameter mapping reference.
//!
//! Every documented Mastercam parameter is either mapped to a field of this crate, carried
//! only partially, deferred to a later operation, or explicitly unsupported. The table is the
//! single source for the docs check in `docs/mastercam-mapping.md`; add a row here and in the
//! document together.

/// How completely a Mastercam parameter is represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingStatus {
    /// Behaviour is implemented.
    Mapped,
    /// The parameter is carried and used, but not the full Mastercam behaviour.
    Partial,
    /// Reserved for a later milestone.
    Deferred,
    /// Deliberately not modelled.
    Unsupported,
}

/// Where the mapping came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingSource {
    /// Public Mastercam documentation/training material.
    PublicDocs,
    /// A stated assumption until a manual is supplied.
    Assumption,
    /// An uploaded manual (none yet).
    Manual,
}

/// One Mastercam parameter and its home in this crate.
#[derive(Debug, Clone, Copy)]
pub struct ParameterMap {
    pub operation: &'static str,
    pub mastercam: &'static str,
    pub ours: &'static str,
    pub status: MappingStatus,
    pub source: MappingSource,
    pub default: &'static str,
    pub note: &'static str,
}

#[cfg(test)]
const DOCS: &str = include_str!("../../../docs/mastercam-mapping.md");

use MappingSource::PublicDocs;
use MappingStatus::{Deferred, Mapped, Partial, Unsupported};

/// The complete mapping table.
pub const MAPPING: &[ParameterMap] = &[
    // Contour
    ParameterMap {
        operation: "Contour",
        mastercam: "Compensation type",
        ours: "OperationOptions::compensation",
        status: Mapped,
        source: PublicDocs,
        default: "None",
        note: "computer compensation only",
    },
    ParameterMap {
        operation: "Contour",
        mastercam: "Lead in",
        ours: "OperationOptions::lead_in_mm",
        status: Partial,
        source: PublicDocs,
        default: "3.0 mm",
        note: "straight lead-in only; lead-out and arc leads deferred",
    },
    ParameterMap {
        operation: "Contour",
        mastercam: "Ramp entry",
        ours: "OperationOptions::entry",
        status: Mapped,
        source: PublicDocs,
        default: "Plunge",
        note: "ramp follows the contour",
    },
    ParameterMap {
        operation: "Contour",
        mastercam: "Depth cuts",
        ours: "RoutingParams::depth_per_pass_mm",
        status: Mapped,
        source: PublicDocs,
        default: "tool depth of cut",
        note: "final pass lands on depth",
    },
    ParameterMap {
        operation: "Contour",
        mastercam: "Multi passes",
        ours: "OperationKind::Contour::finish_passes",
        status: Partial,
        source: PublicDocs,
        default: "1",
        note: "count recorded; wall spacing not generated",
    },
    ParameterMap {
        operation: "Contour",
        mastercam: "Tabs",
        ours: "toolpath::Tab",
        status: Mapped,
        source: PublicDocs,
        default: "none",
        note: "recorded and posted; simulation lift deferred",
    },
    ParameterMap {
        operation: "Contour",
        mastercam: "Stock to leave",
        ours: "—",
        status: Unsupported,
        source: PublicDocs,
        default: "—",
        note: "no allowance generated",
    },
    ParameterMap {
        operation: "Contour",
        mastercam: "Linking: clearance / retract",
        ours: "RoutingParams::clearance_mm",
        status: Partial,
        source: PublicDocs,
        default: "10 / 2 mm",
        note: "feed plane and top-of-stock deferred",
    },
    ParameterMap {
        operation: "Contour",
        mastercam: "Feed rate / spindle speed",
        ours: "CuttingData::feed_mm_min",
        status: Mapped,
        source: PublicDocs,
        default: "material + tool",
        note: "spindle from CuttingData",
    },
    // Pocket
    ParameterMap {
        operation: "Pocket",
        mastercam: "Cutting method",
        ours: "OperationOptions::pocket_strategy",
        status: Partial,
        source: PublicDocs,
        default: "Spiral",
        note: "zig-zag and offset spiral; constant-overlap/morph deferred",
    },
    ParameterMap {
        operation: "Pocket",
        mastercam: "Stepover",
        ours: "stepover_mm",
        status: Mapped,
        source: PublicDocs,
        default: "tool cutting data",
        note: "percentage input not modelled",
    },
    ParameterMap {
        operation: "Pocket",
        mastercam: "Depth cuts",
        ours: "depth_per_pass_mm",
        status: Mapped,
        source: PublicDocs,
        default: "tool depth of cut",
        note: "final pass lands exactly on depth",
    },
    ParameterMap {
        operation: "Pocket",
        mastercam: "Finish pass",
        ours: "OperationOptions::finish_pass",
        status: Partial,
        source: PublicDocs,
        default: "true",
        note: "one wall pass; wall/floor allowances deferred",
    },
    ParameterMap {
        operation: "Pocket",
        mastercam: "Entry",
        ours: "EntryStyle",
        status: Mapped,
        source: PublicDocs,
        default: "Plunge",
        note: "plunge, ramp, helix",
    },
    ParameterMap {
        operation: "Pocket",
        mastercam: "Islands",
        ours: "geometry::Region::islands",
        status: Mapped,
        source: PublicDocs,
        default: "—",
        note: "clipper subtracts islands",
    },
    ParameterMap {
        operation: "Pocket",
        mastercam: "Stock to leave",
        ours: "—",
        status: Unsupported,
        source: PublicDocs,
        default: "—",
        note: "no allowance is generated yet",
    },
    // Face
    ParameterMap {
        operation: "Face",
        mastercam: "Cutting angle",
        ours: "OperationOptions::facing_angle_deg",
        status: Mapped,
        source: PublicDocs,
        default: "0°",
        note: "raster rotates about the stock centre",
    },
    ParameterMap {
        operation: "Face",
        mastercam: "Stepover / overlap",
        ours: "stepover_mm",
        status: Partial,
        source: PublicDocs,
        default: "tool cutting data",
        note: "separate overlap percentage deferred",
    },
    ParameterMap {
        operation: "Face",
        mastercam: "Depth cuts",
        ours: "depth_per_pass_mm",
        status: Mapped,
        source: PublicDocs,
        default: "tool depth of cut",
        note: "repeated raster at each level",
    },
    ParameterMap {
        operation: "Face",
        mastercam: "Stock to leave",
        ours: "—",
        status: Unsupported,
        source: PublicDocs,
        default: "—",
        note: "facing always cuts to the requested depth",
    },
    // Circle Mill
    ParameterMap {
        operation: "Circle Mill",
        mastercam: "Circle diameter",
        ours: "OperationKind::CircleMill::diameter_mm",
        status: Mapped,
        source: PublicDocs,
        default: "—",
        note: "tool diameter subtracted",
    },
    ParameterMap {
        operation: "Circle Mill",
        mastercam: "Centre point",
        ours: "OperationKind::CircleMill::center",
        status: Mapped,
        source: PublicDocs,
        default: "—",
        note: "WCS point, no geometry selection needed",
    },
    ParameterMap {
        operation: "Circle Mill",
        mastercam: "Helical entry",
        ours: "EntryStyle::Helix",
        status: Mapped,
        source: PublicDocs,
        default: "Plunge",
        note: "descending full circle",
    },
    ParameterMap {
        operation: "Circle Mill",
        mastercam: "Depth cuts",
        ours: "depth_per_pass_mm",
        status: Mapped,
        source: PublicDocs,
        default: "tool depth of cut",
        note: "one full circle per level",
    },
    ParameterMap {
        operation: "Circle Mill",
        mastercam: "Lead in/out",
        ours: "—",
        status: Deferred,
        source: PublicDocs,
        default: "—",
        note: "path is a full circle",
    },
    // Slot Mill
    ParameterMap {
        operation: "Slot",
        mastercam: "Slot width",
        ours: "OperationKind::Slot::width_mm",
        status: Mapped,
        source: PublicDocs,
        default: "—",
        note: "two offset passes",
    },
    ParameterMap {
        operation: "Slot",
        mastercam: "Centreline",
        ours: "Operation::selection",
        status: Mapped,
        source: PublicDocs,
        default: "—",
        note: "closed CAM chain",
    },
    ParameterMap {
        operation: "Slot",
        mastercam: "Compensation",
        ours: "OperationOptions::compensation",
        status: Mapped,
        source: PublicDocs,
        default: "None",
        note: "computer side",
    },
    ParameterMap {
        operation: "Slot",
        mastercam: "Multiple width passes",
        ours: "—",
        status: Unsupported,
        source: PublicDocs,
        default: "—",
        note: "wide slots need more passes in Mastercam",
    },
    // Engrave
    ParameterMap {
        operation: "Engrave",
        mastercam: "Curves",
        ours: "Operation::selection",
        status: Mapped,
        source: PublicDocs,
        default: "—",
        note: "open or closed chains",
    },
    ParameterMap {
        operation: "Engrave",
        mastercam: "Depth",
        ours: "Operation::depth_mm",
        status: Mapped,
        source: PublicDocs,
        default: "—",
        note: "single shallow pass by default",
    },
    ParameterMap {
        operation: "Engrave",
        mastercam: "Direction",
        ours: "Chain::reversed",
        status: Mapped,
        source: PublicDocs,
        default: "—",
        note: "follows chain direction",
    },
    ParameterMap {
        operation: "Engrave",
        mastercam: "Clearance / retract",
        ours: "RoutingParams::clearance_mm",
        status: Partial,
        source: PublicDocs,
        default: "10 / 2 mm",
        note: "feed plane deferred",
    },
    ParameterMap {
        operation: "Engrave",
        mastercam: "Stock to leave",
        ours: "—",
        status: Unsupported,
        source: PublicDocs,
        default: "—",
        note: "roll around corners deferred",
    },
    // Deferred operations
    ParameterMap {
        operation: "Dynamic Contour",
        mastercam: "Engagement angle",
        ours: "—",
        status: Deferred,
        source: PublicDocs,
        default: "—",
        note: "needs constant-engagement engine",
    },
    ParameterMap {
        operation: "Dynamic Mill",
        mastercam: "Micro-lift",
        ours: "—",
        status: Deferred,
        source: PublicDocs,
        default: "—",
        note: "rest material and constant engagement",
    },
    ParameterMap {
        operation: "Peel Mill",
        mastercam: "Trochoidal stepover",
        ours: "—",
        status: Deferred,
        source: PublicDocs,
        default: "—",
        note: "trochoidal motion",
    },
    ParameterMap {
        operation: "Area Mill",
        mastercam: "Rest material",
        ours: "—",
        status: Deferred,
        source: PublicDocs,
        default: "—",
        note: "rest-material tracking",
    },
    ParameterMap {
        operation: "Blend Mill",
        mastercam: "Blend chains",
        ours: "—",
        status: Deferred,
        source: PublicDocs,
        default: "—",
        note: "open-chain blending",
    },
    ParameterMap {
        operation: "Model Chamfer",
        mastercam: "Chamfer width",
        ours: "—",
        status: Deferred,
        source: PublicDocs,
        default: "—",
        note: "solid-model chamfer recognition",
    },
    ParameterMap {
        operation: "Helix Bore",
        mastercam: "Bore diameter",
        ours: "—",
        status: Deferred,
        source: PublicDocs,
        default: "—",
        note: "helical bore cycles",
    },
    ParameterMap {
        operation: "Thread Mill",
        mastercam: "Thread pitch",
        ours: "—",
        status: Deferred,
        source: PublicDocs,
        default: "—",
        note: "pitch and multi-helix paths",
    },
];

/// Every row for one operation.
pub fn for_operation(operation: &str) -> impl Iterator<Item = &'static ParameterMap> + '_ {
    MAPPING.iter().filter(move |row| row.operation == operation)
}

/// Counts by status: (mapped, partial, deferred, unsupported).
pub fn counts() -> (usize, usize, usize, usize) {
    let count = |status: MappingStatus| MAPPING.iter().filter(|row| row.status == status).count();
    (count(Mapped), count(Partial), count(Deferred), count(Unsupported))
}

/// The documented operations the mapping covers.
pub const OPERATIONS: &[&str] = &[
    "Contour",
    "Pocket",
    "Face",
    "Circle Mill",
    "Slot",
    "Engrave",
    "Dynamic Contour",
    "Dynamic Mill",
    "Peel Mill",
    "Area Mill",
    "Blend Mill",
    "Model Chamfer",
    "Helix Bore",
    "Thread Mill",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_complete_and_documented() {
        for row in MAPPING {
            assert!(!row.operation.is_empty());
            assert!(!row.mastercam.is_empty());
            assert!(!row.ours.is_empty());
            assert!(!row.note.is_empty(), "{} / {} has no note", row.operation, row.mastercam);
            assert!(DOCS.contains(row.mastercam), "docs missing Mastercam parameter `{}`", row.mastercam);
            assert!(DOCS.contains(row.ours), "docs missing our field `{}`", row.ours);
        }
    }

    #[test]
    fn core_operations_have_real_mappings() {
        for operation in ["Contour", "Pocket", "Face", "Circle Mill", "Slot", "Engrave"] {
            let mapped = for_operation(operation).filter(|row| matches!(row.status, Mapped | Partial)).count();
            assert!(mapped >= 3, "{operation} has only {mapped} implemented mappings");
        }
    }

    #[test]
    fn deferred_operations_stay_deferred() {
        for operation in ["Dynamic Contour", "Dynamic Mill", "Peel Mill", "Area Mill", "Blend Mill", "Model Chamfer", "Helix Bore", "Thread Mill"] {
            let any_implemented = for_operation(operation).any(|row| matches!(row.status, Mapped | Partial));
            assert!(!any_implemented, "{operation} claims an implemented mapping too early");
        }
    }

    #[test]
    fn no_duplicate_operation_parameter_rows() {
        for (index, row) in MAPPING.iter().enumerate() {
            let duplicate = MAPPING[..index].iter().any(|other| other.operation == row.operation && other.mastercam == row.mastercam);
            assert!(!duplicate, "duplicate row: {} / {}", row.operation, row.mastercam);
        }
    }

    #[test]
    fn counts_cover_the_whole_table() {
        let (mapped, partial, deferred, unsupported) = counts();
        assert_eq!(mapped + partial + deferred + unsupported, MAPPING.len());
        assert!(mapped > 0 && partial > 0 && deferred > 0 && unsupported > 0);
    }
}

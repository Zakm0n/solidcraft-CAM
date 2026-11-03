//! Holders, shanks and assemblies: the physical stack above the cutter.
//!
//! A [`Holder`] is a revolved profile of cylinders and cones; a [`ToolAssembly`] references a
//! catalog tool and holder, optionally overriding diameter, stickout or flute length for a
//! particular machine setup. cutrail collision sees the largest enclosing cylinder of the
//! holder and the derived shank; the full profile is kept for display and future mesh collision.

use cutrail::{Holder as CutrailHolder, Shank as CutrailShank, Tool as CutrailTool};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::tool::{Tool, ToolError, ToolId};

#[derive(Debug, Error, PartialEq)]
pub enum ToolingError {
    #[error("holder/tool id or name is empty")]
    EmptyName,
    #[error("a holder needs at least one profile segment")]
    EmptyProfile,
    #[error("profile segment length must be greater than zero, got {0}")]
    BadSegmentLength(f64),
    #[error("profile diameters must be greater than zero, got {0} and {1}")]
    BadSegmentDiameter(f64, f64),
    #[error("gauge length must be greater than zero, got {0}")]
    BadGaugeLength(f64),
    #[error("an entry named `{0}` already exists")]
    DuplicateName(String),
    #[error("the tool does not fit the assembly: gauge {gauge} mm is shorter than stickout {stickout} mm")]
    GaugeTooShort { gauge: f64, stickout: f64 },
    #[error("the cutter has no cutrail shape for this assembly")]
    UnsupportedCutter,
    #[error(transparent)]
    Tool(#[from] ToolError),
}

/// One revolved holder/shank segment: a cylinder when both diameters match, a cone otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProfileSegment {
    pub length_mm: f64,
    pub top_diameter_mm: f64,
    pub bottom_diameter_mm: f64,
}

impl ProfileSegment {
    pub fn new(length_mm: f64, top_diameter_mm: f64, bottom_diameter_mm: f64) -> Result<Self, ToolingError> {
        if !length_mm.is_finite() || length_mm <= 0.0 {
            return Err(ToolingError::BadSegmentLength(length_mm));
        }
        if !top_diameter_mm.is_finite() || !bottom_diameter_mm.is_finite() || top_diameter_mm <= 0.0 || bottom_diameter_mm <= 0.0 {
            return Err(ToolingError::BadSegmentDiameter(top_diameter_mm, bottom_diameter_mm));
        }
        Ok(Self { length_mm, top_diameter_mm, bottom_diameter_mm })
    }

    pub fn max_diameter_mm(&self) -> f64 {
        self.top_diameter_mm.max(self.bottom_diameter_mm)
    }
}

/// A holder modelled as a stack of revolved segments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Holder {
    id: String,
    name: String,
    profile: Vec<ProfileSegment>,
    gauge_length_mm: f64,
}

impl Holder {
    pub fn new(id: impl Into<String>, name: impl Into<String>, profile: Vec<ProfileSegment>, gauge_length_mm: f64) -> Result<Self, ToolingError> {
        let id = id.into();
        let name = name.into();
        if id.trim().is_empty() || name.trim().is_empty() {
            return Err(ToolingError::EmptyName);
        }
        if profile.is_empty() {
            return Err(ToolingError::EmptyProfile);
        }
        if !gauge_length_mm.is_finite() || gauge_length_mm <= 0.0 {
            return Err(ToolingError::BadGaugeLength(gauge_length_mm));
        }
        Ok(Self { id, name, profile, gauge_length_mm })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn profile(&self) -> &[ProfileSegment] {
        &self.profile
    }

    pub fn gauge_length_mm(&self) -> f64 {
        self.gauge_length_mm
    }

    pub fn length_mm(&self) -> f64 {
        self.profile.iter().map(|segment| segment.length_mm).sum()
    }

    pub fn max_diameter_mm(&self) -> f64 {
        self.profile.iter().map(ProfileSegment::max_diameter_mm).fold(0.0, f64::max)
    }

    /// cutrail's collision envelope: the largest diameter over the profile length.
    pub fn to_cutrail(&self) -> CutrailHolder {
        CutrailHolder { diameter: self.max_diameter_mm(), length: self.length_mm() }
    }
}

/// A shank between the flutes and the holder.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Shank {
    pub diameter_mm: f64,
    pub length_mm: f64,
}

impl Shank {
    pub fn to_cutrail(&self) -> CutrailShank {
        CutrailShank { diameter: self.diameter_mm, length: self.length_mm }
    }
}

/// Per-assembly overrides of the catalog tool.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolOverrides {
    #[serde(default)]
    pub diameter_mm: Option<f64>,
    #[serde(default)]
    pub stickout_mm: Option<f64>,
    #[serde(default)]
    pub flute_length_mm: Option<f64>,
}

/// A tool plus a holder, with the data needed at the machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolAssembly {
    id: String,
    name: String,
    tool_id: ToolId,
    #[serde(default)]
    holder_id: Option<String>,
    #[serde(default)]
    overrides: ToolOverrides,
    #[serde(default)]
    tool_number: Option<u32>,
    #[serde(default)]
    diameter_offset_mm: f64,
    #[serde(default)]
    length_offset_mm: f64,
    gauge_length_mm: f64,
}

impl ToolAssembly {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        tool_id: ToolId,
        holder_id: Option<String>,
        gauge_length_mm: f64,
    ) -> Result<Self, ToolingError> {
        let id = id.into();
        let name = name.into();
        if id.trim().is_empty() || name.trim().is_empty() {
            return Err(ToolingError::EmptyName);
        }
        if !gauge_length_mm.is_finite() || gauge_length_mm <= 0.0 {
            return Err(ToolingError::BadGaugeLength(gauge_length_mm));
        }
        Ok(Self {
            id,
            name,
            tool_id,
            holder_id,
            overrides: ToolOverrides::default(),
            tool_number: None,
            diameter_offset_mm: 0.0,
            length_offset_mm: 0.0,
            gauge_length_mm,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    pub fn holder_id(&self) -> Option<&str> {
        self.holder_id.as_deref()
    }

    pub fn overrides(&self) -> ToolOverrides {
        self.overrides
    }

    pub fn with_overrides(mut self, overrides: ToolOverrides) -> Self {
        self.overrides = overrides;
        self
    }

    pub fn set_overrides(&mut self, overrides: ToolOverrides) {
        self.overrides = overrides;
    }

    /// Replace the holder reference (used when a holder is removed from the catalog).
    pub fn set_holder(&mut self, holder_id: Option<String>) {
        self.holder_id = holder_id;
    }

    pub fn tool_number(&self) -> Option<u32> {
        self.tool_number
    }

    pub fn with_tool_number(mut self, number: u32) -> Self {
        self.tool_number = Some(number);
        self
    }

    pub fn diameter_offset_mm(&self) -> f64 {
        self.diameter_offset_mm
    }

    pub fn length_offset_mm(&self) -> f64 {
        self.length_offset_mm
    }

    pub fn with_offsets(mut self, diameter_offset_mm: f64, length_offset_mm: f64) -> Self {
        self.diameter_offset_mm = diameter_offset_mm;
        self.length_offset_mm = length_offset_mm;
        self
    }

    pub fn gauge_length_mm(&self) -> f64 {
        self.gauge_length_mm
    }

    /// The catalog tool with this assembly's overrides applied.
    pub fn effective_tool(&self, tool: &Tool) -> Result<Tool, ToolingError> {
        let mut effective = tool.clone();
        if let Some(diameter) = self.overrides.diameter_mm {
            effective = effective.with_diameter_mm(diameter)?;
        }
        if let Some(flute_length) = self.overrides.flute_length_mm {
            effective = effective.with_flute_length_mm(flute_length)?;
        }
        if let Some(stickout) = self.overrides.stickout_mm {
            effective = effective.with_stickout_mm(stickout)?;
        }
        Ok(effective)
    }

    /// Shank from the flutes to the holder, if there is any exposed length.
    pub fn shank(&self, tool: &Tool) -> Option<Shank> {
        let effective = self.effective_tool(tool).ok()?;
        let length = effective.stickout_mm() - effective.flute_length_mm();
        if length <= 1e-9 {
            return None;
        }
        Some(Shank { diameter_mm: tool.diameter_mm(), length_mm: length })
    }

    /// The assembly as cutrail sees it, with shank and holder envelopes.
    pub fn to_cutrail(&self, tool: &Tool, holder: Option<&Holder>) -> Result<CutrailTool, ToolingError> {
        let effective = self.effective_tool(tool)?;
        if self.gauge_length_mm + 1e-9 < effective.stickout_mm() {
            return Err(ToolingError::GaugeTooShort { gauge: self.gauge_length_mm, stickout: effective.stickout_mm() });
        }
        let mut assembled = effective.to_cutrail().ok_or(ToolingError::UnsupportedCutter)?;
        if let Some(shank) = self.shank(&effective) {
            assembled = assembled.with_shank(shank.to_cutrail());
        }
        if let Some(holder) = holder {
            assembled = assembled.with_holder(holder.to_cutrail());
        }
        Ok(assembled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::Cutter;

    fn tool() -> Tool {
        Tool::new("6mm flat", Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap()
    }

    fn holder() -> Holder {
        Holder::new("er16", "ER16 collet", vec![ProfileSegment::new(15.0, 28.0, 22.0).unwrap(), ProfileSegment::new(20.0, 28.0, 28.0).unwrap()], 45.0)
            .unwrap()
    }

    #[test]
    fn profile_segments_validate_and_envelope() {
        assert!(ProfileSegment::new(0.0, 20.0, 20.0).is_err());
        assert!(ProfileSegment::new(10.0, 0.0, 20.0).is_err());
        assert!(Holder::new("h", "empty", vec![], 40.0).is_err());
        let holder = holder();
        assert_eq!(holder.length_mm(), 35.0);
        assert_eq!(holder.max_diameter_mm(), 28.0);
        let envelope = holder.to_cutrail();
        assert_eq!(envelope.diameter, 28.0);
        assert_eq!(envelope.length, 35.0);
    }

    #[test]
    fn overrides_take_precedence_and_derive_the_shank() {
        let assembly = ToolAssembly::new("asm1", "6mm flat in ER16", tool().id().clone(), Some("er16".into()), 60.0)
            .unwrap()
            .with_overrides(ToolOverrides { diameter_mm: Some(8.0), stickout_mm: Some(45.0), flute_length_mm: Some(25.0) });
        let effective = assembly.effective_tool(&tool()).unwrap();
        assert_eq!(effective.diameter_mm(), 8.0);
        assert_eq!(effective.stickout_mm(), 45.0);
        assert_eq!(effective.flute_length_mm(), 25.0);
        let shank = assembly.shank(&tool()).unwrap();
        assert_eq!(shank.length_mm, 20.0);
        assert_eq!(shank.diameter_mm, 6.0);
    }

    #[test]
    fn an_assembly_converts_to_cutrail_with_holder_and_shank() {
        let assembly = ToolAssembly::new("a", "a", tool().id().clone(), Some("er16".into()), 60.0).unwrap();
        let cutter = assembly.to_cutrail(&tool(), Some(&holder())).unwrap();
        assert!(cutter.shank.is_some());
        assert!(cutter.holder.is_some());
        assert_eq!(cutter.holder.unwrap().diameter, 28.0);
    }

    #[test]
    fn a_short_gauge_is_rejected() {
        let assembly = ToolAssembly::new("a", "a", tool().id().clone(), None, 10.0).unwrap();
        assert!(matches!(assembly.to_cutrail(&tool(), None), Err(ToolingError::GaugeTooShort { .. })));
    }

    #[test]
    fn a_holder_collides_with_the_stock_in_cutrail() {
        use crate::setup::Setup;
        use crate::simulate::Simulation;
        use crate::stock::{Material, Stock};
        use crate::toolpath::{Heights, Toolpath};

        let stock = Stock::new([0.0, 0.0, 0.0], [60.0, 40.0, 20.0], Material::Aluminum).unwrap();
        let setup = Setup::top_centre(&stock);
        let short = Tool::new("6mm flat", Cutter::Flat, 6.0, 3, 4.0, 5.0).unwrap();
        let big = Holder::new("big", "Big holder", vec![ProfileSegment::new(20.0, 40.0, 40.0).unwrap()], 50.0).unwrap();
        let assembly = ToolAssembly::new("a", "a", short.id().clone(), Some("big".into()), 50.0).unwrap();
        let cutter = assembly.to_cutrail(&short, Some(&big)).unwrap();

        let mut path = Toolpath::new([-10.0, 0.0, -8.0], Heights::default()).unwrap();
        path.feed([10.0, 0.0, -8.0], 300.0).unwrap();
        let mut simulation = Simulation::new(&stock, 5.0).unwrap();
        let report = simulation.cut_path_cutrail(&cutter, &path, &setup).unwrap();
        assert!(report.collisions > 0, "the oversized holder did not collide");
    }

    #[test]
    fn unsupported_cutters_are_reported() {
        let drill = Tool::new("drill", Cutter::Drill { point_angle_deg: 118.0 }, 5.0, 2, 20.0, 30.0).unwrap();
        let assembly = ToolAssembly::new("a", "a", drill.id().clone(), None, 60.0).unwrap();
        assert!(matches!(assembly.to_cutrail(&drill, None), Err(ToolingError::UnsupportedCutter)));
    }
}

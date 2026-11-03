//! Tools and cutting data: which cutter, how big, how many flutes, and how fast to run it.
//!
//! A [`Tool`] is the physical cutter (geometry and size); [`CuttingData`] is how the job uses
//! it (spindle speed, feed, depth of cut, stepover). Keeping them separate matters because the
//! same cutter is run differently in aluminum and in steel, and a tool library outlives one job.

use cutrail::Tool as CutrailTool;
use thiserror::Error;

use crate::stock::Material;
use serde::{Deserialize, Serialize};

/// The shape of the cutting end.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Cutter {
    /// Square corner: a flat end mill, the workhorse for pockets and profiles.
    Flat,
    /// Ball nose: for 3D finishing.
    Ball,
    /// Rounded corner: strong enough to rough, smooth enough to finish.
    BullNose { corner_radius_mm: f64 },
    /// A drill. cutrail simulates milling tools only, so a drill has no direct cutrail shape.
    Drill { point_angle_deg: f64 },
    /// Chamfer / V-bit: included angle and flat tip diameter.
    Chamfer { tip_angle_deg: f64, tip_diameter_mm: f64 },
    /// Face mill with a lead angle.
    FaceMill { lead_angle_deg: f64 },
    /// Thread mill with its thread form angle.
    ThreadMill { thread_angle_deg: f64 },
    /// Tapered end mill: half angle and tip diameter.
    Tapered { half_angle_deg: f64, tip_diameter_mm: f64 },
    /// Dovetail cutter with an undercut angle.
    Dovetail { undercut_angle_deg: f64 },
    /// Slot cutter: the cutter thickness is narrower than its diameter.
    Slot { thickness_mm: f64 },
}

impl Cutter {
    pub fn label(&self) -> &'static str {
        match self {
            Cutter::Flat => "Flat end mill",
            Cutter::Ball => "Ball end mill",
            Cutter::BullNose { .. } => "Bull-nose end mill",
            Cutter::Drill { .. } => "Drill",
            Cutter::Chamfer { .. } => "Chamfer mill",
            Cutter::FaceMill { .. } => "Face mill",
            Cutter::ThreadMill { .. } => "Thread mill",
            Cutter::Tapered { .. } => "Tapered end mill",
            Cutter::Dovetail { .. } => "Dovetail cutter",
            Cutter::Slot { .. } => "Slot cutter",
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ToolError {
    #[error("tool name is empty")]
    EmptyName,
    #[error("tool id is empty")]
    EmptyId,
    #[error("tool diameter must be finite and greater than zero, got {0} mm")]
    BadDiameter(f64),
    #[error("a tool needs at least one flute, got {0}")]
    NoFlutes(u32),
    #[error("flute length must be finite and greater than zero, got {0} mm")]
    BadFluteLength(f64),
    #[error("stickout ({0} mm) must be at least the flute length ({1} mm)")]
    StickoutTooShort(f64, f64),
    #[error("corner radius must be greater than zero and less than half the diameter, got {0} mm")]
    BadCornerRadius(f64),
    #[error("drill point angle must be between 0 and 180 degrees, got {0}")]
    BadPointAngle(f64),
    #[error("chamfer tip angle must be between 0 and 180 degrees, got {0}")]
    BadTipAngle(f64),
    #[error("tip diameter must be non-negative and smaller than the cutter diameter, got {0} mm")]
    BadTipDiameter(f64),
    #[error("face-mill lead angle must be between 0 and 90 degrees, got {0}")]
    BadLeadAngle(f64),
    #[error("taper half angle must be between 0 and 45 degrees, got {0}")]
    BadTaperAngle(f64),
    #[error("dovetail undercut angle must be between 0 and 45 degrees, got {0}")]
    BadUndercutAngle(f64),
    #[error("slot thickness must be greater than zero and no more than the diameter, got {0} mm")]
    BadSlotThickness(f64),
    #[error("an entry named `{0}` is already in the library")]
    DuplicateName(String),
    #[error("cutting data must be finite and greater than zero: {0}")]
    BadCuttingData(&'static str),
}

/// A material's starting cutting parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CuttingPreset {
    /// Surface speed, m/min.
    pub surface_speed_m_min: f64,
    /// Chip load per flute per millimetre of diameter.
    pub chip_load_per_mm_diameter: f64,
}

/// Conservative shop presets by material.
pub fn cutting_preset(material: &Material) -> CuttingPreset {
    match material {
        Material::Aluminum => CuttingPreset { surface_speed_m_min: 500.0, chip_load_per_mm_diameter: 0.02 },
        Material::Brass => CuttingPreset { surface_speed_m_min: 300.0, chip_load_per_mm_diameter: 0.015 },
        Material::Plastic => CuttingPreset { surface_speed_m_min: 300.0, chip_load_per_mm_diameter: 0.025 },
        Material::Steel => CuttingPreset { surface_speed_m_min: 120.0, chip_load_per_mm_diameter: 0.008 },
        Material::Other(_) => CuttingPreset { surface_speed_m_min: 150.0, chip_load_per_mm_diameter: 0.012 },
    }
}

/// Spindle speed, feed and depth of cut: how the job runs one tool.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CuttingData {
    spindle_rpm: f64,
    feed_mm_min: f64,
    plunge_feed_mm_min: f64,
    depth_of_cut_mm: f64,
    stepover_mm: f64,
}

impl CuttingData {
    pub fn new(spindle_rpm: f64, feed_mm_min: f64, plunge_feed_mm_min: f64, depth_of_cut_mm: f64, stepover_mm: f64) -> Result<Self, ToolError> {
        for (name, value) in [
            ("spindle speed", spindle_rpm),
            ("feed", feed_mm_min),
            ("plunge feed", plunge_feed_mm_min),
            ("depth of cut", depth_of_cut_mm),
            ("stepover", stepover_mm),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(ToolError::BadCuttingData(name));
            }
        }
        Ok(Self { spindle_rpm, feed_mm_min, plunge_feed_mm_min, depth_of_cut_mm, stepover_mm })
    }

    /// Starting values from the material's surface speed and the cutter's geometry.
    ///
    /// These are conservative shop defaults, not a substitute for a tool maker's chart:
    /// surface speed in m/min, a chip load proportional to diameter, a 45% stepover for
    /// flat/bull-nose cutters and 10% for ball-nose finishing.
    pub fn recommended(material: &Material, tool: &Tool) -> Self {
        let preset = cutting_preset(material);
        let d = tool.diameter_mm;
        let raw_rpm = preset.surface_speed_m_min * 1000.0 / (std::f64::consts::PI * d);
        let spindle_rpm = raw_rpm.clamp(1_000.0, 24_000.0);
        let chip_load = preset.chip_load_per_mm_diameter * d;
        let feed = spindle_rpm * tool.flutes as f64 * chip_load;
        let stepover_factor = match tool.cutter {
            Cutter::Ball => 0.10,
            _ => 0.45,
        };
        Self {
            spindle_rpm,
            feed_mm_min: feed,
            plunge_feed_mm_min: feed * 0.3,
            depth_of_cut_mm: 0.5 * d,
            stepover_mm: (stepover_factor * d).max(0.05),
        }
    }

    pub fn spindle_rpm(&self) -> f64 {
        self.spindle_rpm
    }

    pub fn feed_mm_min(&self) -> f64 {
        self.feed_mm_min
    }

    pub fn plunge_feed_mm_min(&self) -> f64 {
        self.plunge_feed_mm_min
    }

    pub fn depth_of_cut_mm(&self) -> f64 {
        self.depth_of_cut_mm
    }

    pub fn stepover_mm(&self) -> f64 {
        self.stepover_mm
    }
}

/// A stable identifier for a tool in a catalog.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ToolId(String);

impl ToolId {
    pub fn new(id: impl Into<String>) -> Result<Self, ToolError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(ToolError::EmptyId);
        }
        Ok(Self(id))
    }

    /// A stable slug from a display name (letters/digits, other runs become `-`).
    pub fn from_name(name: &str) -> Self {
        let mut slug = String::new();
        let mut dash = false;
        for c in name.chars() {
            if c.is_ascii_alphanumeric() {
                slug.push(c.to_ascii_lowercase());
                dash = false;
            } else if !dash {
                slug.push('-');
                dash = true;
            }
        }
        let slug = slug.trim_matches('-').to_string();
        Self(if slug.is_empty() { "tool".into() } else { slug })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A physical cutter in the job's tool library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    #[serde(default)]
    id: ToolId,
    name: String,
    cutter: Cutter,
    diameter_mm: f64,
    flutes: u32,
    flute_length_mm: f64,
    stickout_mm: f64,
    cutting: Option<CuttingData>,
    #[serde(default)]
    tool_number: Option<u32>,
    #[serde(default)]
    diameter_offset_mm: f64,
    #[serde(default)]
    length_offset_mm: f64,
    #[serde(default)]
    material: Option<String>,
    #[serde(default)]
    coating: Option<String>,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

impl Tool {
    pub fn new(
        name: impl Into<String>,
        cutter: Cutter,
        diameter_mm: f64,
        flutes: u32,
        flute_length_mm: f64,
        stickout_mm: f64,
    ) -> Result<Self, ToolError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ToolError::EmptyName);
        }
        if !diameter_mm.is_finite() || diameter_mm <= 0.0 {
            return Err(ToolError::BadDiameter(diameter_mm));
        }
        if flutes == 0 {
            return Err(ToolError::NoFlutes(flutes));
        }
        if !flute_length_mm.is_finite() || flute_length_mm <= 0.0 {
            return Err(ToolError::BadFluteLength(flute_length_mm));
        }
        if !stickout_mm.is_finite() || stickout_mm < flute_length_mm {
            return Err(ToolError::StickoutTooShort(stickout_mm, flute_length_mm));
        }
        match cutter {
            Cutter::BullNose { corner_radius_mm }
                if !corner_radius_mm.is_finite() || corner_radius_mm <= 0.0 || corner_radius_mm >= diameter_mm / 2.0 =>
            {
                return Err(ToolError::BadCornerRadius(corner_radius_mm));
            }
            Cutter::Drill { point_angle_deg } if !point_angle_deg.is_finite() || point_angle_deg <= 0.0 || point_angle_deg >= 180.0 => {
                return Err(ToolError::BadPointAngle(point_angle_deg));
            }
            Cutter::Chamfer { tip_angle_deg, tip_diameter_mm } => {
                if !tip_angle_deg.is_finite() || tip_angle_deg <= 0.0 || tip_angle_deg >= 180.0 {
                    return Err(ToolError::BadTipAngle(tip_angle_deg));
                }
                if !tip_diameter_mm.is_finite() || tip_diameter_mm < 0.0 || tip_diameter_mm >= diameter_mm {
                    return Err(ToolError::BadTipDiameter(tip_diameter_mm));
                }
            }
            Cutter::FaceMill { lead_angle_deg } if !lead_angle_deg.is_finite() || lead_angle_deg <= 0.0 || lead_angle_deg >= 90.0 => {
                return Err(ToolError::BadLeadAngle(lead_angle_deg));
            }
            Cutter::ThreadMill { thread_angle_deg } if !thread_angle_deg.is_finite() || thread_angle_deg <= 0.0 || thread_angle_deg >= 90.0 => {
                return Err(ToolError::BadTipAngle(thread_angle_deg));
            }
            Cutter::Tapered { half_angle_deg, tip_diameter_mm } => {
                if !half_angle_deg.is_finite() || half_angle_deg <= 0.0 || half_angle_deg >= 45.0 {
                    return Err(ToolError::BadTaperAngle(half_angle_deg));
                }
                if !tip_diameter_mm.is_finite() || tip_diameter_mm < 0.0 || tip_diameter_mm >= diameter_mm {
                    return Err(ToolError::BadTipDiameter(tip_diameter_mm));
                }
            }
            Cutter::Dovetail { undercut_angle_deg } if !undercut_angle_deg.is_finite() || undercut_angle_deg <= 0.0 || undercut_angle_deg >= 45.0 => {
                return Err(ToolError::BadUndercutAngle(undercut_angle_deg));
            }
            Cutter::Slot { thickness_mm } if !thickness_mm.is_finite() || thickness_mm <= 0.0 || thickness_mm > diameter_mm => {
                return Err(ToolError::BadSlotThickness(thickness_mm));
            }
            _ => {}
        }
        Ok(Self {
            id: ToolId::from_name(&name),
            name,
            cutter,
            diameter_mm,
            flutes,
            flute_length_mm,
            stickout_mm,
            cutting: None,
            tool_number: None,
            diameter_offset_mm: 0.0,
            length_offset_mm: 0.0,
            material: None,
            coating: None,
            manufacturer: None,
            description: None,
        })
    }

    pub fn with_diameter_mm(mut self, diameter_mm: f64) -> Result<Self, ToolError> {
        if !diameter_mm.is_finite() || diameter_mm <= 0.0 {
            return Err(ToolError::BadDiameter(diameter_mm));
        }
        if let Cutter::BullNose { corner_radius_mm } = self.cutter
            && corner_radius_mm >= diameter_mm / 2.0
        {
            return Err(ToolError::BadCornerRadius(corner_radius_mm));
        }
        self.diameter_mm = diameter_mm;
        Ok(self)
    }

    pub fn with_stickout_mm(mut self, stickout_mm: f64) -> Result<Self, ToolError> {
        if !stickout_mm.is_finite() || stickout_mm < self.flute_length_mm {
            return Err(ToolError::StickoutTooShort(stickout_mm, self.flute_length_mm));
        }
        self.stickout_mm = stickout_mm;
        Ok(self)
    }

    pub fn with_flute_length_mm(mut self, flute_length_mm: f64) -> Result<Self, ToolError> {
        if !flute_length_mm.is_finite() || flute_length_mm <= 0.0 {
            return Err(ToolError::BadFluteLength(flute_length_mm));
        }
        if flute_length_mm > self.stickout_mm {
            return Err(ToolError::StickoutTooShort(self.stickout_mm, flute_length_mm));
        }
        self.flute_length_mm = flute_length_mm;
        Ok(self)
    }

    pub fn with_cutting(mut self, cutting: CuttingData) -> Self {
        self.cutting = Some(cutting);
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn id(&self) -> &ToolId {
        &self.id
    }

    pub fn with_id(mut self, id: ToolId) -> Self {
        self.id = id;
        self
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

    pub fn material(&self) -> Option<&str> {
        self.material.as_deref()
    }

    pub fn coating(&self) -> Option<&str> {
        self.coating.as_deref()
    }

    pub fn manufacturer(&self) -> Option<&str> {
        self.manufacturer.as_deref()
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn with_material(mut self, material: impl Into<String>) -> Self {
        self.material = Some(material.into());
        self
    }

    pub fn with_coating(mut self, coating: impl Into<String>) -> Self {
        self.coating = Some(coating.into());
        self
    }

    pub fn with_manufacturer(mut self, manufacturer: impl Into<String>) -> Self {
        self.manufacturer = Some(manufacturer.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn cutter(&self) -> Cutter {
        self.cutter
    }

    pub fn diameter_mm(&self) -> f64 {
        self.diameter_mm
    }

    pub fn flutes(&self) -> u32 {
        self.flutes
    }

    pub fn flute_length_mm(&self) -> f64 {
        self.flute_length_mm
    }

    pub fn stickout_mm(&self) -> f64 {
        self.stickout_mm
    }

    pub fn cutting(&self) -> Option<&CuttingData> {
        self.cutting.as_ref()
    }

    /// The cutter as cutrail sees it, or `None` for shapes cutrail cannot sweep (drills).
    ///
    /// The flute length is what the swept volume uses; the stickout is what collision checks
    /// need, and is handled by the simulation layer.
    pub fn to_cutrail(&self) -> Option<CutrailTool> {
        match self.cutter {
            Cutter::Flat => Some(CutrailTool::flat_end_mill(self.diameter_mm, self.flute_length_mm)),
            Cutter::Ball => Some(CutrailTool::ball_end_mill(self.diameter_mm, self.flute_length_mm)),
            Cutter::BullNose { corner_radius_mm } => Some(CutrailTool::bull_nose_end_mill(self.diameter_mm, corner_radius_mm, self.flute_length_mm)),
            Cutter::Drill { .. } => None,
            // cutrail only sweeps the three milling forms above; the other cutters are
            // approximated by a flat end mill of the same diameter until mesh cutters exist.
            Cutter::Chamfer { .. }
            | Cutter::FaceMill { .. }
            | Cutter::ThreadMill { .. }
            | Cutter::Tapered { .. }
            | Cutter::Dovetail { .. }
            | Cutter::Slot { .. } => Some(CutrailTool::flat_end_mill(self.diameter_mm, self.flute_length_mm)),
        }
    }
}

/// The named cutters available to a job.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolLibrary {
    tools: Vec<Tool>,
}

impl ToolLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, tool: Tool) -> Result<(), ToolError> {
        if self.find(tool.name()).is_some() {
            return Err(ToolError::DuplicateName(tool.name().to_string()));
        }
        self.tools.push(tool);
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.name() == name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Tool> {
        self.tools.iter()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> Tool {
        Tool::new("6mm flat", Cutter::Flat, 6.0, 3, 20.0, 30.0).unwrap()
    }

    #[test]
    fn valid_tools_are_built() {
        let tool = flat();
        assert_eq!(tool.diameter_mm(), 6.0);
        assert_eq!(tool.flutes(), 3);
        assert_eq!(tool.cutter(), Cutter::Flat);
    }

    #[test]
    fn material_presets_and_cutting_data_work_for_every_cutter() {
        let materials = [Material::Aluminum, Material::Steel, Material::Brass, Material::Plastic, Material::Other("titanium".into())];
        let cutters = [
            Cutter::Flat,
            Cutter::Ball,
            Cutter::BullNose { corner_radius_mm: 1.0 },
            Cutter::Drill { point_angle_deg: 118.0 },
            Cutter::Chamfer { tip_angle_deg: 90.0, tip_diameter_mm: 1.0 },
            Cutter::FaceMill { lead_angle_deg: 45.0 },
            Cutter::ThreadMill { thread_angle_deg: 60.0 },
            Cutter::Tapered { half_angle_deg: 5.0, tip_diameter_mm: 1.0 },
            Cutter::Dovetail { undercut_angle_deg: 10.0 },
            Cutter::Slot { thickness_mm: 3.0 },
        ];
        for material in &materials {
            assert!(cutting_preset(material).surface_speed_m_min > 0.0);
            for cutter in cutters {
                let tool = Tool::new("t", cutter, 6.0, 3, 20.0, 30.0).unwrap();
                let data = CuttingData::recommended(material, &tool);
                assert!(data.feed_mm_min() > 0.0 && data.stepover_mm() > 0.0, "{cutter:?} / {material:?}");
            }
        }
    }

    #[test]
    fn tool_identity_and_catalog_fields_are_optional_and_readable() {
        let tool = Tool::new("6mm Flat Endmill", Cutter::Flat, 6.0, 3, 20.0, 30.0)
            .unwrap()
            .with_tool_number(7)
            .with_offsets(0.02, -0.01)
            .with_material("carbide")
            .with_coating("AlTiN")
            .with_manufacturer("Example Tooling")
            .with_description("3-flute aluminium rougher");
        assert_eq!(tool.id().as_str(), "6mm-flat-endmill");
        assert_eq!(tool.tool_number(), Some(7));
        assert_eq!(tool.diameter_offset_mm(), 0.02);
        assert_eq!(tool.length_offset_mm(), -0.01);
        assert_eq!(tool.material(), Some("carbide"));
        assert_eq!(tool.coating(), Some("AlTiN"));
        assert_eq!(tool.manufacturer(), Some("Example Tooling"));
        assert_eq!(tool.description(), Some("3-flute aluminium rougher"));
    }

    #[test]
    fn an_old_tool_file_without_catalog_fields_still_loads() {
        let json = r#"{"name":"6mm flat","cutter":"Flat","diameter_mm":6.0,"flutes":3,"flute_length_mm":20.0,"stickout_mm":30.0,"cutting":null}"#;
        let tool: Tool = serde_json::from_str(json).unwrap();
        assert_eq!(tool.name(), "6mm flat");
        assert_eq!(tool.id().as_str(), "");
        assert_eq!(tool.tool_number(), None);
        assert_eq!(tool.diameter_offset_mm(), 0.0);
    }

    #[test]
    fn every_cutter_type_validates_its_own_geometry() {
        let make = |cutter| Tool::new("t", cutter, 10.0, 3, 20.0, 30.0);
        assert!(make(Cutter::Flat).is_ok());
        assert!(make(Cutter::Ball).is_ok());
        assert!(make(Cutter::Chamfer { tip_angle_deg: 90.0, tip_diameter_mm: 2.0 }).is_ok());
        assert_eq!(make(Cutter::Chamfer { tip_angle_deg: 200.0, tip_diameter_mm: 2.0 }), Err(ToolError::BadTipAngle(200.0)));
        assert_eq!(make(Cutter::Chamfer { tip_angle_deg: 90.0, tip_diameter_mm: 10.0 }), Err(ToolError::BadTipDiameter(10.0)));
        assert!(make(Cutter::FaceMill { lead_angle_deg: 45.0 }).is_ok());
        assert!(matches!(make(Cutter::FaceMill { lead_angle_deg: 95.0 }), Err(ToolError::BadLeadAngle(_))));
        assert!(make(Cutter::ThreadMill { thread_angle_deg: 60.0 }).is_ok());
        assert!(make(Cutter::Tapered { half_angle_deg: 10.0, tip_diameter_mm: 2.0 }).is_ok());
        assert!(matches!(make(Cutter::Tapered { half_angle_deg: 50.0, tip_diameter_mm: 2.0 }), Err(ToolError::BadTaperAngle(_))));
        assert!(make(Cutter::Dovetail { undercut_angle_deg: 10.0 }).is_ok());
        assert!(make(Cutter::Slot { thickness_mm: 4.0 }).is_ok());
        assert!(matches!(make(Cutter::Slot { thickness_mm: 12.0 }), Err(ToolError::BadSlotThickness(_))));
    }

    #[test]
    fn new_cutter_types_approximate_to_flat_end_mills() {
        for cutter in [
            Cutter::Chamfer { tip_angle_deg: 90.0, tip_diameter_mm: 1.0 },
            Cutter::FaceMill { lead_angle_deg: 45.0 },
            Cutter::ThreadMill { thread_angle_deg: 60.0 },
            Cutter::Tapered { half_angle_deg: 5.0, tip_diameter_mm: 2.0 },
            Cutter::Dovetail { undercut_angle_deg: 10.0 },
            Cutter::Slot { thickness_mm: 4.0 },
        ] {
            let tool = Tool::new("t", cutter, 10.0, 3, 20.0, 30.0).unwrap();
            assert!(tool.to_cutrail().is_some(), "{cutter:?} has no cutrail shape");
        }
    }

    #[test]
    fn invalid_geometry_is_rejected() {
        assert_eq!(Tool::new("x", Cutter::Flat, 0.0, 3, 20.0, 30.0), Err(ToolError::BadDiameter(0.0)));
        assert_eq!(Tool::new("x", Cutter::Flat, 6.0, 0, 20.0, 30.0), Err(ToolError::NoFlutes(0)));
        assert_eq!(Tool::new("x", Cutter::Flat, 6.0, 3, 20.0, 10.0), Err(ToolError::StickoutTooShort(10.0, 20.0)));
        assert_eq!(Tool::new("x", Cutter::BullNose { corner_radius_mm: 3.0 }, 6.0, 3, 20.0, 30.0), Err(ToolError::BadCornerRadius(3.0)));
        assert_eq!(Tool::new("x", Cutter::Drill { point_angle_deg: 200.0 }, 5.0, 2, 20.0, 30.0), Err(ToolError::BadPointAngle(200.0)));
    }

    #[test]
    fn cutrail_shapes_are_built_for_milling_tools_and_not_for_drills() {
        assert!(flat().to_cutrail().is_some());
        let ball = Tool::new("ball", Cutter::Ball, 6.0, 2, 20.0, 30.0).unwrap();
        assert!(ball.to_cutrail().is_some());
        let bull = Tool::new("bull", Cutter::BullNose { corner_radius_mm: 1.0 }, 6.0, 3, 20.0, 30.0).unwrap();
        assert!(bull.to_cutrail().is_some());
        let drill = Tool::new("drill", Cutter::Drill { point_angle_deg: 118.0 }, 5.0, 2, 20.0, 30.0).unwrap();
        assert!(drill.to_cutrail().is_none());
    }

    #[test]
    fn recommended_cutting_data_is_positive_and_clamped() {
        let data = CuttingData::recommended(&Material::Aluminum, &flat());
        assert_eq!(data.spindle_rpm(), 24_000.0);
        assert!(data.feed_mm_min() > 0.0);
        assert!(data.plunge_feed_mm_min() > 0.0);
        assert_eq!(data.depth_of_cut_mm(), 3.0);
        assert!(data.stepover_mm() > 0.0);
    }

    #[test]
    fn bad_cutting_data_is_rejected() {
        assert_eq!(CuttingData::new(0.0, 100.0, 50.0, 1.0, 2.0), Err(ToolError::BadCuttingData("spindle speed")));
        assert_eq!(CuttingData::new(10_000.0, 100.0, 50.0, 1.0, 0.0), Err(ToolError::BadCuttingData("stepover")));
    }

    #[test]
    fn library_rejects_duplicate_names() {
        let mut library = ToolLibrary::new();
        library.add(flat()).unwrap();
        assert_eq!(library.add(flat()), Err(ToolError::DuplicateName("6mm flat".into())));
        assert_eq!(library.len(), 1);
        assert!(library.find("6mm flat").is_some());
        assert!(library.find("missing").is_none());
    }

    #[test]
    fn cutting_data_can_be_attached_to_a_tool() {
        let data = CuttingData::recommended(&Material::Steel, &flat());
        let tool = flat().with_cutting(data);
        assert_eq!(tool.cutting(), Some(&data));
    }
}

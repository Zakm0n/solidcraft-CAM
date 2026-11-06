//! The tool library: the cutting tools a program can use, their holders, and the assemblies that
//! pair the two.
//!
//! A CAM programmer picks a *tool assembly* — a cutter held in a holder at a known stickout — not
//! a bare cutter, because the holder is what decides whether the tool can reach into a pocket
//! without the spindle crashing. The library therefore stores assemblies; each one carries its
//! [`CuttingTool`] and an optional [`ToolHolder`].
//!
//! All lengths are millimetres and all diameters are cutting diameters, as the tool catalogues
//! state them. The types validate rather than assume: a dialog asks [`ToolAssembly::validate`]
//! before it stores a tool, so a library only ever holds a tool that can actually cut.

use std::fmt;

use thiserror::Error;

/// Why a tool or an assembly cannot be used. Every rejection says which value is wrong, so the
/// dialog can put it beside the field the user typed it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum ToolError {
    /// The assembly has no name to list it by.
    #[error("the tool needs a name")]
    Name,
    /// The cutting diameter is zero or negative.
    #[error("the cutting diameter must be greater than zero")]
    Diameter,
    /// The corner radius is negative, or larger than half the diameter (a ball nose).
    #[error("the corner radius must be between zero and half the diameter")]
    CornerRadius,
    /// The flute length is zero or negative.
    #[error("the flute length must be greater than zero")]
    FluteLength,
    /// A tool with no cutting edge cannot cut.
    #[error("the tool needs at least one flute")]
    Flutes,
    /// The holder's gauge diameter or length is zero or negative.
    #[error("the holder's diameter and length must be greater than zero")]
    Holder,
    /// The tool does not reach past the holder, so the holder would cut first.
    #[error("the stickout must reach past the flute length")]
    Stickout,
}

/// The kinds of cutter the library holds, in the order the dialog lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolType {
    /// A square-cornered endmill: the workhorse for 2D contours.
    FlatEndmill,
    /// An endmill with a full-radius (half the diameter) corner.
    BallEndmill,
    /// An endmill with a corner radius smaller than half the diameter.
    BullNoseEndmill,
    /// A twist drill, cutting on its point.
    Drill,
    /// A chamfer mill, cutting a bevel.
    ChamferMill,
    /// A tap, cutting a thread.
    Tap,
}

impl ToolType {
    /// Every type, in the order the dialog lists them.
    pub const ALL: &'static [Self] = &[Self::FlatEndmill, Self::BallEndmill, Self::BullNoseEndmill, Self::Drill, Self::ChamferMill, Self::Tap];

    /// The type's text in the dialog.
    pub const fn label(self) -> &'static str {
        match self {
            Self::FlatEndmill => "Flat endmill",
            Self::BallEndmill => "Ball endmill",
            Self::BullNoseEndmill => "Bull nose endmill",
            Self::Drill => "Drill",
            Self::ChamferMill => "Chamfer mill",
            Self::Tap => "Tap",
        }
    }

    /// Whether the corner radius may be greater than zero: a drill and a tap cut with their
    /// point, so their corner stays square.
    pub const fn has_corner_radius(self) -> bool {
        matches!(self, Self::FlatEndmill | Self::BallEndmill | Self::BullNoseEndmill | Self::ChamferMill)
    }

    /// The corner radius a type uses whatever the stored value says: a ball endmill is always a
    /// half-diameter radius, and a drill or a tap always has a square corner.
    pub fn corner_radius(self, stored: f64, diameter: f64) -> f64 {
        match self {
            Self::BallEndmill => diameter / 2.0,
            Self::Drill | Self::Tap => 0.0,
            _ => stored,
        }
    }
}

impl fmt::Display for ToolType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One cutting tool: the geometry the toolpath calculation needs and the catalogue's data.
#[derive(Clone, Debug, PartialEq)]
pub struct CuttingTool {
    /// The tool's kind, which fixes whether it has a corner radius.
    pub kind: ToolType,
    /// The cutting diameter, in millimetres.
    pub diameter: f64,
    /// The corner radius, in millimetres: zero for a square corner, half the diameter for a ball.
    pub corner_radius: f64,
    /// The length of the fluted cutting edge, in millimetres.
    pub flute_length: f64,
    /// The number of flutes, which sets the feed per tooth.
    pub flutes: u32,
}

impl CuttingTool {
    /// A tool of `kind` and `diameter`, with a square corner, a flute length of three times the
    /// diameter and two flutes: what a catalogue entry starts from before it is edited.
    pub fn new(kind: ToolType, diameter: f64) -> Self {
        Self { kind, diameter, corner_radius: 0.0, flute_length: diameter.max(0.0) * 3.0, flutes: 2 }
    }

    /// The corner radius the tool actually cuts with, after its kind has its say.
    pub fn effective_corner_radius(&self) -> f64 {
        self.kind.corner_radius(self.corner_radius, self.diameter)
    }

    /// Whether the tool is a ball nose: a full-radius corner.
    pub fn is_ball_nose(&self) -> bool {
        self.diameter > 0.0 && self.effective_corner_radius() >= self.diameter / 2.0
    }

    /// Checks that the tool can cut, naming the first value that cannot.
    pub fn validate(&self) -> Result<(), ToolError> {
        if !(self.diameter > 0.0) || !self.diameter.is_finite() {
            return Err(ToolError::Diameter);
        }
        if !(self.flute_length > 0.0) || !self.flute_length.is_finite() {
            return Err(ToolError::FluteLength);
        }
        if self.flutes == 0 {
            return Err(ToolError::Flutes);
        }
        let radius = if self.kind.has_corner_radius() { self.corner_radius } else { 0.0 };
        if !radius.is_finite() || radius < 0.0 || radius > self.diameter / 2.0 {
            return Err(ToolError::CornerRadius);
        }
        Ok(())
    }

    /// The tool as the library lists it, such as `Ø6.0 flat, 4F`.
    pub fn summary(&self) -> String {
        format!("Ø{:.1} {}, {}F", self.diameter, tool_kind_short(self.kind), self.flutes)
    }
}

/// The short form of a type's name, for the library's one-line summaries.
fn tool_kind_short(kind: ToolType) -> &'static str {
    match kind {
        ToolType::FlatEndmill => "flat",
        ToolType::BallEndmill => "ball",
        ToolType::BullNoseEndmill => "bull nose",
        ToolType::Drill => "drill",
        ToolType::ChamferMill => "chamfer",
        ToolType::Tap => "tap",
    }
}

/// The holder a tool sits in: the spindle-side body that limits how deep a tool can reach.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolHolder {
    /// The holder as the catalogue names it, such as `ER32 collet chuck`.
    pub name: String,
    /// The gauge diameter, in millimetres: the widest body the toolpath must clear.
    pub diameter: f64,
    /// The length below the spindle, in millimetres.
    pub length: f64,
}

impl ToolHolder {
    /// The holder a fresh assembly starts with: a standard collet chuck.
    pub fn standard() -> Self {
        Self { name: "ER32 collet chuck".to_string(), diameter: 50.0, length: 60.0 }
    }

    /// Checks that the holder has a body.
    pub fn validate(&self) -> Result<(), ToolError> {
        if !(self.diameter > 0.0) || !self.diameter.is_finite() || !(self.length > 0.0) || !self.length.is_finite() {
            return Err(ToolError::Holder);
        }
        Ok(())
    }
}

/// A cutting tool held in (or without) a holder at a known stickout: what an operation selects.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolAssembly {
    /// The assembly's identity, unique for the life of its library and assigned by
    /// [`ToolLibrary::add`]. Zero until the assembly is added.
    pub id: usize,
    /// The name the library and the operation's tool selection show.
    pub name: String,
    /// The cutter.
    pub tool: CuttingTool,
    /// The holder, or `None` for a bare tool.
    pub holder: Option<ToolHolder>,
    /// How far the tool sticks out of the holder, in millimetres.
    pub stickout: f64,
}

impl ToolAssembly {
    /// An assembly named `name`, holding `tool` in the standard holder.
    pub fn new(name: impl Into<String>, tool: CuttingTool) -> Self {
        let stickout = (tool.flute_length + 5.0).max(0.0);
        Self { id: 0, name: name.into(), tool, holder: Some(ToolHolder::standard()), stickout }
    }

    /// The assembly a library preset describes.
    pub fn from_preset(preset: Preset) -> Self {
        let tool = CuttingTool {
            kind: preset.kind,
            diameter: preset.diameter,
            corner_radius: preset.corner_radius,
            flute_length: preset.flute_length,
            flutes: preset.flutes,
        };
        Self::new(preset.name, tool)
    }

    /// Checks the name, the tool, the holder and the stickout, naming the first that cannot be
    /// used.
    pub fn validate(&self) -> Result<(), ToolError> {
        if self.name.trim().is_empty() {
            return Err(ToolError::Name);
        }
        self.tool.validate()?;
        if !self.stickout.is_finite() || !(self.stickout > 0.0) {
            return Err(ToolError::Stickout);
        }
        if let Some(holder) = &self.holder {
            holder.validate()?;
            if self.stickout < self.tool.flute_length {
                return Err(ToolError::Stickout);
            }
        }
        Ok(())
    }

    /// The assembly as the library lists it: the tool's summary and its holder.
    pub fn summary(&self) -> String {
        match &self.holder {
            Some(holder) => format!("{} · {}", self.tool.summary(), holder.name),
            None => self.tool.summary(),
        }
    }
}

/// One entry of the library's preset catalogue: the cutters a programmer starts from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    /// The preset's name, also the assembly's name when it is added.
    pub name: &'static str,
    /// The cutter's kind.
    pub kind: ToolType,
    /// The cutting diameter, in millimetres.
    pub diameter: f64,
    /// The corner radius, in millimetres.
    pub corner_radius: f64,
    /// The flute length, in millimetres.
    pub flute_length: f64,
    /// The number of flutes.
    pub flutes: u32,
}

/// The default presets, in catalogue order: the flat endmills most 2D work is cut with, a bull
/// nose and a ball for floors and walls, and the drills and chamfer mill around them.
pub const PRESETS: &[Preset] = &[
    Preset { name: "Ø6 flat endmill", kind: ToolType::FlatEndmill, diameter: 6.0, corner_radius: 0.0, flute_length: 18.0, flutes: 3 },
    Preset { name: "Ø10 flat endmill", kind: ToolType::FlatEndmill, diameter: 10.0, corner_radius: 0.0, flute_length: 30.0, flutes: 4 },
    Preset { name: "Ø12 flat endmill", kind: ToolType::FlatEndmill, diameter: 12.0, corner_radius: 0.0, flute_length: 36.0, flutes: 4 },
    Preset { name: "Ø12 bull nose R1", kind: ToolType::BullNoseEndmill, diameter: 12.0, corner_radius: 1.0, flute_length: 30.0, flutes: 4 },
    Preset { name: "Ø6 ball endmill", kind: ToolType::BallEndmill, diameter: 6.0, corner_radius: 3.0, flute_length: 18.0, flutes: 2 },
    Preset { name: "Ø5 jobber drill", kind: ToolType::Drill, diameter: 5.0, corner_radius: 0.0, flute_length: 26.0, flutes: 2 },
    Preset { name: "Ø10 chamfer mill", kind: ToolType::ChamferMill, diameter: 10.0, corner_radius: 1.0, flute_length: 15.0, flutes: 4 },
];

/// The tool library: the assemblies a program selects from, addressed by their stable ids.
///
/// The library is a value: a dialog edits a copy and hands it back when the user accepts, so
/// cancelling never changes the tool an operation already uses. Ids are handed out once and never
/// reused, so an operation that names a tool keeps naming the same tool after its neighbours are
/// deleted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolLibrary {
    /// The assemblies, in the order the library lists them.
    assemblies: Vec<ToolAssembly>,
    /// The next id to hand out; ids start at 1, so zero never names a stored assembly.
    next_id: usize,
}

impl ToolLibrary {
    /// An empty library.
    pub fn new() -> Self {
        Self::default()
    }

    /// A library holding the default presets, each in the standard holder.
    pub fn with_presets() -> Self {
        let mut library = Self::new();
        for preset in PRESETS {
            library.add(ToolAssembly::from_preset(*preset));
        }
        library
    }

    /// Adds an assembly, giving it its id, and returns that id.
    pub fn add(&mut self, mut assembly: ToolAssembly) -> usize {
        self.next_id = self.next_id.max(1);
        let id = self.next_id;
        self.next_id += 1;
        assembly.id = id;
        self.assemblies.push(assembly);
        id
    }

    /// Replaces the stored assembly with the same id; false when the library does not hold it.
    pub fn replace(&mut self, assembly: ToolAssembly) -> bool {
        match self.assemblies.iter_mut().find(|stored| stored.id == assembly.id) {
            Some(stored) => {
                *stored = assembly;
                true
            }
            None => false,
        }
    }

    /// Removes and returns the assembly with `id`, if the library holds it.
    pub fn remove(&mut self, id: usize) -> Option<ToolAssembly> {
        let index = self.assemblies.iter().position(|assembly| assembly.id == id)?;
        Some(self.assemblies.remove(index))
    }

    /// The assembly with `id`.
    pub fn get(&self, id: usize) -> Option<&ToolAssembly> {
        self.assemblies.iter().find(|assembly| assembly.id == id)
    }

    /// The assembly with `id`, to edit in place.
    pub fn get_mut(&mut self, id: usize) -> Option<&mut ToolAssembly> {
        self.assemblies.iter_mut().find(|assembly| assembly.id == id)
    }

    /// Every assembly, in listing order.
    pub fn assemblies(&self) -> &[ToolAssembly] {
        &self.assemblies
    }

    /// How many assemblies the library holds.
    pub fn len(&self) -> usize {
        self.assemblies.len()
    }

    /// Whether the library holds no assembly at all.
    pub fn is_empty(&self) -> bool {
        self.assemblies.is_empty()
    }

    /// Whether every stored assembly can cut. A library the dialog accepted always passes; the
    /// check keeps imported or hand-built libraries honest.
    pub fn validate(&self) -> Result<(), ToolError> {
        self.assemblies.iter().try_for_each(ToolAssembly::validate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_a_tool_that_can_cut() {
        assert!(PRESETS.len() >= 5, "the catalogue has the common cutters");
        for preset in PRESETS {
            let assembly = ToolAssembly::from_preset(*preset);
            assert_eq!(assembly.validate(), Ok(()), "{} must be usable", preset.name);
            assert_eq!(assembly.id, 0, "a preset has no id until it is added");
        }
    }

    #[test]
    fn a_library_hands_out_ids_once_and_never_reuses_them() {
        let mut library = ToolLibrary::with_presets();
        assert_eq!(library.len(), PRESETS.len());
        assert_eq!(library.validate(), Ok(()));
        let first = library.assemblies()[0].id;
        let last = library.assemblies()[library.len() - 1].id;
        assert_eq!((first, last), (1, PRESETS.len()));
        assert!(library.remove(first).is_some());
        let added = library.add(ToolAssembly::new("Ø8 flat endmill", CuttingTool::new(ToolType::FlatEndmill, 8.0)));
        assert!(added > last, "ids are never reused, got {added} after {last}");
        assert_eq!(library.get(last).map(|assembly| assembly.name.as_str()), Some(PRESETS[PRESETS.len() - 1].name));
        assert!(library.get(first).is_none());
    }

    #[test]
    fn a_tool_that_cannot_cut_is_rejected_with_the_field_that_is_wrong() {
        let mut tool = CuttingTool::new(ToolType::FlatEndmill, 6.0);
        assert_eq!(tool.validate(), Ok(()));
        tool.diameter = 0.0;
        assert_eq!(tool.validate(), Err(ToolError::Diameter));
        tool.diameter = 6.0;
        tool.flutes = 0;
        assert_eq!(tool.validate(), Err(ToolError::Flutes));
        tool.flutes = 2;
        tool.corner_radius = 4.0;
        assert_eq!(tool.validate(), Err(ToolError::CornerRadius), "a corner cannot exceed half the diameter");
        tool.corner_radius = f64::NAN;
        assert_eq!(tool.validate(), Err(ToolError::CornerRadius), "a NaN radius is not a radius");
    }

    #[test]
    fn a_holder_that_would_cut_first_is_rejected() {
        let mut assembly = ToolAssembly::new("too short", CuttingTool::new(ToolType::FlatEndmill, 6.0));
        assembly.stickout = 1.0;
        assert_eq!(assembly.validate(), Err(ToolError::Stickout));
        assembly.stickout = 23.0;
        assert_eq!(assembly.validate(), Ok(()));
    }

    #[test]
    fn a_ball_nose_is_a_half_diameter_radius_whatever_the_stored_corner_says() {
        let mut tool = CuttingTool::new(ToolType::BallEndmill, 8.0);
        tool.corner_radius = 0.5;
        assert_eq!(tool.effective_corner_radius(), 4.0);
        assert!(tool.is_ball_nose());
        let drill = CuttingTool::new(ToolType::Drill, 5.0);
        assert_eq!(drill.effective_corner_radius(), 0.0, "a drill cuts with its point");
    }

    #[test]
    fn an_assembly_replaces_under_its_own_id() {
        let mut library = ToolLibrary::with_presets();
        let id = library.assemblies()[0].id;
        let edited = ToolAssembly { id, name: "renamed".to_string(), ..ToolAssembly::from_preset(PRESETS[0]) };
        assert!(library.replace(edited));
        assert_eq!(library.get(id).map(|assembly| assembly.name.as_str()), Some("renamed"));
        let absent = ToolAssembly { id: 999, ..ToolAssembly::new("absent", CuttingTool::new(ToolType::Drill, 3.0)) };
        assert!(!library.replace(absent));
    }
}

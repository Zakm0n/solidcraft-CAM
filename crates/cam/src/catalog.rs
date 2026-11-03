//! The tool catalog: reusable tools, holders and assemblies.
//!
//! Tools and holders are stored once with stable IDs; assemblies reference them and may carry
//! per-machine overrides. The catalog is versioned JSON and can migrate an older `ToolLibrary`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::tool::{Cutter, Tool, ToolError, ToolId, ToolLibrary};
use crate::tooling::{Holder, ProfileSegment, ToolAssembly, ToolOverrides, ToolingError};

/// Catalog file format this build writes and reads.
pub const CAM_CATALOG_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("a tool named `{0}` or with that id already exists")]
    DuplicateTool(String),
    #[error("a holder named `{0}` or with that id already exists")]
    DuplicateHolder(String),
    #[error("an assembly named `{0}` or with that id already exists")]
    DuplicateAssembly(String),
    #[error("no tool `{0}` in the catalog")]
    UnknownTool(String),
    #[error("no holder `{0}` in the catalog")]
    UnknownHolder(String),
    #[error("invalid catalog file: {0}")]
    Json(#[from] serde_json::Error),
    #[error("could not read or write the catalog: {0}")]
    Io(#[from] std::io::Error),
    #[error("catalog version {found} is not supported (expected {expected})")]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error(transparent)]
    Tooling(#[from] ToolingError),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCatalog {
    #[serde(default)]
    tools: BTreeMap<String, Tool>,
    #[serde(default)]
    holders: BTreeMap<String, Holder>,
    #[serde(default)]
    assemblies: BTreeMap<String, ToolAssembly>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CatalogFile {
    format_version: u32,
    catalog: ToolCatalog,
}

impl ToolCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tools(&self) -> impl Iterator<Item = &Tool> {
        self.tools.values()
    }

    pub fn holders(&self) -> impl Iterator<Item = &Holder> {
        self.holders.values()
    }

    pub fn assemblies(&self) -> impl Iterator<Item = &ToolAssembly> {
        self.assemblies.values()
    }

    pub fn len(&self) -> usize {
        self.tools.len() + self.holders.len() + self.assemblies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn add_tool(&mut self, mut tool: Tool) -> Result<ToolId, CatalogError> {
        if tool.id().as_str().is_empty() {
            let id = ToolId::from_name(tool.name());
            tool = tool.with_id(id);
        }
        let id = tool.id().as_str().to_string();
        if self.tools.contains_key(&id) || self.tool_by_name(tool.name()).is_some() {
            return Err(CatalogError::DuplicateTool(tool.name().to_string()));
        }
        self.tools.insert(id, tool.clone());
        Ok(tool.id().clone())
    }

    pub fn tool(&self, id: &str) -> Option<&Tool> {
        self.tools.get(id)
    }

    pub fn tool_by_name(&self, name: &str) -> Option<&Tool> {
        self.tools.values().find(|tool| tool.name() == name)
    }

    pub fn tool_by_number(&self, number: u32) -> Option<&Tool> {
        self.tools.values().find(|tool| tool.tool_number() == Some(number))
    }

    pub fn remove_tool(&mut self, id: &str) -> Option<Tool> {
        let removed = self.tools.remove(id);
        if removed.is_some() {
            self.assemblies.retain(|_, assembly| assembly.tool_id().as_str() != id);
        }
        removed
    }

    pub fn add_holder(&mut self, holder: Holder) -> Result<(), CatalogError> {
        let id = holder.id().to_string();
        if self.holders.contains_key(&id) || self.holder_by_name(holder.name()).is_some() {
            return Err(CatalogError::DuplicateHolder(holder.name().to_string()));
        }
        self.holders.insert(id, holder);
        Ok(())
    }

    pub fn holder(&self, id: &str) -> Option<&Holder> {
        self.holders.get(id)
    }

    pub fn holder_by_name(&self, name: &str) -> Option<&Holder> {
        self.holders.values().find(|holder| holder.name() == name)
    }

    pub fn remove_holder(&mut self, id: &str) -> Option<Holder> {
        let removed = self.holders.remove(id);
        if removed.is_some() {
            for assembly in self.assemblies.values_mut() {
                if assembly.holder_id() == Some(id) {
                    // Keep the assembly but drop the now-missing holder reference.
                    *assembly = assembly.clone().without_holder();
                }
            }
        }
        removed
    }

    pub fn add_assembly(&mut self, assembly: ToolAssembly) -> Result<(), CatalogError> {
        let id = assembly.id().to_string();
        if self.assemblies.contains_key(&id) || self.assembly_by_name(assembly.name()).is_some() {
            return Err(CatalogError::DuplicateAssembly(assembly.name().to_string()));
        }
        if self.tool(assembly.tool_id().as_str()).is_none() {
            return Err(CatalogError::UnknownTool(assembly.tool_id().as_str().to_string()));
        }
        if let Some(holder) = assembly.holder_id()
            && self.holder(holder).is_none()
        {
            return Err(CatalogError::UnknownHolder(holder.to_string()));
        }
        self.assemblies.insert(id, assembly);
        Ok(())
    }

    pub fn assembly(&self, id: &str) -> Option<&ToolAssembly> {
        self.assemblies.get(id)
    }

    pub fn assembly_by_name(&self, name: &str) -> Option<&ToolAssembly> {
        self.assemblies.values().find(|assembly| assembly.name() == name)
    }

    pub fn assembly_by_name_mut(&mut self, name: &str) -> Option<&mut ToolAssembly> {
        self.assemblies.values_mut().find(|assembly| assembly.name() == name)
    }

    pub fn assembly_by_tool_number(&self, number: u32) -> Option<&ToolAssembly> {
        self.assemblies.values().find(|assembly| assembly.tool_number() == Some(number))
    }

    pub fn remove_assembly(&mut self, id: &str) -> Option<ToolAssembly> {
        self.assemblies.remove(id)
    }

    /// Resolve an assembly to its effective tool and holder.
    pub fn resolve_assembly(&self, name: &str) -> Option<(Tool, Option<Holder>)> {
        let assembly = self.assembly_by_name(name)?;
        let tool = self.tool(assembly.tool_id().as_str())?;
        let effective = assembly.effective_tool(tool).ok()?;
        let holder = assembly.holder_id().and_then(|id| self.holder(id)).cloned();
        Some((effective, holder))
    }

    /// Migrate an older tool library: each tool gets a holder-less assembly of the same name.
    pub fn from_tool_library(library: &ToolLibrary) -> Result<Self, CatalogError> {
        let mut catalog = Self::new();
        for tool in library.iter() {
            let id = catalog.add_tool(tool.clone())?;
            let assembly = ToolAssembly::new(format!("{}-assembly", id.as_str()), tool.name(), id, None, tool.stickout_mm())?;
            catalog.add_assembly(assembly)?;
        }
        Ok(catalog)
    }

    pub fn to_json(&self) -> Result<String, CatalogError> {
        Ok(serde_json::to_string_pretty(&CatalogFile { format_version: CAM_CATALOG_VERSION, catalog: self.clone() })?)
    }

    pub fn from_json(text: &str) -> Result<Self, CatalogError> {
        let file: CatalogFile = serde_json::from_str(text)?;
        if file.format_version != CAM_CATALOG_VERSION {
            return Err(CatalogError::UnsupportedVersion { found: file.format_version, expected: CAM_CATALOG_VERSION });
        }
        Ok(file.catalog)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), CatalogError> {
        fs::write(path, self.to_json()?)?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, CatalogError> {
        Self::from_json(&fs::read_to_string(path)?)
    }

    /// Built-in metric catalog: common cutters, ER collets and an end-mill holder.
    pub fn default_catalog() -> Self {
        let mut catalog = Self::new();
        let er16 = Holder::new(
            "er16",
            "ER16 collet",
            vec![ProfileSegment::new(15.0, 28.0, 22.0).unwrap(), ProfileSegment::new(20.0, 28.0, 28.0).unwrap()],
            45.0,
        )
        .unwrap();
        let er20 = Holder::new(
            "er20",
            "ER20 collet",
            vec![ProfileSegment::new(18.0, 34.0, 28.0).unwrap(), ProfileSegment::new(22.0, 34.0, 34.0).unwrap()],
            55.0,
        )
        .unwrap();
        let emh = Holder::new("emh12", "End mill holder 12 mm", vec![ProfileSegment::new(45.0, 42.0, 42.0).unwrap()], 70.0).unwrap();
        for holder in [er16, er20, emh] {
            catalog.holders.insert(holder.id().to_string(), holder);
        }

        let specs: Vec<(&str, Cutter, f64, u32)> = vec![
            ("Flat 3 mm", Cutter::Flat, 3.0, 3),
            ("Flat 6 mm", Cutter::Flat, 6.0, 3),
            ("Flat 10 mm", Cutter::Flat, 10.0, 3),
            ("Flat 12 mm", Cutter::Flat, 12.0, 3),
            ("Ball 3 mm", Cutter::Ball, 3.0, 2),
            ("Ball 6 mm", Cutter::Ball, 6.0, 2),
            ("Ball 10 mm", Cutter::Ball, 10.0, 2),
            ("Corner radius 6 mm R1", Cutter::BullNose { corner_radius_mm: 1.0 }, 6.0, 3),
            ("Corner radius 10 mm R2", Cutter::BullNose { corner_radius_mm: 2.0 }, 10.0, 3),
            ("Drill 5 mm", Cutter::Drill { point_angle_deg: 118.0 }, 5.0, 2),
            ("Chamfer 6 mm 90 deg", Cutter::Chamfer { tip_angle_deg: 90.0, tip_diameter_mm: 1.0 }, 6.0, 2),
            ("Face mill 50 mm", Cutter::FaceMill { lead_angle_deg: 45.0 }, 50.0, 4),
            ("Thread mill 10 mm", Cutter::ThreadMill { thread_angle_deg: 60.0 }, 10.0, 3),
            ("Tapered 6 mm 5 deg", Cutter::Tapered { half_angle_deg: 5.0, tip_diameter_mm: 1.0 }, 6.0, 3),
        ];

        let mut tool_ids = Vec::new();
        for (index, (name, cutter, diameter, flutes)) in specs.into_iter().enumerate() {
            let flute_length = (diameter * 2.5).max(8.0);
            let stickout = flute_length + 10.0;
            let tool = Tool::new(name, cutter, diameter, flutes, flute_length, stickout)
                .unwrap()
                .with_tool_number(index as u32 + 1)
                .with_material("carbide");
            let id = catalog.add_tool(tool).unwrap();
            tool_ids.push((index, id, diameter, name));
        }

        for (index, id, diameter, name) in tool_ids {
            let (holder_id, gauge) = if diameter <= 8.0 {
                ("er16", 50.0)
            } else if diameter <= 12.0 {
                ("er20", 60.0)
            } else {
                ("emh12", 80.0)
            };
            let holder_name = catalog.holder(holder_id).map(|holder| holder.name()).unwrap_or(holder_id);
            let assembly =
                ToolAssembly::new(format!("{}-asm", id.as_str()), format!("{name} in {holder_name}"), id, Some(holder_id.to_string()), gauge)
                    .unwrap()
                    .with_tool_number(index as u32 + 1)
                    .with_overrides(ToolOverrides::default());
            catalog.add_assembly(assembly).unwrap();
        }
        catalog
    }
}

impl ToolAssembly {
    /// Drop a holder reference (used when a holder is removed from the catalog).
    pub fn without_holder(mut self) -> Self {
        self.set_holder(None);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stock::Material;
    use crate::tool::CuttingData;

    fn tool(name: &str, diameter: f64) -> Tool {
        Tool::new(name, Cutter::Flat, diameter, 3, 20.0, 30.0).unwrap()
    }

    fn holder() -> Holder {
        Holder::new("er16", "ER16 collet", vec![ProfileSegment::new(20.0, 28.0, 22.0).unwrap()], 45.0).unwrap()
    }

    #[test]
    fn catalog_round_trips_and_rejects_duplicates() {
        let mut catalog = ToolCatalog::new();
        let id = catalog.add_tool(tool("6mm flat", 6.0)).unwrap();
        catalog.add_holder(holder()).unwrap();
        let assembly = ToolAssembly::new("a1", "6mm in ER16", id.clone(), Some("er16".into()), 55.0).unwrap().with_tool_number(7);
        catalog.add_assembly(assembly).unwrap();
        assert_eq!(catalog.tool_by_number(7), None);
        assert_eq!(catalog.assembly_by_tool_number(7).unwrap().name(), "6mm in ER16");

        let text = catalog.to_json().unwrap();
        assert_eq!(ToolCatalog::from_json(&text).unwrap(), catalog);
        assert!(matches!(catalog.add_tool(tool("6mm flat", 6.0)), Err(CatalogError::DuplicateTool(_))));
        assert!(matches!(catalog.add_holder(holder()), Err(CatalogError::DuplicateHolder(_))));
    }

    #[test]
    fn migration_keeps_tool_names_and_adds_holderless_assemblies() {
        let mut library = ToolLibrary::new();
        library.add(tool("6mm flat", 6.0)).unwrap();
        library.add(tool("3mm ball", 3.0)).unwrap();
        let catalog = ToolCatalog::from_tool_library(&library).unwrap();
        assert_eq!(catalog.tools().count(), 2);
        assert_eq!(catalog.assemblies().count(), 2);
        for tool in library.iter() {
            let assembly = catalog.assembly_by_name(tool.name()).expect("holderless assembly");
            assert!(assembly.holder_id().is_none());
            let (effective, holder) = catalog.resolve_assembly(tool.name()).unwrap();
            assert_eq!(effective.name(), tool.name());
            assert!(holder.is_none());
        }
    }

    #[test]
    fn the_default_catalog_is_valid_and_covers_cutters() {
        let catalog = ToolCatalog::default_catalog();
        assert!(catalog.tools().count() >= 12);
        assert!(catalog.holders().count() >= 3);
        assert!(catalog.assemblies().count() >= 12);
        let mut covers_ball = false;
        let mut covers_bull = false;
        let mut covers_drill = false;
        let mut covers_chamfer = false;
        let mut covers_face = false;
        let mut covers_thread = false;
        let mut covers_taper = false;
        for tool in catalog.tools() {
            match tool.cutter() {
                Cutter::Ball => covers_ball = true,
                Cutter::BullNose { .. } => covers_bull = true,
                Cutter::Drill { .. } => covers_drill = true,
                Cutter::Chamfer { .. } => covers_chamfer = true,
                Cutter::FaceMill { .. } => covers_face = true,
                Cutter::ThreadMill { .. } => covers_thread = true,
                Cutter::Tapered { .. } => covers_taper = true,
                _ => {}
            }
        }
        assert!(covers_ball && covers_bull && covers_drill && covers_chamfer && covers_face && covers_thread && covers_taper);
        for assembly in catalog.assemblies() {
            let (tool, holder) = catalog.resolve_assembly(assembly.name()).unwrap();
            assert!(tool.diameter_mm() > 0.0);
            assert!(holder.is_some());
        }
        assert!(catalog.assembly_by_tool_number(1).is_some());
    }

    #[test]
    fn a_catalog_file_saves_and_loads() {
        let catalog = ToolCatalog::default_catalog();
        let dir = std::env::temp_dir().join(format!("solidcraft-catalog-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tools.json");
        catalog.save(&file).unwrap();
        assert_eq!(ToolCatalog::load(&file).unwrap(), catalog);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolve_assembly_applies_overrides() {
        let mut catalog = ToolCatalog::new();
        let id = catalog.add_tool(tool("6mm flat", 6.0)).unwrap();
        let assembly = ToolAssembly::new("a1", "6mm in ER16", id, Some("er16".into()), 60.0).unwrap().with_overrides(ToolOverrides {
            diameter_mm: Some(8.0),
            stickout_mm: Some(45.0),
            flute_length_mm: Some(25.0),
        });
        catalog.add_holder(holder()).unwrap();
        catalog.add_assembly(assembly).unwrap();
        let (effective, _) = catalog.resolve_assembly("6mm in ER16").unwrap();
        assert_eq!(effective.diameter_mm(), 8.0);
        assert_eq!(effective.stickout_mm(), 45.0);
        let _ = CuttingData::recommended(&Material::Aluminum, &effective);
    }
}

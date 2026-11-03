//! References from a CAM job to the design it machines.
//!
//! Operations point at design entities by a stable key (a hole id, a face id), never by
//! coordinates: when the design changes, the same key still names the same feature, so a
//! rebuild can follow it. A [`DesignSnapshot`] is the small, cheap view a rebuild needs — the
//! referenced entities and the design revision — without loading the whole model.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// What kind of design entity a reference names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EntityKind {
    Body,
    Face,
    Edge,
    Hole,
    Plane,
    Sketch,
}

#[derive(Debug, Error, PartialEq)]
pub enum ReferenceError {
    #[error("an entity key is empty")]
    EmptyKey,
    #[error("a design id is empty")]
    EmptyDesignId,
}

/// A stable reference to one entity in a design.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityRef {
    kind: EntityKind,
    key: String,
}

impl EntityRef {
    pub fn new(kind: EntityKind, key: impl Into<String>) -> Result<Self, ReferenceError> {
        let key = key.into();
        if key.trim().is_empty() {
            return Err(ReferenceError::EmptyKey);
        }
        Ok(Self { kind, key })
    }

    pub fn kind(&self) -> EntityKind {
        self.kind
    }

    pub fn key(&self) -> &str {
        &self.key
    }
}

/// Which design, at which revision, a job was built against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignRef {
    design_id: String,
    revision: u64,
}

impl DesignRef {
    pub fn new(design_id: impl Into<String>, revision: u64) -> Result<Self, ReferenceError> {
        let design_id = design_id.into();
        if design_id.trim().is_empty() {
            return Err(ReferenceError::EmptyDesignId);
        }
        Ok(Self { design_id, revision })
    }

    pub fn design_id(&self) -> &str {
        &self.design_id
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_same_design(&self, other: &DesignRef) -> bool {
        self.design_id == other.design_id
    }
}

/// One entity as a rebuild sees it: its reference plus the properties CAM cares about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    reference: EntityRef,
    #[serde(default)]
    properties: BTreeMap<String, f64>,
}

impl Entity {
    pub fn new(reference: EntityRef) -> Self {
        Self { reference, properties: BTreeMap::new() }
    }

    /// A hole with its diameter, the property a drilling operation follows.
    pub fn hole(reference: EntityRef, diameter_mm: f64) -> Self {
        let mut entity = Self::new(reference);
        entity.properties.insert("diameter".into(), diameter_mm);
        entity
    }

    pub fn with_property(mut self, name: impl Into<String>, value: f64) -> Self {
        self.properties.insert(name.into(), value);
        self
    }

    pub fn reference(&self) -> &EntityRef {
        &self.reference
    }

    pub fn diameter_mm(&self) -> Option<f64> {
        self.properties.get("diameter").copied()
    }

    pub fn property(&self, name: &str) -> Option<f64> {
        self.properties.get(name).copied()
    }
}

/// The design state a rebuild is checked against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesignSnapshot {
    design: DesignRef,
    entities: BTreeMap<String, Entity>,
}

impl DesignSnapshot {
    pub fn new(design: DesignRef) -> Self {
        Self { design, entities: BTreeMap::new() }
    }

    /// Add or replace an entity, keyed by its reference.
    pub fn insert(&mut self, entity: Entity) {
        self.entities.insert(entity.reference.key().to_string(), entity);
    }

    /// Remove an entity, as deleting a feature does.
    pub fn remove(&mut self, reference: &EntityRef) -> Option<Entity> {
        self.entities.remove(reference.key())
    }

    pub fn design(&self) -> &DesignRef {
        &self.design
    }

    pub fn get(&self, reference: &EntityRef) -> Option<&Entity> {
        self.entities.get(reference.key())
    }

    pub fn contains(&self, reference: &EntityRef) -> bool {
        self.entities.contains_key(reference.key())
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_need_a_key() {
        assert_eq!(EntityRef::new(EntityKind::Hole, "  "), Err(ReferenceError::EmptyKey));
        assert_eq!(DesignRef::new("", 1), Err(ReferenceError::EmptyDesignId));
    }

    #[test]
    fn a_snapshot_finds_entities_by_reference() {
        let design = DesignRef::new("bracket", 4).unwrap();
        let mut snapshot = DesignSnapshot::new(design.clone());
        let hole = EntityRef::new(EntityKind::Hole, "hole.42").unwrap();
        snapshot.insert(Entity::hole(hole.clone(), 6.0));
        assert!(snapshot.contains(&hole));
        assert_eq!(snapshot.get(&hole).unwrap().diameter_mm(), Some(6.0));
        assert_eq!(snapshot.design().revision(), 4);
        snapshot.remove(&hole);
        assert!(!snapshot.contains(&hole));
    }
}

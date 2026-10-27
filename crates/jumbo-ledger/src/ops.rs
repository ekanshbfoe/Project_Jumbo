//! Memory ops proposed by the Librarian. Mirrors `MemoryDelta` in packages/schema; the conformance
//! test in tests/schema_conformance.rs keeps the two in step.

use std::fmt;

use jumbo_core::ChapterId;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Character,
    Faction,
    Location,
    Item,
    Relationship,
    PlotThread,
}

impl EntityType {
    pub const ALL: [EntityType; 6] =
        [Self::Character, Self::Faction, Self::Location, Self::Item, Self::Relationship, Self::PlotThread];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Character => "character",
            Self::Faction => "faction",
            Self::Location => "location",
            Self::Item => "item",
            Self::Relationship => "relationship",
            Self::PlotThread => "plot_thread",
        }
    }

    /// Entity ids must start with `<type>_`, e.g. `plot_thread_double_dungeon`.
    pub fn owns_id(self, id: &str) -> bool {
        id.strip_prefix(self.as_str()).and_then(|rest| rest.strip_prefix('_')).is_some_and(is_snake_slug)
    }
}

impl fmt::Display for EntityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn is_snake_slug(s: &str) -> bool {
    !s.is_empty()
        && s.split('_')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

/// Dot path into an entity's data, e.g. `abilities.shadow_extraction`.
pub fn is_field_path(s: &str) -> bool {
    !s.is_empty()
        && s.split('.').all(|seg| {
            !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub chapter_id: ChapterId,
    pub panel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum MemoryOp {
    #[serde(rename = "ADD")]
    Add {
        entity_type: EntityType,
        entity_id: String,
        data: Map<String, Value>,
        receipts: Vec<Receipt>,
        confidence: f64,
    },
    #[serde(rename = "UPDATE")]
    Update {
        entity_type: EntityType,
        entity_id: String,
        field: String,
        value: Value,
        receipts: Vec<Receipt>,
        confidence: f64,
    },
    #[serde(rename = "DELETE")]
    Delete { entity_type: EntityType, entity_id: String, reason: String, receipts: Vec<Receipt>, confidence: f64 },
    /// `from_id` is the same entity as `entity_id`: it is folded into it and retired.
    #[serde(rename = "MERGE")]
    Merge {
        entity_type: EntityType,
        entity_id: String,
        from_id: String,
        reason: String,
        receipts: Vec<Receipt>,
        confidence: f64,
    },
    #[serde(rename = "NOOP")]
    Noop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

impl MemoryOp {
    /// `(type, id)` for ops that touch an entity; `None` for NOOP.
    pub fn target(&self) -> Option<(EntityType, &str)> {
        match self {
            Self::Add { entity_type, entity_id, .. }
            | Self::Update { entity_type, entity_id, .. }
            | Self::Delete { entity_type, entity_id, .. }
            | Self::Merge { entity_type, entity_id, .. } => Some((*entity_type, entity_id)),
            Self::Noop { .. } => None,
        }
    }

    pub fn receipts(&self) -> &[Receipt] {
        match self {
            Self::Add { receipts, .. }
            | Self::Update { receipts, .. }
            | Self::Delete { receipts, .. }
            | Self::Merge { receipts, .. } => receipts,
            Self::Noop { .. } => &[],
        }
    }

    pub fn confidence(&self) -> Option<f64> {
        match self {
            Self::Add { confidence, .. }
            | Self::Update { confidence, .. }
            | Self::Delete { confidence, .. }
            | Self::Merge { confidence, .. } => Some(*confidence),
            Self::Noop { .. } => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Add { .. } => "ADD",
            Self::Update { .. } => "UPDATE",
            Self::Delete { .. } => "DELETE",
            Self::Merge { .. } => "MERGE",
            Self::Noop { .. } => "NOOP",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryDelta {
    pub schema_version: u32,
    pub chapter_id: ChapterId,
    /// Ledger length the ops were generated against.
    pub base_ledger_offset: u64,
    pub ops: Vec<MemoryOp>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_ids_must_carry_their_type_prefix() {
        assert!(EntityType::PlotThread.owns_id("plot_thread_double_dungeon"));
        assert!(EntityType::Character.owns_id("character_jinwoo2"));
        for bad in ["character", "character_", "character_Jin", "character__a", "faction_x"] {
            assert!(!EntityType::Character.owns_id(bad), "{bad} accepted");
        }
    }

    #[test]
    fn field_paths() {
        assert!(is_field_path("rank"));
        assert!(is_field_path("abilities.shadow_extraction"));
        assert!(!is_field_path("abilities..x"));
        assert!(!is_field_path("Rank"));
    }
}

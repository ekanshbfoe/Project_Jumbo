//! Materialized story state: what you get by replaying every ledger op in order. Applying an op is
//! also how deltas are validated, so the commit path and the replay path cannot disagree.

use std::collections::BTreeMap;

use jumbo_core::ChapterId;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::ops::{EntityType, MemoryOp, Receipt, is_field_path};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub entity_type: EntityType,
    pub id: String,
    pub data: Map<String, Value>,
    /// Every receipt that ever supported this entity, oldest first.
    pub receipts: Vec<Receipt>,
    /// Newest chapter cited for each field ("" is the whole entity, from its ADD). An UPDATE must cite
    /// evidence at least this new, so an old panel cannot silently overwrite a newer fact.
    pub provenance: BTreeMap<String, ChapterId>,
    pub created_seq: u64,
    pub updated_seq: u64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ApplyError {
    #[error("{0} is not a valid id for a {1}")]
    BadId(String, EntityType),
    #[error("{0} already exists")]
    AlreadyExists(String),
    #[error("{0} does not exist")]
    Unknown(String),
    #[error("{id} is a {actual}, not a {claimed}")]
    TypeMismatch { id: String, actual: EntityType, claimed: EntityType },
    #[error("{0:?} is not a valid field path")]
    BadField(String),
    #[error("cannot set {field} on {id}: {at} is not an object")]
    NotAnObject { id: String, field: String, at: String },
    #[error("{id}.{field} is backed by {current}; this update only cites {cited}")]
    OlderEvidence { id: String, field: String, current: ChapterId, cited: ChapterId },
    #[error("{0} op has no receipts")]
    NoReceipts(&'static str),
    #[error("confidence {0} is outside 0..=1")]
    BadConfidence(f64),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BookState {
    entities: BTreeMap<String, Entity>,
}

fn newest(receipts: &[Receipt]) -> Option<ChapterId> {
    receipts.iter().map(|r| r.chapter_id).max()
}

impl BookState {
    pub fn get(&self, id: &str) -> Option<&Entity> {
        self.entities.get(id)
    }

    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    pub fn of_type(&self, entity_type: EntityType) -> impl Iterator<Item = &Entity> {
        self.entities.values().filter(move |e| e.entity_type == entity_type)
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Hash of the full state, stable across platforms and map-ordering features.
    pub fn hash(&self) -> String {
        crate::canonical::hash_of(&self.entities).unwrap_or_default()
    }

    /// Applies one op as ledger entry `seq`. On error the state is unchanged.
    pub fn apply(&mut self, seq: u64, op: &MemoryOp) -> Result<(), ApplyError> {
        let Some((entity_type, id)) = op.target() else { return Ok(()) };
        if !entity_type.owns_id(id) {
            return Err(ApplyError::BadId(id.to_owned(), entity_type));
        }
        let Some(cited) = newest(op.receipts()) else { return Err(ApplyError::NoReceipts(op.kind())) };
        if let Some(c) = op.confidence().filter(|c| !(0.0..=1.0).contains(c)) {
            return Err(ApplyError::BadConfidence(c));
        }
        let existing = self.entities.get(id);
        if let Some(e) = existing
            && e.entity_type != entity_type
        {
            return Err(ApplyError::TypeMismatch { id: id.to_owned(), actual: e.entity_type, claimed: entity_type });
        }

        match op {
            MemoryOp::Add { data, receipts, .. } => {
                if existing.is_some() {
                    return Err(ApplyError::AlreadyExists(id.to_owned()));
                }
                self.entities.insert(
                    id.to_owned(),
                    Entity {
                        entity_type,
                        id: id.to_owned(),
                        data: data.clone(),
                        receipts: receipts.clone(),
                        provenance: BTreeMap::from([(String::new(), cited)]),
                        created_seq: seq,
                        updated_seq: seq,
                    },
                );
            }
            MemoryOp::Update { field, value, receipts, .. } => {
                let entity = existing.ok_or_else(|| ApplyError::Unknown(id.to_owned()))?;
                if !is_field_path(field) {
                    return Err(ApplyError::BadField(field.clone()));
                }
                let current = governing_chapter(&entity.provenance, field);
                if let Some(current) = current.filter(|c| cited < *c) {
                    return Err(ApplyError::OlderEvidence { id: id.to_owned(), field: field.clone(), current, cited });
                }
                let mut next = entity.clone();
                set_path(&mut next.data, field, value.clone()).map_err(|at| ApplyError::NotAnObject {
                    id: id.to_owned(),
                    field: field.clone(),
                    at,
                })?;
                next.receipts.extend(receipts.iter().cloned());
                next.provenance.insert(field.clone(), cited);
                next.updated_seq = seq;
                self.entities.insert(id.to_owned(), next);
            }
            MemoryOp::Delete { .. } => {
                if existing.is_none() {
                    return Err(ApplyError::Unknown(id.to_owned()));
                }
                self.entities.remove(id);
            }
            MemoryOp::Noop { .. } => {}
        }
        Ok(())
    }
}

/// The newest evidence covering `field`: its own entry, any ancestor path, or the entity's ADD.
fn governing_chapter(provenance: &BTreeMap<String, ChapterId>, field: &str) -> Option<ChapterId> {
    let mut best = provenance.get("").copied();
    let mut prefix = String::new();
    for segment in field.split('.') {
        if !prefix.is_empty() {
            prefix.push('.');
        }
        prefix.push_str(segment);
        best = best.max(provenance.get(&prefix).copied());
    }
    best
}

/// Sets a dot path, creating intermediate objects. Returns the blocking path if a non-object is in the way.
fn set_path(data: &mut Map<String, Value>, field: &str, value: Value) -> Result<(), String> {
    let segments: Vec<&str> = field.split('.').collect();
    let (last, parents) = segments.split_last().ok_or_else(String::new)?;
    let mut cursor = data;
    let mut walked = String::new();
    for segment in parents {
        if !walked.is_empty() {
            walked.push('.');
        }
        walked.push_str(segment);
        let slot = cursor.entry(segment.to_string()).or_insert_with(|| Value::Object(Map::new()));
        cursor = slot.as_object_mut().ok_or_else(|| walked.clone())?;
    }
    cursor.insert((*last).to_owned(), value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn receipt(ch: u32) -> Vec<Receipt> {
        vec![Receipt { chapter_id: ChapterId::new(ch).expect("id"), panel_id: "p_0001".into(), quote: None }]
    }

    fn add(id: &str, ch: u32) -> MemoryOp {
        MemoryOp::Add {
            entity_type: EntityType::Character,
            entity_id: id.into(),
            data: json!({"name": "X", "stats": {"str": 1}}).as_object().cloned().expect("obj"),
            receipts: receipt(ch),
            confidence: 0.9,
        }
    }

    fn update(id: &str, field: &str, value: Value, ch: u32) -> MemoryOp {
        MemoryOp::Update {
            entity_type: EntityType::Character,
            entity_id: id.into(),
            field: field.into(),
            value,
            receipts: receipt(ch),
            confidence: 0.9,
        }
    }

    #[test]
    fn add_update_delete_lifecycle() {
        let mut s = BookState::default();
        s.apply(0, &add("character_a", 1)).expect("add");
        s.apply(1, &update("character_a", "stats.agi", json!(5), 2)).expect("update");
        let e = s.get("character_a").expect("entity");
        assert_eq!(Value::Object(e.data.clone()), json!({"name": "X", "stats": {"str": 1, "agi": 5}}));
        assert_eq!((e.created_seq, e.updated_seq, e.receipts.len()), (0, 1, 2));
        s.apply(
            2,
            &MemoryOp::Delete {
                entity_type: EntityType::Character,
                entity_id: "character_a".into(),
                reason: "died".into(),
                receipts: receipt(3),
                confidence: 1.0,
            },
        )
        .expect("delete");
        assert!(s.is_empty());
    }

    #[test]
    fn rejects_inconsistent_ops_without_changing_state() {
        let mut s = BookState::default();
        s.apply(0, &add("character_a", 2)).expect("add");
        let before = s.clone();
        assert_eq!(s.apply(1, &add("character_a", 2)), Err(ApplyError::AlreadyExists("character_a".into())));
        assert_eq!(
            s.apply(1, &update("character_b", "x", json!(1), 2)),
            Err(ApplyError::Unknown("character_b".into()))
        );
        assert!(matches!(
            s.apply(1, &update("character_a", "name.first", json!(1), 2)),
            Err(ApplyError::NotAnObject { .. })
        ));
        assert!(matches!(s.apply(1, &update("character_a", "Name", json!(1), 2)), Err(ApplyError::BadField(_))));
        assert!(matches!(s.apply(1, &add("faction_a", 2)), Err(ApplyError::BadId(..))));
        let mut no_receipts = add("character_c", 2);
        if let MemoryOp::Add { receipts, .. } = &mut no_receipts {
            receipts.clear();
        }
        assert_eq!(s.apply(1, &no_receipts), Err(ApplyError::NoReceipts("ADD")));
        assert_eq!(s, before);
    }

    #[test]
    fn updates_need_evidence_at_least_as_new_as_the_current_value() {
        let mut s = BookState::default();
        s.apply(0, &add("character_a", 3)).expect("add");
        assert!(matches!(
            s.apply(1, &update("character_a", "rank", json!("S"), 2)),
            Err(ApplyError::OlderEvidence { .. })
        ));
        s.apply(1, &update("character_a", "stats.str", json!(9), 5)).expect("newer evidence");
        assert!(matches!(
            s.apply(2, &update("character_a", "stats.str", json!(2), 4)),
            Err(ApplyError::OlderEvidence { .. })
        ));
        s.apply(2, &update("character_a", "rank", json!("D"), 3)).expect("same chapter as ADD");
    }

    #[test]
    fn state_hash_is_deterministic_and_tracks_history() {
        let mut a = BookState::default();
        a.apply(0, &add("character_a", 1)).expect("add");
        a.apply(1, &add("character_b", 1)).expect("add");
        let mut b = BookState::default();
        b.apply(0, &add("character_b", 1)).expect("add");
        b.apply(1, &add("character_a", 1)).expect("add");
        // created_seq differs, so the hashes must differ; equal histories hash equally.
        assert_ne!(a.hash(), b.hash());
        let mut c = BookState::default();
        c.apply(0, &add("character_a", 1)).expect("add");
        c.apply(1, &add("character_b", 1)).expect("add");
        assert_eq!(a.hash(), c.hash());
    }
}

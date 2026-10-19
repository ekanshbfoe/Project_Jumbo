//! Human-readable materialized views under `receipt_book/state/`. They are derived data: always
//! rebuildable from the ledger, and `index.json` records which ledger head they reflect.

use std::fs;
use std::path::Path;

use jumbo_core::fsx;
use serde::{Deserialize, Serialize};

use crate::ops::EntityType;
use crate::state::{BookState, Entity};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewIndex {
    pub ledger_len: u64,
    pub ledger_head: String,
    pub state_hash: String,
}

fn collection_file(entity_type: EntityType) -> Option<&'static str> {
    match entity_type {
        EntityType::Character => None,
        EntityType::Faction => Some("factions.json"),
        EntityType::Location => Some("locations.json"),
        EntityType::Item => Some("items.json"),
        EntityType::Relationship => Some("relationships.json"),
        EntityType::PlotThread => Some("plot_threads.json"),
    }
}

/// Reads `index.json`, if present and readable.
pub fn read_index(dir: &Path) -> Option<ViewIndex> {
    serde_json::from_str(&fs::read_to_string(dir.join("index.json")).ok()?).ok()
}

/// Rewrites every view. Characters get one file each (the Receipt Book's character sheets); the
/// other entity types are one sorted array per type. `index.json` is written last.
pub fn write_views(dir: &Path, state: &BookState, ledger_len: u64, ledger_head: &str) -> std::io::Result<()> {
    let characters = dir.join("characters");
    fs::create_dir_all(&characters)?;
    for entity in state.of_type(EntityType::Character) {
        fsx::atomic_write_json(&characters.join(format!("{}.json", entity.id)), entity)?;
    }
    for entry in fs::read_dir(&characters)? {
        let path = entry?.path();
        let live = path.file_stem().and_then(|s| s.to_str()).is_some_and(|id| state.get(id).is_some());
        if path.extension().is_some_and(|e| e == "json") && !live {
            fs::remove_file(path)?;
        }
    }
    for entity_type in EntityType::ALL {
        if let Some(file) = collection_file(entity_type) {
            let items: Vec<&Entity> = state.of_type(entity_type).collect();
            fsx::atomic_write_json(&dir.join(file), &items)?;
        }
    }
    let index = ViewIndex { ledger_len, ledger_head: ledger_head.to_owned(), state_hash: state.hash() };
    fsx::atomic_write_json(&dir.join("index.json"), &index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{MemoryOp, Receipt};
    use jumbo_core::ChapterId;
    use serde_json::json;

    fn add(entity_type: EntityType, id: &str) -> MemoryOp {
        MemoryOp::Add {
            entity_type,
            entity_id: id.into(),
            data: json!({"name": id}).as_object().cloned().expect("obj"),
            receipts: vec![Receipt {
                chapter_id: ChapterId::new(1).expect("id"),
                panel_id: "p_0001".into(),
                quote: None,
            }],
            confidence: 1.0,
        }
    }

    #[test]
    fn writes_per_character_files_and_collections_and_prunes_deleted_characters() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = BookState::default();
        state.apply(0, &add(EntityType::Character, "character_a")).expect("add");
        state.apply(1, &add(EntityType::Character, "character_b")).expect("add");
        state.apply(2, &add(EntityType::PlotThread, "plot_thread_gate")).expect("add");
        write_views(dir.path(), &state, 3, "h3").expect("views");
        assert!(dir.path().join("characters/character_a.json").is_file());
        let threads: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("plot_threads.json")).expect("read"))
                .expect("json");
        assert_eq!(threads[0]["id"], "plot_thread_gate");
        assert_eq!(fs::read_to_string(dir.path().join("factions.json")).expect("read"), "[]\n");

        state
            .apply(
                3,
                &MemoryOp::Delete {
                    entity_type: EntityType::Character,
                    entity_id: "character_b".into(),
                    reason: "gone".into(),
                    receipts: vec![Receipt {
                        chapter_id: ChapterId::new(2).expect("id"),
                        panel_id: "p_0001".into(),
                        quote: None,
                    }],
                    confidence: 1.0,
                },
            )
            .expect("delete");
        write_views(dir.path(), &state, 4, "h4").expect("views");
        assert!(!dir.path().join("characters/character_b.json").exists());
        let index = read_index(dir.path()).expect("index");
        assert_eq!((index.ledger_len, index.ledger_head.as_str()), (4, "h4"));
        assert_eq!(index.state_hash, state.hash());
    }
}

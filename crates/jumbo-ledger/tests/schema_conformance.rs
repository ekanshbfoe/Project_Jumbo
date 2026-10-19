//! The TypeScript schema package validates the same fixture with zod, so serde and zod agree on the
//! MemoryDelta wire format.

use std::path::PathBuf;

use jumbo_ledger::{EntityType, MemoryDelta, MemoryOp};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/schema/fixtures").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn parses_the_shared_memory_delta_fixture() {
    let delta: MemoryDelta = serde_json::from_str(&fixture("memory-delta.sample.json")).expect("parse");
    assert_eq!(delta.chapter_id.to_string(), "ch_0003");
    assert_eq!(delta.ops.iter().map(MemoryOp::kind).collect::<Vec<_>>(), ["ADD", "ADD", "UPDATE", "NOOP"]);
    assert_eq!(delta.ops[1].target(), Some((EntityType::PlotThread, "plot_thread_double_dungeon")));
}

#[test]
fn round_trips_without_losing_fields() {
    let delta: MemoryDelta = serde_json::from_str(&fixture("memory-delta.sample.json")).expect("parse");
    let again: MemoryDelta = serde_json::from_str(&serde_json::to_string(&delta).expect("ser")).expect("parse");
    assert_eq!(delta, again);
}

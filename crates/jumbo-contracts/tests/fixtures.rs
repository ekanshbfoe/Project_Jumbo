//! The shared fixtures in packages/schema/fixtures must parse with the generated types, survive a
//! round trip, and the patterns from the schema must be enforced.

use std::path::PathBuf;

use jumbo_contracts::{
    audio_manifest::AudioManifest, episode_spec::EpisodeSpec, ledger_event::LedgerEvent, memory_delta::MemoryDelta,
    panels_doc::PanelsDoc, script_doc::ScriptDoc, session_ticket::SessionTicket,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/schema/fixtures").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn round_trip<T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug>(json: &str) -> T {
    let parsed: T = serde_json::from_str(json).expect("parse");
    let again: T = serde_json::from_str(&serde_json::to_string(&parsed).expect("serialize")).expect("reparse");
    assert_eq!(parsed, again);
    parsed
}

#[test]
fn every_shared_fixture_round_trips() {
    let panels: PanelsDoc = round_trip(&fixture("panels-doc.sample.json"));
    assert_eq!(panels.panels.len(), 3);
    let script: ScriptDoc = round_trip(&fixture("script-doc.sample.json"));
    assert_eq!(script.lines.len(), 3);
    let audio: AudioManifest = round_trip(&fixture("audio-manifest.sample.json"));
    assert_eq!(audio.clips.len(), 2);
    let episode: EpisodeSpec = round_trip(&fixture("episode-spec.sample.json"));
    assert!(episode.beats[2].audio.is_none());
    let delta: MemoryDelta = round_trip(&fixture("memory-delta.sample.json"));
    assert_eq!(delta.ops.len(), 4);
    let ticket: SessionTicket = round_trip(&fixture("session-ticket.sample.json"));
    assert_eq!(ticket.idle_s.get(), 1800);
}

#[test]
fn ledger_timestamps_stay_exact_strings() {
    for line in fixture("ledger.sample.jsonl").lines() {
        let event: LedgerEvent = round_trip(line);
        let original: serde_json::Value = serde_json::from_str(line).expect("json");
        assert_eq!(serde_json::to_value(&event.ts).expect("ts"), original["ts"]);
    }
}

#[test]
fn schema_patterns_are_enforced() {
    let bad_chapter = fixture("memory-delta.sample.json").replace("\"ch_0003\"", "\"chapter three\"");
    assert!(serde_json::from_str::<MemoryDelta>(&bad_chapter).is_err());
    let bad_proto = fixture("session-ticket.sample.json").replace("\"proto\": 1", "\"proto\": 2");
    assert!(serde_json::from_str::<SessionTicket>(&bad_proto).is_err());
}

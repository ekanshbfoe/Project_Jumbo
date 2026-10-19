//! `ledger.jsonl`: append-only, hash-chained, grouped into commits.
//!
//! Each line is one op. Every event's hash covers its body and the previous hash, so editing or
//! deleting any line breaks the chain. Ops committed together share a `commit` number and carry
//! `part`/`parts`; a crash mid-append leaves an incomplete final commit (or a torn line), which was
//! never acknowledged to the user and is safely dropped on load.

use std::fs;
use std::path::{Path, PathBuf};

use jumbo_core::{ChapterId, fsx};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::canonical::canonical_json;
use crate::ops::MemoryOp;
use crate::state::{ApplyError, BookState};

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerEvent {
    pub seq: u64,
    pub commit: u64,
    pub part: u32,
    pub parts: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    pub chapter_id: ChapterId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub op: MemoryOp,
    pub prev: String,
    pub hash: String,
}

impl LedgerEvent {
    /// Hash of everything except the `hash` field itself.
    pub fn compute_hash(&self) -> String {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(obj) = value.as_object_mut() {
            obj.remove("hash");
        }
        blake3::hash(canonical_json(&value).as_bytes()).to_hex().to_string()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error(transparent)]
    Core(#[from] jumbo_core::CoreError),
    #[error("ledger line {line} is corrupt: {reason}")]
    Corrupt { line: usize, reason: String },
    #[error("ledger entry {seq} cannot be replayed: {error}")]
    Replay { seq: u64, error: ApplyError },
}

/// What `Ledger::load` had to discard from an interrupted append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repair {
    pub torn_line: bool,
    pub dropped_events: usize,
}

#[derive(Debug, Clone)]
pub struct Ledger {
    path: PathBuf,
    events: Vec<LedgerEvent>,
}

fn io_err(path: &Path, source: std::io::Error) -> LedgerError {
    jumbo_core::CoreError::Io { path: path.to_path_buf(), source }.into()
}

impl Ledger {
    /// Loads and verifies the ledger, repairing an interrupted final commit if there is one.
    pub fn load(path: impl Into<PathBuf>) -> Result<(Self, Option<Repair>), LedgerError> {
        let path = path.into();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(io_err(&path, e)),
        };
        let lines: Vec<(usize, &str)> =
            text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()).map(|(i, l)| (i + 1, l)).collect();

        let mut events = Vec::with_capacity(lines.len());
        let mut torn_line = false;
        for (idx, (line_no, line)) in lines.iter().enumerate() {
            match serde_json::from_str::<LedgerEvent>(line) {
                Ok(event) => events.push(event),
                Err(_) if idx == lines.len() - 1 => torn_line = true,
                Err(e) => return Err(LedgerError::Corrupt { line: *line_no, reason: e.to_string() }),
            }
        }
        verify_chain(&events)?;

        // Drop a trailing commit that did not finish writing all of its parts.
        let mut dropped_events = 0;
        if let Some(last) = events.last()
            && last.part + 1 != last.parts
        {
            let commit = last.commit;
            while events.last().is_some_and(|e| e.commit == commit) {
                events.pop();
                dropped_events += 1;
            }
        }

        let ledger = Self { path, events };
        let repair = (torn_line || dropped_events > 0).then_some(Repair { torn_line, dropped_events });
        if repair.is_some() {
            ledger.rewrite()?;
        }
        Ok((ledger, repair))
    }

    fn rewrite(&self) -> Result<(), LedgerError> {
        let mut text = String::new();
        for event in &self.events {
            text.push_str(&serde_json::to_string(event).map_err(|e| io_err(&self.path, std::io::Error::other(e)))?);
            text.push('\n');
        }
        fsx::atomic_write(&self.path, text.as_bytes()).map_err(|e| io_err(&self.path, e))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn events(&self) -> &[LedgerEvent] {
        &self.events
    }

    pub fn len(&self) -> u64 {
        self.events.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Hash of the newest event, or the genesis hash for an empty ledger.
    pub fn head(&self) -> &str {
        self.events.last().map_or(GENESIS, |e| e.hash.as_str())
    }

    /// Hash of the event at `len - 1`, i.e. the head as it was when the ledger had `len` entries.
    pub fn head_at(&self, len: u64) -> Option<&str> {
        match len {
            0 => Some(GENESIS),
            n => self.events.get(n as usize - 1).map(|e| e.hash.as_str()),
        }
    }

    fn next_commit(&self) -> u64 {
        self.events.last().map_or(0, |e| e.commit + 1)
    }

    /// Appends ops as one commit with a single fsynced write. NOOPs are not recorded. Callers validate
    /// first; this only guarantees the chain.
    pub fn append_commit(
        &mut self,
        chapter_id: ChapterId,
        session: Option<&str>,
        ops: &[MemoryOp],
    ) -> Result<Vec<LedgerEvent>, LedgerError> {
        let ops: Vec<&MemoryOp> = ops.iter().filter(|op| op.target().is_some()).collect();
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        let commit = self.next_commit();
        let ts = OffsetDateTime::now_utc();
        let mut prev = self.head().to_owned();
        let mut batch = Vec::with_capacity(ops.len());
        for (i, op) in ops.iter().enumerate() {
            let mut event = LedgerEvent {
                seq: self.len() + i as u64,
                commit,
                part: i as u32,
                parts: ops.len() as u32,
                ts,
                chapter_id,
                session: session.map(str::to_owned),
                op: (*op).clone(),
                prev: prev.clone(),
                hash: String::new(),
            };
            event.hash = event.compute_hash();
            prev = event.hash.clone();
            batch.push(event);
        }
        let lines = batch
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| io_err(&self.path, std::io::Error::other(e)))?
            .join("\n");
        fsx::append_line(&self.path, &lines).map_err(|e| io_err(&self.path, e))?;
        self.events.extend(batch.iter().cloned());
        Ok(batch)
    }

    /// Rebuilds the state from scratch, optionally only the first `len` events.
    pub fn replay(&self, len: Option<u64>) -> Result<BookState, LedgerError> {
        let mut state = BookState::default();
        let take = len.map_or(self.events.len(), |n| n as usize);
        for event in self.events.iter().take(take) {
            state.apply(event.seq, &event.op).map_err(|error| LedgerError::Replay { seq: event.seq, error })?;
        }
        Ok(state)
    }
}

fn verify_chain(events: &[LedgerEvent]) -> Result<(), LedgerError> {
    let mut prev = GENESIS;
    for (i, event) in events.iter().enumerate() {
        let line = i + 1;
        let fail = |reason: &str| Err(LedgerError::Corrupt { line, reason: reason.to_owned() });
        if event.seq != i as u64 {
            return fail(&format!("expected seq {i}, found {}", event.seq));
        }
        if event.prev != prev {
            return fail("previous-hash link is broken");
        }
        if event.compute_hash() != event.hash {
            return fail("hash does not match contents");
        }
        prev = &event.hash;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{EntityType, Receipt};
    use serde_json::json;

    fn add(id: &str) -> MemoryOp {
        MemoryOp::Add {
            entity_type: EntityType::Item,
            entity_id: id.into(),
            data: json!({"name": id}).as_object().cloned().expect("obj"),
            receipts: vec![Receipt { chapter_id: ch(1), panel_id: "p_0001".into(), quote: None }],
            confidence: 1.0,
        }
    }

    fn ch(n: u32) -> ChapterId {
        ChapterId::new(n).expect("id")
    }

    fn ledger_with_two_commits(dir: &Path) -> Ledger {
        let (mut ledger, repair) = Ledger::load(dir.join("ledger.jsonl")).expect("load");
        assert!(repair.is_none());
        ledger.append_commit(ch(1), Some("s1"), &[add("item_a"), MemoryOp::Noop { reason: None }]).expect("commit");
        ledger.append_commit(ch(2), Some("s1"), &[add("item_b"), add("item_c")]).expect("commit");
        ledger
    }

    #[test]
    fn appends_chain_and_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = ledger_with_two_commits(dir.path());
        assert_eq!(ledger.len(), 3, "NOOPs are not recorded");
        assert_eq!(ledger.events()[0].prev, GENESIS);
        assert_eq!(ledger.events()[2].prev, ledger.events()[1].hash);
        assert_eq!(
            ledger.events().iter().map(|e| (e.commit, e.part, e.parts)).collect::<Vec<_>>(),
            [(0, 0, 1), (1, 0, 2), (1, 1, 2)]
        );

        let (reloaded, repair) = Ledger::load(dir.path().join("ledger.jsonl")).expect("reload");
        assert!(repair.is_none());
        assert_eq!(reloaded.head(), ledger.head());
        assert_eq!(reloaded.replay(None).expect("replay").len(), 3);
        assert_eq!(reloaded.replay(Some(1)).expect("replay").len(), 1);
    }

    #[test]
    fn detects_tampering() {
        let dir = tempfile::tempdir().expect("tempdir");
        ledger_with_two_commits(dir.path());
        let path = dir.path().join("ledger.jsonl");
        let text = fs::read_to_string(&path).expect("read").replacen("item_a", "item_z", 2);
        fs::write(&path, text).expect("write");
        assert!(matches!(Ledger::load(&path), Err(LedgerError::Corrupt { line: 1, .. })));
    }

    #[test]
    fn detects_deleted_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        ledger_with_two_commits(dir.path());
        let path = dir.path().join("ledger.jsonl");
        let text = fs::read_to_string(&path).expect("read");
        let kept: Vec<&str> = text.lines().enumerate().filter(|(i, _)| *i != 1).map(|(_, l)| l).collect();
        fs::write(&path, kept.join("\n")).expect("write");
        assert!(matches!(Ledger::load(&path), Err(LedgerError::Corrupt { line: 2, .. })));
    }

    #[test]
    fn drops_an_interrupted_final_commit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = ledger_with_two_commits(dir.path());
        let path = dir.path().join("ledger.jsonl");
        // Simulate a crash after the first part of commit 1 and part of the second line.
        let text = fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = text.lines().collect();
        fs::write(&path, format!("{}\n{}\n{}", lines[0], lines[1], &lines[2][..40])).expect("write");

        let (recovered, repair) = Ledger::load(&path).expect("load");
        assert_eq!(repair, Some(Repair { torn_line: true, dropped_events: 1 }));
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered.head(), ledger.events()[0].hash);
        // The repair is persisted and appending continues the chain.
        let (mut again, repair) = Ledger::load(&path).expect("load");
        assert!(repair.is_none());
        again.append_commit(ch(3), None, &[add("item_d")]).expect("commit");
        assert_eq!(Ledger::load(&path).expect("load").0.len(), 2);
    }

    #[test]
    fn replay_surfaces_inconsistent_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut ledger, _) = Ledger::load(dir.path().join("ledger.jsonl")).expect("load");
        ledger.append_commit(ch(1), None, &[add("item_a")]).expect("commit");
        ledger.append_commit(ch(1), None, &[add("item_a")]).expect("commit");
        assert!(matches!(ledger.replay(None), Err(LedgerError::Replay { seq: 1, .. })));
    }
}

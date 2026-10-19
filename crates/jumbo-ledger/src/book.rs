//! `ReceiptBook`: the single entry point the app uses to read and change story memory.
//!
//! Lifecycle: `begin_session` unlocks the book, `commit` applies validated deltas, and
//! `snapshot_and_lock` (the Stop / Consolidate step) writes a hash-chained snapshot and locks the
//! book so nothing changes until the next session starts.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use jumbo_core::{CoreError, Project, fsx};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::canonical::canonical_json;
use crate::log::{Ledger, LedgerError, LedgerEvent, Repair};
use crate::ops::MemoryDelta;
use crate::state::BookState;
use crate::validate::{PanelIndex, ValidationReport, validate};
use crate::views;

#[derive(Debug, thiserror::Error)]
pub enum BookError {
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("the Receipt Book is locked; start a session first")]
    Locked,
    #[error("the delta was rejected ({} error(s))", .0.errors().count())]
    Rejected(ValidationReport),
    #[error("{0}")]
    Inconsistent(String),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LockState {
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    pub ledger_len: u64,
    pub ledger_head: String,
    #[serde(default, with = "time::serde::rfc3339::option", skip_serializing_if = "Option::is_none")]
    pub at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub ledger_len: u64,
    pub ledger_head: String,
    pub state_hash: String,
    /// blake3 of every file under `tiers/`, keyed by its path relative to `tiers/`.
    pub tiers: BTreeMap<String, String>,
    /// Hash of the previous snapshot, chaining snapshots like ledger events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<String>,
    pub hash: String,
}

impl SnapshotManifest {
    pub fn compute_hash(&self) -> String {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(obj) = value.as_object_mut() {
            obj.remove("hash");
        }
        blake3::hash(canonical_json(&value).as_bytes()).to_hex().to_string()
    }
}

#[derive(Debug, Clone)]
pub struct CommitOutcome {
    pub events: Vec<LedgerEvent>,
    pub report: ValidationReport,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct VerifyReport {
    pub ledger_len: u64,
    pub snapshots: usize,
    pub problems: Vec<String>,
}

impl VerifyReport {
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

pub struct ReceiptBook<'p> {
    project: &'p Project,
    ledger: Ledger,
    state: BookState,
    lock: LockState,
    repair: Option<Repair>,
}

struct BookPaths {
    root: PathBuf,
}

impl BookPaths {
    fn ledger(&self) -> PathBuf {
        self.root.join("ledger.jsonl")
    }
    fn state(&self) -> PathBuf {
        self.root.join("state")
    }
    fn tiers(&self) -> PathBuf {
        self.root.join("tiers")
    }
    fn snapshots(&self) -> PathBuf {
        self.root.join("snapshots")
    }
    fn lock(&self) -> PathBuf {
        self.root.join("LOCK.json")
    }
}

fn io(path: &Path, source: std::io::Error) -> BookError {
    CoreError::Io { path: path.to_path_buf(), source }.into()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, BookError> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| BookError::Inconsistent(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(path, e)),
    }
}

impl<'p> ReceiptBook<'p> {
    /// Loads the ledger (repairing an interrupted commit), replays it, and refreshes stale views.
    pub fn open(project: &'p Project) -> Result<Self, BookError> {
        let paths = BookPaths { root: project.paths().receipt_book() };
        let (ledger, repair) = Ledger::load(paths.ledger())?;
        let state = ledger.replay(None)?;
        let lock = read_json::<LockState>(&paths.lock())?.unwrap_or_default();
        let book = Self { project, ledger, state, lock, repair };
        let fresh = views::read_index(&paths.state())
            .is_some_and(|i| i.ledger_len == book.ledger.len() && i.ledger_head == book.ledger.head());
        if !fresh {
            book.rebuild_views()?;
        }
        Ok(book)
    }

    fn paths(&self) -> BookPaths {
        BookPaths { root: self.project.paths().receipt_book() }
    }

    pub fn state(&self) -> &BookState {
        &self.state
    }

    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    pub fn lock_state(&self) -> &LockState {
        &self.lock
    }

    pub fn is_locked(&self) -> bool {
        self.lock.locked
    }

    /// Set when opening had to drop an interrupted commit.
    pub fn repair(&self) -> Option<&Repair> {
        self.repair.as_ref()
    }

    pub fn rebuild_views(&self) -> Result<(), BookError> {
        let dir = self.paths().state();
        views::write_views(&dir, &self.state, self.ledger.len(), self.ledger.head()).map_err(|e| io(&dir, e))
    }

    /// Validates without committing, for the Approval Gate.
    pub fn check(&self, delta: &MemoryDelta, panels: &dyn PanelIndex) -> ValidationReport {
        validate(delta, &self.state, self.ledger.len(), panels)
    }

    /// Validates and, if there are no errors, appends the delta as one ledger commit.
    pub fn commit(
        &mut self,
        delta: &MemoryDelta,
        session: Option<&str>,
        panels: &dyn PanelIndex,
    ) -> Result<CommitOutcome, BookError> {
        if self.lock.locked {
            return Err(BookError::Locked);
        }
        let report = self.check(delta, panels);
        if !report.is_committable() {
            return Err(BookError::Rejected(report));
        }
        // Apply to a copy first so a failure cannot leave memory ahead of the ledger.
        let mut next = self.state.clone();
        for (seq, op) in (self.ledger.len()..).zip(delta.ops.iter().filter(|op| op.target().is_some())) {
            next.apply(seq, op).map_err(|e| BookError::Inconsistent(e.to_string()))?;
        }
        let events = self.ledger.append_commit(delta.chapter_id, session, &delta.ops)?;
        self.state = next;
        self.rebuild_views()?;
        Ok(CommitOutcome { events, report })
    }

    /// Starts a working session: verifies nothing changed while locked, then unlocks.
    pub fn begin_session(&mut self) -> Result<(), BookError> {
        if self.lock.locked
            && (self.lock.ledger_len != self.ledger.len() || self.lock.ledger_head != self.ledger.head())
        {
            return Err(BookError::Inconsistent("the ledger changed while the Receipt Book was locked".into()));
        }
        self.write_lock(LockState {
            locked: false,
            snapshot: self.lock.snapshot.clone(),
            ledger_len: self.ledger.len(),
            ledger_head: self.ledger.head().to_owned(),
            at: Some(OffsetDateTime::now_utc()),
        })
    }

    fn write_lock(&mut self, lock: LockState) -> Result<(), BookError> {
        let path = self.paths().lock();
        fsx::atomic_write_json(&path, &lock).map_err(|e| io(&path, e))?;
        self.lock = lock;
        Ok(())
    }

    pub fn snapshots(&self) -> Result<Vec<SnapshotManifest>, BookError> {
        let dir = self.paths().snapshots();
        let mut out = Vec::new();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(io(&dir, e)),
        };
        for entry in entries {
            let entry = entry.map_err(|e| io(&dir, e))?;
            if let Some(manifest) = read_json::<SnapshotManifest>(&entry.path().join("manifest.json"))? {
                out.push(manifest);
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    fn tier_hashes(&self) -> Result<BTreeMap<String, String>, BookError> {
        fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> std::io::Result<()> {
            if !dir.is_dir() {
                return Ok(());
            }
            for entry in fs::read_dir(dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    walk(base, &path, out)?;
                } else {
                    let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                    out.insert(rel, blake3::hash(&fs::read(&path)?).to_hex().to_string());
                }
            }
            Ok(())
        }
        let tiers = self.paths().tiers();
        let mut out = BTreeMap::new();
        walk(&tiers, &tiers, &mut out).map_err(|e| io(&tiers, e))?;
        Ok(out)
    }

    /// The Stop / Consolidate step: records a hash-chained snapshot of the ledger head, state and tier
    /// files, then locks the book.
    pub fn snapshot_and_lock(
        &mut self,
        label: Option<&str>,
        session: Option<&str>,
    ) -> Result<SnapshotManifest, BookError> {
        let previous = self.snapshots()?;
        let number = previous.last().and_then(|s| s.id.strip_prefix("s_")?.parse::<u32>().ok()).unwrap_or(0) + 1;
        let mut manifest = SnapshotManifest {
            id: format!("s_{number:04}"),
            created_at: OffsetDateTime::now_utc(),
            label: label.map(str::to_owned),
            session: session.map(str::to_owned),
            ledger_len: self.ledger.len(),
            ledger_head: self.ledger.head().to_owned(),
            state_hash: self.state.hash(),
            tiers: self.tier_hashes()?,
            prev: previous.last().map(|s| s.hash.clone()),
            hash: String::new(),
        };
        manifest.hash = manifest.compute_hash();
        let path = self.paths().snapshots().join(&manifest.id).join("manifest.json");
        fsx::atomic_write_json(&path, &manifest).map_err(|e| io(&path, e))?;
        self.write_lock(LockState {
            locked: true,
            snapshot: Some(manifest.id.clone()),
            ledger_len: manifest.ledger_len,
            ledger_head: manifest.ledger_head.clone(),
            at: Some(manifest.created_at),
        })?;
        Ok(manifest)
    }

    /// Full integrity check of snapshots and the lock against the (already chain-verified) ledger.
    pub fn verify(&self) -> Result<VerifyReport, BookError> {
        let snapshots = self.snapshots()?;
        let mut report =
            VerifyReport { ledger_len: self.ledger.len(), snapshots: snapshots.len(), problems: Vec::new() };
        let mut prev: Option<&str> = None;
        for snap in &snapshots {
            if snap.compute_hash() != snap.hash {
                report.problems.push(format!("{}: manifest was modified", snap.id));
            }
            if snap.prev.as_deref() != prev {
                report.problems.push(format!("{}: does not link to the previous snapshot", snap.id));
            }
            prev = Some(&snap.hash);
            if self.ledger.head_at(snap.ledger_len) != Some(snap.ledger_head.as_str()) {
                report.problems.push(format!("{}: ledger history before this snapshot was rewritten", snap.id));
                continue;
            }
            match self.ledger.replay(Some(snap.ledger_len)) {
                Ok(state) if state.hash() == snap.state_hash => {}
                Ok(_) => report.problems.push(format!("{}: replayed state does not match", snap.id)),
                Err(e) => report.problems.push(format!("{}: {e}", snap.id)),
            }
        }
        if self.lock.locked {
            let latest = snapshots.last();
            if latest.map(|s| &s.id) != self.lock.snapshot.as_ref() {
                report.problems.push("LOCK.json does not point at the latest snapshot".into());
            }
            if self.lock.ledger_len != self.ledger.len() || self.lock.ledger_head != self.ledger.head() {
                report.problems.push("the ledger changed after the book was locked".into());
            }
            if let Some(latest) = latest
                && latest.tiers != self.tier_hashes()?
            {
                report.problems.push("tier summaries changed after the book was locked".into());
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{EntityType, MemoryOp, Receipt};
    use jumbo_core::ChapterId;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};

    fn ch(n: u32) -> ChapterId {
        ChapterId::new(n).expect("id")
    }

    fn panels() -> HashMap<ChapterId, HashSet<String>> {
        (1..=5).map(|n| (ch(n), HashSet::from(["p_0001".to_owned()]))).collect()
    }

    fn delta(chapter: u32, base: u64, ids: &[&str]) -> MemoryDelta {
        MemoryDelta {
            schema_version: 1,
            chapter_id: ch(chapter),
            base_ledger_offset: base,
            ops: ids
                .iter()
                .map(|id| MemoryOp::Add {
                    entity_type: EntityType::Character,
                    entity_id: (*id).into(),
                    data: json!({"name": id}).as_object().cloned().expect("obj"),
                    receipts: vec![Receipt { chapter_id: ch(chapter), panel_id: "p_0001".into(), quote: None }],
                    confidence: 0.9,
                })
                .collect(),
        }
    }

    #[test]
    fn session_lifecycle_commit_snapshot_lock_resume() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path(), "P").expect("create");
        {
            let mut book = ReceiptBook::open(&project).expect("open");
            assert!(!book.is_locked());
            book.commit(&delta(1, 0, &["character_a", "character_b"]), Some("s1"), &panels()).expect("commit");
            book.commit(&delta(2, 2, &["character_c"]), Some("s1"), &panels()).expect("commit");
            let snap = book.snapshot_and_lock(Some("after ch 2"), Some("s1")).expect("snapshot");
            assert_eq!((snap.id.as_str(), snap.ledger_len, snap.prev.as_deref()), ("s_0001", 3, None));
            assert!(matches!(book.commit(&delta(3, 3, &["character_d"]), None, &panels()), Err(BookError::Locked)));
            assert!(book.verify().expect("verify").is_ok());
        }
        // Days later: reopen, resume, keep going.
        let mut book = ReceiptBook::open(&project).expect("reopen");
        assert!(book.is_locked());
        assert_eq!(book.state().len(), 3);
        book.begin_session().expect("begin");
        book.commit(&delta(3, 3, &["character_d"]), Some("s2"), &panels()).expect("commit");
        let snap = book.snapshot_and_lock(None, Some("s2")).expect("snapshot");
        assert_eq!(snap.id, "s_0002");
        assert_eq!(snap.prev, Some(book.snapshots().expect("list")[0].hash.clone()));
        let report = book.verify().expect("verify");
        assert!(report.is_ok(), "{:?}", report.problems);
        assert!(project.paths().receipt_book().join("state/characters/character_d.json").is_file());
    }

    #[test]
    fn rejected_deltas_change_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path(), "P").expect("create");
        let mut book = ReceiptBook::open(&project).expect("open");
        book.commit(&delta(1, 0, &["character_a"]), None, &panels()).expect("commit");
        let err = book.commit(&delta(2, 1, &["character_b", "character_a"]), None, &panels()).expect_err("dup");
        assert!(matches!(err, BookError::Rejected(r) if r.errors().count() == 1));
        assert_eq!((book.ledger().len(), book.state().len()), (1, 1));
    }

    #[test]
    fn verify_detects_tampered_snapshots_and_edits_while_locked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path(), "P").expect("create");
        let tiers = project.paths().receipt_book().join("tiers/t2_chapters");
        {
            let mut book = ReceiptBook::open(&project).expect("open");
            book.commit(&delta(1, 0, &["character_a"]), None, &panels()).expect("commit");
            fs::create_dir_all(&tiers).expect("mkdir");
            fs::write(tiers.join("ch_0001.summary.json"), "{}").expect("write");
            book.snapshot_and_lock(None, None).expect("snapshot");
        }
        let manifest = project.paths().receipt_book().join("snapshots/s_0001/manifest.json");
        let text = fs::read_to_string(&manifest).expect("read").replace("\"ledger_len\": 1", "\"ledger_len\": 0");
        fs::write(&manifest, text).expect("write");
        fs::write(tiers.join("ch_0001.summary.json"), "{\"edited\":true}").expect("write");

        let book = ReceiptBook::open(&project).expect("open");
        let problems = book.verify().expect("verify").problems;
        assert!(problems.contains(&"s_0001: manifest was modified".to_owned()), "{problems:?}");
        assert!(problems.contains(&"tier summaries changed after the book was locked".to_owned()), "{problems:?}");
    }

    #[test]
    fn reopening_rebuilds_views_that_lag_the_ledger() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path(), "P").expect("create");
        {
            let mut book = ReceiptBook::open(&project).expect("open");
            book.commit(&delta(1, 0, &["character_a"]), None, &panels()).expect("commit");
        }
        let state_dir = project.paths().receipt_book().join("state");
        fs::remove_dir_all(&state_dir).expect("rm");
        ReceiptBook::open(&project).expect("open");
        assert!(state_dir.join("characters/character_a.json").is_file());
    }
}

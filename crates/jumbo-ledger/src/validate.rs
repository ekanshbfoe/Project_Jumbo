//! Validation of a proposed MemoryDelta before it may be committed. This is the deterministic
//! anti-hallucination gate: every op must apply cleanly and cite panels that actually exist.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;

use jumbo_core::{ChapterId, Project};
use serde::Serialize;

use crate::ops::{MemoryDelta, MemoryOp};
use crate::state::BookState;

/// Below this the Approval Gate highlights an op for human attention.
pub const LOW_CONFIDENCE: f64 = 0.5;

pub trait PanelIndex {
    /// Panel ids detected in a chapter, or `None` when the chapter has no panels.json yet.
    fn panels(&self, chapter: ChapterId) -> Option<HashSet<String>>;
}

/// Reads `chapters/<id>/panels/panels.json` from a project.
pub struct ProjectPanelIndex<'a> {
    project: &'a Project,
}

impl<'a> ProjectPanelIndex<'a> {
    pub fn new(project: &'a Project) -> Self {
        Self { project }
    }
}

impl PanelIndex for ProjectPanelIndex<'_> {
    fn panels(&self, chapter: ChapterId) -> Option<HashSet<String>> {
        let path = jumbo_core::chapter::chapter_dir(self.project, chapter).join("panels").join("panels.json");
        let doc: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        let ids = doc.get("panels")?.as_array()?.iter().filter_map(|p| p.get("id")?.as_str().map(str::to_owned));
        Some(ids.collect())
    }
}

/// In-memory index, for tests and for previews built from data already in hand.
impl PanelIndex for HashMap<ChapterId, HashSet<String>> {
    fn panels(&self, chapter: ChapterId) -> Option<HashSet<String>> {
        self.get(&chapter).cloned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// Index into `delta.ops`, or `None` for delta-level findings.
    pub op_index: Option<usize>,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ValidationReport {
    pub findings: Vec<Finding>,
}

impl ValidationReport {
    pub fn errors(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.severity == Severity::Error)
    }

    pub fn is_committable(&self) -> bool {
        self.errors().next().is_none()
    }

    fn push(&mut self, op_index: Option<usize>, severity: Severity, message: impl Into<String>) {
        self.findings.push(Finding { op_index, severity, message: message.into() });
    }
}

fn is_panel_id(s: &str) -> bool {
    s.len() == 6 && s.starts_with("p_") && s[2..].bytes().all(|b| b.is_ascii_digit())
}

/// Checks `delta` against the committed `state` (which has `ledger_len` entries).
pub fn validate(delta: &MemoryDelta, state: &BookState, ledger_len: u64, panels: &dyn PanelIndex) -> ValidationReport {
    let mut report = ValidationReport::default();
    if delta.schema_version != 1 {
        report.push(None, Severity::Error, format!("unsupported schema_version {}", delta.schema_version));
    }
    if delta.base_ledger_offset != ledger_len {
        report.push(
            None,
            Severity::Error,
            format!(
                "stale delta: generated against {} ledger entries, the ledger now has {ledger_len}",
                delta.base_ledger_offset
            ),
        );
    }
    if delta.ops.iter().all(|op| matches!(op, MemoryOp::Noop { .. })) {
        report.push(None, Severity::Warning, "delta changes nothing");
    }

    let mut index_cache: HashMap<ChapterId, Option<HashSet<String>>> = HashMap::new();
    let mut unverifiable: BTreeSet<ChapterId> = BTreeSet::new();
    let mut sim = state.clone();
    let mut seq = ledger_len;

    for (i, op) in delta.ops.iter().enumerate() {
        for receipt in op.receipts() {
            if receipt.chapter_id > delta.chapter_id {
                report.push(Some(i), Severity::Error, format!("receipt cites future chapter {}", receipt.chapter_id));
                continue;
            }
            if !is_panel_id(&receipt.panel_id) {
                report.push(Some(i), Severity::Error, format!("{:?} is not a panel id", receipt.panel_id));
                continue;
            }
            match index_cache.entry(receipt.chapter_id).or_insert_with(|| panels.panels(receipt.chapter_id)) {
                Some(known) if !known.contains(&receipt.panel_id) => report.push(
                    Some(i),
                    Severity::Error,
                    format!("receipt cites {} {}, which does not exist", receipt.chapter_id, receipt.panel_id),
                ),
                Some(_) => {}
                None => {
                    unverifiable.insert(receipt.chapter_id);
                }
            }
        }
        if let Some(c) = op.confidence()
            && c < LOW_CONFIDENCE
        {
            report.push(Some(i), Severity::Warning, format!("low confidence ({c:.2})"));
        }
        if op.target().is_some() {
            match sim.apply(seq, op) {
                Ok(()) => seq += 1,
                Err(e) => report.push(Some(i), Severity::Error, e.to_string()),
            }
        }
    }
    for chapter in unverifiable {
        report.push(
            None,
            Severity::Warning,
            format!("{chapter} has no panels.json, so its receipts cannot be checked"),
        );
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{EntityType, Receipt};
    use serde_json::json;

    fn ch(n: u32) -> ChapterId {
        ChapterId::new(n).expect("id")
    }

    fn index() -> HashMap<ChapterId, HashSet<String>> {
        HashMap::from([
            (ch(1), HashSet::from(["p_0001".to_owned(), "p_0002".to_owned()])),
            (ch(2), HashSet::from(["p_0001".to_owned()])),
        ])
    }

    fn add(id: &str, chapter: u32, panel: &str, confidence: f64) -> MemoryOp {
        MemoryOp::Add {
            entity_type: EntityType::Character,
            entity_id: id.into(),
            data: json!({"name": id}).as_object().cloned().expect("obj"),
            receipts: vec![Receipt { chapter_id: ch(chapter), panel_id: panel.into(), quote: None }],
            confidence,
        }
    }

    fn delta(chapter: u32, base: u64, ops: Vec<MemoryOp>) -> MemoryDelta {
        MemoryDelta { schema_version: 1, chapter_id: ch(chapter), base_ledger_offset: base, ops }
    }

    fn messages(report: &ValidationReport) -> Vec<(Option<usize>, Severity, String)> {
        report.findings.iter().map(|f| (f.op_index, f.severity, f.message.clone())).collect()
    }

    #[test]
    fn a_clean_delta_is_committable() {
        let report =
            validate(&delta(2, 0, vec![add("character_a", 1, "p_0002", 0.9)]), &BookState::default(), 0, &index());
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert!(report.is_committable());
    }

    #[test]
    fn rejects_stale_deltas_and_bad_receipts() {
        let ops = vec![
            add("character_a", 3, "p_0001", 0.9),
            add("character_b", 1, "p_0009", 0.9),
            add("character_c", 1, "panel-1", 0.9),
        ];
        let report = validate(&delta(2, 4, ops), &BookState::default(), 5, &index());
        assert!(!report.is_committable());
        assert_eq!(
            messages(&report),
            [
                (None, Severity::Error, "stale delta: generated against 4 ledger entries, the ledger now has 5".into()),
                (Some(0), Severity::Error, "receipt cites future chapter ch_0003".into()),
                (Some(1), Severity::Error, "receipt cites ch_0001 p_0009, which does not exist".into()),
                (Some(2), Severity::Error, "\"panel-1\" is not a panel id".into()),
            ]
        );
    }

    #[test]
    fn simulates_ops_in_order_against_committed_state() {
        let mut state = BookState::default();
        state.apply(0, &add("character_a", 1, "p_0001", 1.0)).expect("seed");
        let ops = vec![
            add("character_a", 2, "p_0001", 0.9),
            add("character_b", 2, "p_0001", 0.3),
            add("character_b", 2, "p_0001", 0.9),
        ];
        let report = validate(&delta(2, 1, ops), &state, 1, &index());
        assert_eq!(
            messages(&report),
            [
                (Some(0), Severity::Error, "character_a already exists".into()),
                (Some(1), Severity::Warning, "low confidence (0.30)".into()),
                (Some(2), Severity::Error, "character_b already exists".into()),
            ]
        );
    }

    #[test]
    fn warns_when_panels_cannot_be_checked() {
        let report =
            validate(&delta(5, 0, vec![add("character_a", 4, "p_0001", 0.9)]), &BookState::default(), 0, &index());
        assert!(report.is_committable());
        assert_eq!(
            messages(&report),
            [(None, Severity::Warning, "ch_0004 has no panels.json, so its receipts cannot be checked".into())]
        );
    }

    #[test]
    fn reads_panel_ids_from_a_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path(), "P").expect("create");
        let panels_dir = project.paths().chapters().join("ch_0001").join("panels");
        fs::create_dir_all(&panels_dir).expect("mkdir");
        fs::write(panels_dir.join("panels.json"), r#"{"panels":[{"id":"p_0001"},{"id":"p_0002"}]}"#).expect("write");
        let index = ProjectPanelIndex::new(&project);
        assert_eq!(index.panels(ch(1)), Some(HashSet::from(["p_0001".into(), "p_0002".into()])));
        assert_eq!(index.panels(ch(2)), None);
    }
}

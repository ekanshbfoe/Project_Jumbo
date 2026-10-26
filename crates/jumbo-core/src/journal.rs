//! Append-only job journal (`.jumbo/journal/jobs.jsonl`). Every remote job the app dispatches is
//! recorded here so a lost worker session can be resumed by replaying unfinished jobs.

use std::collections::BTreeMap;
use std::fs;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::chapter::ChapterId;
use crate::error::{IoContext, Result};
use crate::fsx;
use crate::project::Project;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Started,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobEvent {
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    /// Content hash of the job's inputs; identical jobs share an id, which makes re-dispatch idempotent.
    pub job_id: String,
    pub stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter: Option<ChapterId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub status: JobStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl JobEvent {
    pub fn now(job_id: impl Into<String>, stage: impl Into<String>, status: JobStatus) -> Self {
        Self {
            ts: OffsetDateTime::now_utc(),
            job_id: job_id.into(),
            stage: stage.into(),
            chapter: None,
            session: None,
            status,
            detail: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct JournalRead {
    pub events: Vec<JobEvent>,
    /// Lines that failed to parse. A torn final line after a crash is expected; anything else is not.
    pub skipped_lines: Vec<usize>,
}

pub fn append(project: &Project, event: &JobEvent) -> Result<()> {
    let path = project.paths().journal();
    let line = serde_json::to_string(event).map_err(|e| crate::error::invalid(&path, e))?;
    fsx::append_line(&path, &line).at(&path)
}

pub fn read(project: &Project) -> Result<JournalRead> {
    let path = project.paths().journal();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(JournalRead::default()),
        Err(e) => return Err(e).at(&path),
    };
    let mut out = JournalRead::default();
    for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        match serde_json::from_str(line) {
            Ok(event) => out.events.push(event),
            Err(_) => out.skipped_lines.push(i + 1),
        }
    }
    Ok(out)
}

/// The latest event of every job that has not finished, oldest first: what a resumed session re-dispatches.
pub fn unfinished(project: &Project) -> Result<Vec<JobEvent>> {
    let mut latest: BTreeMap<String, JobEvent> = BTreeMap::new();
    for event in read(project)?.events {
        latest.insert(event.job_id.clone(), event);
    }
    let mut open: Vec<JobEvent> = latest.into_values().filter(|e| !e.status.is_finished()).collect();
    open.sort_by_key(|e| e.ts);
    Ok(open)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn unfinished_jobs_survive_a_torn_final_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path(), "J").expect("create");
        let mut magi = JobEvent::now("aaa", "magi_detect", JobStatus::Queued);
        magi.chapter = Some("ch_0001".parse().expect("id"));
        append(&project, &magi).expect("append");
        append(&project, &JobEvent::now("aaa", "magi_detect", JobStatus::Started)).expect("append");
        append(&project, &JobEvent::now("bbb", "direct", JobStatus::Queued)).expect("append");
        append(&project, &JobEvent::now("bbb", "direct", JobStatus::Done)).expect("append");
        append(&project, &JobEvent::now("ccc", "voice", JobStatus::Queued)).expect("append");
        let mut f = fs::OpenOptions::new().append(true).open(project.paths().journal()).expect("open");
        f.write_all(b"{\"ts\":\"2026-10-19T").expect("torn write");

        let read = read(&project).expect("read");
        assert_eq!(read.events.len(), 5);
        assert_eq!(read.skipped_lines, vec![6]);
        let open: Vec<_> = unfinished(&project).expect("unfinished").into_iter().map(|e| e.job_id).collect();
        assert_eq!(open, vec!["aaa", "ccc"]);
    }

    #[test]
    fn events_appended_after_a_crash_are_not_lost() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path(), "J").expect("create");
        append(&project, &JobEvent::now("aaa", "magi_detect", JobStatus::Queued)).expect("append");
        let mut f = fs::OpenOptions::new().append(true).open(project.paths().journal()).expect("open");
        f.write_all(b"{\"ts\":\"2026-10-19T").expect("torn write");
        drop(f);
        append(&project, &JobEvent::now("aaa", "magi_detect", JobStatus::Done)).expect("append");

        let read = read(&project).expect("read");
        assert_eq!(read.events.len(), 2);
        assert_eq!(read.skipped_lines, vec![2]);
        assert!(unfinished(&project).expect("unfinished").is_empty());
    }

    #[test]
    fn missing_journal_reads_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path(), "J").expect("create");
        assert!(read(&project).expect("read").events.is_empty());
    }
}

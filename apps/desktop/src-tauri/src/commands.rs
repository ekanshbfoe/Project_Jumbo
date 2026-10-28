//! IPC commands for the Manhwa Studio module. Errors cross the bridge as display strings; the
//! frontend's types live in src/api.ts and must match the DTOs here.

use std::path::PathBuf;
use std::sync::Mutex;

use jumbo_core::{ChapterId, ChapterManifest, Method, Project, chapter};
use jumbo_ledger::{Entity, LockState, ReceiptBook, SnapshotManifest, VerifyReport};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use time::format_description::well_known::Rfc3339;

use crate::recent::{self, RecentProject};

/// The open project, and the long jobs running on it. Holding the project holds its exclusive file lock.
#[derive(Default)]
pub struct AppState {
    project: Mutex<Option<Project>>,
    jobs: Mutex<Vec<(Lane, String)>>,
}

/// What a long job occupies. One job at a time per lane: the T4 session runs one stage at a time (and Make
/// video stops the session when its stage ends), and every render shares the same Kaggle job dataset and kernels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lane {
    Worker,
    Render,
}

/// A running job; dropping it (the job finished, failed or was abandoned) frees its lane.
pub(crate) struct JobGuard<'a> {
    state: &'a AppState,
    label: String,
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut jobs) = self.state.jobs.lock() {
            jobs.retain(|(_, l)| *l != self.label);
        }
    }
}

/// Claims `lane` for a job, or says which job holds it.
pub(crate) fn begin_job<'a>(state: &'a AppState, lane: Lane, label: String) -> CmdResult<JobGuard<'a>> {
    let mut jobs = state.jobs.lock().map_err(msg)?;
    if let Some((_, running)) = jobs.iter().find(|(l, _)| *l == lane) {
        return Err(format!("{running} is still running; wait for it to finish"));
    }
    jobs.push((lane, label.clone()));
    Ok(JobGuard { state, label })
}

/// Refuses to change the open project while a job would write its results into it.
fn ensure_idle(state: &AppState) -> CmdResult<()> {
    match state.jobs.lock().map_err(msg)?.first() {
        Some((_, running)) => {
            Err(format!("{running} is still running in this project; wait for it to finish before switching project"))
        }
        None => Ok(()),
    }
}

pub(crate) type CmdResult<T> = Result<T, String>;

pub(crate) fn msg(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct ProjectInfo {
    root: String,
    id: String,
    title: String,
    language: String,
    created_at: String,
    method: Method,
}

impl ProjectInfo {
    fn of(project: &Project) -> Self {
        let m = project.manifest();
        Self {
            root: project.paths().root().display().to_string(),
            id: m.id.to_string(),
            title: m.title.clone(),
            language: m.language.clone(),
            created_at: m.created_at.format(&Rfc3339).unwrap_or_default(),
            method: m.method,
        }
    }
}

#[derive(Serialize)]
pub struct BookOverview {
    locked: bool,
    lock: LockState,
    ledger_len: u64,
    ledger_head: String,
    entities: Vec<Entity>,
    snapshots: Vec<SnapshotManifest>,
    /// Set when opening the book dropped an interrupted commit.
    repaired: Option<String>,
}

pub(crate) fn with_project<T>(state: &State<AppState>, f: impl FnOnce(&Project) -> CmdResult<T>) -> CmdResult<T> {
    let guard = state.project.lock().map_err(msg)?;
    let project = guard.as_ref().ok_or("no project is open")?;
    f(project)
}

pub(crate) fn config_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path().app_config_dir().map_err(msg)
}

fn install(app: &AppHandle, state: &State<AppState>, project: Project) -> CmdResult<ProjectInfo> {
    let info = ProjectInfo::of(&project);
    // Lets the live preview load this project's panels and audio through the asset protocol. The
    // scope only ever holds folders the user opened, and lasts until the app quits.
    app.asset_protocol_scope().allow_directory(project.paths().root(), true).map_err(msg)?;
    // Recent-list failures must not block opening the project.
    let _ = config_dir(app).map(|dir| recent::touch(&dir, &info.root, &info.title, info.method));
    // Replacing the old project drops it, which releases its lock.
    *state.project.lock().map_err(msg)? = Some(project);
    Ok(info)
}

#[tauri::command]
pub fn create_project(
    app: AppHandle,
    state: State<AppState>,
    path: String,
    title: String,
    method: Option<Method>,
) -> CmdResult<ProjectInfo> {
    ensure_idle(&state)?;
    let mut project = Project::create(&path, &title).map_err(msg)?;
    if let Some(method) = method {
        project.set_method(method).map_err(msg)?;
    }
    install(&app, &state, project)
}

/// Switches the open project's method, while none of its chapters has a script or story memory yet.
fn switch_method(project: &mut Project, method: Method) -> CmdResult<()> {
    if project.manifest().method != method && crate::memory::any_script(project)? {
        return Err("a chapter already has a script; the method can only change before the first script".into());
    }
    project.set_method(method).map_err(msg)
}

#[tauri::command]
pub fn set_project_method(app: AppHandle, state: State<AppState>, method: Method) -> CmdResult<ProjectInfo> {
    let mut guard = state.project.lock().map_err(msg)?;
    let project = guard.as_mut().ok_or("no project is open")?;
    switch_method(project, method)?;
    let info = ProjectInfo::of(project);
    let _ = config_dir(&app).map(|dir| recent::touch(&dir, &info.root, &info.title, info.method));
    Ok(info)
}

#[tauri::command]
pub fn open_project(app: AppHandle, state: State<AppState>, path: String) -> CmdResult<ProjectInfo> {
    ensure_idle(&state)?;
    // Release the current project first so reopening the same folder does not trip over our own lock.
    state.project.lock().map_err(msg)?.take();
    let project = Project::open(&path).map_err(msg)?;
    install(&app, &state, project)
}

/// Opens a project passed on the command line, before the UI asks for one.
pub fn open_at_startup(app: &AppHandle, state: &State<AppState>, path: PathBuf) -> CmdResult<ProjectInfo> {
    let project = Project::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    install(app, state, project)
}

#[tauri::command]
pub fn close_project(state: State<AppState>) -> CmdResult<()> {
    ensure_idle(&state)?;
    state.project.lock().map_err(msg)?.take();
    Ok(())
}

#[tauri::command]
pub fn current_project(state: State<AppState>) -> CmdResult<Option<ProjectInfo>> {
    Ok(state.project.lock().map_err(msg)?.as_ref().map(ProjectInfo::of))
}

#[tauri::command]
pub fn recent_projects(app: AppHandle) -> CmdResult<Vec<RecentProject>> {
    Ok(recent::load(&config_dir(&app)?))
}

#[tauri::command]
pub fn forget_recent_project(app: AppHandle, path: String) -> CmdResult<()> {
    recent::forget(&config_dir(&app)?, &path).map_err(msg)
}

#[tauri::command]
pub fn list_chapters(state: State<AppState>) -> CmdResult<Vec<ChapterManifest>> {
    with_project(&state, |p| chapter::list_chapters(p).map_err(msg))
}

/// Imports PDFs in the order given; stops at the first failure and reports which file failed.
#[tauri::command]
pub fn import_chapters(state: State<AppState>, paths: Vec<String>) -> CmdResult<Vec<ChapterManifest>> {
    with_project(&state, |p| {
        paths
            .iter()
            .map(|path| chapter::import_chapter(p, path.as_ref(), None).map_err(|e| format!("{path}: {e}")))
            .collect()
    })
}

#[tauri::command]
pub fn book_overview(state: State<AppState>) -> CmdResult<BookOverview> {
    with_project(&state, |p| {
        let book = ReceiptBook::open(p).map_err(msg)?;
        Ok(BookOverview {
            locked: book.is_locked(),
            lock: book.lock_state().clone(),
            ledger_len: book.ledger().len(),
            ledger_head: book.ledger().head().to_owned(),
            entities: book.state().entities().cloned().collect(),
            snapshots: book.snapshots().map_err(msg)?,
            repaired: book
                .repair()
                .map(|r| format!("dropped {} event(s) from an interrupted commit", r.dropped_events)),
        })
    })
}

#[tauri::command]
pub fn verify_book(state: State<AppState>) -> CmdResult<VerifyReport> {
    with_project(&state, |p| ReceiptBook::open(p).and_then(|b| b.verify()).map_err(msg))
}

/// The files a chapter's live preview can be built from, as raw JSON. The frontend validates them
/// against the shared schema, so a broken file shows a precise error instead of a blank player.
#[derive(Serialize)]
pub struct PreviewSources {
    /// `render/episode.json`, when one has been built.
    episode: Option<Value>,
    /// `script/approved.json`, or else the newest `script/draft.vN.json`.
    script: Option<Value>,
    script_file: Option<String>,
    panels: Option<Value>,
    audio: Option<Value>,
}

fn read_json(path: &std::path::Path) -> CmdResult<Option<Value>> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// `approved.json` if the script was approved, else the highest-numbered draft.
pub(crate) fn script_file(dir: &std::path::Path) -> Option<String> {
    if dir.join("approved.json").is_file() {
        return Some("approved.json".to_owned());
    }
    let drafts = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()?.file_name().into_string().ok());
    drafts
        .filter_map(|name| {
            let n = name.strip_prefix("draft.v")?.strip_suffix(".json")?.parse::<u32>().ok()?;
            Some((n, name))
        })
        .max()
        .map(|(_, name)| name)
}

#[tauri::command]
pub fn chapter_preview(state: State<AppState>, chapter: String) -> CmdResult<PreviewSources> {
    with_project(&state, |p| {
        let id: ChapterId = chapter.parse().map_err(msg)?;
        let dir = chapter::chapter_dir(p, id);
        let script_file = script_file(&dir.join("script"));
        Ok(PreviewSources {
            episode: read_json(&dir.join("render").join("episode.json"))?,
            script: match &script_file {
                Some(name) => read_json(&dir.join("script").join(name))?,
                None => None,
            },
            script_file,
            panels: read_json(&dir.join("panels").join("panels.json"))?,
            audio: read_json(&dir.join("audio").join("audio.manifest.json"))?,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_the_approved_script_then_the_newest_draft() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(script_file(dir.path()), None);
        for name in ["draft.v2.json", "draft.v10.json", "draft.v9.json", "notes.md"] {
            std::fs::write(dir.path().join(name), "{}").expect("write");
        }
        assert_eq!(script_file(dir.path()).as_deref(), Some("draft.v10.json"));
        std::fs::write(dir.path().join("approved.json"), "{}").expect("write");
        assert_eq!(script_file(dir.path()).as_deref(), Some("approved.json"));
    }

    #[test]
    fn a_lane_holds_one_job_and_is_freed_when_the_job_ends() {
        let state = AppState::default();
        let voice = begin_job(&state, Lane::Worker, "ch_0001 voice".into()).expect("free");
        assert_eq!(
            begin_job(&state, Lane::Worker, "ch_0002 panels".into()).err().as_deref(),
            Some("ch_0001 voice is still running; wait for it to finish")
        );
        let render = begin_job(&state, Lane::Render, "ch_0003 render".into()).expect("other lane");
        assert!(ensure_idle(&state).is_err());
        drop(voice);
        drop(render);
        assert!(ensure_idle(&state).is_ok() && begin_job(&state, Lane::Worker, "ch_0002 panels".into()).is_ok());
    }

    #[test]
    fn missing_files_are_none_and_broken_ones_are_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(read_json(&dir.path().join("nope.json")).expect("missing"), None);
        std::fs::write(dir.path().join("bad.json"), "{ not json").expect("write");
        assert!(read_json(&dir.path().join("bad.json")).expect_err("broken").contains("bad.json"));
    }
}

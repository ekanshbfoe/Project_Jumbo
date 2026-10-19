//! IPC commands for the Manhwa Studio module. Errors cross the bridge as display strings; the
//! frontend's types live in src/api.ts and must match the DTOs here.

use std::path::PathBuf;
use std::sync::Mutex;

use jumbo_core::{ChapterManifest, Project, chapter};
use jumbo_ledger::{Entity, LockState, ReceiptBook, SnapshotManifest, VerifyReport};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use time::format_description::well_known::Rfc3339;

use crate::recent::{self, RecentProject};

/// The open project. Holding it holds the project's exclusive file lock.
#[derive(Default)]
pub struct AppState {
    project: Mutex<Option<Project>>,
}

type CmdResult<T> = Result<T, String>;

fn msg(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct ProjectInfo {
    root: String,
    id: String,
    title: String,
    language: String,
    created_at: String,
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

fn with_project<T>(state: &State<AppState>, f: impl FnOnce(&Project) -> CmdResult<T>) -> CmdResult<T> {
    let guard = state.project.lock().map_err(msg)?;
    let project = guard.as_ref().ok_or("no project is open")?;
    f(project)
}

fn config_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path().app_config_dir().map_err(msg)
}

fn install(app: &AppHandle, state: &State<AppState>, project: Project) -> CmdResult<ProjectInfo> {
    let info = ProjectInfo::of(&project);
    // Recent-list failures must not block opening the project.
    let _ = config_dir(app).map(|dir| recent::touch(&dir, &info.root, &info.title));
    // Replacing the old project drops it, which releases its lock.
    *state.project.lock().map_err(msg)? = Some(project);
    Ok(info)
}

#[tauri::command]
pub fn create_project(app: AppHandle, state: State<AppState>, path: String, title: String) -> CmdResult<ProjectInfo> {
    let project = Project::create(&path, &title).map_err(msg)?;
    install(&app, &state, project)
}

#[tauri::command]
pub fn open_project(app: AppHandle, state: State<AppState>, path: String) -> CmdResult<ProjectInfo> {
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

//! Project root layout, manifest and the single-writer lock.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{CoreError, IoContext, Result, invalid};
use crate::fsx;

pub const PROJECT_FILE: &str = "jumbo.project.toml";
pub const PROJECT_SCHEMA_VERSION: u32 = 1;

/// How a project's scripts are written (ADR 0005, ADR 0006). Chosen when the project is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// The Director on Kaggle (Qwen3-VL) writes every script.
    #[default]
    Method1,
    /// The person gets each script from Gemini in Google AI Studio and pastes it in.
    Method2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub schema_version: u32,
    pub id: Uuid,
    pub title: String,
    /// Narration language, e.g. "hinglish".
    pub language: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Projects made before Method 2 existed have no field and are Method 1.
    #[serde(default)]
    pub method: Method,
}

/// Every well-known location inside a project root. Paths only; nothing here touches the disk.
#[derive(Debug, Clone)]
pub struct ProjectPaths {
    root: PathBuf,
}

impl ProjectPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn manifest(&self) -> PathBuf {
        self.root.join(PROJECT_FILE)
    }
    pub fn internal(&self) -> PathBuf {
        self.root.join(".jumbo")
    }
    pub fn lock_file(&self) -> PathBuf {
        self.internal().join("project.lock")
    }
    pub fn journal(&self) -> PathBuf {
        self.internal().join("journal").join("jobs.jsonl")
    }
    pub fn blobs(&self) -> PathBuf {
        self.internal().join("blobs")
    }
    pub fn sessions(&self) -> PathBuf {
        self.internal().join("sessions")
    }
    pub fn chapters(&self) -> PathBuf {
        self.root.join("chapters")
    }
    pub fn receipt_book(&self) -> PathBuf {
        self.root.join("receipt_book")
    }
    /// The project's voice library: `voices/<speaker>/<emotion>.voice.gguf`, shared by every chapter.
    pub fn voices(&self) -> PathBuf {
        self.root.join("voices")
    }
    pub fn exports(&self) -> PathBuf {
        self.root.join("exports")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    fn skeleton(&self) -> [PathBuf; 9] {
        [
            self.journal().parent().map(Path::to_path_buf).unwrap_or_default(),
            self.blobs(),
            self.sessions(),
            self.chapters(),
            self.receipt_book().join("state"),
            self.receipt_book().join("tiers"),
            self.receipt_book().join("snapshots"),
            self.exports(),
            self.logs(),
        ]
    }
}

/// An open project. Holding this value holds the project's exclusive lock.
#[derive(Debug)]
pub struct Project {
    paths: ProjectPaths,
    manifest: ProjectManifest,
    _lock: File,
}

impl Project {
    /// Creates the folder structure and manifest in `root` (which may already exist but must not
    /// already be a project), then opens it.
    pub fn create(root: impl Into<PathBuf>, title: &str) -> Result<Self> {
        let paths = ProjectPaths::new(root);
        if paths.manifest().exists() {
            return Err(CoreError::AlreadyExists(paths.root().to_path_buf()));
        }
        let title = title.trim();
        if title.is_empty() {
            return Err(invalid(&paths.manifest(), "title must not be empty"));
        }
        for dir in paths.skeleton() {
            fs::create_dir_all(&dir).at(&dir)?;
        }
        let manifest = ProjectManifest {
            schema_version: PROJECT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            title: title.to_owned(),
            language: "hinglish".to_owned(),
            created_at: OffsetDateTime::now_utc(),
            method: Method::default(),
        };
        fsx::atomic_write_toml(&paths.manifest(), &manifest).at(&paths.manifest())?;
        Self::open(paths.root().to_path_buf())
    }

    /// Opens an existing project and takes its exclusive lock.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let paths = ProjectPaths::new(root);
        let manifest_path = paths.manifest();
        if !manifest_path.is_file() {
            return Err(CoreError::NotAProject(paths.root().to_path_buf()));
        }
        let text = fs::read_to_string(&manifest_path).at(&manifest_path)?;
        let manifest: ProjectManifest = toml::from_str(&text).map_err(|e| invalid(&manifest_path, e))?;
        if manifest.schema_version > PROJECT_SCHEMA_VERSION {
            return Err(CoreError::SchemaVersion { found: manifest.schema_version, supported: PROJECT_SCHEMA_VERSION });
        }
        // Older projects may predate some folders; recreate anything missing.
        for dir in paths.skeleton() {
            fs::create_dir_all(&dir).at(&dir)?;
        }
        let lock = acquire_lock(&paths.lock_file())?;
        Ok(Self { paths, manifest, _lock: lock })
    }

    pub fn paths(&self) -> &ProjectPaths {
        &self.paths
    }

    pub fn manifest(&self) -> &ProjectManifest {
        &self.manifest
    }

    /// Switches the project's method. Callers check first that no chapter has a script yet.
    pub fn set_method(&mut self, method: Method) -> Result<()> {
        let mut next = self.manifest.clone();
        next.method = method;
        fsx::atomic_write_toml(&self.paths.manifest(), &next).at(&self.paths.manifest())?;
        self.manifest = next;
        Ok(())
    }

    pub fn rename(&mut self, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() {
            return Err(invalid(&self.paths.manifest(), "title must not be empty"));
        }
        let mut next = self.manifest.clone();
        next.title = title.to_owned();
        fsx::atomic_write_toml(&self.paths.manifest(), &next).at(&self.paths.manifest())?;
        self.manifest = next;
        Ok(())
    }
}

/// OS-level advisory lock, released automatically when the handle closes (including on crash).
fn acquire_lock(path: &Path) -> Result<File> {
    let mut file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(path).at(path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(CoreError::Locked),
        Err(TryLockError::Error(e)) => return Err(CoreError::Io { path: path.to_path_buf(), source: e }),
    }
    // Informational only; the lock itself is what matters.
    let _ = file.set_len(0).and_then(|()| writeln!(file, "pid {}", std::process::id()));
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_writes_manifest_and_skeleton() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path(), "  Solo Leveling Recaps ").expect("create");
        assert_eq!(project.manifest().title, "Solo Leveling Recaps");
        assert_eq!(project.manifest().language, "hinglish");
        for sub in ["chapters", "receipt_book/state", "receipt_book/snapshots", ".jumbo/journal", "exports"] {
            assert!(dir.path().join(sub).is_dir(), "{sub} missing");
        }
    }

    #[test]
    fn refuses_to_create_over_an_existing_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        drop(Project::create(dir.path(), "A").expect("create"));
        assert!(matches!(Project::create(dir.path(), "B"), Err(CoreError::AlreadyExists(_))));
    }

    #[test]
    fn second_open_is_locked_until_the_first_closes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = Project::create(dir.path(), "A").expect("create");
        assert!(matches!(Project::open(dir.path()), Err(CoreError::Locked)));
        drop(first);
        let reopened = Project::open(dir.path()).expect("reopen");
        assert_eq!(reopened.manifest().title, "A");
    }

    #[test]
    fn rejects_non_projects_and_future_schema_versions() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(Project::open(dir.path()), Err(CoreError::NotAProject(_))));

        drop(Project::create(dir.path(), "A").expect("create"));
        let manifest = dir.path().join(PROJECT_FILE);
        let text = fs::read_to_string(&manifest).expect("read").replace("schema_version = 1", "schema_version = 99");
        fs::write(&manifest, text).expect("write");
        assert!(matches!(Project::open(dir.path()), Err(CoreError::SchemaVersion { found: 99, .. })));
    }

    #[test]
    fn rename_persists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut project = Project::create(dir.path(), "A").expect("create");
        project.rename("B").expect("rename");
        drop(project);
        assert_eq!(Project::open(dir.path()).expect("open").manifest().title, "B");
    }

    #[test]
    fn a_project_is_method1_until_switched_and_old_manifests_read_as_method1() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut project = Project::create(dir.path(), "A").expect("create");
        assert_eq!(project.manifest().method, Method::Method1);
        project.set_method(Method::Method2).expect("switch");
        drop(project);
        assert_eq!(Project::open(dir.path()).expect("open").manifest().method, Method::Method2);
        let text = std::fs::read_to_string(dir.path().join(PROJECT_FILE)).expect("manifest");
        assert!(text.contains("method = \"method2\""));
        let old = text.lines().filter(|l| !l.starts_with("method")).collect::<Vec<_>>().join("\n");
        let manifest: ProjectManifest = toml::from_str(&old).expect("old manifest");
        assert_eq!(manifest.method, Method::Method1);
    }
}

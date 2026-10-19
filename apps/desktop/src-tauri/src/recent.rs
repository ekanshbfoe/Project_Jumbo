//! Recently opened projects, kept in the app config dir (never inside a project).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use jumbo_core::fsx;
use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentProject {
    pub path: String,
    pub title: String,
    /// Milliseconds since the Unix epoch.
    pub opened_at: u64,
}

fn file(config_dir: &Path) -> PathBuf {
    config_dir.join("recent_projects.json")
}

pub fn load(config_dir: &Path) -> Vec<RecentProject> {
    fs::read_to_string(file(config_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Moves `path` to the front of the list.
pub fn touch(config_dir: &Path, path: &str, title: &str) -> std::io::Result<()> {
    let opened_at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let mut list = load(config_dir);
    list.retain(|p| p.path != path);
    list.insert(0, RecentProject { path: path.to_owned(), title: title.to_owned(), opened_at });
    list.truncate(MAX_RECENT);
    fsx::atomic_write_json(&file(config_dir), &list)
}

pub fn forget(config_dir: &Path, path: &str) -> std::io::Result<()> {
    let mut list = load(config_dir);
    list.retain(|p| p.path != path);
    fsx::atomic_write_json(&file(config_dir), &list)
}

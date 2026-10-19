use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{0} is not a Jumbo project (no jumbo.project.toml)")]
    NotAProject(PathBuf),
    #[error("{0} already contains a Jumbo project")]
    AlreadyExists(PathBuf),
    #[error("the project is already open in another Jumbo window")]
    Locked,
    #[error("invalid {path}: {message}")]
    Invalid { path: PathBuf, message: String },
    #[error("project schema version {found} is newer than this build supports ({supported})")]
    SchemaVersion { found: u32, supported: u32 },
    #[error("invalid chapter id {0:?} (expected ch_0001)")]
    InvalidChapterId(String),
    #[error("chapter {0} does not exist")]
    ChapterNotFound(String),
    #[error("chapter {id} cannot go from {from} to {to}")]
    InvalidTransition { id: String, from: String, to: String },
}

pub type Result<T, E = CoreError> = std::result::Result<T, E>;

/// Attaches the offending path to io errors.
pub(crate) trait IoContext<T> {
    fn at(self, path: &Path) -> Result<T>;
}

impl<T> IoContext<T> for io::Result<T> {
    fn at(self, path: &Path) -> Result<T> {
        self.map_err(|source| CoreError::Io { path: path.to_path_buf(), source })
    }
}

pub(crate) fn invalid(path: &Path, message: impl ToString) -> CoreError {
    CoreError::Invalid { path: path.to_path_buf(), message: message.to_string() }
}

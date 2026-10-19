//! Local persistence for Project Jumbo. The desktop app is the only writer of a project folder;
//! everything here is plain files under the user-selected project root.

pub mod chapter;
pub mod error;
pub mod fsx;
pub mod journal;
pub mod project;

pub use chapter::{ChapterId, ChapterManifest, ChapterState, Layout};
pub use error::{CoreError, Result};
pub use project::{Project, ProjectManifest, ProjectPaths};

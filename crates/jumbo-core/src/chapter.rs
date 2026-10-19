//! Chapters: import, manifests and the per-chapter pipeline state machine. `chapter.toml` state
//! transitions are the resume points after a crash or a lost worker session.

use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::OffsetDateTime;

use crate::error::{CoreError, IoContext, Result, invalid};
use crate::fsx;
use crate::project::Project;

pub const CHAPTER_FILE: &str = "chapter.toml";
pub const SOURCE_FILE: &str = "original.pdf";

/// `ch_0001` style chapter id. Ordered by number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChapterId(u32);

impl ChapterId {
    pub fn new(number: u32) -> Result<Self> {
        if (1..=9999).contains(&number) {
            Ok(Self(number))
        } else {
            Err(CoreError::InvalidChapterId(number.to_string()))
        }
    }

    pub fn number(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ChapterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ch_{:04}", self.0)
    }
}

impl FromStr for ChapterId {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self> {
        let digits = s.strip_prefix("ch_").filter(|d| d.len() == 4 && d.bytes().all(|b| b.is_ascii_digit()));
        let number = digits.and_then(|d| d.parse().ok()).ok_or_else(|| CoreError::InvalidChapterId(s.to_owned()))?;
        Self::new(number).map_err(|_| CoreError::InvalidChapterId(s.to_owned()))
    }
}

impl Serialize for ChapterId {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ChapterId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// Pipeline stages in order. A chapter only moves forward one stage at a time, but can be rewound to
/// any earlier stage to re-run it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterState {
    Imported,
    Rasterized,
    Analyzed,
    Directed,
    /// Librarian proposed a memory delta; waiting at the Approval Gate.
    Librarian,
    Approved,
    Voiced,
    Rendered,
    Consolidated,
}

impl ChapterState {
    pub const ALL: [ChapterState; 9] = [
        Self::Imported,
        Self::Rasterized,
        Self::Analyzed,
        Self::Directed,
        Self::Librarian,
        Self::Approved,
        Self::Voiced,
        Self::Rendered,
        Self::Consolidated,
    ];

    pub fn next(self) -> Option<Self> {
        Self::ALL.iter().position(|s| *s == self).and_then(|i| Self::ALL.get(i + 1)).copied()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::Rasterized => "rasterized",
            Self::Analyzed => "analyzed",
            Self::Directed => "directed",
            Self::Librarian => "librarian",
            Self::Approved => "approved",
            Self::Voiced => "voiced",
            Self::Rendered => "rendered",
            Self::Consolidated => "consolidated",
        }
    }
}

impl fmt::Display for ChapterState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// Vertical webtoon strip, sliced into pseudo-pages.
    Strip,
    /// Traditional bordered pages.
    Page,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateChange {
    pub from: ChapterState,
    pub to: ChapterState,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterManifest {
    pub id: ChapterId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// File name of the imported PDF as the user picked it.
    pub original_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    pub state: ChapterState,
    /// Set when an upstream chapter's approved memory changed, so this chapter's script must be redone.
    #[serde(default)]
    pub stale: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub imported_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default)]
    pub history: Vec<StateChange>,
}

pub fn chapter_dir(project: &Project, id: ChapterId) -> PathBuf {
    project.paths().chapters().join(id.to_string())
}

fn manifest_path(project: &Project, id: ChapterId) -> PathBuf {
    chapter_dir(project, id).join(CHAPTER_FILE)
}

/// Copies a PDF into the project as the next chapter. The manifest is written last, so an interrupted
/// import leaves a folder without `chapter.toml`, which listing ignores and the next import skips over.
pub fn import_chapter(project: &Project, pdf: &Path, title: Option<&str>) -> Result<ChapterManifest> {
    let mut magic = [0u8; 5];
    File::open(pdf).and_then(|mut f| f.read_exact(&mut magic)).at(pdf)?;
    if &magic != b"%PDF-" {
        return Err(invalid(pdf, "not a PDF file"));
    }
    let id = next_chapter_id(project)?;
    let dir = chapter_dir(project, id);
    let source = dir.join("source").join(SOURCE_FILE);
    fs::create_dir_all(source.parent().unwrap_or(&dir)).at(&dir)?;
    let staged = source.with_extension("pdf.partial");
    fs::copy(pdf, &staged).at(pdf)?;
    fs::rename(&staged, &source).at(&source)?;

    let now = OffsetDateTime::now_utc();
    let manifest = ChapterManifest {
        id,
        title: title.map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned),
        original_name: pdf.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        layout: None,
        state: ChapterState::Imported,
        stale: false,
        imported_at: now,
        updated_at: now,
        history: Vec::new(),
    };
    save_chapter(project, &manifest)?;
    Ok(manifest)
}

fn next_chapter_id(project: &Project) -> Result<ChapterId> {
    let dir = project.paths().chapters();
    let mut max = 0;
    for entry in fs::read_dir(&dir).at(&dir)? {
        let entry = entry.at(&dir)?;
        if let Some(id) = entry.file_name().to_str().and_then(|n| n.parse::<ChapterId>().ok()) {
            max = max.max(id.number());
        }
    }
    ChapterId::new(max + 1)
}

pub fn load_chapter(project: &Project, id: ChapterId) -> Result<ChapterManifest> {
    let path = manifest_path(project, id);
    if !path.is_file() {
        return Err(CoreError::ChapterNotFound(id.to_string()));
    }
    let text = fs::read_to_string(&path).at(&path)?;
    let manifest: ChapterManifest = toml::from_str(&text).map_err(|e| invalid(&path, e))?;
    if manifest.id != id {
        return Err(invalid(&path, format!("manifest id {} does not match its folder", manifest.id)));
    }
    Ok(manifest)
}

pub fn save_chapter(project: &Project, manifest: &ChapterManifest) -> Result<()> {
    let path = manifest_path(project, manifest.id);
    fsx::atomic_write_toml(&path, manifest).at(&path)
}

/// All chapters with a manifest, in chapter order.
pub fn list_chapters(project: &Project) -> Result<Vec<ChapterManifest>> {
    let dir = project.paths().chapters();
    let mut ids: Vec<ChapterId> = Vec::new();
    for entry in fs::read_dir(&dir).at(&dir)? {
        let entry = entry.at(&dir)?;
        if let Some(id) = entry.file_name().to_str().and_then(|n| n.parse::<ChapterId>().ok())
            && entry.path().join(CHAPTER_FILE).is_file()
        {
            ids.push(id);
        }
    }
    ids.sort();
    ids.into_iter().map(|id| load_chapter(project, id)).collect()
}

fn transition(
    project: &Project,
    id: ChapterId,
    to: ChapterState,
    allowed: impl Fn(ChapterState) -> bool,
) -> Result<ChapterManifest> {
    let mut manifest = load_chapter(project, id)?;
    let from = manifest.state;
    if !allowed(from) {
        return Err(CoreError::InvalidTransition { id: id.to_string(), from: from.to_string(), to: to.to_string() });
    }
    let now = OffsetDateTime::now_utc();
    manifest.state = to;
    manifest.updated_at = now;
    manifest.history.push(StateChange { from, to, at: now });
    save_chapter(project, &manifest)?;
    Ok(manifest)
}

/// Moves a chapter to the stage after its current one.
pub fn advance(project: &Project, id: ChapterId, to: ChapterState) -> Result<ChapterManifest> {
    transition(project, id, to, |from| from.next() == Some(to))
}

/// Sends a chapter back to an earlier stage so that stage and everything after it run again.
/// Re-running clears the stale flag once the chapter is back before direction.
pub fn rewind(project: &Project, id: ChapterId, to: ChapterState) -> Result<ChapterManifest> {
    let mut manifest = transition(project, id, to, |from| to < from)?;
    if manifest.stale && to <= ChapterState::Analyzed {
        manifest.stale = false;
        save_chapter(project, &manifest)?;
    }
    Ok(manifest)
}

pub fn set_stale(project: &Project, id: ChapterId, stale: bool) -> Result<ChapterManifest> {
    let mut manifest = load_chapter(project, id)?;
    manifest.stale = stale;
    manifest.updated_at = OffsetDateTime::now_utc();
    save_chapter(project, &manifest)?;
    Ok(manifest)
}

pub fn set_layout(project: &Project, id: ChapterId, layout: Layout) -> Result<ChapterManifest> {
    let mut manifest = load_chapter(project, id)?;
    manifest.layout = Some(layout);
    manifest.updated_at = OffsetDateTime::now_utc();
    save_chapter(project, &manifest)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Project, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path().join("proj"), "Test").expect("create");
        let pdf = dir.path().join("Chapter 1.pdf");
        fs::write(&pdf, b"%PDF-1.7\n%fake\n").expect("pdf");
        (dir, project, pdf)
    }

    #[test]
    fn chapter_ids_round_trip_and_reject_junk() {
        assert_eq!("ch_0042".parse::<ChapterId>().expect("parse").to_string(), "ch_0042");
        for bad in ["ch_42", "ch_0000", "chapter_0001", "ch_00a1", "ch_00001"] {
            assert!(bad.parse::<ChapterId>().is_err(), "{bad} accepted");
        }
    }

    #[test]
    fn import_assigns_sequential_ids_and_copies_the_pdf() {
        let (_dir, project, pdf) = setup();
        let a = import_chapter(&project, &pdf, Some(" The Awakening ")).expect("import");
        let b = import_chapter(&project, &pdf, None).expect("import");
        assert_eq!((a.id.to_string(), b.id.to_string()), ("ch_0001".into(), "ch_0002".into()));
        assert_eq!(a.title.as_deref(), Some("The Awakening"));
        assert_eq!(a.original_name, "Chapter 1.pdf");
        assert!(chapter_dir(&project, a.id).join("source").join(SOURCE_FILE).is_file());
        assert_eq!(list_chapters(&project).expect("list"), vec![a, b]);
    }

    #[test]
    fn import_rejects_non_pdfs() {
        let (dir, project, _) = setup();
        let txt = dir.path().join("notes.pdf");
        fs::write(&txt, b"hello world").expect("write");
        assert!(matches!(import_chapter(&project, &txt, None), Err(CoreError::Invalid { .. })));
    }

    #[test]
    fn interrupted_imports_are_skipped() {
        let (_dir, project, pdf) = setup();
        fs::create_dir_all(project.paths().chapters().join("ch_0001").join("source")).expect("mkdir");
        assert!(list_chapters(&project).expect("list").is_empty());
        assert_eq!(import_chapter(&project, &pdf, None).expect("import").id.to_string(), "ch_0002");
    }

    #[test]
    fn state_machine_advances_one_stage_at_a_time_and_rewinds() {
        let (_dir, project, pdf) = setup();
        let id = import_chapter(&project, &pdf, None).expect("import").id;
        assert!(matches!(advance(&project, id, ChapterState::Analyzed), Err(CoreError::InvalidTransition { .. })));
        advance(&project, id, ChapterState::Rasterized).expect("advance");
        advance(&project, id, ChapterState::Analyzed).expect("advance");
        advance(&project, id, ChapterState::Directed).expect("advance");
        set_stale(&project, id, true).expect("stale");
        assert!(matches!(rewind(&project, id, ChapterState::Voiced), Err(CoreError::InvalidTransition { .. })));
        let rewound = rewind(&project, id, ChapterState::Analyzed).expect("rewind");
        assert_eq!(rewound.state, ChapterState::Analyzed);
        assert!(!rewound.stale);
        assert_eq!(rewound.history.len(), 4);
        assert_eq!(load_chapter(&project, id).expect("load"), rewound);
    }
}

//! The story memory that carries a narration from one chapter to the next, the same for both methods.
//!
//! Each script draft comes with `script/memory.draft.json`; approving the script makes it `script/memory.json`, so
//! only approved stories carry on. A chapter's script continues from exactly the chapter before it: if that one has
//! no approved memory yet, the next chapter waits instead of silently skipping a chapter of story. When an approved
//! memory changes, every later chapter that already has a script is marked stale. Characters are merged over all
//! earlier chapters (a name keeps its first spelling, the rest of its line is the latest), so a name Gemini forgot
//! to repeat is not lost.

use jumbo_core::chapter;
use jumbo_core::{ChapterId, ChapterManifest, ChapterState, Method, Project, fsx};
use serde::{Deserialize, Deserializer, Serialize};

use crate::commands::{CmdResult, msg};

pub const DRAFT: &str = "memory.draft.json";
pub const APPROVED: &str = "memory.json";

fn null_as_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StoryMemory {
    /// The story up to the end of the chapter. Method 1 files written before this format call it "memory".
    #[serde(default, alias = "memory", deserialize_with = "null_as_empty")]
    pub story: String,
    /// "Name | role | he/she | visual tag", one per named character.
    #[serde(default)]
    pub characters: Vec<String>,
    /// The chapter's last narration lines, so the next one does not repeat them.
    #[serde(default)]
    pub last_lines: Vec<String>,
    #[serde(default)]
    pub method: Method,
}

/// What a chapter's script continues from.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StoryContext {
    pub story: String,
    pub characters: Vec<String>,
    pub last_lines: Vec<String>,
    /// The chapter the story comes from; none for the first chapter.
    pub from: Option<String>,
}

fn path(project: &Project, id: ChapterId, name: &str) -> std::path::PathBuf {
    chapter::chapter_dir(project, id).join("script").join(name)
}

fn read(project: &Project, id: ChapterId, name: &str) -> CmdResult<Option<StoryMemory>> {
    let file = path(project, id, name);
    match std::fs::read_to_string(&file) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| format!("{}: {e}", file.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

/// Saves the memory that came with a new script draft; it counts once the script is approved.
pub fn write_draft(project: &Project, id: ChapterId, memory: &StoryMemory) -> CmdResult<()> {
    fsx::atomic_write_json(&path(project, id, DRAFT), memory).map_err(msg)
}

/// The key a character is merged by: the name before the first "|", lowercased, spaces collapsed.
fn character_key(line: &str) -> String {
    line.split('|').next().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Adds `more` (a later chapter's characters) to `into`. A character already there keeps its first spelling of the
/// name but takes the later role, pronoun and look, so a correction made in a later chapter is what comes next.
pub fn merge_characters(into: &mut Vec<String>, more: &[String]) {
    for line in more {
        let key = character_key(line);
        if key.is_empty() {
            continue;
        }
        match into.iter_mut().find(|c| character_key(c) == key) {
            Some(known) => {
                let name = known.split('|').next().unwrap_or("").trim().to_owned();
                let rest: Vec<&str> = line.split('|').skip(1).map(str::trim).collect();
                *known = if rest.is_empty() { name } else { format!("{name} | {}", rest.join(" | ")) };
            }
            None => into.push(line.trim().to_owned()),
        }
    }
}

/// The story chapter `id` continues: the approved memory of the chapter just before it, and every earlier
/// chapter's characters. Refuses while the chapter before has no approved memory.
pub fn context_before(project: &Project, id: ChapterId) -> CmdResult<StoryContext> {
    let mut earlier: Vec<ChapterManifest> =
        chapter::list_chapters(project).map_err(msg)?.into_iter().filter(|c| c.id < id).collect();
    earlier.sort_by_key(|c| c.id);
    let Some(previous) = earlier.last() else { return Ok(StoryContext::default()) };
    let memory = read(project, previous.id, APPROVED)?.filter(|m| !m.story.trim().is_empty()).ok_or_else(|| {
        format!(
            "{id} continues the story of {}, which has no approved script yet. Approve {}'s script first.",
            previous.id, previous.id
        )
    })?;
    let mut characters = Vec::new();
    for c in &earlier {
        if let Some(m) = read(project, c.id, APPROVED)? {
            merge_characters(&mut characters, &m.characters);
        }
    }
    Ok(StoryContext {
        story: memory.story,
        characters,
        last_lines: memory.last_lines,
        from: Some(previous.id.to_string()),
    })
}

/// Makes the draft memory the approved one, with `last_lines` taken from the script as approved (it may have been
/// edited since the draft was written). Returns true when the approved memory changed.
pub fn approve(project: &Project, id: ChapterId, last_lines: Vec<String>) -> CmdResult<bool> {
    let before = read(project, id, APPROVED)?;
    let Some(mut draft) = read(project, id, DRAFT)?.or_else(|| before.clone()) else { return Ok(false) };
    draft.last_lines = last_lines;
    fsx::atomic_write_json(&path(project, id, APPROVED), &draft).map_err(msg)?;
    let _ = std::fs::remove_file(path(project, id, DRAFT));
    Ok(before.is_some_and(|b| b != draft))
}

/// Marks every later chapter that already has a script as stale: its story was written from an older memory.
pub fn mark_later_stale(project: &Project, id: ChapterId) -> CmdResult<Vec<ChapterId>> {
    let mut marked = Vec::new();
    for c in chapter::list_chapters(project).map_err(msg)? {
        if c.id > id && c.state >= ChapterState::Directed && !c.stale {
            chapter::set_stale(project, c.id, true).map_err(msg)?;
            marked.push(c.id);
        }
    }
    Ok(marked)
}

/// True when any chapter has a script or a memory, which fixes the project's method.
pub fn any_script(project: &Project) -> CmdResult<bool> {
    for c in chapter::list_chapters(project).map_err(msg)? {
        let dir = chapter::chapter_dir(project, c.id).join("script");
        if std::fs::read_dir(&dir).map(|mut d| d.next().is_some()).unwrap_or(false) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn project_with_chapters(n: usize) -> (tempfile::TempDir, Project, Vec<ChapterId>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path().join("p"), "Test").expect("project");
        let pdf = dir.path().join("c.pdf");
        std::fs::write(&pdf, b"%PDF-1.4\n").expect("pdf");
        let ids = (0..n)
            .map(|_| {
                let id = chapter::import_chapter(&project, &pdf, None).expect("import").id;
                for to in [ChapterState::Rasterized, ChapterState::Analyzed] {
                    chapter::advance(&project, id, to).expect("advance");
                }
                id
            })
            .collect();
        (dir, project, ids)
    }

    fn memory(story: &str, characters: &[&str]) -> StoryMemory {
        StoryMemory {
            story: story.into(),
            characters: characters.iter().map(|c| (*c).to_owned()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_chapter_continues_from_the_approved_memory_of_the_one_just_before() {
        let (_tmp, project, ids) = project_with_chapters(3);
        assert_eq!(context_before(&project, ids[0]).expect("first"), StoryContext::default());
        write_draft(&project, ids[0], &memory("A happens.", &["Ana | hero | she | red coat"])).expect("draft");
        // A draft does not count until it is approved.
        assert!(context_before(&project, ids[1]).expect_err("unapproved").contains("Approve ch_0001's script first"));
        approve(&project, ids[0], vec![]).expect("approve");
        let ctx = context_before(&project, ids[1]).expect("ctx");
        assert_eq!((ctx.story.as_str(), ctx.from.as_deref()), ("A happens.", Some("ch_0001")));
        // Chapter 3 may not skip chapter 2.
        assert!(context_before(&project, ids[2]).expect_err("gap").contains("ch_0002"));
    }

    #[test]
    fn characters_are_merged_over_every_earlier_chapter_first_name_spelling_latest_details() {
        let (_tmp, project, ids) = project_with_chapters(3);
        write_draft(&project, ids[0], &memory("A.", &["Ana | hero | she | red coat", "Bo | rival | he | scar"]))
            .expect("1");
        approve(&project, ids[0], vec![]).expect("approve 1");
        write_draft(&project, ids[1], &memory("B.", &["  ana  | heroine | she | red coat", "Cy | mentor | he | cane"]))
            .expect("2");
        approve(&project, ids[1], vec![]).expect("approve 2");
        let ctx = context_before(&project, ids[2]).expect("ctx");
        assert_eq!(ctx.story, "B.");
        assert_eq!(
            ctx.characters,
            ["Ana | heroine | she | red coat", "Bo | rival | he | scar", "Cy | mentor | he | cane"]
        );
    }

    #[test]
    fn method1_memory_files_from_before_still_read() {
        let (_tmp, project, ids) = project_with_chapters(2);
        let file = path(&project, ids[0], APPROVED);
        std::fs::create_dir_all(file.parent().expect("dir")).expect("dir");
        std::fs::write(&file, r#"{"memory": "Old story.", "last_lines": ["x"]}"#).expect("old");
        assert_eq!(context_before(&project, ids[1]).expect("ctx").story, "Old story.");
        std::fs::write(&file, r#"{"memory": null, "last_lines": []}"#).expect("empty");
        assert!(context_before(&project, ids[1]).is_err());
    }

    #[test]
    fn changing_an_approved_memory_marks_later_scripted_chapters_stale() {
        let (_tmp, project, ids) = project_with_chapters(3);
        write_draft(&project, ids[0], &memory("A.", &[])).expect("draft");
        assert!(!approve(&project, ids[0], vec![]).expect("first approval is not a change"));
        chapter::advance(&project, ids[1], ChapterState::Directed).expect("directed");
        write_draft(&project, ids[0], &memory("A, rewritten.", &[])).expect("redo");
        assert!(approve(&project, ids[0], vec![]).expect("changed"));
        assert_eq!(mark_later_stale(&project, ids[0]).expect("stale"), [ids[1]]);
        assert!(chapter::load_chapter(&project, ids[1]).expect("ch2").stale);
        assert!(!chapter::load_chapter(&project, ids[2]).expect("ch3").stale, "no script yet, nothing to redo");
    }

    #[test]
    fn the_last_lines_come_from_the_script_as_approved() {
        let (_tmp, project, ids) = project_with_chapters(2);
        write_draft(&project, ids[0], &StoryMemory { last_lines: vec!["पुरानी लाइन।".into()], ..memory("A.", &[]) })
            .expect("draft");
        approve(&project, ids[0], vec!["नई लाइन।".into()]).expect("approve");
        assert_eq!(context_before(&project, ids[1]).expect("ctx").last_lines, ["नई लाइन।"]);
        // Approving an edit again, with no new draft, still updates them.
        approve(&project, ids[0], vec!["फिर बदली लाइन।".into()]).expect("re-approve");
        assert_eq!(context_before(&project, ids[1]).expect("ctx").last_lines, ["फिर बदली लाइन।"]);
    }
}

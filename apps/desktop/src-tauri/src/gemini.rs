//! Method 2 on the app side (ADR 0006): what a chapter's Gemini step needs (the numbered PDF, its pack, and the
//! story it continues), and saving the script read from Gemini's pasted answer.
//!
//! The answer is read and checked in the frontend (packages/schema/src/tagged.ts); saving checks the parts that
//! matter again here, so a stale screen or an old pack can never put a script on the wrong panels.

use std::collections::HashSet;

use jumbo_contracts::script_doc::ScriptDoc;
use jumbo_core::chapter;
use jumbo_core::{ChapterId, ChapterManifest, ChapterState, Method, Project, fsx};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::State;

use crate::commands::{AppState, CmdResult, msg, with_project};
use crate::memory::{self, StoryContext, StoryMemory};
use crate::script::{narration_problems, next_draft};

/// panels/numbered.json, written by the vision job next to the numbered PDF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberedPack {
    pub chapter: String,
    pub pack: String,
    /// Panel ids in reading order.
    pub order: Vec<String>,
    #[serde(default)]
    pub pages: u32,
    #[serde(default)]
    pub bytes: u64,
}

fn numbered(project: &Project, id: ChapterId) -> CmdResult<Option<NumberedPack>> {
    let path = chapter::chapter_dir(project, id).join("panels").join("numbered.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Everything the Gemini screen shows for a chapter.
#[derive(Serialize)]
pub struct GeminiView {
    /// None until the chapter is analyzed with a vision job that writes the numbered pack.
    pack: Option<NumberedPack>,
    /// Project-relative path of the numbered PDF.
    pdf: Option<String>,
    /// The story this chapter continues, or why it cannot be written yet.
    context: Option<StoryContext>,
    context_error: Option<String>,
}

#[tauri::command]
pub fn gemini_pack(state: State<AppState>, chapter: String) -> CmdResult<GeminiView> {
    with_project(&state, |p| {
        let id: ChapterId = chapter.parse().map_err(msg)?;
        let pack = numbered(p, id)?;
        let pdf = chapter::chapter_dir(p, id).join("panels").join("numbered.pdf");
        let (context, context_error) = match memory::context_before(p, id) {
            Ok(c) => (Some(c), None),
            Err(e) => (None, Some(e)),
        };
        Ok(GeminiView {
            pdf: (pack.is_some() && pdf.is_file()).then(|| format!("chapters/{chapter}/panels/numbered.pdf")),
            pack,
            context,
            context_error,
        })
    })
}

/// Copies the chapter's numbered PDF to `dest`, a path the person picked, to drop into AI Studio.
#[tauri::command]
pub fn export_numbered_pdf(state: State<AppState>, chapter: String, dest: String) -> CmdResult<()> {
    with_project(&state, |p| {
        let id: ChapterId = chapter.parse().map_err(msg)?;
        let pdf = chapter::chapter_dir(p, id).join("panels").join("numbered.pdf");
        if !pdf.is_file() {
            return Err("this chapter has no numbered PDF yet; analyze it again".into());
        }
        std::fs::copy(&pdf, &dest).map(|_| ()).map_err(|e| format!("{dest}: {e}"))
    })
}

/// The story part of Gemini's MEMORY block, as the person confirmed it.
#[derive(Debug, Clone, Deserialize)]
pub struct PastedMemory {
    pub story: String,
    #[serde(default)]
    pub characters: Vec<String>,
    #[serde(default)]
    pub open_loops: Vec<String>,
}

/// A hook as the frontend checked it (packages/schema/src/hooks.ts); only its panels are checked again here.
#[derive(Debug, Clone, Deserialize)]
struct HookPanels {
    id: String,
    panel_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct HooksFile {
    chapter_id: String,
    hooks: Vec<HookPanels>,
}

pub const HOOKS: &str = "hooks.json";

/// Saves a script read from Gemini's answer as the chapter's next draft, with its memory, and marks it Directed.
fn save(
    project: &Project,
    id: ChapterId,
    pack: &str,
    script: &Value,
    pasted: &PastedMemory,
    hooks: Option<&Value>,
) -> CmdResult<ChapterManifest> {
    if project.manifest().method != Method::Method2 {
        return Err("this is a Method 1 project: its scripts come from the Director".into());
    }
    let state = chapter::load_chapter(project, id).map_err(msg)?;
    if !matches!(state.state, ChapterState::Analyzed | ChapterState::Directed) {
        return Err(format!("{id} is {}; a pasted script goes onto an analyzed chapter", state.state.as_str()));
    }
    let numbered = numbered(project, id)?.ok_or("this chapter has no numbered PDF yet; analyze it again")?;
    if numbered.pack != pack || numbered.chapter != id.to_string() {
        return Err(format!(
            "the answer was read for pack {pack}, but {id}'s numbered PDF is now pack {}; give Gemini the current PDF",
            numbered.pack
        ));
    }
    let doc: ScriptDoc = serde_json::from_value(script.clone()).map_err(|e| format!("the script is invalid: {e}"))?;
    if doc.chapter_id.as_str() != id.to_string() {
        return Err(format!("this script belongs to {}, not {id}", doc.chapter_id.as_str()));
    }
    let lines = script["lines"].as_array().map(Vec::as_slice).unwrap_or_default();
    let used: Vec<&str> =
        lines.iter().flat_map(|l| l["panel_ids"].as_array().into_iter().flatten()).filter_map(Value::as_str).collect();
    let known: HashSet<&str> = numbered.order.iter().map(String::as_str).collect();
    if let Some(unknown) = used.iter().find(|p| !known.contains(**p)) {
        return Err(format!("the script names {unknown}, which this chapter does not have"));
    }
    let shown: HashSet<&str> = used.iter().copied().collect();
    let missing: Vec<&str> = numbered.order.iter().map(String::as_str).filter(|p| !shown.contains(p)).collect();
    if !missing.is_empty() {
        return Err(format!("no narration for {}; the answer is incomplete", missing.join(", ")));
    }
    let problems = narration_problems(script);
    if !problems.is_empty() {
        return Err(format!("fix these lines first: {}", problems.join("; ")));
    }
    if let Some(hooks) = hooks {
        let file: HooksFile = serde_json::from_value(hooks.clone()).map_err(|e| format!("the hooks are invalid: {e}"))?;
        if file.chapter_id != id.to_string() {
            return Err(format!("these hooks belong to {}, not {id}", file.chapter_id));
        }
        if let Some((h, p)) =
            file.hooks.iter().find_map(|h| h.panel_ids.iter().find(|p| !known.contains(p.as_str())).map(|p| (h, p)))
        {
            return Err(format!("hook {} names {p}, which this chapter does not have", h.id));
        }
    }
    if pasted.story.trim().is_empty() {
        return Err("the answer's MEMORY block is empty; the next chapter would start without the story".into());
    }
    // The story must continue from the chapter just before, approved.
    memory::context_before(project, id)?;

    let dir = chapter::chapter_dir(project, id).join("script");
    std::fs::create_dir_all(&dir).map_err(msg)?;
    fsx::atomic_write_json(&next_draft(&dir), script).map_err(msg)?;
    let last_lines =
        lines.iter().rev().take(2).rev().filter_map(|l| l["narration"].as_str().map(str::to_owned)).collect();
    let story = StoryMemory {
        story: pasted.story.trim().to_owned(),
        characters: pasted.characters.iter().map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).collect(),
        last_lines,
        open_loops: pasted.open_loops.iter().map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).collect(),
        method: Method::Method2,
    };
    memory::write_draft(project, id, &story)?;
    // Hooks come with the answer they were read from: an answer without any leaves none behind.
    match hooks {
        Some(hooks) => fsx::atomic_write_json(&dir.join(HOOKS), hooks).map_err(msg)?,
        None => {
            let _ = std::fs::remove_file(dir.join(HOOKS));
        }
    }
    fsx::atomic_write_json(&dir.join("gemini.json"), &json!({ "pack": numbered.pack })).map_err(msg)?;
    if state.state == ChapterState::Analyzed {
        return chapter::advance(project, id, ChapterState::Directed).map_err(msg);
    }
    Ok(state)
}

#[tauri::command]
pub fn save_gemini_script(
    state: State<AppState>,
    chapter: String,
    pack: String,
    script: Value,
    memory: PastedMemory,
    hooks: Option<Value>,
) -> CmdResult<ChapterManifest> {
    with_project(&state, |p| save(p, chapter.parse().map_err(msg)?, &pack, &script, &memory, hooks.as_ref()))
}

/// Each chapter's hooks (script/hooks.json), or null where its answer had none: what an episode opens and ends on.
#[tauri::command]
pub fn chapter_hooks(state: State<AppState>, chapters: Vec<String>) -> CmdResult<serde_json::Map<String, Value>> {
    with_project(&state, |p| {
        let mut out = serde_json::Map::new();
        for c in chapters {
            let id: ChapterId = c.parse().map_err(msg)?;
            let path = chapter::chapter_dir(p, id).join("script").join(HOOKS);
            let value = match std::fs::read_to_string(&path) {
                Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Null,
                Err(e) => return Err(format!("{}: {e}", path.display())),
            };
            out.insert(c, value);
        }
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::tests::project_with_chapters;

    fn script(id: ChapterId) -> Value {
        let mut script: Value =
            serde_json::from_str(include_str!("../../../../packages/schema/fixtures/script-doc.sample.json"))
                .expect("fixture");
        script["chapter_id"] = json!(id.to_string());
        script
    }

    /// The sample script shows p_0001 to p_0003.
    fn with_pack(project: &Project, id: ChapterId, pack: &str) {
        let order: Vec<String> = (1..=3).map(|n| format!("p_{n:04}")).collect();
        let path = chapter::chapter_dir(project, id).join("panels").join("numbered.json");
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        fsx::atomic_write_json(&path, &json!({ "chapter": id.to_string(), "pack": pack, "order": order }))
            .expect("pack");
    }

    fn story(text: &str) -> PastedMemory {
        PastedMemory {
            story: text.into(),
            characters: vec!["Jinwoo | hunter | he | scar".into()],
            open_loops: vec!["Q1 | Who sent the letter?".into()],
        }
    }

    fn method2(project: &mut Project) {
        project.set_method(Method::Method2).expect("method 2");
    }

    #[test]
    fn a_pasted_script_becomes_a_draft_with_its_memory() {
        let (_tmp, mut project, ids) = project_with_chapters(1);
        method2(&mut project);
        with_pack(&project, ids[0], "7f3a");
        let state = save(&project, ids[0], "7f3a", &script(ids[0]), &story("Jinwoo wakes."), None).expect("save");
        assert_eq!(state.state, ChapterState::Directed);
        let dir = chapter::chapter_dir(&project, ids[0]).join("script");
        assert!(dir.join("draft.v1.json").is_file() && dir.join("memory.draft.json").is_file());
    }

    #[test]
    fn a_pasted_script_is_refused_for_the_wrong_project_pack_or_panels() {
        let (_tmp, mut project, ids) = project_with_chapters(2);
        let s = script(ids[0]);
        assert!(save(&project, ids[0], "7f3a", &s, &story("x"), None).expect_err("method 1").contains("Method 1 project"));
        method2(&mut project);
        assert!(save(&project, ids[0], "7f3a", &s, &story("x"), None).expect_err("no pack").contains("no numbered PDF"));
        with_pack(&project, ids[0], "b201");
        assert!(save(&project, ids[0], "7f3a", &s, &story("x"), None).expect_err("old pack").contains("now pack b201"));
        assert!(
            save(&project, ids[0], "b201", &s, &story(" "), None).expect_err("no memory").contains("MEMORY block is empty")
        );
        let mut short = s.clone();
        short["lines"].as_array_mut().expect("lines").truncate(1);
        assert!(
            save(&project, ids[0], "b201", &short, &story("x"), None)
                .expect_err("incomplete")
                .contains("no narration for p_0002")
        );
        // Chapter 2 waits for chapter 1's approved story.
        with_pack(&project, ids[1], "c0de");
        let err = save(&project, ids[1], "c0de", &script(ids[1]), &story("y"), None).expect_err("chain");
        assert!(err.contains("Approve ch_0001's script first"), "{err}");
    }

    #[test]
    fn the_method_is_fixed_once_a_chapter_has_a_script() {
        let (_tmp, mut project, ids) = project_with_chapters(1);
        assert!(!memory::any_script(&project).expect("none"));
        method2(&mut project);
        with_pack(&project, ids[0], "7f3a");
        save(&project, ids[0], "7f3a", &script(ids[0]), &story("x"), None).expect("save");
        assert!(memory::any_script(&project).expect("one"));
    }

    #[test]
    fn hooks_are_saved_with_the_answer_and_checked_against_the_panels() {
        let (_tmp, mut project, ids) = project_with_chapters(1);
        method2(&mut project);
        with_pack(&project, ids[0], "7f3a");
        let hooks = |panel: &str| {
            json!({ "schema_version": 1, "chapter_id": ids[0].to_string(), "recap_in": "",
                    "hooks": [{ "id": "H1", "panel_ids": [panel], "strength": 5, "kind": "reveal", "spoiler": "low", "cold": "x", "tease": "" }] })
        };
        let err = save(&project, ids[0], "7f3a", &script(ids[0]), &story("x"), Some(&hooks("p_0009"))).expect_err("panel");
        assert!(err.contains("hook H1 names p_0009"), "{err}");
        save(&project, ids[0], "7f3a", &script(ids[0]), &story("x"), Some(&hooks("p_0002"))).expect("save");
        let file = chapter::chapter_dir(&project, ids[0]).join("script").join(HOOKS);
        assert!(file.is_file());
        let draft: StoryMemory =
            serde_json::from_str(&std::fs::read_to_string(file.with_file_name(memory::DRAFT)).expect("draft")).expect("json");
        assert_eq!(draft.open_loops, vec!["Q1 | Who sent the letter?"]);
        // A later answer without hooks leaves none behind.
        save(&project, ids[0], "7f3a", &script(ids[0]), &story("x"), None).expect("save");
        assert!(!file.exists());
    }
}

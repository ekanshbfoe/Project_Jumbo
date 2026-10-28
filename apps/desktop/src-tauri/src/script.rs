//! The Director on the app side: send a chapter's panels to the worker's `director` job, keep what it writes as
//! the next `script/draft.vN.json`, and approve a reviewed script as `script/approved.json`.
//!
//! The story memory the Director writes with each draft becomes the chapter's memory when the script is approved
//! (memory.rs), and the next chapter's run continues from it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use jumbo_contracts::panels_doc::PanelsDoc;
use jumbo_contracts::script_doc::ScriptDoc;
use jumbo_core::chapter;
use jumbo_core::{ChapterId, ChapterManifest, ChapterState, Method, Project, fsx};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Emitter, State};

use crate::bridge::{BridgeState, current_client, download, upload, with_worker};
use crate::commands::{AppState, CmdResult, msg, with_project};
use crate::memory::{self, StoryMemory};

/// What the worker's director job returns (worker/jumbo_worker/pipeline/director/stage.py).
#[derive(Debug, Deserialize)]
struct DirectorManifest {
    chapter_id: String,
    script: Value,
    #[serde(default)]
    memory: Option<String>,
    #[serde(default)]
    last_lines: Vec<String>,
    #[serde(default)]
    report: Value,
    #[serde(default)]
    raw: Value,
}

#[derive(Clone, Serialize)]
struct Progress<'a> {
    chapter: &'a str,
    progress: f64,
    message: String,
}

/// Submits a job and waits for it, reporting its progress. Returns the output blob's hash. A poll that fails is
/// retried (see `with_worker`), so a tunnel hiccup does not abandon a job that is still running on the worker.
pub(crate) async fn run_job(
    bridge: &BridgeState,
    kind: &str,
    input: &str,
    params: Value,
    mut report: impl FnMut(f64, String),
) -> CmdResult<String> {
    let job_id = current_client(bridge).await?.submit_job(kind, input, params).await.map_err(msg)?;
    loop {
        let job = with_worker(bridge, |c| {
            let job_id = job_id.clone();
            async move { c.job(&job_id).await.map_err(msg) }
        })
        .await?;
        match job.status.as_str() {
            "done" => return job.output.ok_or_else(|| format!("{kind} finished without output")),
            "failed" => return Err(job.error.unwrap_or_else(|| format!("{kind} failed"))),
            _ => {
                report(job.progress, format!("{} on the worker", job.status));
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/// `draft.vN.json` for the next N in a chapter's script folder.
pub(crate) fn next_draft(dir: &Path) -> PathBuf {
    let last = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|name| name.strip_prefix("draft.v")?.strip_suffix(".json")?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    dir.join(format!("draft.v{}.json", last + 1))
}

/// Writes the Director's draft, report, raw passes and memory into `script/`, then marks the chapter Directed.
fn install(project: &Project, id: ChapterId, manifest: &DirectorManifest) -> CmdResult<(PathBuf, ChapterManifest)> {
    let script: ScriptDoc = serde_json::from_value(manifest.script.clone())
        .map_err(|e| format!("script from the worker is invalid: {e}"))?;
    if script.chapter_id.as_str() != id.to_string() {
        return Err(format!("worker answered for {} instead of {id}", script.chapter_id.as_str()));
    }
    let dir = chapter::chapter_dir(project, id).join("script");
    std::fs::create_dir_all(&dir).map_err(msg)?;
    let draft = next_draft(&dir);
    fsx::atomic_write_json(&draft, &manifest.script).map_err(msg)?;
    fsx::atomic_write_json(&dir.join("director.report.json"), &manifest.report).map_err(msg)?;
    fsx::atomic_write_json(&dir.join("director.raw.json"), &manifest.raw).map_err(msg)?;
    let story = StoryMemory {
        story: manifest.memory.clone().unwrap_or_default(),
        last_lines: manifest.last_lines.clone(),
        method: Method::Method1,
        ..Default::default()
    };
    memory::write_draft(project, id, &story)?;
    let mut state = chapter::load_chapter(project, id).map_err(msg)?;
    if state.state == ChapterState::Analyzed {
        state = chapter::advance(project, id, ChapterState::Directed).map_err(msg)?;
    }
    Ok((draft, state))
}

/// Runs the Director for one chapter on the connected worker. Progress arrives as `script://progress` events.
#[tauri::command]
pub async fn write_script(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, BridgeState>,
    chapter: String,
) -> CmdResult<ChapterManifest> {
    let id: ChapterId = chapter.parse().map_err(msg)?;
    let (root, dir, panels, context) = with_project(&state, |p| {
        if p.manifest().method == Method::Method2 {
            return Err("this is a Method 2 project: its scripts come from Gemini, not the Director".into());
        }
        let manifest = chapter::load_chapter(p, id).map_err(msg)?;
        if !matches!(manifest.state, ChapterState::Analyzed | ChapterState::Directed) {
            return Err(format!("{chapter} is {}; the Director runs on an analyzed chapter", manifest.state.as_str()));
        }
        let dir = chapter::chapter_dir(p, id);
        let text =
            std::fs::read_to_string(dir.join("panels").join("panels.json")).map_err(|e| format!("panels.json: {e}"))?;
        let panels: Value = serde_json::from_str(&text).map_err(|e| format!("panels.json: {e}"))?;
        Ok((p.paths().root().to_path_buf(), dir, panels, memory::context_before(p, id)?))
    })?;
    let doc: PanelsDoc = serde_json::from_value(panels.clone()).map_err(|e| format!("panels.json is invalid: {e}"))?;
    current_client(&bridge).await?;
    let report = |progress: f64, message: String| {
        let _ = app.emit("script://progress", Progress { chapter: &chapter, progress, message });
    };

    let mut files = Map::new();
    let total = doc.panels.len();
    for (n, panel) in doc.panels.iter().enumerate() {
        let crop = panel.crop.as_ref().ok_or_else(|| format!("{} has no crop; re-analyze the chapter", *panel.id))?;
        report(0.05 * n as f64 / total as f64, format!("uploading panel {}/{total}", n + 1));
        let hash = upload(&bridge, &root.join(crop.as_str())).await?;
        files.insert(crop.to_string(), Value::String(hash));
    }
    let story = (!context.story.is_empty()).then_some(context.story);
    let request = json!({ "panels": panels, "files": files, "memory": story, "last_lines": context.last_lines });
    let request_path = dir.join(".director.request.json");
    fsx::atomic_write_json(&request_path, &request).map_err(msg)?;
    let input = upload(&bridge, &request_path).await;
    let _ = std::fs::remove_file(&request_path);
    let output = run_job(&bridge, "director", &input?, json!({}), |p, m| report(0.05 + p * 0.93, m)).await?;

    let manifest_path = dir.join(".director.manifest.json");
    download(&bridge, &output, &manifest_path).await?;
    let manifest: Result<DirectorManifest, String> =
        std::fs::read(&manifest_path).map_err(msg).and_then(|bytes| serde_json::from_slice(&bytes).map_err(msg));
    let _ = std::fs::remove_file(&manifest_path);
    let manifest = manifest?;
    if manifest.chapter_id != chapter {
        return Err(format!("worker answered for {} instead of {chapter}", manifest.chapter_id));
    }
    let (_, done) = with_project(&state, |p| install(p, id, &manifest))?;
    report(1.0, "script written".into());
    Ok(done)
}

/// What the script screen shows: the newest script, which file it is, the Director's report and the panels.
#[derive(Serialize)]
pub struct ScriptView {
    script: Option<Value>,
    script_file: Option<String>,
    report: Option<Value>,
    panels: Option<Value>,
}

fn read_json(path: &Path) -> CmdResult<Option<Value>> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[tauri::command]
pub fn chapter_script(state: State<AppState>, chapter: String) -> CmdResult<ScriptView> {
    with_project(&state, |p| {
        let id: ChapterId = chapter.parse().map_err(msg)?;
        let dir = chapter::chapter_dir(p, id);
        let script_file = crate::commands::script_file(&dir.join("script"));
        Ok(ScriptView {
            script: match &script_file {
                Some(name) => read_json(&dir.join("script").join(name))?,
                None => None,
            },
            script_file,
            report: read_json(&dir.join("script").join("director.report.json"))?,
            panels: read_json(&dir.join("panels").join("panels.json"))?,
        })
    })
}

/// Every line of a script that breaks the Devanagari narration rule, as "l_0004: why".
pub(crate) fn narration_problems(script: &Value) -> Vec<String> {
    script["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|line| {
            let problem = jumbo_core::hinglish::narration_problem(line["narration"].as_str().unwrap_or(""))?;
            Some(format!("{}: {problem}", line["id"].as_str().unwrap_or("?")))
        })
        .collect()
}

/// Saves a reviewed script as `approved.json` and moves the chapter on to Approved. There is no Librarian step: the
/// script's memory carries the story, so the chapter passes straight through that stage. The narration rule is
/// checked here, before any voice session is spent on the script, and the draft's memory becomes the chapter's;
/// if that changes an approved story, later chapters with scripts are marked stale.
fn approve(project: &Project, id: ChapterId, script: &Value) -> CmdResult<ChapterManifest> {
    let doc: ScriptDoc = serde_json::from_value(script.clone()).map_err(|e| format!("the script is invalid: {e}"))?;
    if doc.chapter_id.as_str() != id.to_string() {
        return Err(format!("this script belongs to {}, not {id}", doc.chapter_id.as_str()));
    }
    let problems = narration_problems(script);
    if !problems.is_empty() {
        return Err(format!("fix these lines before approving: {}", problems.join("; ")));
    }
    let mut manifest = chapter::load_chapter(project, id).map_err(msg)?;
    if !matches!(manifest.state, ChapterState::Directed | ChapterState::Librarian | ChapterState::Approved) {
        return Err(format!("{id} is {}; only a directed chapter's script can be approved", manifest.state.as_str()));
    }
    let dir = chapter::chapter_dir(project, id).join("script");
    fsx::atomic_write_json(&dir.join("approved.json"), script).map_err(msg)?;
    if memory::approve(project, id)? {
        memory::mark_later_stale(project, id)?;
    }
    while manifest.state < ChapterState::Approved {
        let next = manifest.state.next().ok_or("no next stage")?;
        manifest = chapter::advance(project, id, next).map_err(msg)?;
    }
    Ok(manifest)
}

#[tauri::command]
pub fn approve_script(state: State<AppState>, chapter: String, script: Value) -> CmdResult<ChapterManifest> {
    with_project(&state, |p| approve(p, chapter.parse().map_err(msg)?, &script))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_chapter() -> (tempfile::TempDir, Project, ChapterId) {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path().join("p"), "Test").expect("project");
        let pdf = dir.path().join("c.pdf");
        std::fs::write(&pdf, b"%PDF-1.4\n").expect("pdf");
        let id = chapter::import_chapter(&project, &pdf, None).expect("import").id;
        for to in [ChapterState::Rasterized, ChapterState::Analyzed] {
            chapter::advance(&project, id, to).expect("advance");
        }
        (dir, project, id)
    }

    fn sample_script(id: ChapterId) -> Value {
        let mut script: Value =
            serde_json::from_str(include_str!("../../../../packages/schema/fixtures/script-doc.sample.json"))
                .expect("fixture");
        script["chapter_id"] = json!(id.to_string());
        script
    }

    fn manifest(id: ChapterId, memory: &str) -> DirectorManifest {
        DirectorManifest {
            chapter_id: id.to_string(),
            script: sample_script(id),
            memory: Some(memory.into()),
            last_lines: vec!["आख़िरी लाइन।".into()],
            report: json!({ "failed": [] }),
            raw: json!([]),
        }
    }

    #[test]
    fn drafts_are_numbered_and_the_chapter_becomes_directed() {
        let (_tmp, project, id) = project_with_chapter();
        let (first, state) = install(&project, id, &manifest(id, "one")).expect("install");
        assert_eq!(first.file_name().expect("name"), "draft.v1.json");
        assert_eq!(state.state, ChapterState::Directed);
        let (second, state) = install(&project, id, &manifest(id, "two")).expect("again");
        assert_eq!(second.file_name().expect("name"), "draft.v2.json");
        assert_eq!(state.state, ChapterState::Directed);
        let dir = chapter::chapter_dir(&project, id).join("script");
        assert!(dir.join("director.report.json").is_file() && dir.join("director.raw.json").is_file());
    }

    #[test]
    fn the_next_chapter_continues_from_the_approved_memory() {
        let (tmp, project, first) = project_with_chapter();
        let second = chapter::import_chapter(&project, &tmp.path().join("c.pdf"), None).expect("import").id;
        install(&project, first, &manifest(first, "Akira quits.")).expect("install");
        assert!(memory::context_before(&project, second).is_err(), "not approved yet");
        approve(&project, first, &sample_script(first)).expect("approve");
        let ctx = memory::context_before(&project, second).expect("memory");
        assert_eq!(
            (ctx.story.as_str(), ctx.last_lines.as_slice()),
            ("Akira quits.", ["आख़िरी लाइन।".to_owned()].as_slice())
        );
    }

    #[test]
    fn approval_refuses_narration_in_latin_letters() {
        let (_tmp, project, id) = project_with_chapter();
        install(&project, id, &manifest(id, "m")).expect("install");
        let mut script = sample_script(id);
        script["lines"][1]["narration"] = json!("Dono taraf dushman, beech mein sirf woh.");
        let err = approve(&project, id, &script).expect_err("romanized");
        assert!(err.contains("l_0002: narration has Hindi in Latin letters"), "{err}");
    }

    #[test]
    fn a_script_for_another_chapter_is_refused() {
        let (tmp, project, id) = project_with_chapter();
        let other = chapter::import_chapter(&project, &tmp.path().join("c.pdf"), None).expect("import").id;
        let err = install(&project, id, &manifest(other, "x")).expect_err("wrong chapter");
        assert!(err.contains("instead of"), "{err}");
    }

    #[test]
    fn approving_writes_approved_json_and_passes_the_librarian_stage() {
        let (_tmp, project, id) = project_with_chapter();
        let err = approve(&project, id, &sample_script(id)).expect_err("not directed yet");
        assert!(err.contains("only a directed chapter"), "{err}");
        install(&project, id, &manifest(id, "m")).expect("install");
        let state = approve(&project, id, &sample_script(id)).expect("approve");
        assert_eq!(state.state, ChapterState::Approved);
        let dir = chapter::chapter_dir(&project, id).join("script");
        assert_eq!(crate::commands::script_file(&dir).as_deref(), Some("approved.json"));
        // Editing again after approval overwrites the approved script.
        approve(&project, id, &sample_script(id)).expect("re-approve");
        assert!(approve(&project, id, &json!({ "lines": [] })).expect_err("bad").contains("invalid"));
    }
}

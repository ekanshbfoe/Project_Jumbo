//! The voice stage on the app side: send a chapter's approved script and the project's voice files to the
//! worker's `voice` job, install the clips under `audio/`, and keep any voice it created in the library.
//!
//! The library is `voices/<speaker>/<emotion>.voice.gguf`, in two places: the app's config folder, shared by
//! every project (an imported voice goes there, so it is saved once and every new project has it), and the
//! project, which wins for a speaker and emotion both have. A speaker without a file is invented by the worker
//! from its description on the first line, and the take it clones comes back into the project.
//!
//! A speaker's folder may also hold `voice.json` with its speed-up, `{"tempo": 1.25}` (the project's wins): a voice
//! cloned from a slow speaker plays faster than the job's default 1.1.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use jumbo_contracts::audio_manifest::AudioManifest;
use jumbo_core::chapter;
use jumbo_core::{ChapterId, ChapterManifest, ChapterState, Project, fsx};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Emitter, State};

use crate::bridge::{BridgeState, current_client, download, upload};
use crate::commands::{AppState, CmdResult, Lane, begin_job, config_dir, msg, with_project};
use crate::script::run_job;

const VOICE_EXT: &str = ".voice.gguf";
const VOICE_SETTINGS: &str = "voice.json";

/// What the worker's voice job returns (worker/jumbo_worker/pipeline/voice/stage.py).
#[derive(Debug, Deserialize)]
struct VoiceManifest {
    chapter_id: String,
    audio: Value,
    /// Chapter-relative path -> blob hash.
    files: BTreeMap<String, String>,
    /// Speaker -> emotion -> blob hash of a voice file the job created.
    #[serde(default)]
    voices: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    report: Value,
}

#[derive(Clone, Serialize)]
struct Progress<'a> {
    chapter: &'a str,
    progress: f64,
    message: String,
}

/// A speaker or emotion name used as a folder or file name: letters, digits, `_` and `-` only.
fn safe_name(name: &str) -> CmdResult<&str> {
    let ok =
        !name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok { Ok(name) } else { Err(format!("not a usable voice name: {name:?}")) }
}

/// The app-wide voice folder, shared by every project.
fn shared_voices(app: &AppHandle) -> CmdResult<PathBuf> {
    Ok(config_dir(app)?.join("voices"))
}

/// Every voice file in the library: speaker -> emotion -> path; the project's files override the shared ones.
fn library(shared: Option<&Path>, project: &Project) -> BTreeMap<String, BTreeMap<String, PathBuf>> {
    let mut out: BTreeMap<String, BTreeMap<String, PathBuf>> = BTreeMap::new();
    let project_dir = project.paths().voices();
    for root in shared.into_iter().chain([project_dir.as_path()]) {
        add_voices(root, &mut out);
    }
    out
}

/// Each speaker's speed-up from its `voice.json`, the project's over the shared one; speakers without one are left out.
fn tempos(shared: Option<&Path>, project: &Project) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    let project_dir = project.paths().voices();
    for root in shared.into_iter().chain([project_dir.as_path()]) {
        for speaker in std::fs::read_dir(root).into_iter().flatten().flatten() {
            let Some(name) = speaker.file_name().to_str().map(str::to_owned) else { continue };
            let settings = std::fs::read(speaker.path().join(VOICE_SETTINGS)).ok();
            let tempo =
                settings.and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["tempo"].as_f64());
            if let Some(tempo) = tempo.filter(|t| (0.5..=2.0).contains(t)) {
                out.insert(name, tempo);
            }
        }
    }
    out
}

fn add_voices(root: &Path, out: &mut BTreeMap<String, BTreeMap<String, PathBuf>>) {
    for speaker in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let Some(name) = speaker.file_name().to_str().map(str::to_owned) else { continue };
        for file in std::fs::read_dir(speaker.path()).into_iter().flatten().flatten() {
            let file_name = file.file_name().to_string_lossy().into_owned();
            if let Some(emotion) = file_name.strip_suffix(VOICE_EXT) {
                out.entry(name.clone()).or_default().insert(emotion.to_owned(), file.path());
            }
        }
    }
}

fn voice_file(root: &Path, speaker: &str, emotion: &str) -> CmdResult<PathBuf> {
    Ok(root.join(safe_name(speaker)?).join(format!("{}{VOICE_EXT}", safe_name(emotion)?)))
}

fn voice_path(project: &Project, speaker: &str, emotion: &str) -> CmdResult<PathBuf> {
    voice_file(&project.paths().voices(), speaker, emotion)
}

/// The voice library for the Voice screen: speaker -> emotions with a file.
#[tauri::command]
pub fn voice_library(app: AppHandle, state: State<AppState>) -> CmdResult<BTreeMap<String, Vec<String>>> {
    let shared = shared_voices(&app).ok();
    with_project(&state, |p| {
        Ok(library(shared.as_deref(), p).into_iter().map(|(s, e)| (s, e.into_keys().collect())).collect())
    })
}

/// Copies a `.voice.gguf` file into the shared library as the given speaker's voice for one emotion, so every
/// project has it from now on.
#[tauri::command]
pub fn import_voice(app: AppHandle, speaker: String, emotion: String, path: String) -> CmdResult<()> {
    if !path.ends_with(VOICE_EXT) {
        return Err(format!("pick a {VOICE_EXT} file"));
    }
    let dest = voice_file(&shared_voices(&app)?, &speaker, &emotion)?;
    std::fs::create_dir_all(dest.parent().ok_or("bad path")?).map_err(msg)?;
    let staged = dest.with_extension("partial");
    std::fs::copy(&path, &staged).map_err(|e| format!("{path}: {e}"))?;
    std::fs::rename(&staged, &dest).map_err(msg)
}

/// A clip path from the worker is only ever written under the chapter's `audio/`.
fn safe_audio(rel: &str) -> CmdResult<PathBuf> {
    let path = Path::new(rel);
    let ok = path.components().all(|c| matches!(c, Component::Normal(_)))
        && path.components().next().is_some_and(|c| c.as_os_str() == "audio");
    if ok { Ok(path.to_path_buf()) } else { Err(format!("worker sent an unsafe path: {rel}")) }
}

/// Swaps the fetched clips in as the chapter's `audio/`, with the manifest and report, then marks it Voiced.
fn install(project: &Project, id: ChapterId, staged: &Path, manifest: &VoiceManifest) -> CmdResult<ChapterManifest> {
    let _: AudioManifest = serde_json::from_value(manifest.audio.clone())
        .map_err(|e| format!("audio manifest from the worker is invalid: {e}"))?;
    let dir = chapter::chapter_dir(project, id);
    let audio = staged.join("audio");
    std::fs::create_dir_all(&audio).map_err(msg)?;
    fsx::atomic_write_json(&audio.join("audio.manifest.json"), &manifest.audio).map_err(msg)?;
    fsx::atomic_write_json(&audio.join("voice.report.json"), &manifest.report).map_err(msg)?;
    let live = dir.join("audio");
    let old = dir.join(".audio.old");
    let _ = std::fs::remove_dir_all(&old);
    if live.exists() {
        std::fs::rename(&live, &old).map_err(msg)?;
    }
    std::fs::rename(&audio, &live).map_err(msg)?;
    let _ = std::fs::remove_dir_all(&old);
    let mut state = chapter::load_chapter(project, id).map_err(msg)?;
    if state.state == ChapterState::Approved {
        state = chapter::advance(project, id, ChapterState::Voiced).map_err(msg)?;
    }
    Ok(state)
}

/// Voices one chapter's approved script on the connected worker. Progress arrives as `voice://progress` events.
#[tauri::command]
pub async fn record_voice(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, BridgeState>,
    chapter: String,
) -> CmdResult<ChapterManifest> {
    let id: ChapterId = chapter.parse().map_err(msg)?;
    let _job = begin_job(&state, Lane::Worker, format!("{chapter} voice"))?;
    let shared = shared_voices(&app).ok();
    let (dir, script, voices, tempos) = with_project(&state, |p| {
        let manifest = chapter::load_chapter(p, id).map_err(msg)?;
        if !matches!(manifest.state, ChapterState::Approved | ChapterState::Voiced) {
            return Err(format!("{chapter} is {}; approve its script first", manifest.state.as_str()));
        }
        if manifest.stale {
            return Err(format!(
                "{chapter} is stale: the story before it changed after its script was written. Send it back to Directed, check the script against the new story and approve it again"
            ));
        }
        let dir = chapter::chapter_dir(p, id);
        let text = std::fs::read_to_string(dir.join("script").join("approved.json"))
            .map_err(|e| format!("approved.json: {e}"))?;
        let script: Value = serde_json::from_str(&text).map_err(|e| format!("approved.json: {e}"))?;
        Ok((dir, script, library(shared.as_deref(), p), tempos(shared.as_deref(), p)))
    })?;
    current_client(&bridge).await?;
    let report = |progress: f64, message: String| {
        let _ = app.emit("voice://progress", Progress { chapter: &chapter, progress, message });
    };
    let staged = dir.join(".voice.partial");
    let manifest = voice_job(&state, &bridge, &voices, &tempos, &dir, &script, &chapter, &staged, report).await;
    let done = manifest.and_then(|manifest| with_project(&state, |p| install(p, id, &staged, &manifest)));
    let _ = std::fs::remove_dir_all(&staged);
    report(1.0, "voiced".into());
    done
}

/// Runs the worker's voice job on `script` (a ScriptDoc for `chapter`) with every voice in the library (at its own
/// speed-up where `tempos` has one), and fetches
/// its clips into `staged/audio/`. Voice files the job created are kept in the project's library. `work` holds the
/// request and the manifest while they are passed on.
#[allow(clippy::too_many_arguments)]
async fn voice_job(
    state: &State<'_, AppState>,
    bridge: &State<'_, BridgeState>,
    voices: &BTreeMap<String, BTreeMap<String, PathBuf>>,
    tempos: &BTreeMap<String, f64>,
    work: &Path,
    script: &Value,
    chapter: &str,
    staged: &Path,
    report: impl Fn(f64, String),
) -> CmdResult<VoiceManifest> {
    report(0.0, "uploading voices".into());
    let mut specs = Map::new();
    for (speaker, emotions) in voices {
        let mut prompts = Map::new();
        for (emotion, path) in emotions {
            prompts.insert(emotion.clone(), Value::String(upload(bridge, path).await?));
        }
        let mut spec = json!({ "prompts": prompts });
        if let Some(tempo) = tempos.get(speaker) {
            spec["tempo"] = json!(tempo);
        }
        specs.insert(speaker.clone(), spec);
    }
    let request_path = work.join(".voice.request.json");
    fsx::atomic_write_json(&request_path, &json!({ "script": script, "voices": specs })).map_err(msg)?;
    let input = upload(bridge, &request_path).await;
    let _ = std::fs::remove_file(&request_path);
    let output = run_job(bridge, "voice", &input?, json!({}), |p, m| report(0.02 + p * 0.88, m)).await?;

    report(0.9, "downloading clips".into());
    let manifest_path = work.join(".voice.manifest.json");
    download(bridge, &output, &manifest_path).await?;
    let manifest: Result<VoiceManifest, String> =
        std::fs::read(&manifest_path).map_err(msg).and_then(|bytes| serde_json::from_slice(&bytes).map_err(msg));
    let _ = std::fs::remove_file(&manifest_path);
    let manifest = manifest?;
    if manifest.chapter_id != chapter {
        return Err(format!("worker answered for {} instead of {chapter}", manifest.chapter_id));
    }
    let _ = std::fs::remove_dir_all(staged);
    for (rel, hash) in &manifest.files {
        let dest = staged.join(safe_audio(rel)?);
        std::fs::create_dir_all(dest.parent().ok_or("bad path")?).map_err(msg)?;
        download(bridge, hash, &dest).await?;
    }
    for (speaker, emotions) in &manifest.voices {
        for (emotion, hash) in emotions {
            let dest = with_project(state, |p| voice_path(p, speaker, emotion))?;
            std::fs::create_dir_all(dest.parent().ok_or("bad path")?).map_err(msg)?;
            download(bridge, hash, &dest).await?;
        }
    }
    Ok(manifest)
}

/// Swaps the fetched clips in as the wrapper's `audio/` (ADR 0008), each clip's path moved from the chapter the
/// worker filed it under to the wrapper's folder, with the script they were voiced from. `rel` is the wrapper
/// folder's project-relative path.
fn install_wrapper(dir: &Path, rel: &str, staged: &Path, manifest: &VoiceManifest, script: &Value) -> CmdResult<Value> {
    let mut audio = manifest.audio.clone();
    for clip in audio["clips"].as_array_mut().into_iter().flatten() {
        let src = clip["src"].as_str().unwrap_or_default();
        let file = src
            .rsplit_once("/audio/")
            .map(|(_, f)| f)
            .ok_or_else(|| format!("worker sent a clip outside audio/: {src}"))?;
        clip["src"] = Value::String(format!("{rel}/audio/{file}"));
    }
    let _: AudioManifest =
        serde_json::from_value(audio.clone()).map_err(|e| format!("audio manifest from the worker is invalid: {e}"))?;
    let fresh = staged.join("audio");
    std::fs::create_dir_all(&fresh).map_err(msg)?;
    fsx::atomic_write_json(&fresh.join("audio.manifest.json"), &audio).map_err(msg)?;
    fsx::atomic_write_json(&fresh.join("voice.report.json"), &manifest.report).map_err(msg)?;
    let live = dir.join("audio");
    let old = dir.join(".audio.old");
    let _ = std::fs::remove_dir_all(&old);
    if live.exists() {
        std::fs::rename(&live, &old).map_err(msg)?;
    }
    std::fs::rename(&fresh, &live).map_err(msg)?;
    let _ = std::fs::remove_dir_all(&old);
    fsx::atomic_write_json(&dir.join("script.json"), script).map_err(msg)?;
    Ok(json!({ "script": script, "audio": audio }))
}

/// Voices an episode's wrapper (ADR 0008): `script` is its cold open, bridge, recap and tease as one ScriptDoc, built
/// by the app from the episode's plan. Spoken by the same voices and checks as a chapter; progress arrives as
/// `voice://progress` events under the episode's id.
#[tauri::command]
pub async fn record_wrapper(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, BridgeState>,
    episode: String,
    script: Value,
) -> CmdResult<Value> {
    let _job = begin_job(&state, Lane::Worker, format!("{episode} wrapper voice"))?;
    let chapter = script["chapter_id"].as_str().ok_or("the wrapper's script has no chapter_id")?.to_owned();
    let shared = shared_voices(&app).ok();
    let (root, dir, voices, tempos) = with_project(&state, |p| {
        crate::assembly::load_episode(p, &episode)?;
        let dir = crate::assembly::wrapper_dir(p, &episode)?;
        Ok((p.paths().root().to_path_buf(), dir, library(shared.as_deref(), p), tempos(shared.as_deref(), p)))
    })?;
    std::fs::create_dir_all(&dir).map_err(msg)?;
    let rel = dir.strip_prefix(&root).map_err(msg)?.to_string_lossy().replace('\\', "/");
    current_client(&bridge).await?;
    let report = |progress: f64, message: String| {
        let _ = app.emit("voice://progress", Progress { chapter: &episode, progress, message });
    };
    let staged = dir.join(".voice.partial");
    let manifest = voice_job(&state, &bridge, &voices, &tempos, &dir, &script, &chapter, &staged, report).await;
    let done = manifest.and_then(|manifest| install_wrapper(&dir, &rel, &staged, &manifest, &script));
    let _ = std::fs::remove_dir_all(&staged);
    report(1.0, "voiced".into());
    done
}

/// The wrapper an episode was last voiced with: its script and audio manifest, or nulls before the first time.
#[tauri::command]
pub fn episode_wrapper(state: State<AppState>, episode: String) -> CmdResult<Value> {
    with_project(&state, |p| {
        let dir = crate::assembly::wrapper_dir(p, &episode)?;
        let read = |path: PathBuf| -> CmdResult<Value> {
            match std::fs::read_to_string(&path) {
                Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
                Err(e) => Err(msg(e)),
            }
        };
        Ok(
            json!({ "script": read(dir.join("script.json"))?, "audio": read(dir.join("audio").join("audio.manifest.json"))? }),
        )
    })
}

/// The chapter's clips and the voice report, for the Voice screen.
#[tauri::command]
pub fn chapter_audio(state: State<AppState>, chapter: String) -> CmdResult<Value> {
    with_project(&state, |p| {
        let id: ChapterId = chapter.parse().map_err(msg)?;
        let dir = chapter::chapter_dir(p, id).join("audio");
        let read = |name: &str| -> CmdResult<Value> {
            match std::fs::read_to_string(dir.join(name)) {
                Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{name}: {e}")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
                Err(e) => Err(msg(e)),
            }
        };
        Ok(json!({ "audio": read("audio.manifest.json")?, "report": read("voice.report.json")? }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_approved_chapter() -> (tempfile::TempDir, Project, ChapterId) {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path().join("p"), "Test").expect("project");
        let pdf = dir.path().join("c.pdf");
        std::fs::write(&pdf, b"%PDF-1.4\n").expect("pdf");
        let id = chapter::import_chapter(&project, &pdf, None).expect("import").id;
        for to in [
            ChapterState::Rasterized,
            ChapterState::Analyzed,
            ChapterState::Directed,
            ChapterState::Librarian,
            ChapterState::Approved,
        ] {
            chapter::advance(&project, id, to).expect("advance");
        }
        (dir, project, id)
    }

    fn manifest(id: ChapterId) -> VoiceManifest {
        let mut audio: Value =
            serde_json::from_str(include_str!("../../../../packages/schema/fixtures/audio-manifest.sample.json"))
                .expect("fixture");
        audio["chapter_id"] = json!(id.to_string());
        VoiceManifest {
            chapter_id: id.to_string(),
            audio,
            files: BTreeMap::new(),
            voices: BTreeMap::new(),
            report: json!({}),
        }
    }

    #[test]
    fn the_library_lists_voice_files_by_speaker_and_emotion() {
        let (_tmp, project, _) = project_with_approved_chapter();
        for (speaker, emotion) in [("narrator", "neutral"), ("narrator", "sad"), ("akira", "neutral")] {
            let path = voice_path(&project, speaker, emotion).expect("path");
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
            std::fs::write(path, b"gguf").expect("voice");
        }
        std::fs::write(project.paths().voices().join("narrator").join("notes.txt"), b"x").expect("other file");
        let lib = library(None, &project);
        assert_eq!(lib["narrator"].keys().collect::<Vec<_>>(), ["neutral", "sad"]);
        assert_eq!(lib["akira"].keys().collect::<Vec<_>>(), ["neutral"]);
        assert!(voice_path(&project, "../x", "neutral").is_err() && voice_path(&project, "a", "b/c").is_err());
    }

    #[test]
    fn shared_voices_reach_every_project_and_the_project_wins() {
        let (tmp, project, _) = project_with_approved_chapter();
        let shared = tmp.path().join("shared-voices");
        for (root, speaker, emotion, body) in [
            (shared.clone(), "narrator", "neutral", b"shared".as_slice()),
            (shared.clone(), "narrator", "sad", b"shared"),
            (project.paths().voices(), "narrator", "neutral", b"project"),
        ] {
            let path = voice_file(&root, speaker, emotion).expect("path");
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
            std::fs::write(path, body).expect("voice");
        }
        let lib = library(Some(&shared), &project);
        assert_eq!(std::fs::read(&lib["narrator"]["neutral"]).expect("read"), b"project");
        assert_eq!(std::fs::read(&lib["narrator"]["sad"]).expect("read"), b"shared");
    }

    #[test]
    fn a_speakers_voice_json_sets_its_speed_up_and_the_project_wins() {
        let (tmp, project, _) = project_with_approved_chapter();
        let shared = tmp.path().join("shared-voices");
        for (root, speaker, body) in [
            (shared.clone(), "narrator", r#"{"tempo": 1.25}"#),
            (shared.clone(), "akira", r#"{"tempo": 1.2}"#),
            (project.paths().voices(), "akira", r#"{"tempo": 1.3}"#),
            (shared.clone(), "kenji", r#"{"tempo": 9}"#),
        ] {
            std::fs::create_dir_all(root.join(speaker)).expect("dir");
            std::fs::write(root.join(speaker).join(VOICE_SETTINGS), body).expect("settings");
        }
        // An out-of-range speed-up is ignored, so the job's default applies.
        assert_eq!(tempos(Some(&shared), &project), BTreeMap::from([("akira".into(), 1.3), ("narrator".into(), 1.25)]));
        assert!(!library(Some(&shared), &project).contains_key("kenji"));
    }

    #[test]
    fn installing_replaces_the_audio_folder_and_marks_the_chapter_voiced() {
        let (tmp, project, id) = project_with_approved_chapter();
        let dir = chapter::chapter_dir(&project, id);
        std::fs::create_dir_all(dir.join("audio")).expect("old audio");
        std::fs::write(dir.join("audio").join("stale.wav"), b"old").expect("stale");
        let staged = tmp.path().join("staged");
        std::fs::create_dir_all(staged.join("audio")).expect("staged");
        std::fs::write(staged.join("audio").join("l_0001.wav"), b"new").expect("clip");
        let state = install(&project, id, &staged, &manifest(id)).expect("install");
        assert_eq!(state.state, ChapterState::Voiced);
        assert_eq!(std::fs::read(dir.join("audio").join("l_0001.wav")).expect("clip"), b"new");
        assert!(!dir.join("audio").join("stale.wav").exists());
        assert!(
            dir.join("audio").join("audio.manifest.json").is_file()
                && dir.join("audio").join("voice.report.json").is_file()
        );
    }

    #[test]
    fn the_wrapper_keeps_its_clips_and_script_in_the_episode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (wrapper, staged) = (dir.path().join("wrapper"), dir.path().join("staged"));
        std::fs::create_dir_all(wrapper.join("audio")).expect("old audio");
        std::fs::write(wrapper.join("audio").join("old.wav"), b"old").expect("old clip");
        let id = ChapterId::new(1).expect("id");
        let script = json!({"chapter_id": "ch_0001", "lines": []});
        let saved =
            install_wrapper(&wrapper, "episodes/ep_0001/wrapper", &staged, &manifest(id), &script).expect("install");
        assert_eq!(saved["audio"]["clips"][0]["src"], "episodes/ep_0001/wrapper/audio/l_0001.wav");
        assert_eq!(saved["script"], script);
        assert!(!wrapper.join("audio").join("old.wav").exists());
        let on_disk: Value = serde_json::from_str(
            &std::fs::read_to_string(wrapper.join("audio/audio.manifest.json")).expect("manifest"),
        )
        .expect("json");
        assert_eq!(on_disk, saved["audio"]);
        assert!(wrapper.join("script.json").is_file());
    }

    #[test]
    fn clip_paths_stay_inside_audio() {
        for bad in ["../x.wav", "script/approved.json", "/etc/passwd", "audio/../../x"] {
            assert!(safe_audio(bad).is_err(), "{bad}");
        }
        assert!(safe_audio("audio/l_0001.wav").is_ok());
    }
}

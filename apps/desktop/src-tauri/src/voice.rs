//! The voice stage on the app side: send a chapter's approved script and the project's voice files to the
//! worker's `voice` job, install the clips under `audio/`, and keep any voice it created in the library.
//!
//! The library is `voices/<speaker>/<emotion>.voice.gguf` in the project, so a voice sounds the same in every
//! chapter. A speaker without a file is invented by the worker from its description on the first line, and
//! the take it clones comes back here.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use jumbo_contracts::audio_manifest::AudioManifest;
use jumbo_core::chapter;
use jumbo_core::{ChapterId, ChapterManifest, ChapterState, Project, fsx};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Emitter, State};

use crate::bridge::{BridgeState, current_client};
use crate::commands::{AppState, CmdResult, msg, with_project};
use crate::script::run_job;

const VOICE_EXT: &str = ".voice.gguf";

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

/// Every voice file in the library: speaker -> emotion -> path.
fn library(project: &Project) -> BTreeMap<String, BTreeMap<String, PathBuf>> {
    let mut out: BTreeMap<String, BTreeMap<String, PathBuf>> = BTreeMap::new();
    for speaker in std::fs::read_dir(project.paths().voices()).into_iter().flatten().flatten() {
        let Some(name) = speaker.file_name().to_str().map(str::to_owned) else { continue };
        for file in std::fs::read_dir(speaker.path()).into_iter().flatten().flatten() {
            let file_name = file.file_name().to_string_lossy().into_owned();
            if let Some(emotion) = file_name.strip_suffix(VOICE_EXT) {
                out.entry(name.clone()).or_default().insert(emotion.to_owned(), file.path());
            }
        }
    }
    out
}

fn voice_path(project: &Project, speaker: &str, emotion: &str) -> CmdResult<PathBuf> {
    Ok(project.paths().voices().join(safe_name(speaker)?).join(format!("{}{VOICE_EXT}", safe_name(emotion)?)))
}

/// The voice library for the Voice screen: speaker -> emotions with a file.
#[tauri::command]
pub fn voice_library(state: State<AppState>) -> CmdResult<BTreeMap<String, Vec<String>>> {
    with_project(&state, |p| Ok(library(p).into_iter().map(|(s, e)| (s, e.into_keys().collect())).collect()))
}

/// Copies a `.voice.gguf` file into the library as the given speaker's voice for one emotion.
#[tauri::command]
pub fn import_voice(state: State<AppState>, speaker: String, emotion: String, path: String) -> CmdResult<()> {
    with_project(&state, |p| {
        if !path.ends_with(VOICE_EXT) {
            return Err(format!("pick a {VOICE_EXT} file"));
        }
        let dest = voice_path(p, &speaker, &emotion)?;
        std::fs::create_dir_all(dest.parent().ok_or("bad path")?).map_err(msg)?;
        let staged = dest.with_extension("partial");
        std::fs::copy(&path, &staged).map_err(|e| format!("{path}: {e}"))?;
        std::fs::rename(&staged, &dest).map_err(msg)
    })
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
    let (dir, script, voices) = with_project(&state, |p| {
        let manifest = chapter::load_chapter(p, id).map_err(msg)?;
        if !matches!(manifest.state, ChapterState::Approved | ChapterState::Voiced) {
            return Err(format!("{chapter} is {}; approve its script first", manifest.state.as_str()));
        }
        let dir = chapter::chapter_dir(p, id);
        let text = std::fs::read_to_string(dir.join("script").join("approved.json"))
            .map_err(|e| format!("approved.json: {e}"))?;
        let script: Value = serde_json::from_str(&text).map_err(|e| format!("approved.json: {e}"))?;
        Ok((dir, script, library(p)))
    })?;
    let client = current_client(&bridge).await?;
    let report = |progress: f64, message: String| {
        let _ = app.emit("voice://progress", Progress { chapter: &chapter, progress, message });
    };

    report(0.0, "uploading voices".into());
    let mut specs = Map::new();
    for (speaker, emotions) in &voices {
        let mut prompts = Map::new();
        for (emotion, path) in emotions {
            prompts.insert(emotion.clone(), Value::String(client.upload_file(path, |_, _| {}).await.map_err(msg)?));
        }
        specs.insert(speaker.clone(), json!({ "prompts": prompts }));
    }
    let request_path = dir.join(".voice.request.json");
    fsx::atomic_write_json(&request_path, &json!({ "script": script, "voices": specs })).map_err(msg)?;
    let input = client.upload_file(&request_path, |_, _| {}).await.map_err(msg);
    let _ = std::fs::remove_file(&request_path);
    let output = run_job(&client, "voice", &input?, json!({}), |p, m| report(0.02 + p * 0.88, m)).await?;

    report(0.9, "downloading clips".into());
    let manifest_path = dir.join(".voice.manifest.json");
    client.download(&output, &manifest_path).await.map_err(msg)?;
    let manifest: Result<VoiceManifest, String> =
        std::fs::read(&manifest_path).map_err(msg).and_then(|bytes| serde_json::from_slice(&bytes).map_err(msg));
    let _ = std::fs::remove_file(&manifest_path);
    let manifest = manifest?;
    if manifest.chapter_id != chapter {
        return Err(format!("worker answered for {} instead of {chapter}", manifest.chapter_id));
    }
    let staged = dir.join(".voice.partial");
    let _ = std::fs::remove_dir_all(&staged);
    for (rel, hash) in &manifest.files {
        let dest = staged.join(safe_audio(rel)?);
        std::fs::create_dir_all(dest.parent().ok_or("bad path")?).map_err(msg)?;
        client.download(hash, &dest).await.map_err(msg)?;
    }
    for (speaker, emotions) in &manifest.voices {
        for (emotion, hash) in emotions {
            let dest = with_project(&state, |p| voice_path(p, speaker, emotion))?;
            std::fs::create_dir_all(dest.parent().ok_or("bad path")?).map_err(msg)?;
            client.download(hash, &dest).await.map_err(msg)?;
        }
    }
    let done = with_project(&state, |p| install(p, id, &staged, &manifest));
    let _ = std::fs::remove_dir_all(&staged);
    report(1.0, "voiced".into());
    done
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
        let lib = library(&project);
        assert_eq!(lib["narrator"].keys().collect::<Vec<_>>(), ["neutral", "sad"]);
        assert_eq!(lib["akira"].keys().collect::<Vec<_>>(), ["neutral"]);
        assert!(voice_path(&project, "../x", "neutral").is_err() && voice_path(&project, "a", "b/c").is_err());
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
    fn clip_paths_stay_inside_audio() {
        for bad in ["../x.wav", "script/approved.json", "/etc/passwd", "audio/../../x"] {
            assert!(safe_audio(bad).is_err(), "{bad}");
        }
        assert!(safe_audio("audio/l_0001.wav").is_ok());
    }
}

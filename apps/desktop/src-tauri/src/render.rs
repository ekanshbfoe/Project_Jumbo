//! Rendering a voiced chapter on Kaggle CPU machines (ADR 0005): the app uploads the render job, runs the chunk
//! machines in parallel, pushes again any machine that was handed the previous job, then runs the join machine
//! and downloads `render/<chapter>.mp4`. The episode itself is built in the app (buildEpisodeSpec) and saved first.
//! A chapter rendered as a part of an episode (ADR 0007) keeps its soundtrack too, for the episode's assembly.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jumbo_bridge::kaggle::{DatasetFile, KaggleClient, KernelPush};
use jumbo_bridge::render::{self, CHUNKS};
use jumbo_contracts::episode_spec::EpisodeSpec;
use jumbo_core::chapter;
use jumbo_core::{ChapterId, ChapterManifest, ChapterState, fsx};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::assembly;
use crate::bridge::kaggle_client;
use crate::commands::{AppState, CmdResult, Lane, begin_job, msg, with_project};

const POLL: Duration = Duration::from_secs(15);
/// Times a machine handed the previous job is pushed again before the render gives up.
const STALE_RETRIES: u32 = 12;
/// Seconds to wait for Kaggle to process the job dataset.
const DATASET_WAIT_S: u64 = 20 * 60;
/// How long one Kaggle call keeps retrying a transient failure (a dropped connection, a 5xx, rate limiting)
/// before the render gives up. A 20 to 40 minute render makes many calls; one hiccup must not end it.
const KAGGLE_GRACE: Duration = Duration::from_secs(300);

/// Runs a Kaggle call, retrying transient failures for up to `KAGGLE_GRACE`.
async fn kaggle<T, F, Fut>(mut call: F) -> CmdResult<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = jumbo_bridge::Result<T>>,
{
    let deadline = std::time::Instant::now() + KAGGLE_GRACE;
    let mut wait = Duration::from_secs(5);
    loop {
        match call().await {
            Ok(value) => return Ok(value),
            Err(e) if e.is_transient() && std::time::Instant::now() + wait < deadline => {
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(60));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[derive(Clone, Serialize)]
struct Progress<'a> {
    chapter: &'a str,
    progress: f64,
    message: String,
    /// Each machine's Kaggle status, chunk machines first, then the join machine.
    machines: Vec<String>,
}

/// Every project file the episode plays: each panel's image and backdrop, and each beat's narration.
pub(crate) fn assets(spec: &Value) -> Vec<String> {
    let mut out = BTreeSet::new();
    for panel in spec["panels"].as_array().into_iter().flatten() {
        let image = panel["clean"].as_str().or(panel["crop"].as_str()).or(panel["page"].as_str());
        out.extend(image.map(str::to_owned));
        out.extend(panel["backdrop"].as_str().map(str::to_owned));
    }
    for beat in spec["beats"].as_array().into_iter().flatten() {
        out.extend(beat["audio"]["src"].as_str().map(str::to_owned));
    }
    out.into_iter().collect()
}

/// Saves the episode the Render screen built as `render/episode.json`.
#[tauri::command]
pub fn save_episode(state: State<AppState>, chapter: String, episode: Value) -> CmdResult<()> {
    with_project(&state, |p| {
        let id: ChapterId = chapter.parse().map_err(msg)?;
        let manifest = chapter::load_chapter(p, id).map_err(msg)?;
        if manifest.state < ChapterState::Voiced {
            return Err(format!("{chapter} is {}; voice it before building the episode", manifest.state.as_str()));
        }
        let _: EpisodeSpec =
            serde_json::from_value(episode.clone()).map_err(|e| format!("the episode is invalid: {e}"))?;
        fsx::atomic_write_json(&chapter::chapter_dir(p, id).join("render").join("episode.json"), &episode).map_err(msg)
    })
}

/// Pushes `kernel` and waits until it finishes. A run handed the previous job is pushed again after a minute.
async fn run_machine(client: &KaggleClient, kernel: &KernelPush, mut on_status: impl FnMut(&str)) -> CmdResult<()> {
    let mut blind_retry = false;
    for _ in 0..=STALE_RETRIES {
        kaggle(|| client.push(kernel)).await?;
        tokio::time::sleep(POLL).await;
        let status = loop {
            let status = kaggle(|| client.status(&kernel.slug)).await?;
            on_status(&status.status);
            if status.is_finished() {
                break status;
            }
            tokio::time::sleep(POLL).await;
        };
        if status.status == "COMPLETE" {
            return Ok(());
        }
        let log = match kaggle(|| client.output_log(&kernel.slug)).await {
            Ok(log) => log,
            // Without its log a stale job cannot be told from a real failure: run the machine once more.
            Err(_) if !blind_retry => {
                blind_retry = true;
                on_status("LOG_UNAVAILABLE");
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }
            Err(e) => {
                return Err(format!("{} ended {} and its log could not be read: {e}", kernel.slug, status.status));
            }
        };
        if !log.contains("stale render job") {
            let tail = jumbo_bridge::kaggle::log_tail(&log, 15);
            return Err(format!(
                "{} ended {}: {}\n{tail}",
                kernel.slug,
                status.status,
                status.failure_message.unwrap_or_default()
            ));
        }
        on_status("STALE_JOB");
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
    Err(format!("{} kept getting the previous render job from Kaggle", kernel.slug))
}

/// Renders `episode` (an EpisodeSpec's JSON) on Kaggle CPU machines and downloads `<name>.mp4` into `dest`, with
/// `<name>.wav` (the soundtrack) when `soundtrack` is set. Progress arrives as `render://progress` events.
async fn render_spec(
    app: &AppHandle,
    root: &Path,
    chapter: &str,
    episode: &[u8],
    name: &str,
    dest: &Path,
    soundtrack: bool,
) -> CmdResult<()> {
    let spec: Value = serde_json::from_slice(episode).map_err(|e| format!("the episode is invalid: {e}"))?;
    let (client, owner) = kaggle_client(app)?;
    let machines = std::sync::Mutex::new(vec!["WAITING".to_owned(); CHUNKS as usize + 1]);
    let report = |progress: f64, message: String| {
        let machines = machines.lock().map(|m| m.clone()).unwrap_or_default();
        let _ = app.emit("render://progress", Progress { chapter, progress, message, machines });
    };

    let stamp = format!("{chapter}-{}", time::OffsetDateTime::now_utc().unix_timestamp());
    report(0.0, "packing the render job".into());
    let files = assets(&spec);
    let archive = render::job_archive(root, episode, &files, &stamp).map_err(msg)?;
    report(0.02, format!("uploading the render job ({:.0} MB)", archive.len() as f64 / 1e6));
    let dataset = render::job_dataset(&owner);
    let job_files = [DatasetFile { name: render::JOB_FILE.into(), bytes: archive }];
    kaggle(|| client.put_private_dataset(&dataset, "Jumbo render job", &job_files)).await?;
    let waited = std::time::Instant::now();
    // Right after an upload Kaggle can still report the previous version as ready, so give it a moment first.
    report(0.06, "Kaggle is processing the render job".into());
    tokio::time::sleep(Duration::from_secs(60)).await;
    while !kaggle(|| client.dataset_status(&dataset)).await?.is_some_and(|s| s.is_ready()) {
        if waited.elapsed().as_secs() > DATASET_WAIT_S {
            return Err(format!("Kaggle did not finish processing {dataset}"));
        }
        report(0.08, "Kaggle is processing the render job".into());
        tokio::time::sleep(Duration::from_secs(10)).await;
    }

    report(0.1, format!("rendering on {CHUNKS} machines"));
    let done = std::sync::atomic::AtomicU32::new(0);
    let chunks = (1..=CHUNKS).map(|i| {
        let kernel = render::chunk_kernel(&owner, i, CHUNKS, &stamp);
        let (client, machines, done, report) = (&client, &machines, &done, &report);
        async move {
            let result = run_machine(client, &kernel, |s| {
                if let Ok(mut m) = machines.lock() {
                    m[i as usize - 1] = s.to_owned();
                }
                let n = done.load(std::sync::atomic::Ordering::Relaxed);
                report(0.1 + 0.7 * f64::from(n) / f64::from(CHUNKS), format!("{n}/{CHUNKS} chunks rendered"));
            })
            .await;
            done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            result
        }
    });
    for result in futures_util::future::join_all(chunks).await {
        result?;
    }

    report(0.82, "joining the chunks with the soundtrack".into());
    let join = render::join_kernel(&owner, CHUNKS, &stamp, name);
    run_machine(&client, &join, |s| {
        if let Ok(mut m) = machines.lock() {
            m[CHUNKS as usize] = s.to_owned();
        }
        report(0.85, "joining the chunks with the soundtrack".into());
    })
    .await?;

    report(0.95, "downloading the video".into());
    let outputs = kaggle(|| client.output_files(&join.slug)).await?;
    std::fs::create_dir_all(dest).map_err(msg)?;
    let wanted: &[&str] = if soundtrack { &["mp4", "wav"] } else { &["mp4"] };
    for ext in wanted {
        let file_name = format!("{name}.{ext}");
        let file = outputs
            .iter()
            .find(|f| f.file_name == file_name)
            .ok_or_else(|| format!("{} wrote no {file_name}", join.slug))?;
        let out = dest.join(&file_name);
        kaggle(|| client.download_output(file, &out)).await?;
    }
    report(1.0, format!("render/{name}.mp4"));
    Ok(())
}

/// Moves a voiced chapter on to Rendered once a video of it exists.
fn mark_rendered(state: &State<'_, AppState>, id: ChapterId) -> CmdResult<ChapterManifest> {
    with_project(state, |p| {
        let manifest = chapter::load_chapter(p, id).map_err(msg)?;
        if manifest.state == ChapterState::Voiced {
            chapter::advance(p, id, ChapterState::Rendered).map_err(msg)
        } else {
            Ok(manifest)
        }
    })
}

/// The project root and chapter folder of a chapter that is voiced (or rendered already).
fn voiced_chapter(state: &State<'_, AppState>, id: ChapterId) -> CmdResult<(PathBuf, PathBuf)> {
    with_project(state, |p| {
        let manifest = chapter::load_chapter(p, id).map_err(msg)?;
        if !matches!(manifest.state, ChapterState::Voiced | ChapterState::Rendered) {
            return Err(format!("{id} is {}; voice it first", manifest.state.as_str()));
        }
        Ok((p.paths().root().to_path_buf(), chapter::chapter_dir(p, id)))
    })
}

/// Renders a voiced chapter's saved episode on Kaggle CPU machines. Progress arrives as `render://progress` events.
#[tauri::command]
pub async fn render_chapter(app: AppHandle, state: State<'_, AppState>, chapter: String) -> CmdResult<ChapterManifest> {
    let id: ChapterId = chapter.parse().map_err(msg)?;
    let _job = begin_job(&state, Lane::Render, format!("{chapter} render"))?;
    let (root, dir) = voiced_chapter(&state, id)?;
    let episode =
        std::fs::read(dir.join("render").join("episode.json")).map_err(|e| format!("render/episode.json: {e}"))?;
    render_spec(&app, &root, &chapter, &episode, &chapter, &dir.join("render"), false).await?;
    mark_rendered(&state, id)
}

/// Renders a voiced chapter as its part of episode `episode` (ADR 0007): `spec` is the part's EpisodeSpec, built by
/// the app with the chapter's opening card and the part's follow-card settings. The part's video and soundtrack are
/// kept as `render/part.mp4` and `render/part.wav`, with the fingerprint that tells later whether they are current.
#[tauri::command]
pub async fn render_part(
    app: AppHandle,
    state: State<'_, AppState>,
    episode: String,
    chapter: String,
    spec: Value,
) -> CmdResult<ChapterManifest> {
    let id: ChapterId = chapter.parse().map_err(msg)?;
    let _job = begin_job(&state, Lane::Render, format!("{chapter} render (part of {episode})"))?;
    let (root, dir) = voiced_chapter(&state, id)?;
    with_project(&state, |p| assembly::check_member(p, &episode, id))?;
    let _: EpisodeSpec = serde_json::from_value(spec.clone()).map_err(|e| format!("the part is invalid: {e}"))?;
    let bytes = serde_json::to_vec(&spec).map_err(msg)?;
    let fingerprint = assembly::fingerprint(&root, &spec)?;
    let render = dir.join("render");
    let name = format!("{chapter}-part");
    let staging = render.join(".part");
    render_spec(&app, &root, &chapter, &bytes, &name, &staging, true).await?;
    for ext in ["mp4", "wav"] {
        std::fs::rename(staging.join(format!("{name}.{ext}")), render.join(format!("{}.{ext}", assembly::PART)))
            .map_err(msg)?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    assembly::write_part(&render, &episode, &fingerprint, &spec)?;
    mark_rendered(&state, id)
}

/// The rendered video's path (project-relative) and the saved episode, for the Render screen.
#[tauri::command]
pub fn chapter_render(state: State<AppState>, chapter: String) -> CmdResult<Value> {
    with_project(&state, |p| {
        let id: ChapterId = chapter.parse().map_err(msg)?;
        let dir = chapter::chapter_dir(p, id).join("render");
        let video = dir.join(format!("{chapter}.mp4"));
        Ok(serde_json::json!({
            "episode": dir.join("episode.json").is_file(),
            "video": video.is_file().then(|| format!("chapters/{chapter}/render/{chapter}.mp4")),
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_job_carries_each_panel_image_backdrop_and_clip_once() {
        let spec = json!({
            "panels": [
                {"page": "chapters/ch_0001/pages/1.webp", "crop": "chapters/ch_0001/panels/crops/p_0001.webp", "backdrop": "chapters/ch_0001/panels/backdrops/p_0001.jpg"},
                {"page": "chapters/ch_0001/pages/1.webp"},
            ],
            "beats": [{"audio": {"src": "chapters/ch_0001/audio/l_0001.wav"}}, {"audio": null}, {"audio": {"src": "chapters/ch_0001/audio/l_0001.wav"}}],
        });
        assert_eq!(
            assets(&spec),
            [
                "chapters/ch_0001/audio/l_0001.wav",
                "chapters/ch_0001/pages/1.webp",
                "chapters/ch_0001/panels/backdrops/p_0001.jpg",
                "chapters/ch_0001/panels/crops/p_0001.webp",
            ]
        );
    }
}

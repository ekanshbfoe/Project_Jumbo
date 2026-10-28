//! Rendering a voiced chapter on Kaggle CPU machines (ADR 0005): the app uploads the render job, runs the chunk
//! machines in parallel, pushes again any machine that was handed the previous job, then runs the join machine
//! and downloads `render/<chapter>.mp4`. The episode itself is built in the app (buildEpisodeSpec) and saved first.

use std::collections::BTreeSet;
use std::time::Duration;

use jumbo_bridge::kaggle::{DatasetFile, KaggleClient, KernelPush};
use jumbo_bridge::render::{self, CHUNKS};
use jumbo_contracts::episode_spec::EpisodeSpec;
use jumbo_core::chapter;
use jumbo_core::{ChapterId, ChapterManifest, ChapterState, fsx};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::bridge::kaggle_client;
use crate::commands::{AppState, CmdResult, msg, with_project};

const POLL: Duration = Duration::from_secs(15);
/// Times a machine handed the previous job is pushed again before the render gives up.
const STALE_RETRIES: u32 = 12;
/// Seconds to wait for Kaggle to process the job dataset.
const DATASET_WAIT_S: u64 = 20 * 60;

#[derive(Clone, Serialize)]
struct Progress<'a> {
    chapter: &'a str,
    progress: f64,
    message: String,
    /// Each machine's Kaggle status, chunk machines first, then the join machine.
    machines: Vec<String>,
}

/// Every project file the episode plays: each panel's image and backdrop, and each beat's narration.
fn assets(spec: &Value) -> Vec<String> {
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
    for _ in 0..=STALE_RETRIES {
        client.push(kernel).await.map_err(msg)?;
        tokio::time::sleep(POLL).await;
        let status = loop {
            let status = client.status(&kernel.slug).await.map_err(msg)?;
            on_status(&status.status);
            if status.is_finished() {
                break status;
            }
            tokio::time::sleep(POLL).await;
        };
        if status.status == "COMPLETE" {
            return Ok(());
        }
        let log = client.output_log(&kernel.slug).await.unwrap_or_default();
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

/// Renders a voiced chapter's saved episode on Kaggle CPU machines. Progress arrives as `render://progress` events.
#[tauri::command]
pub async fn render_chapter(app: AppHandle, state: State<'_, AppState>, chapter: String) -> CmdResult<ChapterManifest> {
    let id: ChapterId = chapter.parse().map_err(msg)?;
    let (root, dir, episode) = with_project(&state, |p| {
        let manifest = chapter::load_chapter(p, id).map_err(msg)?;
        if !matches!(manifest.state, ChapterState::Voiced | ChapterState::Rendered) {
            return Err(format!("{chapter} is {}; voice it first", manifest.state.as_str()));
        }
        let dir = chapter::chapter_dir(p, id);
        let episode =
            std::fs::read(dir.join("render").join("episode.json")).map_err(|e| format!("render/episode.json: {e}"))?;
        Ok((p.paths().root().to_path_buf(), dir, episode))
    })?;
    let spec: Value = serde_json::from_slice(&episode).map_err(|e| format!("render/episode.json: {e}"))?;
    let (client, owner) = kaggle_client(&app)?;
    let machines = std::sync::Mutex::new(vec!["WAITING".to_owned(); CHUNKS as usize + 1]);
    let report = |progress: f64, message: String| {
        let machines = machines.lock().map(|m| m.clone()).unwrap_or_default();
        let _ = app.emit("render://progress", Progress { chapter: &chapter, progress, message, machines });
    };

    let stamp = format!("{chapter}-{}", time::OffsetDateTime::now_utc().unix_timestamp());
    report(0.0, "packing the render job".into());
    let files = assets(&spec);
    let archive = render::job_archive(&root, &episode, &files, &stamp).map_err(msg)?;
    report(0.02, format!("uploading the render job ({:.0} MB)", archive.len() as f64 / 1e6));
    let dataset = render::job_dataset(&owner);
    client
        .put_private_dataset(
            &dataset,
            "Jumbo render job",
            &[DatasetFile { name: render::JOB_FILE.into(), bytes: archive }],
        )
        .await
        .map_err(msg)?;
    let waited = std::time::Instant::now();
    // Right after an upload Kaggle can still report the previous version as ready, so give it a moment first.
    report(0.06, "Kaggle is processing the render job".into());
    tokio::time::sleep(Duration::from_secs(60)).await;
    while !client.dataset_status(&dataset).await.map_err(msg)?.is_some_and(|s| s.is_ready()) {
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
    let join = render::join_kernel(&owner, CHUNKS, &stamp, &chapter);
    run_machine(&client, &join, |s| {
        if let Ok(mut m) = machines.lock() {
            m[CHUNKS as usize] = s.to_owned();
        }
        report(0.85, "joining the chunks with the soundtrack".into());
    })
    .await?;

    report(0.95, "downloading the video".into());
    let name = format!("{chapter}.mp4");
    let files = client.output_files(&join.slug).await.map_err(msg)?;
    let video = files.iter().find(|f| f.file_name == name).ok_or_else(|| format!("{} wrote no {name}", join.slug))?;
    let out = dir.join("render").join(&name);
    std::fs::create_dir_all(out.parent().ok_or("bad path")?).map_err(msg)?;
    client.download_output(video, &out).await.map_err(msg)?;
    let manifest = with_project(&state, |p| {
        let manifest = chapter::load_chapter(p, id).map_err(msg)?;
        if manifest.state == ChapterState::Voiced {
            chapter::advance(p, id, ChapterState::Rendered).map_err(msg)
        } else {
            Ok(manifest)
        }
    })?;
    report(1.0, format!("render/{name}"));
    Ok(manifest)
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

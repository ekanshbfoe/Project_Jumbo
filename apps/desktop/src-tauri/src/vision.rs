//! Step 5 on the app side: send a chapter's PDF to the worker's `vision` job and install the result
//! (pages, crops, panels.json and the stage report) into the chapter folder, then advance the
//! chapter to Analyzed. The worker only proposes files; the app writes the project.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Component, Path, PathBuf};

use jumbo_contracts::panels_doc::PanelsDoc;
use jumbo_core::chapter::{self, Layout};
use jumbo_core::{ChapterId, ChapterManifest, ChapterState, fsx};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};

use crate::bridge::{BridgeState, current_client, download, upload};
use crate::script::run_job;
use crate::commands::{AppState, CmdResult, msg, with_project};

/// What the worker's vision job returns (worker/jumbo_worker/pipeline/vision/stage.py).
#[derive(Debug, Deserialize)]
pub struct VisionManifest {
    pub chapter_id: String,
    pub panels: Value,
    /// Chapter-relative path -> blob hash.
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub report: Value,
}

#[derive(Clone, Serialize)]
struct Progress<'a> {
    chapter: &'a str,
    progress: f64,
    message: String,
}

/// Folders the vision stage owns inside a chapter.
const OWNED: [&str; 2] = ["pages", "panels"];

/// A manifest path is only ever written under `pages/` or `panels/` of the chapter.
fn safe_rel(rel: &str) -> CmdResult<PathBuf> {
    let path = Path::new(rel);
    let ok = path.components().all(|c| matches!(c, Component::Normal(_)))
        && path.components().next().is_some_and(|c| OWNED.iter().any(|o| c.as_os_str() == *o));
    if ok { Ok(path.to_path_buf()) } else { Err(format!("worker sent an unsafe path: {rel}")) }
}

/// Fetches every file into a staging folder, writes panels.json and the report, then swaps the
/// staged `pages/` and `panels/` in, so a failed run never leaves a half-written chapter.
pub async fn install<F, Fut>(chapter_dir: &Path, manifest: &VisionManifest, mut fetch: F) -> CmdResult<Layout>
where
    F: FnMut(String, PathBuf) -> Fut,
    Fut: Future<Output = CmdResult<()>>,
{
    let doc: PanelsDoc = serde_json::from_value(manifest.panels.clone())
        .map_err(|e| format!("panels.json from the worker is invalid: {e}"))?;
    let prefix = format!("chapters/{}/", manifest.chapter_id);
    for panel in &doc.panels {
        if !panel.page.starts_with(&prefix) {
            return Err(format!("{} points outside this chapter: {}", *panel.id, *panel.page));
        }
    }
    let staging = chapter_dir.join(".vision.partial");
    let _ = std::fs::remove_dir_all(&staging);
    for (rel, hash) in &manifest.files {
        let dest = staging.join(safe_rel(rel)?);
        std::fs::create_dir_all(dest.parent().ok_or("bad path")?).map_err(msg)?;
        fetch(hash.clone(), dest).await?;
    }
    std::fs::create_dir_all(staging.join("panels")).map_err(msg)?;
    fsx::atomic_write_json(&staging.join("panels").join("panels.json"), &manifest.panels).map_err(msg)?;
    fsx::atomic_write_json(&staging.join("panels").join("vision.report.json"), &manifest.report).map_err(msg)?;
    std::fs::create_dir_all(staging.join("pages")).map_err(msg)?;
    for name in OWNED {
        let live = chapter_dir.join(name);
        let old = chapter_dir.join(format!(".{name}.old"));
        let _ = std::fs::remove_dir_all(&old);
        if live.exists() {
            std::fs::rename(&live, &old).map_err(msg)?;
        }
        std::fs::rename(staging.join(name), &live).map_err(msg)?;
        let _ = std::fs::remove_dir_all(&old);
    }
    let _ = std::fs::remove_dir_all(&staging);
    Ok(if manifest.panels["layout"] == "strip" { Layout::Strip } else { Layout::Page })
}

/// Moves an Imported or Rasterized chapter on to Analyzed (re-running an Analyzed one keeps it there).
fn mark_analyzed(project: &jumbo_core::Project, id: ChapterId, layout: Layout) -> CmdResult<ChapterManifest> {
    let mut manifest = chapter::load_chapter(project, id).map_err(msg)?;
    while manifest.state < ChapterState::Analyzed {
        let next = manifest.state.next().ok_or("no next stage")?;
        manifest = chapter::advance(project, id, next).map_err(msg)?;
    }
    chapter::set_layout(project, id, layout).map_err(msg)
}

/// Runs the vision stage for one chapter on the connected worker. Progress arrives as
/// `vision://progress` events.
#[tauri::command]
pub async fn analyze_chapter(
    app: AppHandle,
    state: State<'_, AppState>,
    bridge: State<'_, BridgeState>,
    chapter: String,
    detector: String,
) -> CmdResult<ChapterManifest> {
    let id: ChapterId = chapter.parse().map_err(msg)?;
    let (dir, pdf) = with_project(&state, |p| {
        let manifest = chapter::load_chapter(p, id).map_err(msg)?;
        if manifest.state > ChapterState::Analyzed {
            return Err(format!(
                "{chapter} is already {}; send it back to Analyzed before re-running vision",
                manifest.state.as_str()
            ));
        }
        let dir = chapter::chapter_dir(p, id);
        Ok((dir.clone(), dir.join("source").join("original.pdf")))
    })?;
    current_client(&bridge).await?;
    let report = |progress: f64, message: String| {
        let _ = app.emit("vision://progress", Progress { chapter: &chapter, progress, message });
    };

    report(0.0, "uploading the PDF".into());
    let input = upload(&bridge, &pdf).await?;
    let params = json!({ "chapter_id": chapter, "detector": detector });
    let output = run_job(&bridge, "vision", &input, params, |p, m| report(p * 0.9, m)).await?;

    report(0.9, "downloading pages and crops".into());
    let manifest_path = dir.join(".vision.manifest.json");
    download(&bridge, &output, &manifest_path).await?;
    let manifest: VisionManifest = serde_json::from_slice(&std::fs::read(&manifest_path).map_err(msg)?).map_err(msg)?;
    let _ = std::fs::remove_file(&manifest_path);
    if manifest.chapter_id != chapter {
        return Err(format!("worker answered for {} instead of {chapter}", manifest.chapter_id));
    }
    let bridge = &*bridge;
    let layout = install(&dir, &manifest, |hash, dest| async move { download(bridge, &hash, &dest).await }).await?;
    report(1.0, format!("{} panels", manifest.panels["panels"].as_array().map_or(0, Vec::len)));
    with_project(&state, |p| mark_analyzed(p, id, layout))
}

/// A chapter's panels.json and vision report, for the panel review screen.
#[tauri::command]
pub fn chapter_panels(state: State<AppState>, chapter: String) -> CmdResult<Value> {
    with_project(&state, |p| {
        let id: ChapterId = chapter.parse().map_err(msg)?;
        let dir = chapter::chapter_dir(p, id).join("panels");
        let read = |name: &str| -> CmdResult<Value> {
            match std::fs::read_to_string(dir.join(name)) {
                Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{name}: {e}")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
                Err(e) => Err(msg(e)),
            }
        };
        Ok(json!({ "panels": read("panels.json")?, "report": read("vision.report.json")? }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(files: &[(&str, &str)]) -> VisionManifest {
        let doc: Value =
            serde_json::from_str(include_str!("../../../../packages/schema/fixtures/panels-doc.sample.json"))
                .expect("fixture");
        VisionManifest {
            chapter_id: doc["chapter_id"].as_str().expect("chapter id").to_owned(),
            panels: doc,
            files: files.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
            report: json!({ "joins": [] }),
        }
    }

    #[tokio::test]
    async fn install_replaces_pages_and_panels_with_the_fetched_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("panels/crops")).expect("old");
        std::fs::write(dir.path().join("panels/crops/stale.webp"), b"old").expect("old crop");
        let m = manifest(&[("pages/page_001.webp", "h1"), ("panels/crops/p_0001.webp", "h2")]);
        let layout = install(dir.path(), &m, |hash, dest| async move { std::fs::write(dest, hash).map_err(msg) })
            .await
            .expect("install");
        assert_eq!(layout, Layout::Page);
        assert_eq!(std::fs::read(dir.path().join("pages/page_001.webp")).expect("page"), b"h1");
        assert_eq!(std::fs::read(dir.path().join("panels/crops/p_0001.webp")).expect("crop"), b"h2");
        assert!(!dir.path().join("panels/crops/stale.webp").exists(), "an old run's crops are gone");
        assert!(
            dir.path().join("panels/panels.json").exists() && dir.path().join("panels/vision.report.json").exists()
        );
        assert!(!dir.path().join(".vision.partial").exists());
    }

    #[tokio::test]
    async fn install_refuses_paths_outside_the_vision_folders() {
        let dir = tempfile::tempdir().expect("tempdir");
        for bad in ["../escape.webp", "script/approved.json", "/etc/passwd", "pages/../../x"] {
            let m = manifest(&[(bad, "h")]);
            let err = install(dir.path(), &m, |_, _| async { Ok(()) }).await.expect_err(bad);
            assert!(err.contains("unsafe path"), "{bad}: {err}");
        }
    }

    #[tokio::test]
    async fn a_failed_fetch_leaves_the_chapter_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("pages")).expect("pages");
        std::fs::write(dir.path().join("pages/page_001.webp"), b"keep").expect("page");
        let m = manifest(&[("pages/page_001.webp", "h1")]);
        install(dir.path(), &m, |_, _| async { Err("network down".to_owned()) }).await.expect_err("fails");
        assert_eq!(std::fs::read(dir.path().join("pages/page_001.webp")).expect("page"), b"keep");
    }
}

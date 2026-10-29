//! Rendering on Kaggle CPU machines: a chapter's episode is cut into equal frame ranges, each rendered by its own
//! machine (no GPU or TPU quota), then one more machine joins them without re-encoding and lays the soundtrack under.
//!
//! - The render job (EpisodeSpec, the panels and narration it plays, and a stamp) goes up as the private dataset
//!   `<owner>/jumbo-render-job`. Kaggle can hand a machine the previous version for a few minutes after an update,
//!   so every machine checks the stamp and exits with [`STALE_EXIT`] on a mismatch, and is pushed again.
//! - The render kit (motion and schema sources, render script, pinned packages) is embedded in each chunk kernel's
//!   source, so code never goes stale.

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;

use crate::error::{BridgeError, Result};
use crate::kaggle::KernelPush;

const KIT: &str = include_str!(concat!(env!("OUT_DIR"), "/render_kit.json"));
const CHUNK: &str = include_str!("../render/chunk.py");
const JOIN: &str = include_str!("../render/join.py");

/// The number of chunk machines a render uses.
pub const CHUNKS: u32 = 5;
/// A machine's exit code when it was given an older render job than the one it was pushed for.
pub const STALE_EXIT: i32 = 3;
/// The file the job dataset holds.
pub const JOB_FILE: &str = "job.tar";

pub fn job_dataset(owner: &str) -> String {
    format!("{owner}/jumbo-render-job")
}

pub fn chunk_slug(owner: &str, i: u32) -> String {
    format!("{owner}/jumbo-render-c{i}")
}

pub fn join_slug(owner: &str) -> String {
    format!("{owner}/jumbo-render-join")
}

/// The kit as embedded: path inside the kit -> file text.
pub fn render_kit() -> &'static str {
    KIT
}

fn title(slug: &str) -> String {
    slug.split_once('/').map_or(slug, |(_, s)| s).to_owned()
}

/// The chunk and join scripts as embedded, so a change to either one shows in an episode part's fingerprint.
pub fn render_scripts() -> String {
    format!("{CHUNK}\0{JOIN}")
}

/// The kernel that renders chunk `i` of `n` of the job stamped `stamp`.
pub fn chunk_kernel(owner: &str, i: u32, n: u32, stamp: &str) -> KernelPush {
    let source = CHUNK
        .replace("__KIT__", &format!("json.loads(base64.b64decode(\"{}\"))", B64.encode(KIT)))
        .replace("__CHUNKS__", &n.to_string())
        .replace("__CHUNK__", &i.to_string())
        .replace("__STAMP__", stamp);
    let slug = chunk_slug(owner, i);
    KernelPush {
        title: title(&slug),
        slug,
        source,
        machine_shape: None,
        dataset_sources: vec![job_dataset(owner)],
        model_sources: vec![],
        kernel_sources: vec![],
    }
}

/// The kernel that joins the `n` chunks into `<name>.mp4` with the soundtrack.
pub fn join_kernel(owner: &str, n: u32, stamp: &str, name: &str) -> KernelPush {
    let source = JOIN.replace("__CHUNKS__", &n.to_string()).replace("__STAMP__", stamp).replace("__NAME__", name);
    let slug = join_slug(owner);
    KernelPush {
        title: title(&slug),
        slug,
        source,
        machine_shape: None,
        dataset_sources: vec![job_dataset(owner)],
        model_sources: vec![],
        kernel_sources: (1..=n).map(|i| chunk_slug(owner, i)).collect(),
    }
}

/// The job archive: `STAMP`, `episode.json`, and every asset under `public/` at its project-relative path.
/// `assets` are project-relative paths, read from `project_root`.
pub fn job_archive(project_root: &Path, episode: &[u8], assets: &[String], stamp: &str) -> Result<Vec<u8>> {
    let mut tar = tar::Builder::new(Vec::new());
    let mut add = |name: &str, bytes: &[u8]| -> Result<()> {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, name, bytes).map_err(|e| BridgeError::io(Path::new(name), e))
    };
    add("STAMP", stamp.as_bytes())?;
    add("episode.json", episode)?;
    for rel in assets {
        let safe = !rel.is_empty() && Path::new(rel).components().all(|c| matches!(c, std::path::Component::Normal(_)));
        if !safe {
            return Err(BridgeError::Protocol(format!("asset path leaves the project: {rel}")));
        }
        let path = project_root.join(rel);
        let bytes = std::fs::read(&path).map_err(|e| BridgeError::io(&path, e))?;
        add(&format!("public/{rel}"), &bytes)?;
    }
    tar.into_inner().map_err(|e| BridgeError::io(Path::new(JOB_FILE), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kit_carries_the_render_script_and_the_motion_and_schema_sources() {
        let kit: std::collections::BTreeMap<String, String> = serde_json::from_str(render_kit()).expect("kit");
        for file in [
            "package.json",
            "render-episode.mts",
            "schema/package.json",
            "schema/src/index.ts",
            "motion/src/entry.ts",
            "motion/remotion.config.ts",
        ] {
            assert!(kit.contains_key(file), "{file} missing");
        }
        assert!(kit["package.json"].contains(r#""@jumbo/schema": "file:./schema""#));
        assert!(!kit.keys().any(|k| k.contains("node_modules") || k.contains("/test/")));
    }

    #[test]
    fn chunk_kernels_split_the_frames_and_read_the_job_dataset() {
        let push = chunk_kernel("alice", 2, 5, "s-1");
        assert_eq!(push.slug, "alice/jumbo-render-c2");
        assert_eq!(push.dataset_sources, ["alice/jumbo-render-job"]);
        assert!(push.machine_shape.is_none() && push.kernel_sources.is_empty());
        for part in ["CHUNK, CHUNKS, STAMP = 2, 5, \"s-1\"", "--chunk={CHUNK}/{CHUNKS}", "sys.exit(3)"] {
            assert!(push.source.contains(part), "{part}");
        }
        for placeholder in ["__KIT__", "__CHUNK__", "__CHUNKS__", "__STAMP__"] {
            assert!(!push.source.contains(placeholder), "{placeholder} left in the source");
        }
    }

    #[test]
    fn the_join_kernel_reads_every_chunk() {
        let push = join_kernel("alice", 5, "s-1", "ch_0001");
        assert_eq!(push.kernel_sources, (1..=5).map(|i| format!("alice/jumbo-render-c{i}")).collect::<Vec<_>>());
        assert!(push.source.contains("CHUNKS, STAMP, NAME = 5, \"s-1\", \"ch_0001\""));
        assert!(!push.source.contains("__CHUNKS__"));
    }

    #[test]
    fn the_job_archive_holds_the_spec_the_stamp_and_the_assets() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("chapters/ch_0001/audio")).expect("dir");
        std::fs::write(dir.path().join("chapters/ch_0001/audio/l_0001.wav"), b"RIFF").expect("clip");
        let bytes =
            job_archive(dir.path(), b"{}", &["chapters/ch_0001/audio/l_0001.wav".into()], "s-1").expect("archive");
        let mut archive = tar::Archive::new(bytes.as_slice());
        let names: Vec<String> = archive
            .entries()
            .expect("entries")
            .map(|e| e.expect("entry").path().expect("path").display().to_string())
            .collect();
        assert_eq!(names, ["STAMP", "episode.json", "public/chapters/ch_0001/audio/l_0001.wav"]);
        assert!(job_archive(dir.path(), b"{}", &["../secret".into()], "s").is_err());
    }
}

//! Bundles the Python worker's source into the binary, so every pushed kernel runs exactly the worker
//! version this app was built with.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn main() {
    let worker = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("../../worker");
    let package = worker.join("jumbo_worker");
    println!("cargo:rerun-if-changed={}", package.display());
    let mut files = BTreeMap::new();
    // Project metadata and the model registry the resource manager loads.
    for name in ["pyproject.toml", "model_specs.json"] {
        println!("cargo:rerun-if-changed={}", worker.join(name).display());
        files.insert(name.to_owned(), read(&worker.join(name)));
    }
    let mut sources = Vec::new();
    collect(&package, &mut sources);
    sources.sort();
    for path in sources {
        println!("cargo:rerun-if-changed={}", path.display());
        let rel = path.strip_prefix(&worker).expect("inside worker").to_string_lossy().replace('\\', "/");
        files.insert(rel, read(&path));
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir"));
    std::fs::write(out.join("worker_bundle.json"), serde_json::to_string(&files).expect("serialize bundle"))
        .expect("write bundle");
    std::fs::write(out.join("render_kit.json"), serde_json::to_string(&render_kit()).expect("serialize kit"))
        .expect("write kit");
}

/// The render kit a chunk machine installs: the kit's package.json and render script, plus the schema and motion
/// sources as the repo has them, so every render uses exactly the motion library this app was built with.
fn render_kit() -> BTreeMap<String, String> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let packages = manifest.join("../../packages");
    let mut kit = BTreeMap::new();
    let mut add = |rel: String, path: &Path| {
        println!("cargo:rerun-if-changed={}", path.display());
        kit.insert(rel, read(path));
    };
    for name in ["package.json", "render-episode.mts"] {
        add(name.to_owned(), &manifest.join("render").join(name));
    }
    for (package, extra) in [("schema", &[][..]), ("motion", &["remotion.config.ts"][..])] {
        let dir = packages.join(package);
        add(format!("{package}/package.json"), &dir.join("package.json"));
        for name in extra {
            add(format!("{package}/{name}"), &dir.join(name));
        }
        let mut sources = Vec::new();
        collect_ext(&dir.join("src"), &["ts", "tsx", "json"], &mut sources);
        sources.sort();
        for path in sources {
            let rel = path.strip_prefix(&dir).expect("inside package").to_string_lossy().replace('\\', "/");
            add(format!("{package}/{rel}"), &path);
        }
    }
    println!("cargo:rerun-if-changed={}", packages.join("schema/src").display());
    println!("cargo:rerun-if-changed={}", packages.join("motion/src").display());
    kit
}

/// Every file with one of `exts` under `dir`.
fn collect_ext(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("package sources exist") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_ext(&path, exts, out);
        } else if path.extension().is_some_and(|e| exts.iter().any(|x| e == *x)) {
            out.push(path);
        }
    }
}

/// Files the worker reads at run time: sources plus the prompts and schemas next to them.
const SHIPPED: [&str; 3] = ["py", "md", "json"];

/// Every shipped file under `dir`, including subpackages, skipping bytecode caches.
fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("worker/jumbo_worker exists") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "__pycache__") {
                collect(&path, out);
            }
        } else if path.extension().is_some_and(|e| SHIPPED.iter().any(|s| e == *s)) {
            out.push(path);
        }
    }
}

fn read(path: &Path) -> String {
    // Normalize line endings so the bundle is identical whichever OS built it.
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).replace("\r\n", "\n")
}

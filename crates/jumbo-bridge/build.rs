//! Bundles the Python worker's source into the binary, so every pushed kernel runs exactly the worker
//! version this app was built with.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn main() {
    let worker = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("../../worker");
    let package = worker.join("jumbo_worker");
    println!("cargo:rerun-if-changed={}", package.display());
    println!("cargo:rerun-if-changed={}", worker.join("pyproject.toml").display());

    let mut files = BTreeMap::new();
    files.insert("pyproject.toml".to_owned(), read(&worker.join("pyproject.toml")));
    let mut sources = Vec::new();
    collect_py(&package, &mut sources);
    sources.sort();
    for path in sources {
        println!("cargo:rerun-if-changed={}", path.display());
        let rel = path.strip_prefix(&worker).expect("inside worker").to_string_lossy().replace('\\', "/");
        files.insert(rel, read(&path));
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("worker_bundle.json");
    std::fs::write(out, serde_json::to_string(&files).expect("serialize bundle")).expect("write bundle");
}

/// Every `.py` file under `dir`, including subpackages, skipping bytecode caches.
fn collect_py(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("worker/jumbo_worker exists") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "__pycache__") {
                collect_py(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "py") {
            out.push(path);
        }
    }
}

fn read(path: &Path) -> String {
    // Normalize line endings so the bundle is identical whichever OS built it.
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).replace("\r\n", "\n")
}

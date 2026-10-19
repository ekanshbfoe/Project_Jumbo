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
    let mut sources: Vec<PathBuf> = std::fs::read_dir(&package)
        .expect("worker/jumbo_worker exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "py"))
        .collect();
    sources.sort();
    for path in sources {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = path.file_name().expect("file name").to_string_lossy();
        files.insert(format!("jumbo_worker/{name}"), read(&path));
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("worker_bundle.json");
    std::fs::write(out, serde_json::to_string(&files).expect("serialize bundle")).expect("write bundle");
}

fn read(path: &Path) -> String {
    // Normalize line endings so the bundle is identical whichever OS built it.
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).replace("\r\n", "\n")
}

use std::path::PathBuf;
use std::process::Command;

use jumbo_bridge::crypto::AppKeys;
use jumbo_bridge::launcher::{LaunchConfig, bootstrap_script, kernel_push, worker_bundle};
use jumbo_bridge::protocol::{RendezvousSpec, SessionTicket};
use serde_json::Value;

fn ticket() -> SessionTicket {
    SessionTicket::new(&AppKeys::generate().expect("keys"), RendezvousSpec::Gist { gist_id: "g".into() })
        .expect("ticket")
}

fn venv_python() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../worker/.venv");
    [dir.join("Scripts").join("python.exe"), dir.join("bin").join("python")].into_iter().find(|p| p.is_file())
}

#[test]
fn bundle_carries_the_whole_worker_package() {
    let bundle: Value = serde_json::from_str(worker_bundle()).expect("bundle json");
    for file in ["pyproject.toml", "jumbo_worker/__main__.py", "jumbo_worker/app.py", "jumbo_worker/crypto.py"] {
        assert!(bundle[file].is_string(), "{file} missing from bundle");
    }
    assert!(!bundle.as_object().expect("object").keys().any(|k| k.contains("test")));
}

/// Subpackages (pool/, pipeline/ …) must ship too, or the kernel would fail to import them.
#[test]
fn bundle_matches_every_worker_source_file_on_disk() {
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "__pycache__") {
                    walk(&path, base, out);
                }
            } else if path.extension().is_some_and(|e| e == "py") {
                out.push(path.strip_prefix(base).expect("prefix").to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../worker");
    let mut on_disk = vec!["pyproject.toml".to_owned()];
    walk(&worker.join("jumbo_worker"), &worker, &mut on_disk);
    on_disk.sort();
    let bundle: Value = serde_json::from_str(worker_bundle()).expect("bundle json");
    let bundled: Vec<String> = bundle.as_object().expect("object").keys().cloned().collect();
    assert_eq!(bundled, on_disk);
}

#[test]
fn kernel_push_targets_the_configured_kernel() {
    let config = LaunchConfig {
        kernel_slug: "alice/jumbo-worker".into(),
        machine_shape: Some("TpuV5E8".into()),
        model_sources: vec![],
        dataset_sources: vec![],
    };
    let push = kernel_push(&config, &ticket()).expect("push");
    assert_eq!((push.slug.as_str(), push.title.as_str()), ("alice/jumbo-worker", "jumbo-worker"));
    assert_eq!(push.machine_shape.as_deref(), Some("TpuV5E8"));
    assert!(push.source.starts_with("# Jumbo worker bootstrap"));
    assert!(!push.source.contains("__TICKET__") && !push.source.contains("__BUNDLE__"));
}

/// Runs the generated script in dry-run mode with the worker's Python: it must unpack an importable
/// worker and the exact ticket.
#[test]
fn bootstrap_unpacks_an_importable_worker_and_the_ticket() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: worker/.venv not found");
        return;
    };
    let ticket = ticket();
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = tmp.path().join("kernel.py");
    std::fs::write(&script, bootstrap_script(&ticket).expect("script")).expect("write");
    let home = tmp.path().join("home");
    let out =
        Command::new(&python).arg(&script).env("JUMBO_HOME", &home).env("JUMBO_DRY_RUN", "1").output().expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let written: SessionTicket =
        serde_json::from_slice(&std::fs::read(home.join("ticket.json")).expect("ticket.json")).expect("parse");
    assert_eq!(written, ticket);
    let import = Command::new(&python)
        .args(["-I", "-c", "import sys; sys.path.insert(0, sys.argv[1]); import jumbo_worker.app, jumbo_worker.__main__; print(jumbo_worker.__version__)"])
        .arg(home.join("src"))
        .output()
        .expect("import");
    assert!(import.status.success(), "{}", String::from_utf8_lossy(&import.stderr));
    assert_eq!(String::from_utf8_lossy(&import.stdout).trim(), "0.1.0");
}

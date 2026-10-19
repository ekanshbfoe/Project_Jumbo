//! Crash-safe file writes. Every persisted project file goes through here so a crash or power loss
//! leaves either the old contents or the new contents, never a torn file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;

/// Writes `bytes` to `path` atomically: write a sibling temp file, fsync it, rename over the target,
/// then fsync the directory where the platform allows it.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory"))?;
    fs::create_dir_all(dir)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let tmp = dir.join(format!(".{}.{}.tmp", file_name.to_string_lossy(), uuid::Uuid::new_v4().simple()));

    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // On Windows std::fs::rename replaces an existing target (MoveFileEx with REPLACE_EXISTING).
        fs::rename(&tmp, path)?;
        sync_dir(dir);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Pretty JSON with a trailing newline, written atomically.
pub fn atomic_write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

/// TOML, written atomically.
pub fn atomic_write_toml<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    let text = toml::to_string_pretty(value).map_err(io::Error::other)?;
    atomic_write(path, text.as_bytes())
}

/// Appends one line and fsyncs before returning, for append-only logs.
pub fn append_line(path: &Path, line: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut buf = Vec::with_capacity(line.len() + 1);
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');
    file.write_all(&buf)?;
    file.sync_all()
}

fn sync_dir(dir: &Path) {
    // Directories cannot be opened for fsync on Windows; NTFS journals the rename itself.
    if cfg!(unix)
        && let Ok(d) = File::open(dir)
    {
        let _ = d.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_existing_file_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("a.json");
        atomic_write(&path, b"one").expect("first write");
        atomic_write(&path, b"two").expect("second write");
        assert_eq!(fs::read(&path).expect("read"), b"two");
        let entries: Vec<_> = fs::read_dir(path.parent().expect("parent")).expect("ls").collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn append_line_adds_newline_terminated_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("log.jsonl");
        append_line(&path, "{\"a\":1}").expect("append");
        append_line(&path, "{\"a\":2}").expect("append");
        assert_eq!(fs::read_to_string(&path).expect("read"), "{\"a\":1}\n{\"a\":2}\n");
    }
}

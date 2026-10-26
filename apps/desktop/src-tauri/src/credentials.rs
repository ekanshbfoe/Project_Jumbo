//! Secrets (Kaggle and GitHub tokens). The OS credential store is preferred: Windows Credential
//! Manager, the macOS Keychain or the Linux Secret Service. A machine without one (e.g. a headless
//! Linux box with no keyring daemon) falls back to a file only this user can read, in the app config
//! dir and never inside a project.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use jumbo_core::fsx;
use serde::Serialize;

const KEYRING_SERVICE: &str = "dev.jumbo.desktop";

/// Where saved secrets currently live, shown on the Session screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Storage {
    Os,
    File,
}

pub trait OsStore {
    fn get(&self, name: &str) -> keyring::Result<Option<String>>;
    fn set(&self, name: &str, value: &str) -> keyring::Result<()>;
}

/// The platform credential store through the `keyring` crate.
pub struct Keyring;

impl OsStore for Keyring {
    fn get(&self, name: &str) -> keyring::Result<Option<String>> {
        match keyring::Entry::new(KEYRING_SERVICE, name)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn set(&self, name: &str, value: &str) -> keyring::Result<()> {
        keyring::Entry::new(KEYRING_SERVICE, name)?.set_password(value)
    }
}

/// Kept in its own owner-only folder, so the file is never readable by others, even mid-write.
fn secrets_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("secrets")
}

fn secrets_file(config_dir: &Path) -> PathBuf {
    secrets_dir(config_dir).join("credentials.json")
}

fn load_file(config_dir: &Path) -> BTreeMap<String, String> {
    fs::read_to_string(secrets_file(config_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_file(config_dir: &Path, secrets: &BTreeMap<String, String>) -> std::io::Result<()> {
    let dir = secrets_dir(config_dir);
    fs::create_dir_all(&dir)?;
    restrict(&dir, 0o700)?;
    let path = secrets_file(config_dir);
    fsx::atomic_write_json(&path, secrets)?;
    restrict(&path, 0o600)
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> std::io::Result<()> {
    // The per-user app config dir is already private on Windows.
    Ok(())
}

pub fn get(os: &dyn OsStore, config_dir: &Path, name: &str) -> Option<String> {
    match os.get(name) {
        Ok(Some(value)) => Some(value),
        Ok(None) | Err(_) => load_file(config_dir).remove(name),
    }
}

/// Saves to the OS store when it works, otherwise to the fallback file.
pub fn set(os: &dyn OsStore, config_dir: &Path, name: &str, value: &str) -> std::io::Result<Storage> {
    let mut file = load_file(config_dir);
    if os.set(name, value).is_ok() {
        // Drop any older fallback copy so the two never disagree.
        if file.remove(name).is_some() {
            save_file(config_dir, &file)?;
        }
        return Ok(Storage::Os);
    }
    file.insert(name.to_owned(), value.to_owned());
    save_file(config_dir, &file)?;
    Ok(Storage::File)
}

pub fn storage(config_dir: &Path) -> Storage {
    if load_file(config_dir).is_empty() { Storage::Os } else { Storage::File }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    /// No keyring daemon, as on a headless Linux machine.
    struct Unavailable;

    impl OsStore for Unavailable {
        fn get(&self, _: &str) -> keyring::Result<Option<String>> {
            Err(keyring::Error::NoStorageAccess("no secret service".into()))
        }
        fn set(&self, _: &str, _: &str) -> keyring::Result<()> {
            Err(keyring::Error::NoStorageAccess("no secret service".into()))
        }
    }

    #[derive(Default)]
    struct Memory(RefCell<HashMap<String, String>>);

    impl OsStore for Memory {
        fn get(&self, name: &str) -> keyring::Result<Option<String>> {
            Ok(self.0.borrow().get(name).cloned())
        }
        fn set(&self, name: &str, value: &str) -> keyring::Result<()> {
            self.0.borrow_mut().insert(name.to_owned(), value.to_owned());
            Ok(())
        }
    }

    #[test]
    fn falls_back_to_a_private_file_and_survives_a_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(set(&Unavailable, dir.path(), "github-gist", "ghp_one").expect("set"), Storage::File);
        set(&Unavailable, dir.path(), "kaggle", "{\"token\":\"KGAT_x\"}").expect("set");
        // A fresh lookup reads from disk, as after an app restart.
        assert_eq!(get(&Unavailable, dir.path(), "github-gist").as_deref(), Some("ghp_one"));
        assert_eq!(get(&Unavailable, dir.path(), "kaggle").as_deref(), Some("{\"token\":\"KGAT_x\"}"));
        assert_eq!(get(&Unavailable, dir.path(), "missing"), None);
        assert_eq!(storage(dir.path()), Storage::File);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: PathBuf| fs::metadata(p).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(mode(secrets_dir(dir.path())), 0o700);
            assert_eq!(mode(secrets_file(dir.path())), 0o600);
        }
    }

    #[test]
    fn prefers_the_os_store_and_retires_the_fallback_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        set(&Unavailable, dir.path(), "github-gist", "ghp_old").expect("set");

        let os = Memory::default();
        assert_eq!(set(&os, dir.path(), "github-gist", "ghp_new").expect("set"), Storage::Os);
        assert_eq!(get(&os, dir.path(), "github-gist").as_deref(), Some("ghp_new"));
        assert!(load_file(dir.path()).is_empty());
        assert_eq!(storage(dir.path()), Storage::Os);
    }
}

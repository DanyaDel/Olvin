//! `<project>/library.lock.ron`: which packs are installed and what they wrote.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const LOCK_FILE: &str = "library.lock.ron";
const LOCK_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Lock {
    pub version: u32,
    pub packs: Vec<InstalledPack>,
}

/// One installed pack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstalledPack {
    pub id: String,
    pub version: String,
    /// Asked for by the user (not only pulled in as a `requires`).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub explicit: bool,
    pub files: Vec<InstalledFile>,
}

fn yes() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstalledFile {
    /// Where it is now, relative to `assets/`.
    pub path: String,
    /// SHA-256 of the content the pack wrote (hex).
    pub hash: String,
    /// The file was already there, identical: removing the pack should keep it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preexisting: bool,
    /// "Keep both" wrote the pack's file under another name: its path in the pack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

impl Lock {
    pub fn path(project: &Path) -> PathBuf {
        project.join(LOCK_FILE)
    }

    /// A missing file is an empty lock; a broken one is an error (never overwritten silently).
    pub fn load(project: &Path) -> Result<Lock, String> {
        let path = Self::path(project);
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Lock::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let lock: Lock = ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if lock.version > LOCK_VERSION {
            return Err(format!(
                "{}: written by a newer Olvin (version {})",
                path.display(),
                lock.version
            ));
        }
        Ok(lock)
    }

    pub fn save(&self, project: &Path) -> Result<(), String> {
        let path = Self::path(project);
        let mut lock = self.clone();
        lock.version = LOCK_VERSION;
        lock.packs.sort_by(|a, b| a.id.cmp(&b.id));
        let text = ron::ser::to_string_pretty(&lock, ron::ser::PrettyConfig::default())
            .map_err(|e| e.to_string())?;
        let tmp = path.with_extension("ron.tmp");
        fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn get(&self, id: &str) -> Option<&InstalledPack> {
        self.packs.iter().find(|p| p.id == id)
    }

    pub fn upsert(&mut self, pack: InstalledPack) {
        match self.packs.iter_mut().find(|p| p.id == pack.id) {
            Some(slot) => *slot = pack,
            None => self.packs.push(pack),
        }
    }
}

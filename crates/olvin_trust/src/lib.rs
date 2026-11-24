//! Which projects may run code on this machine (roadmap T1).
//!
//! Building a game project means running its code: `cargo` executes the
//! project's `build.rs` and procedural macros (and those of every dependency)
//! with your permissions, and then the game itself. A project you downloaded or
//! cloned can do anything you can. So the hub and the editor ask before they
//! build or run a project they did not create, and remember the answer here.
//!
//! * One list for both tools, `<config dir>/olvin/trusted-projects.toml`
//!   (`XDG_CONFIG_HOME` on Linux): a decision made in the hub is not asked
//!   again in the editor.
//! * A project is trusted by its canonical folder path. Trust is not inherited
//!   by sub-folders, and it does not follow a moved or copied project.
//! * Projects the hub creates are trusted at once; everything else is not.
//! * Trust is about the folder, not its contents: after a `git pull` the new
//!   code is trusted too. Review before pulling, or untrust the project.
//!
//! What this is. A consent step against *running by accident* what you merely
//! opened. It is not a sandbox: once trusted, a project (and its dependencies)
//! can do anything, and an untrusted one is only "not built by Olvin's tools":
//! the editor still reads its scenes and scripts, and `cargo` run by hand does
//! not ask anyone.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `1`, `true` or `yes`: trust the project for this run (the CLI's `--trust`).
pub const TRUST_ENV: &str = "OLVIN_TRUST";
/// Set by the hub to `readonly` when the user chose "Open read-only": the
/// editor then does not ask again and refuses to run cargo or shell jobs.
pub const MODE_ENV: &str = "OLVIN_TRUST_MODE";

/// What the user is told before trusting a project. One text for every front-end.
pub const EXPLANATION: &str = "Building or running a project executes its code on this computer. \
Cargo runs the project's build scripts (build.rs) and procedural macros, and those of every \
dependency, with your permissions, and then the game itself. A project from someone else can \
read or delete your files. Trust it only if you know where it came from. \
The choice is remembered for this folder; it is not re-checked when the files change.";

/// The one-line warning for the CLI and logs.
pub fn warning(project: &Path) -> String {
    format!(
        "{} is not a trusted project: building it runs its build.rs and proc-macros on this machine",
        project.display()
    )
}

/// Whether `OLVIN_TRUST` asks to trust everything for this run.
pub fn env_trusts() -> bool {
    std::env::var(TRUST_ENV)
        .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

/// Whether the hub opened this process read-only (`OLVIN_TRUST_MODE=readonly`).
pub fn env_read_only() -> bool {
    std::env::var(MODE_ENV).is_ok_and(|v| v.trim().eq_ignore_ascii_case("readonly"))
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    /// Canonical project folders.
    trusted: Vec<PathBuf>,
}

/// The list of trusted projects, in memory and on disk.
#[derive(Clone, Debug, Default)]
pub struct TrustStore {
    path: Option<PathBuf>,
    trusted: Vec<PathBuf>,
}

impl TrustStore {
    /// `<config dir>/olvin/trusted-projects.toml`.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("olvin/trusted-projects.toml"))
    }

    /// A store that is never saved (tests, tools that must not touch the config).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// The user's list. A missing file is an empty list; a damaged one is an
    /// empty list too (nothing is trusted by mistake) with a note on stderr.
    pub fn load() -> Self {
        let Some(path) = Self::default_path() else {
            return Self::default();
        };
        Self::load_from(&path).unwrap_or_else(|err| {
            eprintln!("olvin: ignoring {}: {err}", path.display());
            Self {
                path: Some(path),
                trusted: Vec::new(),
            }
        })
    }

    /// The list stored at `path` (missing file: empty). `save` writes back there.
    pub fn load_from(path: &Path) -> Result<Self, String> {
        let trusted = match fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str::<File>(&text)
                    .map_err(|e| format!("not a trust list: {e}"))?
                    .trusted
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        Ok(Self {
            path: Some(path.to_path_buf()),
            trusted,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn list(&self) -> &[PathBuf] {
        &self.trusted
    }

    /// Whether `project` (any spelling of its folder) is on the list.
    pub fn is_trusted(&self, project: &Path) -> bool {
        let key = canonical(project);
        self.trusted.contains(&key)
    }

    /// Adds the project and saves. `Ok(false)` if it was already trusted.
    pub fn trust(&mut self, project: &Path) -> io::Result<bool> {
        let key = canonical(project);
        if self.trusted.contains(&key) {
            return Ok(false);
        }
        self.trusted.push(key);
        self.save()?;
        Ok(true)
    }

    /// Removes the project and saves. `Ok(false)` if it was not trusted.
    pub fn untrust(&mut self, project: &Path) -> io::Result<bool> {
        let key = canonical(project);
        let before = self.trusted.len();
        self.trusted.retain(|p| *p != key);
        if self.trusted.len() == before {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    /// Writes the list (nothing for an in-memory store).
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(&File {
            trusted: self.trusted.clone(),
        })
        .map_err(io::Error::other)?;
        // Written whole beside the file and renamed: a crash never leaves half a list.
        let partial = path.with_extension("toml.partial");
        fs::write(&partial, text)?;
        fs::rename(partial, path)
    }
}

/// The folder as a stable key: symlinks and `..` resolved. A folder that is
/// gone keeps its absolute spelling, so it can still be untrusted.
fn canonical(project: &Path) -> PathBuf {
    project
        .canonicalize()
        .or_else(|_| std::path::absolute(project))
        .unwrap_or_else(|_| project.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("olvin-trust-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn nothing_is_trusted_by_default_and_trust_survives_a_reload() {
        let dir = temp_dir("roundtrip");
        let list = dir.join("config/olvin/trusted-projects.toml");
        let game = dir.join("game");
        fs::create_dir_all(&game).unwrap();

        let mut store = TrustStore::load_from(&list).unwrap();
        assert!(!store.is_trusted(&game));
        assert!(store.trust(&game).unwrap());
        assert!(!store.trust(&game).unwrap(), "second time is a no-op");
        assert!(store.is_trusted(&game));

        let again = TrustStore::load_from(&list).unwrap();
        assert!(again.is_trusted(&game));
        assert_eq!(again.list().len(), 1);
        // Another spelling of the same folder is the same project.
        assert!(again.is_trusted(&dir.join("game/../game/.")));
        // A sibling, a child and a lookalike are not.
        fs::create_dir_all(game.join("sub")).unwrap();
        fs::create_dir_all(dir.join("game2")).unwrap();
        assert!(!again.is_trusted(&game.join("sub")));
        assert!(!again.is_trusted(&dir.join("game2")));
        assert!(!again.is_trusted(&dir));

        let mut store = again;
        assert!(store.untrust(&game).unwrap());
        assert!(!store.untrust(&game).unwrap());
        assert!(!TrustStore::load_from(&list).unwrap().is_trusted(&game));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_trusted_project_is_the_same_project() {
        let dir = temp_dir("symlink");
        let game = dir.join("game");
        fs::create_dir_all(&game).unwrap();
        std::os::unix::fs::symlink(&game, dir.join("link")).unwrap();
        let mut store = TrustStore::in_memory();
        store.trust(&dir.join("link")).unwrap();
        assert!(store.is_trusted(&game));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_damaged_list_is_an_error_not_a_free_pass() {
        let dir = temp_dir("damaged");
        let list = dir.join("trusted-projects.toml");
        fs::write(&list, "trusted = [oops").unwrap();
        assert!(TrustStore::load_from(&list).is_err());
        fs::write(&list, "trusted = \"/not/a/list\"").unwrap();
        assert!(TrustStore::load_from(&list).is_err());
        // Unknown keys and an empty file are fine (older or newer versions).
        fs::write(&list, "future = 1\n").unwrap();
        assert!(TrustStore::load_from(&list).unwrap().list().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_in_memory_store_never_touches_the_disk() {
        let dir = temp_dir("memory");
        let mut store = TrustStore::in_memory();
        assert!(store.trust(&dir).unwrap());
        assert!(store.is_trusted(&dir));
        assert!(store.path().is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_explanation_is_honest_about_what_runs() {
        for word in [
            "build.rs",
            "procedural macros",
            "your permissions",
            "dependency",
        ] {
            assert!(EXPLANATION.contains(word), "{word}");
        }
        assert!(warning(Path::new("/x/game")).contains("/x/game"));
    }
}

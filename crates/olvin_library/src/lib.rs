//! Olvin's built-in preset library: packs of ready prefabs, scripts, shaders,
//! materials and looks that the editor, the hub CLI and tests import into a
//! project's `assets/`, like a small offline asset store.
//!
//! The packs of the repo's `library/` folder are compiled into the binary
//! (`include_dir`); extra packs come from `$OLVIN_LIBRARY` and
//! `<project>/library/`. See `library/README.md` for the pack format.
//!
//! - [`Library`]: `list`, `search`, `get`, `resolve` (dependencies, cycles),
//!   `plan`/`import`.
//! - [`Manifest`], [`Pack`], [`Category`]: the data.
//! - [`Lock`] (`library.lock.ron`), [`installed`]: what a project has.

mod import;
mod lock;
mod manifest;
mod pack;

use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};

pub use import::{
    FilePlan, FileState, ImportOptions, ImportPlan, ImportReport, OnConflict, PackPlan, PackReport,
    free_name,
};
pub use lock::{InstalledFile, InstalledPack, LOCK_FILE, Lock};
pub use manifest::{
    ALLOWED_EXTENSIONS, Category, MAX_FILE_BYTES, MAX_FILES, MAX_PACK_BYTES, MAX_PREVIEW_BYTES,
    Manifest, check_extension, check_path, is_kebab,
};
pub use pack::{Pack, PackFile, Source, hash_hex};

/// The repo's `library/` folder, compiled in. `include_dir` over `rust-embed`:
/// no derive, no runtime feature flags, plain `&'static [u8]` slices (nothing
/// is copied at startup) and one macro; the debug-time disk reads of
/// `rust-embed` would make the editor and the tests see different packs.
/// `build.rs` makes cargo rebuild when a file under `library/` changes.
static EMBEDDED: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../library");

/// Environment variable with extra pack folders (a path list: `:` or `;`).
pub const ENV_VAR: &str = "OLVIN_LIBRARY";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryError {
    /// No pack with this id.
    NotFound(String),
    /// `pack` requires `needed`, which is not in the library.
    MissingDependency {
        pack: String,
        needed: String,
    },
    /// The `requires` chain loops: `a -> b -> a`.
    Cycle(Vec<String>),
    Io(PathBuf, String),
    /// `library.lock.ron` could not be read or written.
    Lock(String),
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LibraryError::NotFound(id) => write!(f, "no pack `{id}` in the library"),
            LibraryError::MissingDependency { pack, needed } => {
                write!(
                    f,
                    "pack `{pack}` requires `{needed}`, which is not in the library"
                )
            }
            LibraryError::Cycle(chain) => write!(f, "requires loop: {}", chain.join(" -> ")),
            LibraryError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            LibraryError::Lock(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for LibraryError {}

/// The packs the process can see. Cheap to clone-free: embedded packs borrow
/// the binary's data.
#[derive(Debug, Default)]
pub struct Library {
    packs: Vec<Pack>,
    problems: Vec<String>,
}

impl Library {
    /// Only the packs compiled into the binary.
    pub fn embedded() -> Library {
        let mut lib = Library::default();
        for dir in EMBEDDED.dirs() {
            // A folder with neither a manifest nor an `assets/` is not a pack
            // (tool scripts, notes); one with `assets/` but no manifest is a
            // pack missing its `pack.ron`, which `from_embedded` reports.
            let has_manifest = dir.get_file(dir.path().join("pack.ron")).is_some();
            let has_assets = dir.get_dir(dir.path().join("assets")).is_some();
            if !has_manifest && !has_assets {
                continue;
            }
            match Pack::from_embedded(dir) {
                Ok(pack) => lib.add(pack, &folder_name(dir.path())),
                Err(e) => lib.problems.push(e),
            }
        }
        lib.finish()
    }

    /// Embedded packs plus the folders in `$OLVIN_LIBRARY` and
    /// `<project>/library/` (when a project is given). A user pack with the id
    /// of another pack is a problem and is not loaded.
    pub fn for_project(project: Option<&Path>) -> Library {
        let mut dirs: Vec<PathBuf> = std::env::var_os(ENV_VAR)
            .map(|v| {
                std::env::split_paths(&v)
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(project) = project {
            dirs.push(project.join("library"));
        }
        Library::embedded().with_dirs(&dirs)
    }

    /// Adds the pack folders found directly under each of `dirs` (a missing
    /// folder is fine).
    pub fn with_dirs(mut self, dirs: &[PathBuf]) -> Library {
        for root in dirs {
            let Ok(entries) = std::fs::read_dir(root) else {
                continue;
            };
            let mut folders: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.join("pack.ron").is_file())
                .collect();
            folders.sort();
            for folder in folders {
                match Pack::from_dir(&folder) {
                    Ok(pack) => self.add(pack, &folder_name(&folder)),
                    Err(e) => self.problems.push(e),
                }
            }
        }
        self.finish()
    }

    /// Validates, checks the id is free, and adds.
    fn add(&mut self, pack: Pack, folder: &str) {
        let errors = pack.validate(folder);
        if !errors.is_empty() {
            self.problems
                .push(format!("pack `{folder}`: {}", errors.join("; ")));
        } else if self.get(pack.id()).is_some() {
            self.problems
                .push(format!("pack `{folder}`: the id `{}` is taken", pack.id()));
        } else {
            self.packs.push(pack);
        }
    }

    fn finish(mut self) -> Library {
        self.packs.sort_by(|a, b| {
            (a.manifest.category, &a.manifest.name).cmp(&(b.manifest.category, &b.manifest.name))
        });
        self
    }

    /// Every pack, by category and name.
    pub fn list(&self) -> &[Pack] {
        &self.packs
    }

    /// Packs that could not be loaded, with the reason (for the UI and `cargo test`).
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    pub fn get(&self, id: &str) -> Option<&Pack> {
        self.packs.iter().find(|p| p.manifest.id == id)
    }

    /// Packs whose id, name, description or tags contain every word of `query`
    /// (case-insensitive), optionally of one category. An empty query matches all.
    pub fn search(&self, query: &str, category: Option<Category>) -> Vec<&Pack> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.packs
            .iter()
            .filter(|p| category.is_none_or(|c| p.manifest.category == c))
            .filter(|p| {
                let m = &p.manifest;
                let hay = format!("{} {} {} {}", m.id, m.name, m.description, m.tags.join(" "))
                    .to_lowercase();
                words.iter().all(|w| hay.contains(w))
            })
            .collect()
    }

    /// `id` and everything it requires, dependencies first, each once.
    pub fn resolve(&self, id: &str) -> Result<Vec<&Pack>, LibraryError> {
        let mut order = Vec::new();
        let mut stack = Vec::new();
        self.visit(id, None, &mut stack, &mut order)?;
        Ok(order)
    }

    fn visit<'a>(
        &'a self,
        id: &str,
        needed_by: Option<&str>,
        stack: &mut Vec<String>,
        order: &mut Vec<&'a Pack>,
    ) -> Result<(), LibraryError> {
        if order.iter().any(|p| p.manifest.id == id) {
            return Ok(());
        }
        if let Some(at) = stack.iter().position(|s| s == id) {
            let mut chain = stack[at..].to_vec();
            chain.push(id.to_owned());
            return Err(LibraryError::Cycle(chain));
        }
        let Some(pack) = self.get(id) else {
            return Err(match needed_by {
                Some(pack) => LibraryError::MissingDependency {
                    pack: pack.to_owned(),
                    needed: id.to_owned(),
                },
                None => LibraryError::NotFound(id.to_owned()),
            });
        };
        stack.push(id.to_owned());
        for dep in &pack.manifest.requires {
            self.visit(dep, Some(id), stack, order)?;
        }
        stack.pop();
        order.push(pack);
        Ok(())
    }

    /// Whether `id` is installed in `project`, and which version.
    pub fn status(&self, id: &str, project: &Path) -> Status {
        let Some(installed) = installed(project)
            .ok()
            .and_then(|l| l.into_iter().find(|p| p.id == id))
        else {
            return Status::NotInstalled;
        };
        match self.get(id) {
            Some(pack) if pack.manifest.version != installed.version => Status::Outdated {
                installed: installed.version,
                available: pack.manifest.version.clone(),
            },
            _ => Status::Installed(installed.version),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    Installed(String),
    /// A different version is installed than the library has.
    Outdated {
        installed: String,
        available: String,
    },
}

/// The packs recorded in `<project>/library.lock.ron`.
pub fn installed(project: &Path) -> Result<Vec<InstalledPack>, String> {
    Lock::load(project).map(|l| l.packs)
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;

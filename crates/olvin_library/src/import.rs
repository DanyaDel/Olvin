//! Importing a pack into a project: plan, conflicts, writing, `.meta`, lock.

use std::fs;
use std::path::Path;

use olvin_assets::AssetRoot;

use crate::lock::{InstalledFile, InstalledPack, Lock};
use crate::pack::{Pack, hash_hex};
use crate::{Library, LibraryError};

/// What an existing, different file at the destination should do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OnConflict {
    /// Write nothing at all and report the conflicts (the default: never
    /// overwrite silently).
    #[default]
    Abort,
    /// Keep the project's file, import the rest.
    Skip,
    /// Overwrite with the pack's file.
    Replace,
    /// Write the pack's file next to it as `name-2.ext`, `name-3.ext`, ...
    KeepBoth,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ImportOptions {
    pub on_conflict: OnConflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    /// Not in the project yet.
    New,
    /// Already there with the same bytes.
    Identical,
    /// Already there, different.
    Conflict,
}

#[derive(Clone, Debug)]
pub struct FilePlan {
    pub path: String,
    pub state: FileState,
}

#[derive(Clone, Debug)]
pub struct PackPlan {
    pub id: String,
    pub version: String,
    /// Only `requires` of the requested pack: the user did not ask for this one.
    pub dependency: bool,
    pub files: Vec<FilePlan>,
}

/// What an import would do, in the order it would do it (dependencies first).
#[derive(Clone, Debug, Default)]
pub struct ImportPlan {
    pub packs: Vec<PackPlan>,
}

impl ImportPlan {
    /// `(pack id, path)` of every conflicting file.
    pub fn conflicts(&self) -> Vec<(&str, &str)> {
        self.packs
            .iter()
            .flat_map(|p| {
                p.files
                    .iter()
                    .filter(|f| f.state == FileState::Conflict)
                    .map(|f| (p.id.as_str(), f.path.as_str()))
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackReport {
    pub id: String,
    pub version: String,
    pub added: Vec<String>,
    pub identical: Vec<String>,
    pub replaced: Vec<String>,
    /// `(path in the pack, path written)`.
    pub renamed: Vec<(String, String)>,
    /// Conflicting files left alone (`OnConflict::Skip`).
    pub skipped: Vec<String>,
}

impl PackReport {
    pub fn written(&self) -> usize {
        self.added.len() + self.replaced.len() + self.renamed.len()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    /// False: `OnConflict::Abort` found conflicts and nothing was written.
    pub completed: bool,
    pub packs: Vec<PackReport>,
    /// `(pack id, path)` of the files that differ from the project's (also
    /// filled when the import went on with Skip, Replace or KeepBoth).
    pub conflicts: Vec<(String, String)>,
    /// `(pack id, notes)` of the packs imported, for the "how to use" toast.
    pub notes: Vec<(String, String)>,
    /// `.meta` files that could not be made, and the like.
    pub warnings: Vec<String>,
}

impl ImportReport {
    pub fn written(&self) -> usize {
        self.packs.iter().map(PackReport::written).sum()
    }

    /// One line for a toast or the CLI.
    pub fn summary(&self) -> String {
        if !self.completed {
            return format!(
                "Nothing imported: {} file(s) differ from the project's",
                self.conflicts.len()
            );
        }
        let ids: Vec<String> = self
            .packs
            .iter()
            .map(|p| format!("{} {}", p.id, p.version))
            .collect();
        let identical: usize = self.packs.iter().map(|p| p.identical.len()).sum();
        let skipped: usize = self.packs.iter().map(|p| p.skipped.len()).sum();
        let mut text = format!(
            "Imported {}: {} file(s) written",
            ids.join(", "),
            self.written()
        );
        if identical > 0 {
            text += &format!(", {identical} already there");
        }
        if skipped > 0 {
            text += &format!(", {skipped} kept as in the project");
        }
        text
    }
}

impl Library {
    /// What importing `id` into `project` would do. Packs already installed at
    /// the same state show all their files as `Identical`.
    pub fn plan(&self, id: &str, project: &Path) -> Result<ImportPlan, LibraryError> {
        let assets = project.join("assets");
        let order = self.resolve(id)?;
        let mut plan = ImportPlan::default();
        for pack in order {
            let files = pack
                .files
                .iter()
                .map(|f| {
                    let state = match fs::read(assets.join(&f.path)) {
                        Ok(existing) if existing == *f.data => FileState::Identical,
                        Ok(_) => FileState::Conflict,
                        Err(_) => FileState::New,
                    };
                    FilePlan {
                        path: f.path.clone(),
                        state,
                    }
                })
                .collect();
            plan.packs.push(PackPlan {
                id: pack.manifest.id.clone(),
                version: pack.manifest.version.clone(),
                dependency: pack.manifest.id != id,
                files,
            });
        }
        Ok(plan)
    }

    /// Copies the pack and its `requires` (first) into `<project>/assets`,
    /// makes `.meta` GUIDs, and records them in `<project>/library.lock.ron`.
    pub fn import(
        &self,
        id: &str,
        project: &Path,
        options: &ImportOptions,
    ) -> Result<ImportReport, LibraryError> {
        let plan = self.plan(id, project)?;
        let mut report = ImportReport {
            conflicts: plan
                .conflicts()
                .into_iter()
                .map(|(p, f)| (p.to_owned(), f.to_owned()))
                .collect(),
            ..Default::default()
        };
        if !report.conflicts.is_empty() && options.on_conflict == OnConflict::Abort {
            return Ok(report);
        }
        let assets = project.join("assets");
        fs::create_dir_all(&assets).map_err(|e| LibraryError::Io(assets.clone(), e.to_string()))?;
        let mut lock = Lock::load(project).map_err(LibraryError::Lock)?;
        let index = AssetRoot::new(&assets);
        for pack_plan in &plan.packs {
            let pack = self.get(&pack_plan.id).expect("resolved above");
            let (pack_report, installed) = write_pack(
                pack,
                pack_plan,
                &assets,
                &index,
                options.on_conflict,
                &mut report.warnings,
            )?;
            // A pack the user asked for stays explicit when another pack pulls it in later.
            let explicit =
                !pack_plan.dependency || lock.get(&pack.manifest.id).is_some_and(|p| p.explicit);
            lock.upsert(InstalledPack {
                explicit,
                ..installed
            });
            // Saved per pack: a failure halfway still describes what is on disk.
            lock.save(project).map_err(LibraryError::Lock)?;
            if let Some(notes) = pack
                .manifest
                .notes
                .as_ref()
                .filter(|n| !n.trim().is_empty())
            {
                report.notes.push((pack.manifest.id.clone(), notes.clone()));
            }
            report.packs.push(pack_report);
        }
        report.completed = true;
        Ok(report)
    }
}

fn write_pack(
    pack: &Pack,
    plan: &PackPlan,
    assets: &Path,
    index: &AssetRoot,
    on_conflict: OnConflict,
    warnings: &mut Vec<String>,
) -> Result<(PackReport, InstalledPack), LibraryError> {
    let mut report = PackReport {
        id: pack.manifest.id.clone(),
        version: pack.manifest.version.clone(),
        ..Default::default()
    };
    let mut files = Vec::new();
    for (file, planned) in pack.files.iter().zip(&plan.files) {
        let hash = hash_hex(&file.data);
        let mut dest = file.path.clone();
        let mut entry_from = None;
        let mut preexisting = false;
        match planned.state {
            FileState::Identical => {
                report.identical.push(file.path.clone());
                preexisting = true;
            }
            FileState::New => {
                write(assets, &dest, &file.data)?;
                report.added.push(dest.clone());
            }
            FileState::Conflict => match on_conflict {
                OnConflict::Skip | OnConflict::Abort => {
                    report.skipped.push(file.path.clone());
                    continue;
                }
                OnConflict::Replace => {
                    write(assets, &dest, &file.data)?;
                    report.replaced.push(dest.clone());
                }
                OnConflict::KeepBoth => {
                    dest = free_name(assets, &file.path);
                    write(assets, &dest, &file.data)?;
                    report.renamed.push((file.path.clone(), dest.clone()));
                    entry_from = Some(file.path.clone());
                }
            },
        }
        // The GUID of the new (or replaced) file; an existing .meta is kept.
        if let Err(e) = index.meta(&dest) {
            warnings.push(format!("{dest}: {e}"));
        }
        files.push(InstalledFile {
            path: dest,
            hash,
            preexisting,
            from: entry_from,
        });
    }
    Ok((
        report,
        InstalledPack {
            id: pack.manifest.id.clone(),
            version: pack.manifest.version.clone(),
            explicit: true,
            files,
        },
    ))
}

fn write(assets: &Path, rel: &str, data: &[u8]) -> Result<(), LibraryError> {
    let path = assets.join(rel);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| LibraryError::Io(dir.to_path_buf(), e.to_string()))?;
    }
    fs::write(&path, data).map_err(|e| LibraryError::Io(path, e.to_string()))
}

/// Compound asset extensions: the suffix goes before them (`Player-2.prefab.ron`).
const COMPOUND: [&str; 4] = [".scene.ron", ".mat.ron", ".prefab.ron", ".animgraph.ron"];

/// `dir/name.ext` → the first free `dir/name-2.ext`, `dir/name-3.ext`, ...
pub fn free_name(assets: &Path, rel: &str) -> String {
    let (dir, file) = rel.rsplit_once('/').map_or(("", rel), |(d, f)| (d, f));
    let lower = file.to_lowercase();
    let split = COMPOUND
        .iter()
        .find(|c| lower.ends_with(*c))
        .map(|c| file.len() - c.len())
        .or_else(|| file.rfind('.'))
        .unwrap_or(file.len());
    let (stem, ext) = file.split_at(split);
    (2..)
        .map(|n| {
            let name = format!("{stem}-{n}{ext}");
            if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            }
        })
        .find(|candidate| !assets.join(candidate).exists())
        .expect("unbounded range")
}

//! File operations on assets (rename, move, delete) as undo steps.
//!
//! A move is undone by the opposite move, which rewrites the references back.
//! A delete does not remove anything: the file and its `.meta` go to the
//! project's trash (`<project>/.olvin/trash`), and the step remembers what
//! unpacking the deleted prefabs rewrote (files on disk, the open scene with
//! its history), so undo puts all of it back. Redo repeats the operation.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use olvin_scene::file::SceneFile;

use super::{EditorState, Step, UNDO_LIMIT};

/// Trash sessions older than this are emptied when a project opens.
const TRASH_KEEP: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) fn is_scene(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".scene.ron")
}

pub(crate) fn is_prefab(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".prefab.ron")
}

/// A file operation in the history; paths are relative to `assets/`.
pub(super) enum FileOp {
    Move {
        from: String,
        to: String,
    },
    Delete {
        rel: String,
        /// Where the file (and its `.meta`) waits in the trash.
        trash: PathBuf,
        /// Scene and prefab files rewritten by unpacking, with their old text.
        rewritten: Vec<(PathBuf, String)>,
        /// The open scene and its history before unpacking (only when the
        /// deleted asset had prefabs).
        history: Option<Box<History>>,
    },
}

/// The scenes the history holds, for putting back what unpacking changed.
pub(super) struct History {
    scene: SceneFile,
    committed: SceneFile,
    saved: SceneFile,
    /// The scenes of the edit steps below, oldest first.
    undo: Vec<SceneFile>,
}

impl EditorState {
    /// Renames or moves a file or folder (its `.meta` and GUID go with it) and
    /// rewrites every reference to it: scenes and materials on disk, and the
    /// open scene with its history. One undo step.
    pub(crate) fn move_asset(&mut self, root: &Path, from: &Path, to: &Path) -> Result<(), String> {
        let assets = olvin_assets::AssetRoot::new(root);
        let (Some(from), Some(to)) = (assets.relative(from), assets.relative(to)) else {
            return Err("outside assets/".into());
        };
        self.commit();
        self.move_rel(root, &from, &to)?;
        self.info(format!("moved {from} to {to}"));
        self.record(FileOp::Move { from, to });
        Ok(())
    }

    /// Deletes a file or folder: it goes to the project's trash, its prefab
    /// instances are unpacked. One undo step.
    pub(crate) fn delete_asset(&mut self, root: &Path, path: &Path) -> Result<(), String> {
        let rel = olvin_assets::AssetRoot::new(root)
            .relative(path)
            .ok_or("outside assets/")?;
        self.commit();
        let op = self.delete_rel(root, &rel)?;
        self.info(format!("deleted {rel}"));
        self.record(op);
        Ok(())
    }

    fn record(&mut self, op: FileOp) {
        self.undo.push(Step::File(op));
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Undoes `op`; returns the step for redo.
    pub(super) fn undo_file(&mut self, op: FileOp) -> Result<FileOp, String> {
        let root = self.assets_dir().ok_or("no project")?;
        match op {
            FileOp::Move { from, to } => {
                self.move_rel(&root, &to, &from)?;
                self.info(format!("undo: moved {to} back to {from}"));
                Ok(FileOp::Move { from, to })
            }
            FileOp::Delete {
                rel,
                trash,
                rewritten,
                history,
            } => {
                let abs = root.join(&rel);
                if abs.exists() {
                    return Err(format!("{rel}: a file with this name exists again"));
                }
                if let Some(dir) = abs.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                std::fs::rename(&trash, &abs).map_err(|e| format!("{rel}: {e}"))?;
                let meta = olvin_assets::meta_path(&trash);
                if meta.exists() {
                    std::fs::rename(&meta, olvin_assets::meta_path(&abs))
                        .map_err(|e| format!("{rel}.meta: {e}"))?;
                }
                for (path, text) in &rewritten {
                    if let Err(e) = std::fs::write(path, text) {
                        self.warn(format!("{}: {e}", path.display()));
                    }
                }
                if let Some(history) = history {
                    self.restore_history(*history);
                }
                self.info(format!("undo: restored {rel}"));
                // Redo deletes it again (and takes a fresh snapshot).
                Ok(FileOp::Delete {
                    rel,
                    trash,
                    rewritten: Vec::new(),
                    history: None,
                })
            }
        }
    }

    /// Repeats `op`; returns the step for undo.
    pub(super) fn redo_file(&mut self, op: FileOp) -> Result<FileOp, String> {
        let root = self.assets_dir().ok_or("no project")?;
        match op {
            FileOp::Move { from, to } => {
                self.move_rel(&root, &from, &to)?;
                self.info(format!("redo: moved {from} to {to}"));
                Ok(FileOp::Move { from, to })
            }
            FileOp::Delete { rel, .. } => {
                let op = self.delete_rel(&root, &rel)?;
                self.info(format!("redo: deleted {rel}"));
                Ok(op)
            }
        }
    }

    fn move_rel(&mut self, root: &Path, from: &str, to: &str) -> Result<(), String> {
        if to == from || to.starts_with(&format!("{from}/")) {
            return Err("cannot move a folder into itself".into());
        }
        olvin_assets::AssetRoot::new(root).move_asset(from, to)?;
        let (from_abs, to_abs) = (root.join(from), root.join(to));
        if let Some(open) = self.scene_path.clone()
            && let Ok(rest) = open.strip_prefix(&from_abs)
        {
            self.scene_path = Some(to_abs.join(rest));
        }
        rewrite_references(root, from, to);
        self.map_scenes(|scene| {
            if let Ok(text) = ron::to_string(scene)
                && let Some(new) = replace_path(&text, from, to)
                && let Ok(mut fixed) = ron::from_str::<SceneFile>(&new)
            {
                // Which instance a prefab node belongs to is not part of the file text.
                for (e, old) in fixed.entities.iter_mut().zip(&scene.entities) {
                    e.prefab_node = old.prefab_node.clone();
                }
                *scene = fixed;
            }
        });
        self.forget_materials(from);
        self.forget_prefabs(from);
        Ok(())
    }

    fn delete_rel(&mut self, root: &Path, rel: &str) -> Result<FileOp, String> {
        let abs = root.join(rel);
        if !abs.exists() {
            return Err(format!("{rel}: not found"));
        }
        let trash = self
            .trash_slot()?
            .join(abs.file_name().ok_or("no file name")?);
        // Make sure the GUID exists, so the file comes back with it.
        if abs.is_file() {
            olvin_assets::AssetRoot::new(root).meta(rel)?;
        }
        let mut gone = Vec::new();
        prefabs_under(root, root, rel, &mut gone);
        let history = (!gone.is_empty()).then(|| Box::new(self.history()));
        let rewritten = unpack_instances(self, root, &gone);
        std::fs::rename(&abs, &trash).map_err(|e| format!("{rel}: {e}"))?;
        let meta = olvin_assets::meta_path(&abs);
        if meta.exists() {
            std::fs::rename(&meta, olvin_assets::meta_path(&trash))
                .map_err(|e| format!("{rel}.meta: {e}"))?;
        }
        self.forget_materials(rel);
        self.forget_prefabs(rel);
        Ok(FileOp::Delete {
            rel: rel.to_owned(),
            trash,
            rewritten,
            history,
        })
    }

    fn history(&self) -> History {
        History {
            scene: self.scene.clone(),
            committed: self.committed.clone(),
            saved: self.saved.clone(),
            undo: self.edit_scenes().cloned().collect(),
        }
    }

    /// Puts back the scenes of `history`. The edit steps are matched from the
    /// top: the oldest may have dropped off the history since.
    fn restore_history(&mut self, history: History) {
        self.scene = history.scene;
        self.committed = history.committed;
        self.saved = history.saved;
        let steps = self.undo.iter_mut().rev().filter_map(|s| match s {
            Step::Edit(scene, _) => Some(scene),
            Step::File(_) => None,
        });
        for (now, old) in steps.zip(history.undo.into_iter().rev()) {
            *now = old;
        }
        self.after_history_jump();
    }

    fn edit_scenes(&self) -> impl Iterator<Item = &SceneFile> {
        self.undo.iter().filter_map(|s| match s {
            Step::Edit(scene, _) => Some(scene),
            Step::File(_) => None,
        })
    }

    /// A fresh folder in this session's trash.
    fn trash_slot(&self) -> Result<PathBuf, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let trash = trash_dir(self.project.as_deref().ok_or("no project")?);
        static SESSION: OnceLock<String> = OnceLock::new();
        let session = SESSION.get_or_init(|| {
            let secs = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            format!("{}-{secs}", std::process::id())
        });
        let slot = trash
            .join(session)
            .join(NEXT.fetch_add(1, Ordering::Relaxed).to_string());
        std::fs::create_dir_all(&slot).map_err(|e| format!("{}: {e}", slot.display()))?;
        Ok(slot)
    }
}

fn trash_dir(project: &Path) -> PathBuf {
    project.join(".olvin").join("trash")
}

/// Empties the trash sessions of `project` older than a day: deleted assets
/// stay recoverable for the session that deleted them and a while after.
pub(super) fn purge_trash(project: &Path) {
    let Ok(read) = std::fs::read_dir(trash_dir(project)) else {
        return;
    };
    for entry in read.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > TRASH_KEEP);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// `text` with the quoted path `from` (or anything under it) renamed to `to`;
/// `None` when it does not occur.
fn replace_path(text: &str, from: &str, to: &str) -> Option<String> {
    // Also inside the RON text of a prefab override (`"\"path\""`).
    let new = text
        .replace(&format!("\\\"{from}\\\""), &format!("\\\"{to}\\\""))
        .replace(&format!("\\\"{from}/"), &format!("\\\"{to}/"))
        .replace(&format!("\"{from}\""), &format!("\"{to}\""))
        .replace(&format!("\"{from}/"), &format!("\"{to}/"));
    (new != text).then_some(new)
}

/// Scene and material files name assets by path: follow a move.
fn rewrite_references(root: &Path, from: &str, to: &str) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if (is_scene(&path)
                || is_prefab(&path)
                || path.to_string_lossy().ends_with(".mat.ron"))
                && let Ok(text) = std::fs::read_to_string(&path)
                && let Some(new) = replace_path(&text, from, to)
            {
                let _ = std::fs::write(&path, new);
            }
        }
    }
}

/// Every `*.prefab.ron` under `dir`, relative to `root` and starting with `prefix`.
fn prefabs_under(root: &Path, dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            prefabs_under(root, &path, prefix, out);
        } else if is_prefab(&path)
            && let Ok(rel) = path.strip_prefix(root)
        {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel == prefix || rel.starts_with(&format!("{prefix}/")) {
                out.push(rel);
            }
        }
    }
}

/// The prefabs `gone` are about to be deleted: their instances stop being
/// instances and keep their objects, in the open scene and its history and in
/// every scene and prefab file that uses them. Returns the rewritten files
/// with their old text.
fn unpack_instances(
    state: &mut EditorState,
    root: &Path,
    gone: &[String],
) -> Vec<(PathBuf, String)> {
    let mut rewritten = Vec::new();
    if gone.is_empty() {
        return rewritten;
    }
    for asset in gone {
        state.map_scenes(|scene| scene.unpack_asset(asset));
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(own) = path
                .strip_prefix(root)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
            else {
                continue;
            };
            if !(is_scene(&path) || is_prefab(&path)) || gone.contains(&own) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if !gone.iter().any(|a| text.contains(a.as_str())) {
                continue;
            }
            let mut load = |p: &str| SceneFile::load(root.join(p));
            if let Ok(mut scene) = SceneFile::load(&path) {
                // The files are still there: expand, detach, write back as a file.
                scene.expand_prefabs(&mut load);
                for asset in gone {
                    scene.unpack_asset(asset);
                }
                match scene.collapse_prefabs(&mut load).save(&path) {
                    Ok(()) => rewritten.push((path, text)),
                    Err(e) => state.warn(format!("{own}: {e}")),
                }
            }
        }
    }
    rewritten
}

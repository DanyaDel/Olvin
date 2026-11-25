//! A safety copy of the edited scene, written when Play starts with unsaved
//! changes: whatever takes the process down during Play (an abort, a driver
//! reset), the edits survive. The scene file itself is never touched; the copy
//! lives in `<project>/.olvin/autosave/` and goes away when the scene is saved
//! or its changes are discarded.

use std::path::PathBuf;

use olvin_scene::file::SceneFile;

use super::EditorState;

impl EditorState {
    /// Where the copy of the open scene goes: its path under `assets/`, or
    /// its file name when it lies elsewhere.
    pub(crate) fn autosave_path(&self) -> Option<PathBuf> {
        let project = self.project.as_ref()?;
        let scene = self.scene_path.as_ref()?;
        let rel = self
            .assets_dir()
            .and_then(|a| scene.strip_prefix(a).ok().map(PathBuf::from))
            .or_else(|| scene.file_name().map(PathBuf::from))?;
        Some(project.join(".olvin").join("autosave").join(rel))
    }

    /// Writes the copy when there is something unsaved.
    pub(crate) fn autosave(&mut self) {
        if !self.is_dirty() {
            return;
        }
        let Some(path) = self.autosave_path() else {
            return;
        };
        let scene = self.file_form(&self.scene.clone());
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .map_err(|e| e.to_string())
            .and_then(|()| scene.save(&path));
        match written {
            Ok(()) => self.info(format!("autosaved to {}", path.display())),
            Err(e) => self.warn(format!("autosave failed: {e}")),
        }
    }

    /// The copy is no longer needed: the scene was saved or its changes dropped.
    pub fn drop_autosave(&mut self) {
        if let Some(path) = self.autosave_path() {
            let _ = std::fs::remove_file(path);
        }
        self.recovery = None;
    }

    /// After opening a scene: a copy newer than the file and different from
    /// it means the last session ended with unsaved changes.
    pub(super) fn check_autosave(&mut self) {
        self.recovery = None;
        let (Some(copy), Some(file)) = (self.autosave_path(), self.scene_path.clone()) else {
            return;
        };
        let modified = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let Some(copy_time) = modified(&copy) else {
            return;
        };
        let newer = modified(&file).is_none_or(|t| copy_time > t);
        let differs = std::fs::read(&copy).ok() != std::fs::read(&file).ok();
        if newer && differs {
            self.warn(format!(
                "unsaved changes of this scene from an earlier session were kept in {}: File → Recover Autosave",
                copy.display()
            ));
            self.recovery = Some(copy);
        }
    }

    /// Loads the copy over the open scene, as one undo step (undo goes back
    /// to the file). The scene stays unsaved until Save.
    pub fn recover_autosave(&mut self) {
        let Some(copy) = self.recovery.take() else {
            return;
        };
        match SceneFile::load(&copy) {
            Ok(mut scene) => {
                self.expand_prefabs(&mut scene);
                self.commit();
                self.scene = scene;
                self.commit();
                self.scene_generation += 1;
                self.info("recovered the autosaved scene: save it to keep it");
            }
            Err(e) => self.error(format!("could not read the autosave: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewEntity;
    use olvin_scene::file::Shape;

    #[test]
    fn play_keeps_a_copy_that_the_next_session_offers_back() {
        let root = std::env::temp_dir().join(format!("olvin-autosave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("assets/scenes")).unwrap();
        let mut s = EditorState::new(Some(root.clone()));
        assert!(s.save(), "the starter scene goes to disk");
        let n = s.scene.entities.len();
        s.create(NewEntity::Primitive(Shape::Cube), None);
        s.end_frame(false);
        s.start_play();
        s.stop_play();
        let copy = s.autosave_path().unwrap();
        assert!(copy.starts_with(root.join(".olvin/autosave")), "{copy:?}");
        assert!(copy.exists(), "unsaved changes were copied before Play");

        // "Crash": a new session on the same project, the file has the old scene.
        std::thread::sleep(std::time::Duration::from_millis(20));
        filetime_touch(&copy);
        let mut s = EditorState::new(Some(root.clone()));
        assert_eq!(s.scene.entities.len(), n);
        assert!(s.recovery.is_some(), "the copy is offered");
        s.recover_autosave();
        assert_eq!(s.scene.entities.len(), n + 1);
        assert!(s.is_dirty());
        s.undo();
        assert_eq!(s.scene.entities.len(), n, "undo goes back to the file");
        s.redo();
        assert!(s.save());
        assert!(!copy.exists(), "saving drops the copy");

        // Nothing unsaved: Play writes no copy.
        s.start_play();
        s.stop_play();
        assert!(!copy.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// Makes the copy strictly newer than the scene file on coarse clocks.
    fn filetime_touch(path: &std::path::Path) {
        let text = std::fs::read(path).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

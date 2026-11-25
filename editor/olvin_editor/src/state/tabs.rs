//! Several scenes open at once, one tab each, like Godot. The active scene
//! lives in `EditorState`'s own fields (so everything that reads
//! `state.scene` keeps working); the others wait in `tabs`, and switching
//! swaps them. Each scene keeps its own history, selection and previews;
//! material and prefab assets are shared by all of them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use olvin_scene::Transform;
use olvin_scene::file::SceneFile;

use super::{EditorState, Step, starter_scene};

/// A scene that is open but not active (its slot holds an empty one while
/// it is active).
#[derive(Default)]
pub(crate) struct SceneDoc {
    pub scene: SceneFile,
    pub path: Option<PathBuf>,
    pub saved: SceneFile,
    pub committed: SceneFile,
    pub undo: Vec<Step>,
    pub redo: Vec<Step>,
    selection: Option<u64>,
    multi: Vec<u64>,
    preview: HashMap<u64, Transform>,
    model_previews: HashMap<u64, crate::engine::ModelPreview>,
    anim_frame: i32,
    recovery: Option<PathBuf>,
    mesh_selection: Option<(u64, Vec<u32>)>,
}

impl SceneDoc {
    fn is_dirty(&self) -> bool {
        self.scene != self.saved
    }
}

/// What the tab strip shows for one open scene.
pub struct TabInfo {
    pub title: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub active: bool,
}

fn title_of(path: Option<&Path>) -> String {
    path.and_then(|p| p.file_name())
        .map(|n| {
            n.to_string_lossy()
                .trim_end_matches(".scene.ron")
                .to_owned()
        })
        .unwrap_or_else(|| "untitled".into())
}

impl EditorState {
    /// Swaps the active scene's state with the doc in slot `active_tab`.
    fn swap_active(&mut self) {
        let doc = &mut self.tabs[self.active_tab];
        std::mem::swap(&mut self.scene, &mut doc.scene);
        std::mem::swap(&mut self.scene_path, &mut doc.path);
        std::mem::swap(&mut self.saved, &mut doc.saved);
        std::mem::swap(&mut self.committed, &mut doc.committed);
        std::mem::swap(&mut self.undo, &mut doc.undo);
        std::mem::swap(&mut self.redo, &mut doc.redo);
        std::mem::swap(&mut self.selection, &mut doc.selection);
        std::mem::swap(&mut self.multi, &mut doc.multi);
        std::mem::swap(&mut self.preview, &mut doc.preview);
        std::mem::swap(&mut self.model_previews, &mut doc.model_previews);
        std::mem::swap(&mut self.anim_frame, &mut doc.anim_frame);
        std::mem::swap(&mut self.recovery, &mut doc.recovery);
        std::mem::swap(&mut self.mesh_selection, &mut doc.mesh_selection);
    }

    /// After another scene became active: asset edits so far belong to no
    /// step of it, and per-scene caches start over.
    fn after_switch(&mut self) {
        self.committed_edits = self.edits_now();
        self.prefab_diff = Default::default();
        self.scene_generation += 1;
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn active_tab(&self) -> usize {
        self.active_tab
    }

    /// Every open scene, in tab order.
    pub fn tabs(&self) -> Vec<TabInfo> {
        (0..self.tabs.len())
            .map(|i| {
                if i == self.active_tab {
                    TabInfo {
                        title: title_of(self.scene_path.as_deref()),
                        path: self.scene_path.clone(),
                        dirty: self.is_dirty(),
                        active: true,
                    }
                } else {
                    let doc = &self.tabs[i];
                    TabInfo {
                        title: title_of(doc.path.as_deref()),
                        path: doc.path.clone(),
                        dirty: doc.is_dirty(),
                        active: false,
                    }
                }
            })
            .collect()
    }

    /// Some open scene, active or not, has unsaved changes.
    pub fn any_dirty(&self) -> bool {
        self.is_dirty()
            || self
                .tabs
                .iter()
                .enumerate()
                .any(|(i, d)| i != self.active_tab && d.is_dirty())
    }

    /// Makes tab `i` the active scene. Not while playing: the game belongs
    /// to the active scene.
    pub fn switch_tab(&mut self, i: usize) {
        if i == self.active_tab || i >= self.tabs.len() || self.play.is_some() {
            return;
        }
        self.commit();
        self.swap_active();
        self.active_tab = i;
        self.swap_active();
        self.after_switch();
    }

    /// The tab showing `path`, if it is open.
    pub(super) fn tab_of(&self, path: &Path) -> Option<usize> {
        (0..self.tabs.len()).find(|&i| {
            let open = if i == self.active_tab {
                self.scene_path.as_deref()
            } else {
                self.tabs[i].path.as_deref()
            };
            open == Some(path)
        })
    }

    /// An untouched scene that was never saved (the starter scene of a fresh
    /// project, a new scene nobody edited): opening another one replaces it
    /// instead of leaving it as a tab.
    fn active_is_blank(&self) -> bool {
        // A never-saved scene counts as unsaved (Save is offered), so "blank"
        // means nothing was done to it, not "not dirty".
        self.undo.is_empty()
            && self.redo.is_empty()
            && self.scene == self.committed
            && self.scene_path.as_ref().is_none_or(|p| !p.exists())
    }

    /// Opens a new, empty tab and makes it active; the caller fills it.
    fn push_tab(&mut self) {
        self.commit();
        self.swap_active();
        self.tabs.push(SceneDoc::default());
        self.active_tab = self.tabs.len() - 1;
        self.swap_active();
        self.after_switch();
    }

    /// Where a new scene goes: `untitled`, then `untitled-2`... free on disk
    /// and among the open tabs.
    pub(super) fn untitled_path(&self) -> Option<PathBuf> {
        let dir = self.default_scene_path()?.parent()?.to_path_buf();
        (1..)
            .map(|n| match n {
                1 => dir.join("untitled.scene.ron"),
                n => dir.join(format!("untitled-{n}.scene.ron")),
            })
            .find(|p| !p.exists() && self.tab_of(p).is_none())
    }

    /// Makes the active slot free for a scene being opened: a new tab,
    /// unless the active scene is blank (then it is replaced).
    pub(super) fn make_room(&mut self) {
        if self.play.is_some() {
            self.stop_play();
        }
        if !self.active_is_blank() {
            self.push_tab();
        }
    }

    /// A new scene, always in a tab of its own (asked for explicitly).
    pub(super) fn new_tab(&mut self) {
        if self.play.is_some() {
            self.stop_play();
        }
        self.push_tab();
        let path = self.untitled_path();
        self.replace_scene(starter_scene(), path, false);
    }

    /// Closes the active scene, dropping its unsaved changes (ask first).
    /// The last tab is never closed: it becomes a new scene instead.
    pub fn close_active_tab(&mut self) {
        if self.play.is_some() {
            self.stop_play();
        }
        if self.is_dirty() {
            self.drop_autosave();
        }
        let name = title_of(self.scene_path.as_deref());
        if self.tabs.len() == 1 {
            let path = self.untitled_path();
            self.replace_scene(starter_scene(), path, false);
        } else {
            let closing = self.active_tab;
            let next = if closing + 1 < self.tabs.len() {
                closing + 1
            } else {
                closing - 1
            };
            // The neighbour becomes active; the closed scene sits in its slot.
            self.switch_tab(next);
            self.tabs.remove(closing);
            if self.active_tab > closing {
                self.active_tab -= 1;
            }
        }
        self.info(format!("closed {name}"));
    }

    /// Quitting without saving: the autosaves of every changed scene go too,
    /// so the next session does not offer them back.
    pub fn discard_all(&mut self) {
        let back = self.active_tab;
        for i in 0..self.tabs.len() {
            self.switch_tab(i);
            if self.is_dirty() {
                self.drop_autosave();
            }
        }
        self.switch_tab(back);
    }

    /// Saves every open scene with unsaved changes; `false` if one failed
    /// (that one is left active).
    pub fn save_all(&mut self) -> bool {
        let back = self.active_tab;
        for i in 0..self.tabs.len() {
            let dirty = if i == self.active_tab {
                self.is_dirty()
            } else {
                self.tabs[i].is_dirty()
            };
            if dirty {
                self.switch_tab(i);
                if !self.save() {
                    return false;
                }
            }
        }
        self.switch_tab(back);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewEntity;
    use olvin_scene::file::{EntityDesc, Shape};

    /// A project with scenes `a` and `b` (one cube each, `b`'s uses a material).
    fn project(name: &str) -> (PathBuf, EditorState) {
        let root = std::env::temp_dir().join(format!("olvin-tabs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let scenes = root.join("assets/scenes");
        std::fs::create_dir_all(&scenes).unwrap();
        std::fs::create_dir_all(root.join("assets/materials")).unwrap();
        olvin_scene::material::MaterialDesc::default()
            .save(root.join("assets/materials/red.mat.ron"))
            .unwrap();
        for (file, id) in [("a", 1), ("b", 2)] {
            let mut cube = EntityDesc::new(id, file);
            cube.mesh = Some(olvin_scene::file::MeshDesc {
                shape: Shape::Cube,
                color: [1.0; 4],
            });
            if file == "b" {
                cube.material = Some("materials/red.mat.ron".into());
            }
            SceneFile {
                entities: vec![cube],
                ..Default::default()
            }
            .save(scenes.join(format!("{file}.scene.ron")))
            .unwrap();
        }
        let state = EditorState::new(Some(root.clone()));
        (root, state)
    }

    fn scene(root: &Path, name: &str) -> PathBuf {
        root.join(format!("assets/scenes/{name}.scene.ron"))
    }

    #[test]
    fn scenes_open_in_tabs_with_their_own_history_and_selection() {
        let (root, mut s) = project("history");
        // The starter scene of the fresh project is blank: `a` replaces it.
        s.open(&scene(&root, "a"));
        assert_eq!(s.tab_count(), 1);
        let sphere = s.create(NewEntity::Primitive(Shape::Sphere), None);
        s.end_frame(false);
        assert!(s.is_dirty());
        assert_eq!(s.selection, Some(sphere));

        s.open(&scene(&root, "b"));
        assert_eq!((s.tab_count(), s.active_tab()), (2, 1));
        assert_eq!(s.scene.entities.len(), 1, "b as on disk");
        assert!(
            !s.is_dirty() && !s.can_undo(),
            "b has its own clean history"
        );
        assert!(s.any_dirty(), "a still has unsaved changes");
        let tabs = s.tabs();
        assert_eq!(tabs[0].title, "a");
        assert!(tabs[0].dirty && !tabs[1].dirty && tabs[1].active);

        // Opening an open scene switches to it; a's edit and selection are back.
        s.open(&scene(&root, "a"));
        assert_eq!((s.tab_count(), s.active_tab()), (2, 0));
        assert_eq!(s.scene.entities.len(), 2);
        assert_eq!(s.selection, Some(sphere), "a's selection came back");
        s.undo();
        assert_eq!(s.scene.entities.len(), 1, "undo stays within a");
        s.switch_tab(1);
        assert_eq!(s.scene.entities[0].name, "b");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn closing_tabs_and_new_scenes() {
        let (root, mut s) = project("close");
        s.open(&scene(&root, "a"));
        s.new_scene();
        s.new_scene();
        let titles: Vec<String> = s.tabs().into_iter().map(|t| t.title).collect();
        assert_eq!(
            titles,
            ["a", "untitled", "untitled-2"],
            "new scenes get free names"
        );
        s.switch_tab(1);
        s.close_active_tab();
        assert_eq!(s.tab_count(), 2);
        assert_eq!(
            s.tabs()[s.active_tab()].title,
            "untitled-2",
            "the neighbour takes over"
        );
        s.close_active_tab();
        assert_eq!(s.tabs()[0].title, "a");
        // The last tab becomes a new scene instead of disappearing.
        s.close_active_tab();
        assert_eq!(s.tab_count(), 1);
        assert_eq!(s.tabs()[0].title, "untitled");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn asset_moves_reach_scenes_in_other_tabs() {
        let (root, mut s) = project("move");
        s.open(&scene(&root, "b"));
        s.open(&scene(&root, "a"));
        assert_eq!(s.active_tab(), 1);
        let assets = root.join("assets");
        s.move_asset(
            &assets,
            &assets.join("materials/red.mat.ron"),
            &assets.join("materials/blue.mat.ron"),
        )
        .unwrap();
        s.switch_tab(0);
        assert_eq!(
            s.scene.entities[0].material.as_deref(),
            Some("materials/blue.mat.ron"),
            "the inactive tab follows the move"
        );
        assert!(!s.is_dirty(), "and it is not an unsaved change there");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn switching_is_off_while_playing_and_saving_all() {
        let (root, mut s) = project("play");
        s.open(&scene(&root, "a"));
        s.open(&scene(&root, "b"));
        s.create(NewEntity::Primitive(Shape::Sphere), None);
        s.end_frame(false);
        s.switch_tab(0);
        s.create(NewEntity::Primitive(Shape::Sphere), None);
        s.end_frame(false);
        s.start_play();
        s.switch_tab(1);
        assert_eq!(s.active_tab(), 0, "the game belongs to the active scene");
        s.stop_play();
        assert!(s.save_all());
        assert!(!s.any_dirty());
        assert_eq!(s.active_tab(), 0, "save all comes back to the active tab");
        for name in ["a", "b"] {
            assert_eq!(
                SceneFile::load(scene(&root, name)).unwrap().entities.len(),
                2
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }
}

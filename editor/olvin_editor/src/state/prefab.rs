//! Prefab work of the editor state. The open scene holds instances *expanded*
//! (every node an ordinary entity, so panels and tools need no special cases);
//! a node's edits are the instance's overrides, found again by comparing with
//! the asset when the scene is saved (`olvin_scene::prefab`).

use std::collections::HashMap;

use glam::Vec3;
use olvin_scene::file::{EntityDesc, PrefabInstance, SceneFile};

use super::EditorState;
use crate::prefabs::{POLL, PrefabLibrary};

/// What the inspector shows about the prefab instance an object belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct PrefabReport {
    /// The instance's root entity.
    pub root: u64,
    /// The prefab asset, relative to `assets/`.
    pub asset: String,
    /// The object is the instance's root (not a node inside it).
    pub is_root: bool,
    /// Why the asset could not be read; the instance is a single object then.
    pub missing: Option<String>,
    /// Fields of this object that differ from the asset: `position`, `mesh`,
    /// `components.Door`...
    pub overridden: Vec<String>,
    /// Differences in the whole instance.
    pub overrides: usize,
    pub removed: usize,
    pub added: usize,
}

/// The last instance compared with its asset: the comparison is redone when a node changes.
#[derive(Default)]
pub(super) struct DiffCache {
    root: u64,
    members: Vec<EntityDesc>,
    diff: Option<PrefabInstance>,
}

/// `scene` collapsed against `old` (the prefabs it was expanded from) and
/// expanded again against `lib`'s current ones.
fn reexpand(
    scene: &mut SceneFile,
    old: &HashMap<String, SceneFile>,
    lib: &mut PrefabLibrary,
) -> Vec<String> {
    if !scene.entities.iter().any(|e| e.prefab.is_some()) {
        return Vec::new();
    }
    let mut collapsed = scene.collapse_prefabs(&mut |rel| {
        old.get(rel)
            .cloned()
            .ok_or_else(|| format!("{rel}: not loaded"))
    });
    let warnings = collapsed.expand_prefabs(&mut lib.loader());
    *scene = collapsed;
    warnings
}

impl EditorState {
    /// A node of a prefab instance, not its root: it stays where the prefab puts it.
    pub fn is_prefab_member(&self, id: u64) -> bool {
        self.scene
            .get(id)
            .and_then(|e| e.prefab_node.as_ref())
            .is_some_and(|n| !n.path.is_empty())
    }

    /// The outermost prefab instance root that `id` belongs to (`id` itself
    /// when it is not a prefab member).
    pub fn prefab_outer_root(&self, id: u64) -> u64 {
        let mut id = id;
        // Bounded: a broken file must not loop the viewport forever.
        for _ in 0..64 {
            match self.scene.get(id).and_then(|e| e.prefab_node.as_ref()) {
                Some(n) if !n.path.is_empty() && n.root != id => id = n.root,
                _ => break,
            }
        }
        id
    }

    /// What a viewport click on `hit` selects. The first click on a prefab
    /// instance takes its root, so the whole instance moves; a click on an
    /// instance whose root or member is already selected takes the node itself.
    pub fn click_target(&self, hit: u64) -> u64 {
        let root = self.prefab_outer_root(hit);
        let inside = self
            .selection
            .is_some_and(|s| s == root || self.prefab_outer_root(s) == root);
        if inside { hit } else { root }
    }

    /// Expands the prefab instances of a freshly read scene, logging problems.
    pub(super) fn expand_prefabs(&mut self, scene: &mut SceneFile) {
        let warnings = scene.expand_prefabs(&mut self.prefabs.loader());
        for warning in warnings {
            self.warn(warning);
        }
    }

    /// Expands the instances that were just added to the open scene.
    pub(super) fn expand_prefabs_now(&mut self) {
        let mut scene = std::mem::take(&mut self.scene);
        self.expand_prefabs(&mut scene);
        self.scene = scene;
    }

    /// `scene` as the file stores it: instances reduced to their roots and overrides.
    pub(super) fn file_form(&mut self, scene: &SceneFile) -> SceneFile {
        if !scene.entities.iter().any(|e| e.prefab.is_some()) {
            return scene.clone();
        }
        scene.collapse_prefabs(&mut self.prefabs.loader())
    }

    /// Adds an instance of the prefab `rel` to the scene (at `at`, under `parent`) and selects it.
    pub fn instantiate_prefab(
        &mut self,
        rel: &str,
        parent: Option<u64>,
        at: Vec3,
    ) -> Result<u64, String> {
        self.prefabs.get(rel).map_err(|e| format!("prefab: {e}"))?;
        let own = self.assets_dir().zip(self.scene_path.as_ref());
        if own.is_some_and(|(dir, path)| *path == dir.join(rel)) {
            return Err("a prefab cannot contain itself".into());
        }
        let id = self.next_id();
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let mut e = EntityDesc::new(id, name.trim_end_matches(".prefab.ron"));
        e.parent = parent.filter(|p| self.scene.get(*p).is_some());
        e.position = at;
        e.prefab = Some(PrefabInstance::new(rel));
        self.scene.entities.push(e);
        self.expand_prefabs_now();
        self.select(Some(id));
        Ok(id)
    }

    /// Saves the object and its children as `prefabs/<name>.prefab.ron` and
    /// turns the object into an instance of it. Returns the asset's path.
    pub fn create_prefab(&mut self, id: u64) -> Result<String, String> {
        let e = self.scene.get(id).ok_or("no such object")?;
        if e.prefab.is_some() || e.prefab_node.is_some() {
            return Err("already part of a prefab: unpack it first".into());
        }
        let rel = self
            .prefabs
            .unique_path(&e.name)
            .ok_or("open a project first")?;
        let inside: Vec<u64> = self
            .scene
            .entities
            .iter()
            .filter(|e| e.id == id || self.is_descendant(e.id, id))
            .map(|e| e.id)
            .collect();
        let scene = self.scene.clone();
        let file = self.file_form(&scene);
        let nodes: Vec<&EntityDesc> = file
            .entities
            .iter()
            .filter(|e| inside.contains(&e.id))
            .collect();
        let local: HashMap<u64, u64> = nodes.iter().map(|e| e.id).zip(1..).collect();
        let mut asset = SceneFile::default();
        for node in nodes {
            let mut copy = node.clone();
            copy.id = local[&node.id];
            copy.parent = node.parent.and_then(|p| local.get(&p).copied());
            if node.id == id {
                // The root is placed by each instance.
                copy.active = true;
                (copy.position, copy.rotation, copy.scale) = (Vec3::ZERO, Vec3::ZERO, Vec3::ONE);
            }
            asset.entities.push(copy);
        }
        self.prefabs.write(&rel, &asset)?;

        // The object becomes the instance's root; its subtree comes from the asset now.
        self.scene
            .entities
            .retain(|e| e.id == id || !inside.contains(&e.id));
        if let Some(root) = self.scene.get_mut(id) {
            let mut lean = EntityDesc::new(id, root.name.clone());
            lean.active = root.active;
            lean.parent = root.parent;
            lean.position = root.position;
            lean.rotation = root.rotation;
            lean.scale = root.scale;
            lean.prefab = Some(PrefabInstance::new(rel.clone()));
            *root = lean;
        }
        self.expand_prefabs_now();
        self.prefab_diff = DiffCache::default();
        self.info(format!("created {rel}"));
        Ok(rel)
    }

    /// Create Prefab on the selected object; problems go to the log.
    pub fn create_prefab_selected(&mut self) {
        if let Some(id) = self.selection
            && let Err(err) = self.create_prefab(id)
        {
            self.warn(format!("create prefab: {err}"));
        }
    }

    /// The instance `id` belongs to and its asset, if any.
    fn prefab_root_and_asset(&self, id: u64) -> Option<(u64, String)> {
        let e = self.scene.get(id)?;
        let root = match (&e.prefab_node, &e.prefab) {
            (Some(n), _) => n.root,
            (None, Some(_)) => id,
            _ => return None,
        };
        let asset = self.scene.get(root)?.prefab.as_ref()?.asset.clone();
        Some((root, asset))
    }

    /// How `id` differs from its prefab; `None` when it is not part of one.
    pub fn prefab_report(&mut self, id: u64) -> Option<PrefabReport> {
        let (root, asset) = self.prefab_root_and_asset(id)?;
        let node = self.scene.get(id)?.prefab_node.clone();
        let is_root = node.as_ref().is_none_or(|n| n.path.is_empty());
        let expanded = self.scene.get(root)?.prefab_node.is_some();
        let mut report = PrefabReport {
            root,
            asset: asset.clone(),
            is_root,
            missing: None,
            overridden: Vec::new(),
            overrides: 0,
            removed: 0,
            added: 0,
        };
        if !expanded {
            report.missing = Some(
                self.prefabs
                    .get(&asset)
                    .err()
                    .unwrap_or_else(|| "not expanded".into()),
            );
            return Some(report);
        }
        let (diff, added) = self.instance_diff(root)?;
        report.overrides = diff.overrides.values().map(|f| f.len()).sum();
        report.removed = diff.removed.len();
        report.added = added;
        let path = node.map(|n| n.path).unwrap_or_default();
        report.overridden = diff
            .overrides
            .get(&path)
            .map(|f| f.keys().cloned().collect())
            .unwrap_or_default();
        Some(report)
    }

    /// The instance's differences and the number of children added to it.
    fn instance_diff(&mut self, root: u64) -> Option<(PrefabInstance, usize)> {
        let nodes = || {
            self.scene
                .entities
                .iter()
                .filter(move |e| e.prefab_node.as_ref().is_some_and(|n| n.root == root))
        };
        let cache = &self.prefab_diff;
        let hit = cache.root == root
            && cache.diff.is_some()
            && cache.members.len() == nodes().count()
            && cache.members.iter().zip(nodes()).all(|(a, b)| a == b);
        if !hit {
            let diff = self
                .scene
                .prefab_diff(root, &mut self.prefabs.loader())
                .ok()?;
            self.prefab_diff = DiffCache {
                root,
                members: nodes().cloned().collect(),
                diff: Some(diff),
            };
        }
        let added = self.scene.prefab_added(root).len();
        Some((self.prefab_diff.diff.clone()?, added))
    }

    /// Apply All: writes the instance's changes into the prefab asset; every
    /// instance of it in the scene follows. The write joins the undo history.
    pub fn apply_prefab(&mut self, root: u64) -> Result<(), String> {
        let (_, asset) = self.prefab_root_and_asset(root).ok_or("not a prefab")?;
        let new = self.scene.apply_prefab(root, &mut self.prefabs.loader())?;
        let added = self.scene.prefab_added(root);
        let old = self.prefabs.snapshot();
        self.prefabs.write(&asset, &new)?;
        self.scene.entities.retain(|e| !added.contains(&e.id));
        self.refresh_instances(&old, false);
        self.info(format!("applied to {asset}"));
        Ok(())
    }

    /// Revert All: the instance goes back to what its asset says.
    pub fn revert_prefab(&mut self, root: u64) {
        self.scene.revert_prefab(root);
        self.expand_prefabs_now();
        self.after_instances_changed();
    }

    /// Puts one field of an instance's node back to the asset's value.
    pub fn revert_prefab_field(&mut self, id: u64, key: &str) {
        let result = self
            .scene
            .revert_prefab_field(id, key, &mut self.prefabs.loader());
        if let Err(err) = result {
            self.warn(format!("revert {key}: {err}"));
        }
    }

    /// Detaches the instance from its asset: its objects become ordinary ones.
    pub fn unpack_prefab(&mut self, root: u64) {
        self.scene.unpack_prefab(root);
        self.after_instances_changed();
    }

    /// Opens the prefab asset as a scene to edit (instances follow on save).
    pub fn open_prefab_asset(&mut self, rel: &str) {
        if let Some(dir) = self.assets_dir() {
            self.request_open(dir.join(rel));
        }
    }

    /// Asks the Assets panel to show the prefab asset.
    pub fn select_prefab_asset(&mut self, rel: &str) {
        if let Some(dir) = self.assets_dir() {
            self.asset_focus = Some(dir.join(rel));
        }
    }

    /// Instances follow prefab files changed behind the editor's back (another
    /// program, the IDE, saving the prefab opened as a scene). Checked a few
    /// times a second; `force` checks now.
    pub fn sync_prefabs(&mut self, force: bool) {
        if self.play.is_some() || (!force && self.prefab_polled.elapsed() < POLL) {
            return;
        }
        self.prefab_polled = std::time::Instant::now();
        let changed = self.prefabs.changed();
        if changed.is_empty() {
            return;
        }
        let old = self.prefabs.snapshot();
        for rel in &changed {
            self.prefabs.reload(rel);
        }
        self.refresh_instances(&old, true);
        self.info(format!("prefab changed on disk: {}", changed.join(", ")));
    }

    /// Re-expands the instances of the open scene (and, for an outside change,
    /// the undo baseline too) against the prefabs now in the library; `old`
    /// holds what they were expanded from.
    fn refresh_instances(&mut self, old: &HashMap<String, SceneFile>, baseline: bool) {
        let mut warnings = reexpand(&mut self.scene, old, &mut self.prefabs);
        reexpand(&mut self.saved, old, &mut self.prefabs);
        if baseline {
            reexpand(&mut self.committed, old, &mut self.prefabs);
        }
        for warning in warnings.drain(..) {
            self.warn(warning);
        }
        self.after_instances_changed();
    }

    fn after_instances_changed(&mut self) {
        self.prefab_diff = DiffCache::default();
        if self
            .selection
            .is_some_and(|id| self.scene.get(id).is_none())
        {
            self.selection = None;
        }
        let scene = &self.scene;
        self.multi.retain(|id| scene.get(*id).is_some());
        self.scene_generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewEntity;
    use olvin_scene::file::Shape;

    fn project(name: &str) -> (std::path::PathBuf, EditorState) {
        let root =
            std::env::temp_dir().join(format!("olvin-prefab-ed-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("assets")).unwrap();
        let mut state = EditorState::new(Some(root.clone()));
        // Not the starter scene.
        state.replace_scene(SceneFile::default(), None, false);
        (root, state)
    }

    /// A "Crate" with a "Lid" child, turned into a prefab. Returns the instance's root id.
    fn crate_instance(s: &mut EditorState) -> (u64, String) {
        let root = s.create(NewEntity::Primitive(Shape::Cube), None);
        s.scene.get_mut(root).unwrap().name = "Crate".into();
        s.scene.get_mut(root).unwrap().position = glam::Vec3::new(3.0, 0.0, 0.0);
        let lid = s.create(NewEntity::Primitive(Shape::Plane), Some(root));
        s.scene.get_mut(lid).unwrap().name = "Lid".into();
        s.end_frame(false);
        let rel = s.create_prefab(root).unwrap();
        s.end_frame(false);
        (root, rel)
    }

    fn lid_of(_: &EditorState, root: u64) -> u64 {
        olvin_scene::prefab::member_id(root, 2)
    }

    #[test]
    fn viewport_click_takes_the_root_first_then_the_node() {
        let (root_dir, mut s) = project("click");
        let (root, rel) = crate_instance(&mut s);
        let lid = lid_of(&s, root);
        assert!(s.is_prefab_member(lid), "{rel}");
        assert_eq!(s.prefab_outer_root(lid), root);
        s.select(None);
        assert_eq!(s.click_target(lid), root);
        s.select(Some(root));
        assert_eq!(s.click_target(lid), lid);
        s.select(Some(lid));
        assert_eq!(s.click_target(root), root);
        // A plain object is its own target.
        let other = s.create(NewEntity::Primitive(Shape::Cube), None);
        assert_eq!(s.click_target(other), other);
        let _ = std::fs::remove_dir_all(root_dir);
    }

    #[test]
    fn create_prefab_saves_the_subtree_and_turns_the_object_into_an_instance() {
        let (dir, mut s) = project("create");
        let (root, rel) = crate_instance(&mut s);
        assert_eq!(rel, "prefabs/Crate.prefab.ron");
        let asset = SceneFile::load(dir.join("assets").join(&rel)).unwrap();
        assert_eq!(asset.entities.len(), 2);
        assert_eq!(
            asset.entities[0].position,
            Vec3::ZERO,
            "placed by each instance"
        );
        assert_eq!(asset.entities[1].parent, Some(asset.entities[0].id));
        // In the scene: the same two objects, as an instance.
        assert_eq!(s.scene.entities.len(), 2);
        assert_eq!(s.scene.get(root).unwrap().position.x, 3.0);
        assert!(s.scene.get(root).unwrap().prefab.is_some());
        assert!(s.is_prefab_member(lid_of(&s, root)));
        assert!(!s.is_prefab_member(root));
        // On disk the instance is one entity.
        s.scene_path = Some(dir.join("assets/scenes/a.scene.ron"));
        assert!(s.save());
        let saved = SceneFile::load(dir.join("assets/scenes/a.scene.ron")).unwrap();
        assert_eq!(saved.entities.len(), 1);
        assert!(!s.is_dirty());
        // Another instance, and a second prefab of the same name gets a new file.
        let two = s
            .instantiate_prefab(&rel, None, Vec3::new(0.0, 0.0, 5.0))
            .unwrap();
        assert_eq!(s.scene.entities.len(), 4);
        assert_eq!(s.scene.get(two).unwrap().name, "Crate");
        assert!(
            s.create_prefab(two).is_err(),
            "an instance is not made a prefab again"
        );
        let plain = s.create(NewEntity::Empty, None);
        s.scene.get_mut(plain).unwrap().name = "Crate".into();
        assert_eq!(
            s.create_prefab(plain).unwrap(),
            "prefabs/Crate_2.prefab.ron"
        );
        // Reopening gives the same expanded scene.
        let before = s.scene.clone();
        assert!(s.save());
        s.open(&dir.join("assets/scenes/a.scene.ron"));
        assert_eq!(s.scene, before);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn edits_are_overrides_that_can_be_reverted_one_by_one_or_all() {
        let (dir, mut s) = project("overrides");
        let (root, rel) = crate_instance(&mut s);
        let lid = lid_of(&s, root);
        let report = s.prefab_report(lid).unwrap();
        assert!(report.overridden.is_empty() && !report.is_root);
        assert_eq!(report.asset, rel);

        s.scene.get_mut(lid).unwrap().position.y = 2.0;
        s.scene.get_mut(lid).unwrap().name = "Top".into();
        s.scene.get_mut(root).unwrap().position.x = 9.0; // placing is no override
        let report = s.prefab_report(lid).unwrap();
        assert_eq!(report.overridden, ["name", "position"]);
        assert_eq!(report.overrides, 2);
        assert_eq!(
            s.prefab_report(root).unwrap().overridden,
            Vec::<String>::new()
        );

        s.revert_prefab_field(lid, "name");
        assert_eq!(s.scene.get(lid).unwrap().name, "Lid");
        assert_eq!(s.prefab_report(lid).unwrap().overridden, ["position"]);
        s.scene.entities.push(EntityDesc::new(77, "Ribbon"));
        s.scene.get_mut(77).unwrap().parent = Some(lid);
        assert_eq!(s.prefab_report(root).unwrap().added, 1);
        s.revert_prefab(root);
        assert_eq!(s.scene.entities.len(), 2, "the added child is gone");
        assert_eq!(s.scene.get(lid).unwrap().position.y, 0.0);
        assert_eq!(
            s.scene.get(root).unwrap().position.x,
            9.0,
            "the placement stays"
        );
        assert!(s.prefab_report(root).unwrap().overrides == 0);

        // Nodes of an instance keep their place in the hierarchy.
        let other = s.create(NewEntity::Empty, None);
        assert!(!s.reparent(lid, Some(other)));
        assert!(s.duplicate(lid).is_none());
        assert!(!s.move_sibling(lid, 1));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn apply_writes_the_asset_and_undo_restores_it() {
        let (dir, mut s) = project("apply");
        let (root, rel) = crate_instance(&mut s);
        let other = s
            .instantiate_prefab(&rel, None, Vec3::new(0.0, 0.0, 5.0))
            .unwrap();
        s.end_frame(false);
        let lid = lid_of(&s, root);
        let lid_other = lid_of(&s, other);
        let asset_path = dir.join("assets").join(&rel);
        let asset_lid_y = || {
            SceneFile::load(&asset_path)
                .unwrap()
                .entities
                .iter()
                .find(|e| e.name == "Lid")
                .unwrap()
                .position
                .y
        };
        assert_eq!(asset_lid_y(), 0.0);

        s.scene.get_mut(lid).unwrap().position.y = 4.0;
        s.end_frame(false);
        s.apply_prefab(root).unwrap();
        s.end_frame(false);
        assert_eq!(asset_lid_y(), 4.0, "written into the asset");
        assert_eq!(
            s.scene.get(lid_other).unwrap().position.y,
            4.0,
            "the other instance follows"
        );
        assert!(s.prefab_report(root).unwrap().overridden.is_empty());
        assert_eq!(s.prefab_report(lid).unwrap().overrides, 0);

        s.undo();
        assert_eq!(asset_lid_y(), 0.0, "undo rewrites the prefab file");
        assert_eq!(s.scene.get(lid_other).unwrap().position.y, 0.0);
        assert_eq!(
            s.scene.get(lid).unwrap().position.y,
            4.0,
            "the override is back"
        );
        assert_eq!(s.prefab_report(lid).unwrap().overridden, ["position"]);
        s.redo();
        assert_eq!(asset_lid_y(), 4.0);
        assert_eq!(s.scene.get(lid_other).unwrap().position.y, 4.0);
        // Nothing to pick up from the disk after our own writes.
        let before = s.scene.clone();
        s.sync_prefabs(true);
        assert_eq!(s.scene, before);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn outside_edits_of_a_prefab_update_the_instances() {
        let (dir, mut s) = project("outside");
        let (root, rel) = crate_instance(&mut s);
        let lid = lid_of(&s, root);
        s.scene.get_mut(lid).unwrap().position.x = 1.5; // an override that must survive
        let path = dir.join("assets").join(&rel);
        let mut asset = SceneFile::load(&path).unwrap();
        asset.entities[1].scale.y = 8.0;
        asset.entities[0].tag = "Loot".into();
        asset.save(&path).unwrap();
        // The modification time can be the same second: make the change visible.
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        s.sync_prefabs(true);
        assert_eq!(s.scene.get(lid).unwrap().scale.y, 8.0, "the asset's value");
        assert_eq!(
            s.scene.get(lid).unwrap().position.x,
            1.5,
            "the override stays"
        );
        assert_eq!(s.scene.get(root).unwrap().tag, "Loot");
        assert_eq!(s.prefab_report(lid).unwrap().overridden, ["position"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn duplicating_an_instance_copies_its_overrides_and_added_children() {
        let (dir, mut s) = project("dup");
        let (root, _) = crate_instance(&mut s);
        let lid = lid_of(&s, root);
        s.scene.get_mut(lid).unwrap().position.y = 6.0;
        let mut ribbon = EntityDesc::new(90, "Ribbon");
        ribbon.parent = Some(lid);
        s.scene.entities.push(ribbon);
        let copy = s.duplicate(root).unwrap();
        assert_ne!(copy, root);
        // root + lid + ribbon, twice.
        assert_eq!(s.scene.entities.len(), 6);
        let copy_lid = lid_of(&s, copy);
        assert_eq!(s.scene.get(copy_lid).unwrap().position.y, 6.0);
        let ribbon_copy = s
            .scene
            .entities
            .iter()
            .find(|e| e.name == "Ribbon" && e.id != 90)
            .unwrap();
        assert_eq!(ribbon_copy.parent, Some(copy_lid), "under the copy's node");
        assert_eq!(s.prefab_report(copy).unwrap().overrides, 1);
        // Deleting an instance takes its nodes along.
        s.delete(root);
        assert_eq!(s.scene.entities.len(), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_prefab_that_cannot_be_read_leaves_a_marked_empty_object() {
        let (dir, mut s) = project("broken");
        std::fs::create_dir_all(dir.join("assets/prefabs")).unwrap();
        std::fs::write(dir.join("assets/prefabs/bad.prefab.ron"), "(entities: [").unwrap();
        assert!(
            s.instantiate_prefab("prefabs/bad.prefab.ron", None, Vec3::ZERO)
                .is_err()
        );
        let mut e = EntityDesc::new(5, "Lost");
        e.prefab = Some(PrefabInstance::new("prefabs/gone.prefab.ron"));
        s.scene.entities.push(e);
        let report = s.prefab_report(5).unwrap();
        assert!(report.missing.is_some());
        // Saving keeps the reference, so a restored file is picked up again.
        s.scene_path = Some(dir.join("assets/scenes/b.scene.ron"));
        assert!(s.save());
        let saved = SceneFile::load(dir.join("assets/scenes/b.scene.ron")).unwrap();
        assert_eq!(
            saved.get(5).unwrap().prefab.as_ref().unwrap().asset,
            "prefabs/gone.prefab.ron"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

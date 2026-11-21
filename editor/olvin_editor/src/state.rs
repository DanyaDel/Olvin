use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use glam::{EulerRot, Mat4, Vec3};
use olvin_scene::Transform;
use olvin_scene::file::{
    AudioSourceDesc, CameraDesc, ColliderDesc, EntityDesc, LightDesc, LightKind, MeshDesc,
    SceneFile, Shape,
};

use crate::engine::{PlaySession, ScenePreview};
use crate::ext::Extensions;

const UNDO_LIMIT: usize = 200;

/// Built-in viewport tools; elements add their own ids (e.g. `"csg.draw"`).
pub mod tools {
    pub const VIEW: &str = "view";
    pub const MOVE: &str = "move";
    pub const ROTATE: &str = "rotate";
    pub const SCALE: &str = "scale";
}

#[derive(Clone, Copy, Debug)]
pub struct SnapSettings {
    pub enabled: bool,
    /// Fixed grid step; `None` follows the adaptive grid.
    pub step: Option<f32>,
    pub angle_deg: f32,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            step: None,
            angle_deg: 15.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub level: LogLevel,
    /// Seconds since the editor started.
    pub time: f64,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewEntity {
    Empty,
    Primitive(Shape),
    Camera,
    Light(LightKind),
    AudioSource,
}

impl NewEntity {
    /// Unity's GameObject menu: (submenu, items); `None` is the top level.
    pub const MENU: [(Option<&'static str>, &'static [NewEntity]); 5] = [
        (None, &[NewEntity::Empty]),
        (
            Some("3D Object"),
            &[
                NewEntity::Primitive(Shape::Cube),
                NewEntity::Primitive(Shape::Sphere),
                NewEntity::Primitive(Shape::Capsule),
                NewEntity::Primitive(Shape::Cylinder),
                NewEntity::Primitive(Shape::Plane),
                NewEntity::Primitive(Shape::Quad),
            ],
        ),
        (
            Some("Light"),
            &[
                NewEntity::Light(LightKind::Directional),
                NewEntity::Light(LightKind::Point),
                NewEntity::Light(LightKind::Spot),
            ],
        ),
        (Some("Audio"), &[NewEntity::AudioSource]),
        (None, &[NewEntity::Camera]),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "Create Empty",
            Self::Primitive(shape) => shape.label(),
            Self::Camera => "Camera",
            Self::Light(LightKind::Directional) => "Directional Light",
            Self::Light(LightKind::Point) => "Point Light",
            Self::Light(LightKind::Spot) => "Spot Light",
            Self::AudioSource => "Audio Source",
        }
    }

    fn object_name(self) -> &'static str {
        match self {
            Self::Empty => "GameObject",
            other => other.label(),
        }
    }
}

/// Everything panels share. Panels mutate `scene` directly; undo snapshots
/// are taken automatically once the user lets go of the mouse / keyboard.
pub struct EditorState {
    pub scene: SceneFile,
    pub scene_path: Option<PathBuf>,
    pub project: Option<PathBuf>,
    /// The active object (gizmo, inspector).
    pub selection: Option<u64>,
    /// All selected objects when several are (always includes `selection`).
    pub multi: Vec<u64>,
    /// Local-pose overrides for previews (animation scrubbing). Never saved.
    pub preview: HashMap<u64, Transform>,
    pub log: Vec<LogLine>,
    /// Bumped whenever `scene` is replaced wholesale (open, new, undo, redo).
    pub scene_generation: u64,
    /// Viewports frame the selection when set; the first viewport to see it clears it.
    pub frame_selection: bool,
    pub(crate) open_request: Option<PathBuf>,
    /// Active viewport tool, see [`tools`].
    pub tool: String,
    /// Gizmos use the object's axes instead of the world's.
    pub local_space: bool,
    /// Hold Ctrl to invert `snap.enabled` temporarily.
    pub snap: SnapSettings,
    /// Set by a tool that handles Delete / Ctrl+D itself (e.g. editing
    /// vertices); the editor then won't delete or duplicate the object.
    /// Cleared every frame.
    pub tool_owns_keys: bool,
    /// `tool_owns_keys` of the previous frame: panels drawn before the tool
    /// (the viewport's own hotkeys) check this one.
    pub keys_owned_last_frame: bool,
    /// Editor preferences (Settings window).
    pub prefs: crate::prefs::Preferences,
    /// Open Settings page, `None` when the window is closed.
    pub settings_page: Option<crate::settings::Page>,
    /// Last frame's render statistics per view.
    pub render_stats: HashMap<u64, olvin::render::RenderStats>,
    /// Material, texture and shader assets of the project.
    pub materials: crate::materials::MaterialLibrary,
    /// Mesh Edit Mode selection (entity, selected vertices), published by the
    /// editing element for others (the UV editor's "only selected in 3D").
    pub mesh_selection: Option<(u64, Vec<u32>)>,
    /// Set every frame by a tool in a modal operation (grab, inset...): the
    /// whole operation becomes one undo step. Cleared every frame.
    pub busy: bool,
    pub(crate) ext: Extensions,
    /// egui's GPU context; the engine renderer draws through the same device.
    pub(crate) gpu: Option<eframe::egui_wgpu::RenderState>,
    pub(crate) engine_scene: ScenePreview,
    /// The running game in Play Mode.
    pub play: Option<PlaySession>,
    committed: SceneFile,
    saved: SceneFile,
    undo: Vec<SceneFile>,
    redo: Vec<SceneFile>,
    started: Instant,
}

impl EditorState {
    pub fn new(project: Option<PathBuf>) -> Self {
        let mut state = Self {
            scene: SceneFile::default(),
            scene_path: None,
            project,
            selection: None,
            multi: Vec::new(),
            preview: HashMap::new(),
            log: Vec::new(),
            scene_generation: 0,
            frame_selection: false,
            open_request: None,
            tool: tools::MOVE.to_owned(),
            local_space: false,
            snap: SnapSettings::default(),
            tool_owns_keys: false,
            keys_owned_last_frame: false,
            prefs: crate::prefs::Preferences::default(),
            settings_page: None,
            render_stats: HashMap::new(),
            materials: Default::default(),
            mesh_selection: None,
            busy: false,
            ext: Extensions::default(),
            gpu: None,
            engine_scene: ScenePreview::default(),
            play: None,
            committed: SceneFile::default(),
            saved: SceneFile::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            started: Instant::now(),
        };
        match state.default_scene_path() {
            Some(path) if path.is_file() => state.open(&path),
            Some(path) => {
                state.replace_scene(starter_scene(), Some(path), false);
                state.info("new scene: save it (Ctrl+S) and the game will load it on start");
            }
            None => {
                state.replace_scene(starter_scene(), None, false);
                state.warn("no project given: olvin-editor <project folder>");
            }
        }
        state
    }

    pub fn assets_dir(&self) -> Option<PathBuf> {
        self.project.as_ref().map(|p| p.join("assets"))
    }

    pub fn default_scene_path(&self) -> Option<PathBuf> {
        self.assets_dir().map(|a| a.join("scenes/main.scene.ron"))
    }

    // --- log -------------------------------------------------------------

    pub fn log(&mut self, level: LogLevel, text: impl Into<String>) {
        let time = self.started.elapsed().as_secs_f64();
        self.log.push(LogLine {
            level,
            time,
            text: text.into(),
        });
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.log(LogLevel::Info, text);
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.log(LogLevel::Warn, text);
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.log(LogLevel::Error, text);
    }

    // --- files -----------------------------------------------------------

    /// Unsaved changes. While playing, the scene being edited is the one from
    /// before Play: changes made to the running game are discarded.
    pub fn is_dirty(&self) -> bool {
        let scene = self.play.as_ref().map_or(&self.scene, |p| &p.saved);
        *scene != self.saved
    }

    /// Asks the app to open a scene; it confirms first if there are unsaved changes.
    pub fn request_open(&mut self, path: PathBuf) {
        self.open_request = Some(path);
    }

    pub fn open(&mut self, path: &Path) {
        match SceneFile::load(path) {
            Ok(scene) => {
                self.replace_scene(scene, Some(path.to_path_buf()), true);
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.info(format!("opened scene {name}"));
            }
            Err(err) => self.error(format!("could not open scene: {err}")),
        }
    }

    pub fn new_scene(&mut self) {
        let path = self
            .default_scene_path()
            .map(|p| p.with_file_name("untitled.scene.ron"));
        self.replace_scene(starter_scene(), path, false);
        self.info("new scene");
    }

    pub fn save(&mut self) -> bool {
        let Some(path) = self.scene_path.clone() else {
            self.error("nowhere to save: no project, use Save As");
            return false;
        };
        self.save_as(&path)
    }

    pub fn save_as(&mut self, path: &Path) -> bool {
        // While playing, `scene` is the running game: save what Stop restores.
        let scene = match &self.play {
            Some(session) => session.saved.clone(),
            None => {
                self.commit();
                self.scene.clone()
            }
        };
        match scene.save(path) {
            Ok(()) => {
                self.saved = scene;
                self.scene_path = Some(path.to_path_buf());
                self.info(format!("saved {}", path.display()));
                true
            }
            Err(err) => {
                self.error(format!("could not save: {err}"));
                false
            }
        }
    }

    fn replace_scene(&mut self, scene: SceneFile, path: Option<PathBuf>, saved: bool) {
        self.saved = if saved {
            scene.clone()
        } else {
            SceneFile::default()
        };
        self.committed = scene.clone();
        self.scene = scene;
        self.scene_path = path;
        // A running game belongs to the scene that was replaced.
        self.play = None;
        self.selection = None;
        self.multi.clear();
        self.preview.clear();
        self.undo.clear();
        self.redo.clear();
        self.scene_generation += 1;
    }

    // --- selection ---------------------------------------------------------

    /// Everything selected; code that only sets `selection` selects one object.
    pub fn selected_ids(&self) -> Vec<u64> {
        match self.selection {
            None => Vec::new(),
            Some(p) if self.multi.contains(&p) => self.multi.clone(),
            Some(p) => vec![p],
        }
    }

    pub fn is_selected(&self, id: u64) -> bool {
        self.selection == Some(id)
            || (self.selection.is_some_and(|p| self.multi.contains(&p)) && self.multi.contains(&id))
    }

    pub fn open_settings(&mut self, page: crate::settings::Page) {
        self.settings_page = Some(page);
    }

    pub fn select(&mut self, id: Option<u64>) {
        self.selection = id;
        self.multi.clear();
    }

    /// Ctrl+click: add or remove one object.
    pub fn toggle_selected(&mut self, id: u64) {
        let mut ids = self.selected_ids();
        if let Some(i) = ids.iter().position(|x| *x == id) {
            ids.remove(i);
            self.selection = ids.last().copied();
        } else {
            ids.push(id);
            self.selection = Some(id);
        }
        self.multi = ids;
    }

    /// Shift+click: everything between the active object and `id`, in `order`.
    pub fn select_range(&mut self, id: u64, order: &[u64]) {
        let Some(anchor) = self.selection else {
            return self.select(Some(id));
        };
        let (Some(a), Some(b)) = (
            order.iter().position(|x| *x == anchor),
            order.iter().position(|x| *x == id),
        ) else {
            return self.select(Some(id));
        };
        let (lo, hi) = (a.min(b), a.max(b));
        self.multi = order[lo..=hi].to_vec();
        self.selection = Some(id);
    }

    // --- play mode -------------------------------------------------------

    pub fn is_playing(&self) -> bool {
        self.play.is_some()
    }

    /// Runs the scene as a game inside the editor.
    pub fn start_play(&mut self) {
        if self.play.is_none() {
            self.commit();
            let assets = self.assets_dir();
            self.play = Some(PlaySession::start(&self.scene, self.undo.len(), assets));
            self.info("play");
        }
    }

    /// Back to editing: changes made while playing are discarded, like Unity.
    pub fn stop_play(&mut self) {
        if let Some(session) = self.play.take() {
            self.scene = session.saved.clone();
            self.committed = session.saved;
            self.undo.truncate(session.undo_len);
            self.redo.clear();
            self.scene_generation += 1;
            self.info("stopped");
        }
    }

    // --- undo ------------------------------------------------------------

    /// Called once per frame. Edits made while a drag or text edit is in
    /// progress collapse into a single undo step.
    pub(crate) fn end_frame(&mut self, interacting: bool) {
        if !interacting {
            self.commit();
        }
    }

    fn commit(&mut self) {
        if self.scene != self.committed {
            self.undo
                .push(std::mem::replace(&mut self.committed, self.scene.clone()));
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    /// History is frozen while playing: everything done then is discarded by Stop.
    pub fn can_undo(&self) -> bool {
        self.play.is_none() && (!self.undo.is_empty() || self.scene != self.committed)
    }

    pub fn can_redo(&self) -> bool {
        self.play.is_none() && !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if self.play.is_some() {
            return;
        }
        self.commit();
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.committed, prev));
            self.scene = self.committed.clone();
            self.after_history_jump();
        }
    }

    pub fn redo(&mut self) {
        if self.play.is_some() {
            return;
        }
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.committed, next));
            self.scene = self.committed.clone();
            self.after_history_jump();
        }
    }

    fn after_history_jump(&mut self) {
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

    // --- hierarchy -------------------------------------------------------

    pub fn children(&self, parent: Option<u64>) -> Vec<u64> {
        self.scene
            .entities
            .iter()
            .filter(|e| e.parent == parent)
            .map(|e| e.id)
            .collect()
    }

    pub fn is_descendant(&self, id: u64, ancestor: u64) -> bool {
        let mut current = self.scene.get(id).and_then(|e| e.parent);
        while let Some(p) = current {
            if p == ancestor {
                return true;
            }
            current = self.scene.get(p).and_then(|e| e.parent);
        }
        false
    }

    fn next_id(&self) -> u64 {
        self.scene.entities.iter().map(|e| e.id).max().unwrap_or(0) + 1
    }

    /// Imports a model (`.gltf`/`.glb` under assets/) into the scene: its node
    /// hierarchy with editable meshes and per-face UVs, plus `.mat.ron`
    /// materials next to it. Returns the root's id.
    pub fn import_model(&mut self, path: &std::path::Path) -> Result<u64, String> {
        let root_dir = self.assets_dir().ok_or("open a project first")?;
        let mut root = olvin_assets::AssetRoot::new(&root_dir);
        root.scan();
        let rel = root
            .relative(path)
            .ok_or("the model must be inside assets/")?;
        let model = root.import_model(&rel)?;
        let dir = format!("materials/{}", model.name);
        let materials = root.write_materials(&model, &dir)?;
        let imported = olvin_assets::model_to_entities(&model, self.next_id(), &materials);
        let first = imported
            .entities
            .first()
            .map(|e| e.id)
            .ok_or("the model has no nodes")?;
        for mut e in imported.entities {
            if let (Some(poly), Some(uvs)) = (&mut e.poly, imported.uvs_per_face.get(&e.id))
                && uvs.len() == poly.faces.len()
            {
                poly.uvs = uvs.clone();
            }
            self.scene.entities.push(e);
        }
        for warning in &model.skipped {
            self.warn(format!("{}: {warning}", model.name));
        }
        self.info(format!("imported {rel}: {} materials", materials.len()));
        self.select(Some(first));
        Ok(first)
    }

    pub fn create(&mut self, kind: NewEntity, parent: Option<u64>) -> u64 {
        let id = self.next_id();
        let mut e = EntityDesc::new(id, kind.object_name());
        e.parent = parent.filter(|p| self.scene.get(*p).is_some());
        match kind {
            NewEntity::Empty => {}
            NewEntity::Primitive(shape) => {
                // Like Unity: primitives come with their matching collider.
                e.mesh = Some(MeshDesc {
                    shape,
                    color: [0.8, 0.8, 0.8, 1.0],
                });
                e.collider = Some(ColliderDesc::for_shape(shape));
            }
            NewEntity::Camera => {
                e.camera = Some(CameraDesc::default());
                e.audio_listener = true;
            }
            NewEntity::Light(kind) => {
                e.light = Some(LightDesc::new(kind));
                e.position = Vec3::new(0.0, 3.0, 0.0);
                if kind != LightKind::Point {
                    e.rotation = Vec3::new(-50.0, -30.0, 0.0);
                }
            }
            NewEntity::AudioSource => e.audio_source = Some(AudioSourceDesc::default()),
        }
        self.scene.entities.push(e);
        self.selection = Some(id);
        id
    }

    /// Deletes an entity with all its children.
    pub fn delete(&mut self, id: u64) {
        let doomed: Vec<u64> = self
            .scene
            .entities
            .iter()
            .filter(|e| e.id == id || self.is_descendant(e.id, id))
            .map(|e| e.id)
            .collect();
        self.scene.entities.retain(|e| !doomed.contains(&e.id));
        if self.selection.is_some_and(|s| doomed.contains(&s)) {
            self.selection = None;
        }
        self.multi.retain(|id| !doomed.contains(id));
    }

    /// Deletes everything selected.
    pub fn delete_selected(&mut self) {
        for id in self.selected_ids() {
            self.delete(id);
        }
    }

    /// The selected objects whose ancestors are not selected too: acting on
    /// them already covers the whole subtree.
    fn selected_roots(&self) -> Vec<u64> {
        let ids = self.selected_ids();
        ids.iter()
            .copied()
            .filter(|id| !ids.iter().any(|a| self.is_descendant(*id, *a)))
            .collect()
    }

    /// Duplicates everything selected (a selected parent brings its selected
    /// children along once) and selects the copies.
    pub fn duplicate_selected(&mut self) {
        let copies: Vec<u64> = self
            .selected_roots()
            .into_iter()
            .filter_map(|id| self.duplicate(id))
            .collect();
        if !copies.is_empty() {
            self.selection = copies.last().copied();
            self.multi = copies;
        }
    }

    /// Copies an entity with its children; returns the copy's id.
    pub fn duplicate(&mut self, id: u64) -> Option<u64> {
        let subtree: Vec<EntityDesc> = self
            .scene
            .entities
            .iter()
            .filter(|e| e.id == id || self.is_descendant(e.id, id))
            .cloned()
            .collect();
        if subtree.is_empty() {
            return None;
        }
        let remap: HashMap<u64, u64> = subtree.iter().map(|e| e.id).zip(self.next_id()..).collect();
        for mut e in subtree {
            if e.id == id {
                e.name = format!("{} (copy)", e.name);
            } else {
                e.parent = e.parent.map(|p| remap[&p]);
            }
            e.id = remap[&e.id];
            self.scene.entities.push(e);
        }
        let copy = remap[&id];
        self.selection = Some(copy);
        Some(copy)
    }

    /// Moves an entity up (-1) or down (+1) among its siblings. Order matters
    /// for CSG brushes.
    pub fn move_sibling(&mut self, id: u64, dir: i32) -> bool {
        let Some(parent) = self.scene.get(id).map(|e| e.parent) else {
            return false;
        };
        let siblings: Vec<usize> = (0..self.scene.entities.len())
            .filter(|&i| self.scene.entities[i].parent == parent)
            .collect();
        let Some(pos) = siblings
            .iter()
            .position(|&i| self.scene.entities[i].id == id)
        else {
            return false;
        };
        let other = pos as i32 + dir;
        if other < 0 || other as usize >= siblings.len() {
            return false;
        }
        self.scene
            .entities
            .swap(siblings[pos], siblings[other as usize]);
        // Swapping may put a subtree before its parent; restore the invariant.
        self.fix_order();
        true
    }

    /// Moves `id` under `parent` keeping its world transform (like Unity).
    pub fn reparent(&mut self, id: u64, parent: Option<u64>) -> bool {
        if parent == Some(id) || parent.is_some_and(|p| self.is_descendant(p, id)) {
            return false;
        }
        if self.scene.get(id).is_some_and(|e| e.parent == parent) {
            return true; // already there; decomposing would only rewrite the transform
        }
        let worlds = self.scene.world_matrices();
        let Some(&world) = worlds.get(&id) else {
            return false;
        };
        let parent_world = parent
            .and_then(|p| worlds.get(&p).copied())
            .unwrap_or(Mat4::IDENTITY);
        // A parent scaled to zero has no inverse: keep the local transform.
        let local = (parent_world.determinant().abs() > 1e-12)
            .then(|| (parent_world.inverse() * world).to_scale_rotation_translation())
            .filter(|(s, r, t)| s.is_finite() && r.is_finite() && t.is_finite());
        let Some(e) = self.scene.get_mut(id) else {
            return false;
        };
        e.parent = parent;
        if let Some((scale, rotation, translation)) = local {
            let (y, x, z) = rotation.to_euler(EulerRot::YXZ);
            e.position = translation;
            e.rotation = Vec3::new(x, y, z).map(f32::to_degrees);
            e.scale = scale;
        }
        // Keep the "parents first" invariant: move the entity to the end, then its subtree.
        self.fix_order();
        true
    }

    /// Stable topological order: every entity after its parent.
    fn fix_order(&mut self) {
        let mut pending = std::mem::take(&mut self.scene.entities);
        let mut placed = std::collections::HashSet::new();
        while !pending.is_empty() {
            let before = pending.len();
            let mut rest = Vec::with_capacity(pending.len());
            for e in pending {
                if e.parent.is_none_or(|p| placed.contains(&p)) {
                    placed.insert(e.id);
                    self.scene.entities.push(e);
                } else {
                    rest.push(e);
                }
            }
            if rest.len() == before {
                // Orphans (parent missing): promote to roots rather than lose them.
                for mut e in rest {
                    e.parent = None;
                    self.scene.entities.push(e);
                }
                break;
            }
            pending = rest;
        }
    }

    /// World matrices with preview overrides applied.
    pub fn world_matrices(&self) -> HashMap<u64, Mat4> {
        self.scene.world_matrices_with(|e| {
            self.preview
                .get(&e.id)
                .copied()
                .unwrap_or_else(|| e.local_transform())
        })
    }
}

fn starter_scene() -> SceneFile {
    let mut camera = EntityDesc::new(1, "Main Camera");
    camera.position = Vec3::new(0.0, 3.0, 8.0);
    camera.rotation = Vec3::new(-18.0, 0.0, 0.0);
    camera.camera = Some(CameraDesc::default());

    let mut ground = EntityDesc::new(2, "Ground");
    ground.scale = Vec3::splat(2.0);
    ground.collider = Some(ColliderDesc::for_shape(Shape::Plane));
    ground.mesh = Some(MeshDesc {
        shape: Shape::Plane,
        color: [0.18, 0.2, 0.22, 1.0],
    });

    let mut cube = EntityDesc::new(3, "Cube");
    cube.position = Vec3::new(0.0, 0.5, 0.0);
    cube.mesh = Some(MeshDesc {
        shape: Shape::Cube,
        color: [0.9, 0.45, 0.15, 1.0],
    });
    cube.collider = Some(ColliderDesc::for_shape(Shape::Cube));

    let mut sun = EntityDesc::new(4, "Directional Light");
    sun.position = Vec3::new(0.0, 3.0, 0.0);
    sun.rotation = Vec3::new(-50.0, -30.0, 0.0);
    sun.light = Some(LightDesc::new(LightKind::Directional));

    SceneFile {
        entities: vec![camera, sun, ground, cube],
        // A daylight sky reads better than a black void in a new scene.
        environment: (olvin_scene::file::Environment::PRESETS[0].1)(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> EditorState {
        EditorState::new(None)
    }

    #[test]
    fn edits_collapse_into_one_undo_step() {
        let mut s = state();
        let id = s.create(NewEntity::Primitive(Shape::Cube), None);
        s.end_frame(false);
        // A drag: many frames of edits while the mouse is down.
        for i in 0..10 {
            s.scene.get_mut(id).unwrap().position.x = i as f32;
            s.end_frame(true);
        }
        s.end_frame(false);
        s.undo();
        assert_eq!(s.scene.get(id).unwrap().position.x, 0.0);
        s.undo();
        assert!(s.scene.get(id).is_none());
        s.redo();
        s.redo();
        assert_eq!(s.scene.get(id).unwrap().position.x, 9.0);
    }

    #[test]
    fn reparent_keeps_world_position_and_order() {
        let mut s = state();
        let a = s.create(NewEntity::Empty, None);
        s.scene.get_mut(a).unwrap().position = Vec3::new(5.0, 0.0, 0.0);
        let b = s.create(NewEntity::Primitive(Shape::Cube), None);
        s.scene.get_mut(b).unwrap().position = Vec3::new(1.0, 2.0, 3.0);
        // b was created after a; put a under b and back.
        assert!(s.reparent(a, Some(b)));
        let pa = s.scene.entities.iter().position(|e| e.id == a).unwrap();
        let pb = s.scene.entities.iter().position(|e| e.id == b).unwrap();
        assert!(pb < pa);
        let world = s.scene.world_matrices()[&a].transform_point3(Vec3::ZERO);
        assert!(world.abs_diff_eq(Vec3::new(5.0, 0.0, 0.0), 1e-4));
        assert!(!s.reparent(b, Some(a)), "cycles are refused");

        let copy = s.duplicate(b).unwrap();
        assert_eq!(s.children(Some(copy)).len(), 1);
        s.delete(b);
        assert!(s.scene.get(a).is_none());
        assert_eq!(s.children(Some(copy)).len(), 1);
    }

    #[test]
    fn stopping_play_discards_play_time_edits() {
        let mut s = state();
        let id = s.create(NewEntity::Primitive(Shape::Cube), None);
        s.end_frame(false);
        s.start_play();
        s.scene.get_mut(id).unwrap().position.x = 42.0;
        s.end_frame(false);
        s.stop_play();
        assert_eq!(s.scene.get(id).unwrap().position.x, 0.0);
        s.undo();
        assert!(s.scene.get(id).is_none(), "undo continues from before Play");
    }

    #[test]
    fn multi_selection_toggle_and_range() {
        let mut s = state();
        let ids: Vec<u64> = (0..4).map(|_| s.create(NewEntity::Empty, None)).collect();
        s.select(Some(ids[0]));
        s.toggle_selected(ids[2]);
        assert_eq!(s.selected_ids(), vec![ids[0], ids[2]]);
        s.toggle_selected(ids[0]);
        assert_eq!(s.selected_ids(), vec![ids[2]]);
        s.select_range(ids[0], &ids);
        assert_eq!(s.selected_ids(), ids[0..=2].to_vec());
        // Plain assignment (used by tools) still means one object.
        s.selection = Some(ids[3]);
        assert_eq!(s.selected_ids(), vec![ids[3]]);
    }

    #[test]
    fn reparent_is_safe_and_history_is_frozen_while_playing() {
        let mut s = state();
        let a = s.create(NewEntity::Empty, None);
        let b = s.create(NewEntity::Empty, None);
        s.scene.get_mut(a).unwrap().scale = Vec3::ZERO;
        s.scene.get_mut(b).unwrap().position = Vec3::new(1.0, 2.0, 3.0);
        // A parent scaled to zero has no inverse: no NaN, local transform kept.
        assert!(s.reparent(b, Some(a)));
        assert_eq!(s.scene.get(b).unwrap().position, Vec3::new(1.0, 2.0, 3.0));
        // Same parent again changes nothing.
        let before = s.scene.clone();
        assert!(s.reparent(b, Some(a)));
        assert_eq!(s.scene, before);

        s.end_frame(false);
        s.start_play();
        assert!(!s.can_undo() && !s.can_redo());
        s.undo();
        assert!(s.scene.get(b).is_some(), "undo is ignored while playing");
        s.stop_play();
        assert!(s.can_undo());
    }
}

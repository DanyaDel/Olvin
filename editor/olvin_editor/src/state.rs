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
use crate::materials::MaterialEdits;
use crate::prefabs::{PrefabEdits, PrefabLibrary};

mod autosave;
mod files;
mod prefab;
pub(crate) use files::{is_prefab, is_scene};
pub use prefab::PrefabReport;

const UNDO_LIMIT: usize = 200;
/// Log lines kept; the oldest tenth goes when it is exceeded.
const LOG_LIMIT: usize = 5000;

/// The asset files an undo step restores: edited materials and prefabs.
#[derive(Clone, Default, PartialEq)]
struct AssetEdits {
    materials: MaterialEdits,
    prefabs: PrefabEdits,
}

/// One step of the history: the scene before an edit, or a file operation.
// Nearly every step is an edit: boxing it would cost an allocation per step for nothing.
#[allow(clippy::large_enum_variant)]
enum Step {
    Edit(SceneFile, AssetEdits),
    File(files::FileOp),
}

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
    /// The user allowed this project to be built and run (T1). Panels that start
    /// cargo or shell jobs must check it; `start_job` in the IDE does.
    pub project_trusted: bool,
    /// The trust question is still to be asked (a modal in the app).
    pub trust_pending: bool,
    /// The active object (gizmo, inspector).
    pub selection: Option<u64>,
    /// An autosave of the open scene from an earlier session, newer than the
    /// file: `recover_autosave` loads it, `drop_autosave` throws it away.
    pub recovery: Option<PathBuf>,
    /// Tabs whose panel panicked: tab id → (panel title, panic message). They
    /// show a crash card with Restart instead of taking the editor down;
    /// removing an entry restarts the panel.
    pub crashed_panels: HashMap<u32, (String, String)>,
    /// All selected objects when several are (always includes `selection`).
    pub multi: Vec<u64>,
    /// Local-pose overrides for previews (animation scrubbing). Never saved.
    pub preview: HashMap<u64, Transform>,
    /// Model animation previews in the scene view (by object id). Never saved
    /// and not an undo step.
    pub model_previews: HashMap<u64, crate::engine::ModelPreview>,
    /// The animation's current frame (Blender's `frame_current`), shared by
    /// the timeline, the viewport's `I` key and the inspector's key colours.
    pub anim_frame: i32,
    pub log: Vec<LogLine>,
    /// Bumped whenever `scene` is replaced wholesale (open, new, undo, redo).
    pub scene_generation: u64,
    /// Viewports frame the selection when set; the first viewport to see it clears it.
    pub frame_selection: bool,
    /// The pointer was over a panel that owns Delete and Ctrl+D (the Game view
    /// while playing, Assets); set by the panel, read by next frame's shortcuts.
    pub panel_owns_edit_keys: bool,
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
    /// The Settings window waits for a key to bind: no shortcut fires meanwhile.
    pub capturing_key: bool,
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
    /// Prefab assets of the project; the scene holds their instances expanded.
    pub prefabs: PrefabLibrary,
    /// The Assets panel shows this file next time (set by "Select Prefab Asset").
    pub asset_focus: Option<PathBuf>,
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
    /// The edited materials and prefabs (already on disk) at the last undo step.
    committed_edits: AssetEdits,
    saved: SceneFile,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// The expanded members of the prefab instance last compared with its asset.
    prefab_diff: prefab::DiffCache,
    prefab_polled: Instant,
    started: Instant,
}

impl EditorState {
    pub fn new(project: Option<PathBuf>) -> Self {
        let mut state = Self {
            scene: SceneFile::default(),
            scene_path: None,
            project,
            project_trusted: true,
            trust_pending: false,
            crashed_panels: HashMap::new(),
            recovery: None,
            selection: None,
            multi: Vec::new(),
            preview: HashMap::new(),
            model_previews: HashMap::new(),
            anim_frame: 1,
            log: Vec::new(),
            scene_generation: 0,
            frame_selection: false,
            panel_owns_edit_keys: false,
            open_request: None,
            tool: tools::MOVE.to_owned(),
            local_space: false,
            snap: SnapSettings::default(),
            tool_owns_keys: false,
            capturing_key: false,
            keys_owned_last_frame: false,
            prefs: crate::prefs::Preferences::default(),
            settings_page: None,
            render_stats: HashMap::new(),
            materials: Default::default(),
            prefabs: Default::default(),
            asset_focus: None,
            mesh_selection: None,
            busy: false,
            ext: Extensions::default(),
            gpu: None,
            engine_scene: ScenePreview::default(),
            play: None,
            committed: SceneFile::default(),
            committed_edits: AssetEdits::default(),
            saved: SceneFile::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            prefab_diff: Default::default(),
            prefab_polled: Instant::now(),
            started: Instant::now(),
        };
        state.prefabs.set_root(state.assets_dir());
        if let Some(project) = &state.project {
            files::purge_trash(project);
        }
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
        // A long session must not grow the log (and every frame's filter) forever.
        if self.log.len() > LOG_LIMIT {
            self.log.drain(..LOG_LIMIT / 10);
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.log(LogLevel::Info, text);
    }

    /// Decides the trust of the open project from the shared list (`olvin_trust`):
    /// no project or a trusted one is fine; "Open read-only" from the hub
    /// (`OLVIN_TRUST_MODE=readonly`) means untrusted without asking again;
    /// anything else is untrusted and asks.
    pub fn check_trust(&mut self, store: &olvin_trust::TrustStore, read_only: bool) {
        let Some(project) = &self.project else {
            return;
        };
        // `OLVIN_TRUST=1` trusts everything for this run, like the CLI.
        self.project_trusted = olvin_trust::env_trusts() || store.is_trusted(project);
        self.trust_pending = !self.project_trusted && !read_only;
        if !self.project_trusted {
            self.warn(
                "project is not trusted: building and running are off (open the hub and trust it, or answer the question)",
            );
        }
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
            Ok(mut scene) => {
                self.prefabs.set_root(self.assets_dir());
                self.expand_prefabs(&mut scene);
                self.replace_scene(scene, Some(path.to_path_buf()), true);
                self.check_autosave();
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
        match self.file_form(&scene).save(path) {
            Ok(()) => {
                self.saved = scene;
                self.drop_autosave();
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
        self.committed_edits = self.edits_now();
        self.scene = scene;
        self.scene_path = path;
        // A running game belongs to the scene that was replaced.
        self.play = None;
        self.selection = None;
        self.multi.clear();
        self.preview.clear();
        self.model_previews.clear();
        self.undo.clear();
        self.redo.clear();
        self.recovery = None;
        self.scene_generation += 1;
    }

    /// Evaluates the scene's animation at `frame` into the objects without an
    /// undo step or a dirty mark: moving the playhead is not an edit.
    pub fn apply_animation_quietly(&mut self, frame: f32) {
        let action = self.scene.animation.clone();
        let clean = self.scene == self.saved;
        self.scene.apply_action(&action, frame);
        self.committed.apply_action(&action, frame);
        if clean {
            self.saved.apply_action(&action, frame);
        }
    }

    /// Applies `f` to the open scene, its saved and committed copies and the
    /// whole history (an asset moved: every copy must name the new path).
    pub(crate) fn map_scenes(&mut self, f: impl Fn(&mut SceneFile)) {
        let all = [&mut self.scene, &mut self.committed, &mut self.saved]
            .into_iter()
            .chain(
                self.undo
                    .iter_mut()
                    .chain(self.redo.iter_mut())
                    .filter_map(|s| match s {
                        Step::Edit(scene, _) => Some(scene),
                        Step::File(_) => None,
                    }),
            );
        all.for_each(f);
    }

    /// Drops the material edits under `prefix` (an asset moved or deleted)
    /// from the library and from every undo step.
    pub(crate) fn forget_materials(&mut self, prefix: &str) {
        let under = |k: &String| k == prefix || k.starts_with(&format!("{prefix}/"));
        self.materials.forget(&under);
        self.committed_edits.materials.retain(|k, _| !under(k));
        for step in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            if let Step::Edit(_, m) = step {
                m.materials.retain(|k, _| !under(k));
            }
        }
    }

    /// Drops the prefab edits under `prefix` (an asset moved or deleted) from
    /// the library and from every undo step.
    pub(crate) fn forget_prefabs(&mut self, prefix: &str) {
        let under = |k: &String| k == prefix || k.starts_with(&format!("{prefix}/"));
        self.prefabs.forget(&under);
        self.committed_edits.prefabs.retain(|k, _| !under(k));
        for step in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            if let Step::Edit(_, m) = step {
                m.prefabs.retain(|k, _| !under(k));
            }
        }
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
            // Whatever takes the process down during Play, the edits survive.
            self.autosave();
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

    fn edits_now(&self) -> AssetEdits {
        AssetEdits {
            materials: self.materials.edits().clone(),
            prefabs: self.prefabs.edits().clone(),
        }
    }

    /// Materials or prefabs were written since the last undo step.
    fn edits_changed(&self) -> bool {
        self.materials.edits() != &self.committed_edits.materials
            || self.prefabs.edits() != &self.committed_edits.prefabs
    }

    fn commit(&mut self) {
        if self.scene != self.committed || self.edits_changed() {
            let edits = self.edits_now();
            self.undo.push(Step::Edit(
                std::mem::replace(&mut self.committed, self.scene.clone()),
                std::mem::replace(&mut self.committed_edits, edits),
            ));
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    /// History is frozen while playing: everything done then is discarded by Stop.
    pub fn can_undo(&self) -> bool {
        self.play.is_none()
            && (!self.undo.is_empty() || self.scene != self.committed || self.edits_changed())
    }

    pub fn can_redo(&self) -> bool {
        self.play.is_none() && !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if self.play.is_some() {
            return;
        }
        self.commit();
        match self.undo.pop() {
            Some(Step::Edit(scene, edits)) => {
                self.redo.push(Step::Edit(
                    std::mem::replace(&mut self.committed, scene),
                    std::mem::replace(&mut self.committed_edits, edits),
                ));
                self.scene = self.committed.clone();
                self.after_history_jump();
            }
            Some(Step::File(op)) => match self.undo_file(op) {
                Ok(op) => self.redo.push(Step::File(op)),
                Err(e) => self.error(format!("undo: {e}")),
            },
            None => {}
        }
    }

    pub fn redo(&mut self) {
        if self.play.is_some() {
            return;
        }
        match self.redo.pop() {
            Some(Step::Edit(scene, edits)) => {
                self.undo.push(Step::Edit(
                    std::mem::replace(&mut self.committed, scene),
                    std::mem::replace(&mut self.committed_edits, edits),
                ));
                self.scene = self.committed.clone();
                self.after_history_jump();
            }
            Some(Step::File(op)) => match self.redo_file(op) {
                Ok(op) => self.undo.push(Step::File(op)),
                Err(e) => self.error(format!("redo: {e}")),
            },
            None => {}
        }
    }

    fn after_history_jump(&mut self) {
        // Material and prefab files follow the step: they are rewritten to its content.
        if let Err(err) = self.materials.restore(&self.committed_edits.materials) {
            self.error(format!("material undo: {err}"));
        }
        if let Err(err) = self.prefabs.restore(&self.committed_edits.prefabs) {
            self.error(format!("prefab undo: {err}"));
        }
        if self
            .selection
            .is_some_and(|id| self.scene.get(id).is_none())
        {
            self.selection = None;
        }
        let scene = &self.scene;
        self.multi.retain(|id| scene.get(*id).is_some());
        self.prefab_diff = Default::default();
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

    /// A free scene id: above every id of the scene, ignoring prefab nodes
    /// (their ids are hashes, see `olvin_scene::prefab::member_id`).
    fn next_id(&self) -> u64 {
        self.scene
            .entities
            .iter()
            .map(|e| e.id)
            .filter(|id| !olvin_scene::prefab::is_member_id(*id))
            .max()
            .unwrap_or(0)
            + 1
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

    /// Copies an entity with its children; returns the copy's id. A prefab
    /// instance is copied as an instance (with its overrides); a node inside
    /// an instance cannot be copied on its own.
    pub fn duplicate(&mut self, id: u64) -> Option<u64> {
        if self.is_prefab_member(id) {
            return None;
        }
        let inside: Vec<u64> = self
            .scene
            .entities
            .iter()
            .filter(|e| e.id == id || self.is_descendant(e.id, id))
            .map(|e| e.id)
            .collect();
        if inside.is_empty() {
            return None;
        }
        // The file form has the instances' roots and not their nodes: those
        // come back from the asset, under ids derived from the copy's root.
        let scene = self.scene.clone();
        let file = self.file_form(&scene);
        let subtree: Vec<EntityDesc> = file
            .entities
            .into_iter()
            .filter(|e| inside.contains(&e.id))
            .collect();
        let remap: HashMap<u64, u64> = subtree.iter().map(|e| e.id).zip(self.next_id()..).collect();
        for mut e in subtree {
            if e.id == id {
                e.name = format!("{} (copy)", e.name);
            } else {
                e.parent = e.parent.map(|p| {
                    remap.get(&p).copied().unwrap_or_else(|| {
                        // The parent is a node of a copied instance: the copy's node.
                        scene
                            .get(p)
                            .and_then(|n| n.prefab_node.as_ref())
                            .and_then(|n| {
                                let root = remap.get(&n.root)?;
                                Some(olvin_scene::prefab::node_id(*root, &n.path))
                            })
                            .unwrap_or(p)
                    })
                });
            }
            e.id = remap[&e.id];
            self.scene.entities.push(e);
        }
        self.expand_prefabs_now();
        let copy = remap[&id];
        self.selection = Some(copy);
        Some(copy)
    }

    /// Moves an entity up (-1) or down (+1) among its siblings. Order matters
    /// for CSG brushes.
    pub fn move_sibling(&mut self, id: u64, dir: i32) -> bool {
        if self.is_prefab_member(id) {
            return false;
        }
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
        // The nodes of an instance stay where the prefab put them.
        if self.is_prefab_member(id) {
            return false;
        }
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
    fn material_edits_join_the_undo_history() {
        let root = std::env::temp_dir().join(format!("olvin-mat-undo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("assets")).unwrap();
        let mut s = EditorState::new(Some(root.clone()));
        s.materials.refresh(Some(&root.join("assets")));
        let rel = s.materials.create("m", Default::default()).unwrap();
        let metallic = || {
            olvin_scene::material::MaterialDesc::load(root.join("assets").join(&rel))
                .unwrap()
                .metallic
        };
        s.end_frame(false);
        for value in [0.5, 1.0] {
            s.materials.get(&rel).unwrap().metallic = value;
            s.materials.save(&rel).unwrap();
            s.end_frame(false);
        }
        assert_eq!(metallic(), 1.0);
        s.undo();
        assert_eq!(metallic(), 0.5, "the file follows undo");
        s.undo();
        assert_eq!(metallic(), 0.0, "back to the content before the first edit");
        s.end_frame(false);
        assert!(!s.can_undo() || s.undo.len() <= 1, "no phantom steps");
        s.redo();
        s.redo();
        assert_eq!(metallic(), 1.0);
        assert_eq!(s.materials.get(&rel).unwrap().metallic, 1.0);
        std::fs::remove_dir_all(&root).unwrap();
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

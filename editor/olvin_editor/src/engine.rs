//! The engine inside the editor: scenes are drawn by the game's own renderer
//! (`olvin_render::RenderCore`, same wgpu as egui), and Play Mode runs a real
//! engine `App` with the game's plugins.

use std::collections::HashMap;

use eframe::egui_wgpu;
use glam::{Mat4, Vec3};
use olvin::core::App;
use olvin::ecs::{EcsPlugin, World};
use olvin::physics::PhysicsPlugin;
use olvin::render::{
    Color, GpuMeshes, MeshData, MeshHandle, MeshRenderer, Meshes, RenderCore, RenderSettings,
    ViewData,
};
use olvin::render::{Materials, Shaders, Textures};
use olvin::render_assets::{Registries, RenderAssets};
use olvin::scene::file::{SceneFile, Shape};
use olvin::scene::{Camera, Transform};
use olvin::transform::{GlobalTransform, TransformPlugin};

use crate::render::{DEPTH_FORMAT, MSAA};

/// Which world a view draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The scene being edited.
    Edit,
    /// The running game.
    Play,
}

/// GPU half, stored in egui's callback resources.
pub struct EngineGpu {
    pub core: RenderCore,
    edit: GpuMeshes,
    play: GpuMeshes,
    views: HashMap<u64, ViewData>,
}

impl EngineGpu {
    pub fn new(state: &egui_wgpu::RenderState) -> Self {
        let core = RenderCore::new(
            state.device.clone(),
            state.queue.clone(),
            state.target_format,
            MSAA,
            DEPTH_FORMAT,
        );
        Self {
            core,
            edit: GpuMeshes::default(),
            play: GpuMeshes::default(),
            views: HashMap::new(),
        }
    }

    pub fn reset_play(&mut self) {
        self.play = GpuMeshes::default();
    }

    /// Uploads new meshes and prepares view `id`; draw it later with [`Self::draw`].
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        id: u64,
        source: Source,
        world: &World,
        meshes: &mut Meshes,
        assets: Registries,
        view_proj: Mat4,
        settings: &RenderSettings,
    ) -> olvin::render::RenderStats {
        let gpu = match source {
            Source::Edit => &mut self.edit,
            Source::Play => &mut self.play,
        };
        self.core.upload(gpu, meshes);
        self.core
            .upload_assets(gpu, assets.textures, assets.materials, assets.shaders);
        let view = self
            .views
            .entry(id)
            .or_insert_with(|| self.core.create_view());
        self.core.prepare(view, gpu, world, view_proj, settings)
    }

    /// Shadow pass for view `id`; record it before the pass `draw` goes into.
    pub fn render_shadows(
        &self,
        id: u64,
        source: Source,
        encoder: &mut eframe::wgpu::CommandEncoder,
    ) {
        let gpu = match source {
            Source::Edit => &self.edit,
            Source::Play => &self.play,
        };
        if let Some(view) = self.views.get(&id) {
            self.core.render_shadows(view, gpu, encoder);
        }
    }

    pub fn draw(&self, id: u64, source: Source, pass: &mut eframe::wgpu::RenderPass<'static>) {
        let gpu = match source {
            Source::Edit => &self.edit,
            Source::Play => &self.play,
        };
        if let Some(view) = self.views.get(&id) {
            self.core.draw(view, gpu, pass);
        }
    }
}

/// A brush and its material path: what the textured CSG was built from.
type CsgKey = (olvin::csg::BrushInput, Option<String>);
/// A textured CSG mesh: (mesh, colour, material path).
type CsgMesh = (MeshHandle, [f32; 4], Option<String>);
/// A model's parts in its own space: mesh, placement, colour, material.
type ModelPart = (
    MeshHandle,
    Mat4,
    [f32; 4],
    Option<olvin::render::MaterialHandle>,
);

/// The edited scene as an engine world, rebuilt once per frame.
#[derive(Default)]
pub struct ScenePreview {
    pub world: World,
    pub meshes: Meshes,
    shapes: HashMap<Shape, MeshHandle>,
    /// Edited meshes: entity -> (handle, geometry it was uploaded from).
    polys: HashMap<u64, (MeshHandle, olvin::scene::file::PolyMesh)>,
    pub textures: Textures,
    pub materials: Materials,
    pub shaders: Shaders,
    pub assets: RenderAssets,
    /// CSG of brushes with materials (the CSG element draws the rest itself):
    /// rebuilt when the brush inputs or their materials change.
    csg: (Vec<CsgKey>, Vec<CsgMesh>),
    /// glTF models by path, imported once (`None`: failed), drawn in bind pose.
    models: HashMap<String, Option<Vec<ModelPart>>>,
    frame: Option<u64>,
}

impl ScenePreview {
    pub fn sync(&mut self, scene: &SceneFile, frame: u64, assets_dir: Option<std::path::PathBuf>) {
        if self.frame == Some(frame) {
            return;
        }
        self.frame = Some(frame);
        self.assets.set_root(assets_dir);
        self.world.clear();
        self.sync_csg(scene);
        let worlds = scene.world_matrices();
        for e in scene
            .entities
            .iter()
            .filter(|e| scene.active_in_hierarchy(e.id))
        {
            let Some(m) = worlds.get(&e.id) else { continue };
            // A modelled CSG brush is drawn by the CSG level, not as a mesh.
            if let Some(poly) = e.poly.as_ref().filter(|_| e.brush.is_none()) {
                let color = e.mesh.as_ref().map_or([0.8; 4], |m| m.color);
                let handle = match self.polys.get(&e.id) {
                    Some((h, uploaded)) if uploaded == poly => *h,
                    cached => {
                        let (_, data) = olvin::render_assets::poly_mesh_data(poly);
                        let h = match cached {
                            Some((h, _)) => {
                                let h = *h;
                                self.meshes.set(h, data);
                                h
                            }
                            None => self.meshes.add(data),
                        };
                        self.polys.insert(e.id, (h, poly.clone()));
                        h
                    }
                };
                let material = self.material(e.material.as_deref());
                self.world.spawn((
                    Transform::IDENTITY,
                    GlobalTransform(*m),
                    MeshRenderer {
                        mesh: handle,
                        color: Color(color),
                        material,
                    },
                ));
            } else if let Some(mesh) = &e.mesh {
                let meshes = &mut self.meshes;
                let handle = *self
                    .shapes
                    .entry(mesh.shape)
                    .or_insert_with(|| meshes.add(MeshData::primitive(mesh.shape)));
                let material = self.material(e.material.as_deref());
                self.world.spawn((
                    Transform::IDENTITY,
                    GlobalTransform(*m),
                    MeshRenderer {
                        mesh: handle,
                        color: Color(mesh.color),
                        material,
                    },
                ));
            }
            if let Some(model) = &e.model {
                for (mesh, at, color, material) in self.model(&model.path).into_iter().flatten() {
                    self.world.spawn((
                        Transform::IDENTITY,
                        GlobalTransform(*m * at),
                        MeshRenderer {
                            mesh,
                            color: Color(color),
                            material,
                        },
                    ));
                }
            }
            if let Some(camera) = &e.camera {
                self.world.spawn((
                    Transform::IDENTITY,
                    GlobalTransform(*m),
                    Camera::from(camera),
                ));
            }
            if let Some(light) = &e.light {
                self.world.spawn((
                    Transform::IDENTITY,
                    GlobalTransform(*m),
                    olvin::scene_file::runtime_light(light),
                ));
            }
        }
    }
}

impl ScenePreview {
    fn material(&mut self, rel: Option<&str>) -> Option<olvin::render::MaterialHandle> {
        self.assets.material(
            rel?,
            &mut Registries {
                textures: &mut self.textures,
                materials: &mut self.materials,
                shaders: &mut self.shaders,
            },
        )
    }

    /// A model's parts (imported on first use; static, skinned meshes in bind pose).
    fn model(&mut self, rel: &str) -> Option<Vec<ModelPart>> {
        if let Some(parts) = self.models.get(rel) {
            return parts.clone();
        }
        let parts = self
            .assets
            .root()
            .map(|r| r.to_path_buf())
            .and_then(|root| {
                let model = olvin_assets::import_gltf(
                    root.join(rel),
                    &root,
                    &olvin_assets::ModelImport::default(),
                )
                .ok()?;
                let materials: Vec<_> = model
                    .materials
                    .iter()
                    .enumerate()
                    .map(|(i, d)| {
                        self.assets.inline_material(
                            &format!("{rel}#{i}"),
                            d,
                            &mut Registries {
                                textures: &mut self.textures,
                                materials: &mut self.materials,
                                shaders: &mut self.shaders,
                            },
                        )
                    })
                    .collect();
                let parts = olvin::models::placements(&model)
                    .into_iter()
                    .map(|(i, at)| {
                        let prim = &model.meshes[i];
                        let material = prim.material.and_then(|m| materials.get(m).copied());
                        let color = if material.is_some() {
                            [1.0; 4]
                        } else {
                            [0.8; 4]
                        };
                        let mesh = self.meshes.add(olvin::models::primitive_mesh(prim));
                        (mesh, at, color, material)
                    })
                    .collect();
                Some(parts)
            });
        self.models.insert(rel.to_owned(), parts.clone());
        parts
    }

    /// The renderer's sky for the scene's skybox (loads the panorama).
    pub fn sky(&mut self, sky: &olvin::scene::file::Sky) -> olvin::render::SkyMode {
        self.assets.sky(sky, &mut self.textures)
    }

    /// Textured CSG: brushes that have a material are built here, grouped by
    /// (colour, material), so the engine draws them with textures.
    fn sync_csg(&mut self, scene: &SceneFile) {
        let key: Vec<_> = scene
            .csg_brushes()
            .into_iter()
            .filter(|b| {
                scene.active_in_hierarchy(b.id)
                    && scene.get(b.id).is_some_and(|e| e.rigidbody.is_none())
            })
            .map(|b| {
                let m = scene.get(b.id).and_then(|e| e.material.clone());
                (b, m)
            })
            .collect();
        if key != self.csg.0 {
            let inputs: Vec<_> = key.iter().map(|(b, _)| b.clone()).collect();
            let polys = olvin::csg::build(&inputs);
            let mut groups: HashMap<([u32; 4], String), Vec<&olvin::csg::Polygon>> = HashMap::new();
            for p in &polys {
                if let Some(m) = key
                    .iter()
                    .find(|(b, _)| b.id == p.brush)
                    .and_then(|(_, m)| m.clone())
                {
                    groups
                        .entry((p.color.map(f32::to_bits), m))
                        .or_default()
                        .push(p);
                }
            }
            let mut out = Vec::new();
            for ((color, material), polys) in groups {
                let tri = olvin::csg::TriMesh::from_polygons(polys);
                let mesh = self.meshes.add(MeshData::from_triangles(
                    &tri.positions,
                    &tri.normals,
                    tri.indices,
                ));
                out.push((mesh, color.map(f32::from_bits), Some(material)));
            }
            self.csg = (key, out);
        }
        for (mesh, color, material) in self.csg.1.clone() {
            let material = self.material(material.as_deref());
            self.world.spawn((
                Transform::IDENTITY,
                MeshRenderer {
                    mesh,
                    color: Color(color),
                    material,
                },
            ));
        }
    }
}

/// The game's key for an editor key (letters, digits, arrows, common keys).
fn key_code(key: egui::Key) -> Option<olvin::window::KeyCode> {
    use egui::Key as E;
    use olvin::window::KeyCode as K;
    let letters = [
        K::KeyA,
        K::KeyB,
        K::KeyC,
        K::KeyD,
        K::KeyE,
        K::KeyF,
        K::KeyG,
        K::KeyH,
        K::KeyI,
        K::KeyJ,
        K::KeyK,
        K::KeyL,
        K::KeyM,
        K::KeyN,
        K::KeyO,
        K::KeyP,
        K::KeyQ,
        K::KeyR,
        K::KeyS,
        K::KeyT,
        K::KeyU,
        K::KeyV,
        K::KeyW,
        K::KeyX,
        K::KeyY,
        K::KeyZ,
    ];
    let digits = [
        K::Digit0,
        K::Digit1,
        K::Digit2,
        K::Digit3,
        K::Digit4,
        K::Digit5,
        K::Digit6,
        K::Digit7,
        K::Digit8,
        K::Digit9,
    ];
    let name = key.name();
    if name.len() == 1 {
        let c = name.as_bytes()[0];
        if c.is_ascii_uppercase() {
            return Some(letters[(c - b'A') as usize]);
        }
        if c.is_ascii_digit() {
            return Some(digits[(c - b'0') as usize]);
        }
    }
    Some(match key {
        E::Space => K::Space,
        E::Enter => K::Enter,
        E::Escape => K::Escape,
        E::Tab => K::Tab,
        E::Backspace => K::Backspace,
        E::ArrowUp => K::ArrowUp,
        E::ArrowDown => K::ArrowDown,
        E::ArrowLeft => K::ArrowLeft,
        E::ArrowRight => K::ArrowRight,
        E::F1 => K::F1,
        E::F2 => K::F2,
        E::F3 => K::F3,
        E::F4 => K::F4,
        _ => return None,
    })
}

/// Lighting for a scene: its sun and environment (shared with the game loader).
pub fn settings_for(scene: &SceneFile) -> RenderSettings {
    let mut settings = RenderSettings::default();
    olvin::scene_file::apply_lighting(scene, &mut settings);
    settings
}

/// How the scene view is lit (Blender's viewport shading).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shading {
    /// Studio light from behind the camera, scene lights off: for modelling.
    Solid,
    /// Scene lights, shadows, environment and fog, as in the game.
    #[default]
    Rendered,
}

/// Neutral studio lighting that follows the view.
fn solid_settings(scene: &SceneFile, view_proj: Mat4) -> RenderSettings {
    let inv = view_proj.inverse();
    let forward = (inv.project_point3(Vec3::new(0.0, 0.0, 1.0)) - inv.project_point3(Vec3::ZERO))
        .normalize_or(Vec3::NEG_Z);
    let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
    RenderSettings {
        light_dir: (forward * 0.6 + right * 0.35 - Vec3::Y * 0.6).normalize(),
        light_color: Color([1.05, 1.03, 1.0, 1.0]),
        ambient: Color([0.36, 0.37, 0.4, 1.0]),
        ground_ambient: Color([0.16, 0.16, 0.17, 1.0]),
        clear_color: Color([
            scene.environment.background[0],
            scene.environment.background[1],
            scene.environment.background[2],
            1.0,
        ]),
        tonemapping: olvin::render::Tonemapping::None,
        scene_lights: false,
        ..RenderSettings::default()
    }
}

/// Editor-only quality switches from the Settings window.
fn editor_quality(
    prefs: &crate::prefs::Preferences,
    mut settings: RenderSettings,
) -> RenderSettings {
    if !prefs.viewport_shadows {
        settings.shadow_distance = 0.0;
        settings.max_spot_shadows = 0;
        settings.max_point_shadows = 0;
    }
    if !prefs.viewport_fog {
        settings.fog = None;
    }
    settings
}

/// Prepares engine view `id` for this frame: the running game while playing,
/// otherwise the edited scene. Returns what to draw, or `None` without a GPU
/// (or without a camera, when `view_proj` finds none).
pub(crate) fn prepare_view(
    state: &mut crate::EditorState,
    id: u64,
    frame: u64,
    shading: Shading,
    view_proj: impl FnOnce(&World) -> Option<Mat4>,
) -> Option<Source> {
    let gpu = state.gpu.clone()?;
    let mut renderer = gpu.renderer.write();
    let res = renderer
        .callback_resources
        .get_mut::<crate::render::Resources>()?;
    if let Some(play) = &state.play {
        let world = play.app.resources.fetch::<World>();
        let mut meshes = play.app.resources.fetch_mut::<Meshes>();
        let settings = play
            .app
            .resources
            .get::<RenderSettings>()
            .map(|s| s.clone())
            .unwrap_or_default();
        let vp = view_proj(&world)?;
        let settings = editor_quality(&state.prefs, settings);
        let r = &play.app.resources;
        let (mut t, mut m, mut s) = (
            r.fetch_mut::<Textures>(),
            r.fetch_mut::<Materials>(),
            r.fetch_mut::<Shaders>(),
        );
        let regs = Registries {
            textures: &mut t,
            materials: &mut m,
            shaders: &mut s,
        };
        let stats = res
            .engine
            .prepare(id, Source::Play, &world, &mut meshes, regs, vp, &settings);
        state.render_stats.insert(id, stats);
        Some(Source::Play)
    } else {
        let assets_dir = state.assets_dir();
        state.engine_scene.sync(&state.scene, frame, assets_dir);
        let vp = view_proj(&state.engine_scene.world)?;
        let mut settings = match shading {
            Shading::Rendered => settings_for(&state.scene),
            Shading::Solid => solid_settings(&state.scene, vp),
        };
        if shading == Shading::Rendered {
            settings.sky = state.engine_scene.sky(&state.scene.environment.skybox);
        }
        let settings = editor_quality(&state.prefs, settings);
        let p = &mut state.engine_scene;
        let regs = Registries {
            textures: &mut p.textures,
            materials: &mut p.materials,
            shaders: &mut p.shaders,
        };
        let stats = res.engine.prepare(
            id,
            Source::Edit,
            &p.world,
            &mut p.meshes,
            regs,
            vp,
            &settings,
        );
        state.render_stats.insert(id, stats);
        Some(Source::Edit)
    }
}

/// Forgets GPU meshes of the previous game (a new session restarts mesh handles).
pub(crate) fn reset_play_gpu(state: &crate::EditorState) {
    if let Some(gpu) = &state.gpu
        && let Some(res) = gpu
            .renderer
            .write()
            .callback_resources
            .get_mut::<crate::render::Resources>()
    {
        res.engine.reset_play();
    }
}

/// A running game inside the editor.
pub struct PlaySession {
    pub app: App,
    pub paused: bool,
    step: bool,
    pub(crate) saved: SceneFile,
    pub(crate) undo_len: usize,
}

impl PlaySession {
    pub fn start(
        scene: &SceneFile,
        undo_len: usize,
        assets_dir: Option<std::path::PathBuf>,
    ) -> Self {
        let mut app = App::new();
        // The editor cuts profiler frames itself, once per editor frame.
        app.set_profile_frames(false);
        app.add_plugin(EcsPlugin)
            .add_plugin(TransformPlugin)
            .add_plugin(PhysicsPlugin::default());
        app.insert_resource(Meshes::default())
            .insert_resource(Textures::default())
            .insert_resource(Materials::default())
            .insert_resource(Shaders::default())
            .insert_resource(RenderAssets::new(assets_dir.clone()))
            .insert_resource(settings_for(scene));
        // Scene models with their animators (skinned meshes stay in bind
        // pose: CPU skinning is part of RenderPlugin, which Play doesn't run).
        if let Some(dir) = &assets_dir {
            app.insert_resource(olvin_assets::AssetManager::new(dir.clone()));
        }
        app.add_plugin(olvin::anim::AnimPlugin)
            .add_plugin(olvin::models::ModelPlugin);
        // Scripts (the built-in behaviours; game code isn't linked into the
        // editor) and the project's input map, fed from the Game view.
        let map = assets_dir.map(|d| olvin::input::InputMap::load_or_default(d.join("input.ron")));
        app.add_plugin(olvin::input::InputPlugin { map })
            .add_plugin(olvin::script::ScriptPlugin::default());
        olvin::scene_file::spawn(&app.resources, scene);
        app.startup();
        Self {
            app,
            paused: false,
            step: false,
            saved: scene.clone(),
            undo_len,
        }
    }

    pub fn step_once(&mut self) {
        self.step = true;
    }

    /// Feeds the Game view's keyboard and mouse into the game (`focused`:
    /// the pointer is over the view); otherwise everything is released.
    pub fn feed_input(&mut self, input: &egui::InputState, focused: bool) {
        use olvin::window::{Input, MouseButton};
        let Some(mut game) = self.app.resources.get_mut::<Input>() else {
            return;
        };
        if !focused {
            game.release_all();
            return;
        }
        for event in &input.events {
            if let egui::Event::Key {
                key,
                pressed,
                repeat: false,
                ..
            } = event
                && let Some(code) = key_code(*key)
            {
                game.set_key(code, *pressed);
            }
        }
        use olvin::window::KeyCode as K;
        let m = input.modifiers;
        game.set_key(K::ShiftLeft, m.shift);
        game.set_key(K::ControlLeft, m.ctrl);
        game.set_key(K::AltLeft, m.alt);
        for (b, down) in [
            (MouseButton::Left, input.pointer.primary_down()),
            (MouseButton::Right, input.pointer.secondary_down()),
            (MouseButton::Middle, input.pointer.middle_down()),
        ] {
            game.set_button(b, down);
        }
        let d = input.pointer.delta();
        game.mouse_delta += glam::Vec2::new(d.x, d.y);
        game.scroll +=
            glam::Vec2::new(input.smooth_scroll_delta.x, input.smooth_scroll_delta.y) / 50.0;
    }

    /// One editor frame of game time.
    pub fn tick(&mut self) {
        if !self.paused || std::mem::take(&mut self.step) {
            self.app.update();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use olvin::scene::file::{ColliderDesc, EntityDesc, MeshDesc, RigidbodyDesc};

    #[test]
    fn play_session_runs_physics_and_leaves_the_scene_alone() {
        let mut ground = EntityDesc::new(1, "Ground");
        ground.mesh = Some(MeshDesc {
            shape: Shape::Cube,
            color: [1.0; 4],
        });
        ground.collider = Some(ColliderDesc::for_shape(Shape::Cube));
        ground.scale = Vec3::new(10.0, 1.0, 10.0);
        let mut ball = EntityDesc::new(2, "Ball");
        ball.mesh = Some(MeshDesc {
            shape: Shape::Sphere,
            color: [1.0; 4],
        });
        ball.collider = Some(ColliderDesc::for_shape(Shape::Sphere));
        ball.rigidbody = Some(RigidbodyDesc::default());
        ball.position = Vec3::new(0.0, 5.0, 0.0);
        let scene = SceneFile {
            entities: vec![ground, ball],
            ..Default::default()
        };

        let mut play = PlaySession::start(&scene, 0, None);
        // Physics runs on game time; push fixed steps directly to keep the test fast.
        for _ in 0..200 {
            olvin::physics::step(&play.app.resources, olvin::physics::FIXED_DT);
        }
        play.tick();
        let world = play.app.resources.fetch::<World>();
        let y = world
            .query::<(&olvin::scene::Name, &Transform)>()
            .iter()
            .find(|(n, _)| n.0 == "Ball")
            .map(|(_, t)| t.translation.y)
            .unwrap();
        assert!(
            (y - 1.0).abs() < 0.1,
            "ball rests on the 1 m thick ground: {y}"
        );
        assert_eq!(play.saved, scene);
    }

    /// The running game plays the authored animation through `olvin_anim`.
    #[test]
    fn play_mode_runs_the_authored_animation() {
        use olvin::scene::SceneId;
        let mut scene = SceneFile {
            entities: vec![EntityDesc::new(7, "Mover")],
            ..Default::default()
        };
        scene.animation.frame_end = 25;
        scene
            .animation
            .insert_key(7, olvin_scene::file::Prop::Location, 0, 1.0, 0.0);
        scene
            .animation
            .insert_key(7, olvin_scene::file::Prop::Location, 0, 25.0, 12.0);
        let mut play = PlaySession::start(&scene, 0, None);
        play.app
            .resources
            .fetch_mut::<olvin::core::Time>()
            .set_fixed_delta(Some(0.5));
        play.tick();
        let world = play.app.resources.fetch::<World>();
        let x = world
            .query::<(&SceneId, &Transform)>()
            .iter()
            .find(|(id, _)| id.0 == 7)
            .map(|(_, t)| t.translation.x)
            .unwrap();
        // Half a second at 24 fps is frame 13: the middle of a symmetric ease.
        assert!((x - 6.0).abs() < 1e-3, "x = {x}");
    }

    #[test]
    fn preview_mirrors_meshes_and_cameras() {
        let mut cube = EntityDesc::new(1, "Cube");
        cube.mesh = Some(MeshDesc {
            shape: Shape::Cube,
            color: [1.0; 4],
        });
        let mut cam = EntityDesc::new(2, "Camera");
        cam.camera = Some(Default::default());
        let mut hidden = EntityDesc::new(3, "Hidden");
        hidden.mesh = Some(MeshDesc {
            shape: Shape::Cube,
            color: [1.0; 4],
        });
        hidden.active = false;
        let scene = SceneFile {
            entities: vec![cube, cam, hidden],
            ..Default::default()
        };
        let mut preview = ScenePreview::default();
        preview.sync(&scene, 1, None);
        assert_eq!(preview.world.query::<&MeshRenderer>().iter().count(), 1);
        assert_eq!(preview.world.query::<&Camera>().iter().count(), 1);
        preview.sync(&scene, 1, None);
        assert_eq!(preview.world.len(), 2, "same frame: not rebuilt twice");
    }
}

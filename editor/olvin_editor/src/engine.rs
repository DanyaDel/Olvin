//! The engine inside the editor: scenes are drawn by the game's own renderer
//! (`olvin_render::RenderCore`, same wgpu as egui), and Play Mode runs a real
//! engine `App` with the game's plugins.

use std::collections::HashMap;

use eframe::egui_wgpu;
use glam::Mat4;
use olvin::core::App;
use olvin::ecs::{EcsPlugin, World};
use olvin::physics::PhysicsPlugin;
use olvin::render::{
    Color, GpuMeshes, MeshData, MeshHandle, MeshRenderer, Meshes, RenderCore, RenderSettings,
    ViewData,
};
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
    pub fn prepare(
        &mut self,
        id: u64,
        source: Source,
        world: &World,
        meshes: &mut Meshes,
        view_proj: Mat4,
        settings: &RenderSettings,
    ) {
        let gpu = match source {
            Source::Edit => &mut self.edit,
            Source::Play => &mut self.play,
        };
        self.core.upload(gpu, meshes);
        let view = self
            .views
            .entry(id)
            .or_insert_with(|| self.core.create_view());
        self.core.prepare(view, gpu, world, view_proj, settings);
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

/// The edited scene as an engine world, rebuilt once per frame.
#[derive(Default)]
pub struct ScenePreview {
    pub world: World,
    pub meshes: Meshes,
    shapes: HashMap<Shape, MeshHandle>,
    /// Edited meshes: entity -> (handle, geometry it was uploaded from).
    polys: HashMap<u64, (MeshHandle, olvin::scene::file::PolyMesh)>,
    frame: Option<u64>,
}

impl ScenePreview {
    pub fn sync(&mut self, scene: &SceneFile, frame: u64) {
        if self.frame == Some(frame) {
            return;
        }
        self.frame = Some(frame);
        self.world.clear();
        let worlds = scene.world_matrices();
        for e in scene
            .entities
            .iter()
            .filter(|e| scene.active_in_hierarchy(e.id))
        {
            let Some(m) = worlds.get(&e.id) else { continue };
            if let Some(poly) = &e.poly {
                let color = e.mesh.as_ref().map_or([0.8; 4], |m| m.color);
                let handle = match self.polys.get(&e.id) {
                    Some((h, uploaded)) if uploaded == poly => *h,
                    cached => {
                        let (p, n, i) = poly.triangulate();
                        let data = MeshData::from_triangles(&p, &n, i);
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
                self.world.spawn((
                    Transform::IDENTITY,
                    GlobalTransform(*m),
                    MeshRenderer {
                        mesh: handle,
                        color: Color(color),
                    },
                ));
            } else if let Some(mesh) = &e.mesh {
                let meshes = &mut self.meshes;
                let handle = *self
                    .shapes
                    .entry(mesh.shape)
                    .or_insert_with(|| meshes.add(MeshData::primitive(mesh.shape)));
                self.world.spawn((
                    Transform::IDENTITY,
                    GlobalTransform(*m),
                    MeshRenderer {
                        mesh: handle,
                        color: Color(mesh.color),
                    },
                ));
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

/// Lighting for a scene: its first directional light, or a default sun.
pub fn settings_for(scene: &SceneFile) -> RenderSettings {
    let mut settings = RenderSettings::default();
    if let Some((dir, color, intensity)) = scene.sun() {
        settings.light_dir = dir;
        settings.light_color = Color([
            color[0] * intensity,
            color[1] * intensity,
            color[2] * intensity,
            1.0,
        ]);
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
        res.engine
            .prepare(id, Source::Play, &world, &mut meshes, vp, &settings);
        Some(Source::Play)
    } else {
        state.engine_scene.sync(&state.scene, frame);
        let vp = view_proj(&state.engine_scene.world)?;
        let settings = settings_for(&state.scene);
        res.engine.prepare(
            id,
            Source::Edit,
            &state.engine_scene.world,
            &mut state.engine_scene.meshes,
            vp,
            &settings,
        );
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
    pub fn start(scene: &SceneFile, undo_len: usize) -> Self {
        let mut app = App::new();
        app.add_plugin(EcsPlugin)
            .add_plugin(TransformPlugin)
            .add_plugin(PhysicsPlugin::default());
        app.insert_resource(Meshes::default())
            .insert_resource(settings_for(scene));
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

        let mut play = PlaySession::start(&scene, 0);
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
        preview.sync(&scene, 1);
        assert_eq!(preview.world.query::<&MeshRenderer>().iter().count(), 1);
        assert_eq!(preview.world.query::<&Camera>().iter().count(), 1);
        preview.sync(&scene, 1);
        assert_eq!(preview.world.len(), 2, "same frame: not rebuilt twice");
    }
}

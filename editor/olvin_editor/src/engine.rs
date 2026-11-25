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
    Color, GpuMeshes, MeshData, MeshHandle, MeshRenderer, Meshes, ParticleRenderer, RenderCore,
    RenderSettings, ViewData,
};
use olvin::render::{
    HDR_FORMAT, Materials, POST_DEPTH_FORMAT, PostProcess, Shaders, Textures, begin_scene_pass,
};
use olvin::render_assets::{Registries, RenderAssets};
use olvin::scene::file::{SceneFile, Shape};
use olvin::scene::{Camera, Transform};
use olvin::transform::{GlobalTransform, TransformPlugin};

use crate::render::{DEPTH_FORMAT, MSAA};

/// Which world a view draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// The scene being edited.
    Edit,
    /// The running game.
    Play,
}

/// GPU half, stored in egui's callback resources.
///
/// Two ways to draw, by tier. Compat (GL): the core draws straight into
/// egui's MSAA pass, tonemapping in the mesh shader. Full: like the game, the
/// core draws an HDR frame offscreen and the game's post chain (bloom, AO,
/// FXAA, colour grading, tonemapping) turns it into an image that the
/// viewport composites into egui's pass together with the scene's depth, so
/// editor geometry is still hidden behind objects.
pub struct EngineGpu {
    pub core: RenderCore,
    edit: GpuMeshes,
    play: GpuMeshes,
    views: HashMap<u64, ViewData>,
    /// Particles of each view and source (GPU emitters simulate once per renderer).
    particles: HashMap<(u64, Source), ParticleRenderer>,
    device: eframe::wgpu::Device,
    queue: eframe::wgpu::Queue,
    /// egui's target format: what the viewport and the preview end up in.
    format: eframe::wgpu::TextureFormat,
    /// What the core draws into: (colour, samples, depth).
    scene_target: (
        eframe::wgpu::TextureFormat,
        u32,
        eframe::wgpu::TextureFormat,
    ),
    /// Full tier: HDR scene + post chain per view (see the type's docs).
    hdr: bool,
    posts: HashMap<u64, ViewPost>,
    /// Offscreen target of the material preview (made on first use).
    preview: Option<PreviewTarget>,
}

/// One view's post chain and what it needs at render time.
struct ViewPost {
    post: PostProcess,
    /// The post chain's output (egui's format), at the view's pixel size.
    out: Option<(eframe::wgpu::TextureView, (u32, u32))>,
    settings: RenderSettings,
    view_proj: Mat4,
}

/// Where a material preview is drawn, resolved into a texture egui shows
/// as an image: through MSAA (Compat) or through a post chain (Full).
pub(crate) struct PreviewTarget {
    /// Compat: MSAA colour and depth.
    direct: Option<(eframe::wgpu::TextureView, eframe::wgpu::TextureView)>,
    /// Full: the post chain.
    post: Option<PostProcess>,
    pub resolved: eframe::wgpu::TextureView,
    /// Set once the texture is registered with egui.
    pub egui_id: Option<egui::TextureId>,
}

/// Engine view id of the material preview (scene views use small ids).
const PREVIEW_VIEW: u64 = u64::MAX;

fn color_target(
    device: &eframe::wgpu::Device,
    format: eframe::wgpu::TextureFormat,
    samples: u32,
    (w, h): (u32, u32),
    sampled: bool,
) -> eframe::wgpu::TextureView {
    use eframe::wgpu;
    let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    if sampled {
        usage |= wgpu::TextureUsages::TEXTURE_BINDING;
    }
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("olvin editor view"),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

impl EngineGpu {
    pub fn new(state: &egui_wgpu::RenderState) -> Self {
        let hdr = olvin::render::tier_of(&state.adapter) == olvin::render::RenderTier::Full;
        let scene_target = if hdr {
            (HDR_FORMAT, 1, POST_DEPTH_FORMAT)
        } else {
            (state.target_format, MSAA, DEPTH_FORMAT)
        };
        let mut core = RenderCore::new(
            state.device.clone(),
            state.queue.clone(),
            scene_target.0,
            scene_target.1,
            scene_target.2,
        );
        core.set_hdr_output(hdr);
        Self {
            core,
            edit: GpuMeshes::default(),
            play: GpuMeshes::default(),
            views: HashMap::new(),
            particles: HashMap::new(),
            device: state.device.clone(),
            queue: state.queue.clone(),
            format: state.target_format,
            scene_target,
            hdr,
            posts: HashMap::new(),
            preview: None,
        }
    }

    /// Full tier: views go through the post chain and are composited.
    pub fn is_hdr(&self) -> bool {
        self.hdr
    }

    /// Draws `world` from `view_proj` into the preview target (`size`
    /// pixels square) and submits it; returns the target.
    pub(crate) fn render_preview(
        &mut self,
        world: &World,
        meshes: &mut Meshes,
        assets: Registries,
        view_proj: Mat4,
        settings: &RenderSettings,
        size: u32,
    ) -> &mut PreviewTarget {
        use eframe::wgpu;
        let gpu = &mut self.edit;
        self.core.upload(gpu, meshes);
        self.core
            .upload_assets(gpu, assets.textures, assets.materials, assets.shaders);
        let view = self
            .views
            .entry(PREVIEW_VIEW)
            .or_insert_with(|| self.core.create_view());
        self.core.prepare(view, gpu, world, view_proj, settings);
        let device = &self.device;
        let (format, hdr, scene) = (self.format, self.hdr, self.scene_target);
        let target = self.preview.get_or_insert_with(|| PreviewTarget {
            direct: (!hdr).then(|| {
                (
                    color_target(device, scene.0, scene.1, (size, size), false),
                    color_target(device, scene.2, scene.1, (size, size), false),
                )
            }),
            post: hdr.then(|| PostProcess::new(device, format)),
            resolved: color_target(device, format, 1, (size, size), true),
            egui_id: None,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        self.core.render_shadows(view, &self.edit, &mut encoder);
        if let Some(post) = &mut target.post {
            post.resize(device, (size, size));
            post.sync_lut(device, &self.edit, settings);
            if let (Some(hdr_view), Some(depth)) = (post.hdr_view(), post.depth_view()) {
                let ao = settings.post.ao.is_some();
                let mut pass = begin_scene_pass(&mut encoder, hdr_view, depth, settings, ao);
                self.core.draw(view, &self.edit, &mut pass);
            }
            post.run(
                &self.queue,
                &mut encoder,
                &target.resolved,
                settings,
                view_proj,
            );
        } else if let Some((msaa, depth)) = &target.direct {
            let [r, g, b, a] = settings.clear_color.0.map(f64::from);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("material preview"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: msaa,
                    depth_slice: None,
                    resolve_target: Some(&target.resolved),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            self.core.draw(view, &self.edit, &mut pass);
        }
        self.queue.submit([encoder.finish()]);
        target
    }

    pub(crate) fn set_preview_egui_id(&mut self, id: egui::TextureId) {
        if let Some(t) = &mut self.preview {
            t.egui_id = Some(id);
        }
    }

    pub fn reset_play(&mut self) {
        self.play = GpuMeshes::default();
        self.particles.retain(|(_, s), _| *s != Source::Play);
    }

    /// Uploads new meshes and prepares view `id`; render it later with
    /// [`Self::render_offscreen`] and [`Self::draw`] (or the composite).
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
        particles: &World,
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
        let stats = self.core.prepare(view, gpu, world, view_proj, settings);
        let (device, queue, scene, hdr) = (&self.device, &self.queue, self.scene_target, self.hdr);
        self.particles
            .entry((id, source))
            .or_insert_with(|| {
                let mut p = ParticleRenderer::new(device, queue, scene.0, scene.1, scene.2);
                p.set_hdr_output(hdr);
                p
            })
            .prepare(device, queue, gpu, particles, view_proj);
        if hdr {
            let format = self.format;
            let post = self.posts.entry(id).or_insert_with(|| ViewPost {
                post: PostProcess::new(device, format),
                out: None,
                settings: RenderSettings::default(),
                view_proj: Mat4::IDENTITY,
            });
            post.settings = settings.clone();
            post.view_proj = view_proj;
        }
        stats
    }

    /// Before egui's pass: the shadow maps of view `id`, and on the Full tier
    /// its HDR frame and post chain into an image of `size` pixels.
    pub fn render_offscreen(
        &mut self,
        id: u64,
        source: Source,
        encoder: &mut eframe::wgpu::CommandEncoder,
        size: (u32, u32),
    ) {
        let gpu = match source {
            Source::Edit => &self.edit,
            Source::Play => &self.play,
        };
        let Some(view) = self.views.get(&id) else {
            return;
        };
        self.core.render_shadows(view, gpu, encoder);
        let Some(vp) = self.posts.get_mut(&id).filter(|_| self.hdr) else {
            return;
        };
        let size = (size.0.max(1), size.1.max(1));
        // Retro low internal resolution is scaled up by the chain, as in the game.
        let scene_size = vp.settings.retro.scene_size(size).unwrap_or(size);
        vp.post.resize(&self.device, scene_size);
        vp.post.sync_lut(&self.device, gpu, &vp.settings);
        if vp.out.as_ref().is_none_or(|(_, s)| *s != size) {
            vp.out = Some((color_target(&self.device, self.format, 1, size, true), size));
        }
        let (Some(hdr), Some(depth), Some((out, _))) =
            (vp.post.hdr_view(), vp.post.depth_view(), &vp.out)
        else {
            return;
        };
        {
            // Kept even without AO: the composite writes it into egui's pass,
            // and a discarded depth would hide every editor line and the grid.
            let mut pass = begin_scene_pass(encoder, hdr, depth, &vp.settings, true);
            self.core.draw(view, gpu, &mut pass);
            if let Some(particles) = self.particles.get(&(id, source)) {
                particles.draw(&mut pass);
            }
        }
        vp.post
            .run(&self.queue, encoder, out, &vp.settings, vp.view_proj);
    }

    /// Full tier: the finished image of view `id` and its scene depth, for
    /// the composite into egui's pass.
    pub fn composite_views(
        &self,
        id: u64,
    ) -> Option<(&eframe::wgpu::TextureView, &eframe::wgpu::TextureView)> {
        let vp = self.posts.get(&id).filter(|_| self.hdr)?;
        Some((&vp.out.as_ref()?.0, vp.post.depth_view()?))
    }

    /// Compat tier: draws view `id` straight into egui's pass.
    pub fn draw(&self, id: u64, source: Source, pass: &mut eframe::wgpu::RenderPass<'static>) {
        if self.hdr {
            return;
        }
        let gpu = match source {
            Source::Edit => &self.edit,
            Source::Play => &self.play,
        };
        if let Some(view) = self.views.get(&id) {
            self.core.draw(view, gpu, pass);
        }
        if let Some(particles) = self.particles.get(&(id, source)) {
            particles.draw(pass);
        }
    }
}

/// A brush and its material path: what the textured CSG was built from.
type CsgKey = (olvin::csg::BrushInput, Option<String>);
/// A textured CSG mesh: (mesh, colour, material path).
type CsgMesh = (MeshHandle, [f32; 4], Option<String>);
/// A static model part in its own space: mesh, placement, colour, material.
type ModelPart = (
    MeshHandle,
    Mat4,
    [f32; 4],
    Option<olvin::render::MaterialHandle>,
);

/// A model imported for the scene view: static parts shared by every
/// instance, plus what skinned instances ([`Rig`]) are built from.
struct CachedModel {
    data: std::sync::Arc<olvin_assets::ModelData>,
    materials: Vec<olvin::render::MaterialHandle>,
    statics: Vec<ModelPart>,
    /// Primitives with a skin (indices into `data.meshes`).
    skinned: Vec<usize>,
}

/// Scene-view preview of a model's animation: where the editor plays a clip
/// (or the scene's own setup) on the selected model. Lives in the editor
/// state only: never saved, never an undo step.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPreview {
    pub playing: bool,
    /// Clip to play; `None` plays the object's own `ModelDesc::animation`.
    pub clip: Option<String>,
    pub speed: f32,
    pub looping: bool,
}

impl Default for ModelPreview {
    fn default() -> Self {
        Self {
            playing: false,
            clip: None,
            speed: 1.0,
            looping: true,
        }
    }
}

/// What a [`Rig`]'s animators were built for.
#[derive(Clone, Debug, PartialEq)]
enum RigSetup {
    /// Rest pose, nothing plays.
    Rest,
    Playing {
        animation: Option<olvin::scene::file::AnimationDesc>,
        looping: bool,
    },
}

struct RigPart {
    mesh: MeshHandle,
    skin: olvin::render::SkinnedMesh,
    /// Index of the animator (skeleton) that poses it.
    skeleton: usize,
    color: [f32; 4],
    material: Option<olvin::render::MaterialHandle>,
}

/// The skinned parts of one model object, with their animators; persists
/// across frames (the preview world is rebuilt every frame, these are not).
struct Rig {
    path: String,
    parts: Vec<RigPart>,
    animators: Vec<olvin::anim::Animator>,
    setup: RigSetup,
}

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
    /// Particle systems running in the Scene view (own world, see `fx`).
    pub fx: crate::fx::FxPreview,
    /// CSG of brushes with materials (the CSG element draws the rest itself):
    /// rebuilt when the brush inputs or their materials change.
    csg: (Vec<CsgKey>, Vec<CsgMesh>),
    /// glTF models by path, imported once (`None`: failed).
    models: HashMap<String, Option<std::sync::Arc<CachedModel>>>,
    /// Skinned models by object id: bind pose until a preview plays.
    rigs: HashMap<u64, Rig>,
    /// When the animations were last advanced.
    clock: Option<std::time::Instant>,
    frame: Option<u64>,
    /// Terrains by object id: their data file and built parts.
    terrains: HashMap<u64, (String, olvin::terrain::TerrainParts)>,
    terrain_frame: Option<u64>,
}

impl ScenePreview {
    /// Rebuilds the world for `frame` (once per frame, however many views
    /// ask) and advances the playing [`ModelPreview`]s by the time since the
    /// last call.
    pub fn sync(
        &mut self,
        scene: &SceneFile,
        frame: u64,
        assets_dir: Option<std::path::PathBuf>,
        previews: &HashMap<u64, ModelPreview>,
    ) {
        let now = std::time::Instant::now();
        let dt = self
            .clock
            .replace(now)
            .map_or(0.0, |t| (now - t).as_secs_f32().min(0.1));
        self.sync_dt(scene, frame, assets_dir, previews, dt);
    }

    /// [`Self::sync`] with an explicit animation time step.
    pub fn sync_dt(
        &mut self,
        scene: &SceneFile,
        frame: u64,
        assets_dir: Option<std::path::PathBuf>,
        previews: &HashMap<u64, ModelPreview>,
        dt: f32,
    ) {
        if self.frame == Some(frame) {
            return;
        }
        self.frame = Some(frame);
        let mut seen = Vec::new();
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
            if let Some(desc) = &e.model {
                seen.push(e.id);
                self.spawn_model(e.id, desc, *m, previews.get(&e.id), dt);
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
        self.rigs.retain(|id, _| seen.contains(id));
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

    /// Adds the scene's terrains to the world rebuilt for `frame` (after
    /// [`Self::sync`]): parts are built once per object and then follow the
    /// data's changed rectangles and the description.
    pub(crate) fn sync_terrains(
        &mut self,
        scene: &SceneFile,
        frame: u64,
        store: &mut crate::terrain::TerrainStore,
    ) {
        if self.terrain_frame == Some(frame) {
            return;
        }
        self.terrain_frame = Some(frame);
        let worlds = scene.world_matrices();
        let mut seen = Vec::new();
        for e in scene
            .entities
            .iter()
            .filter(|e| scene.active_in_hierarchy(e.id))
        {
            let (Some(desc), Some(m)) = (&e.terrain, worlds.get(&e.id)) else {
                continue;
            };
            seen.push(e.id);
            let root = self.assets.root().map(|p| p.to_path_buf());
            let data = &store.entry(root.as_deref(), &desc.data).data;
            let mut r = Registries {
                textures: &mut self.textures,
                materials: &mut self.materials,
                shaders: &mut self.shaders,
            };
            let rebuild = self
                .terrains
                .get(&e.id)
                .is_none_or(|(rel, _)| *rel != desc.data);
            if rebuild {
                let parts = olvin::terrain::TerrainParts::build(
                    data,
                    desc,
                    &mut self.meshes,
                    &mut self.assets,
                    &mut r,
                );
                self.terrains.insert(e.id, (desc.data.clone(), parts));
                // Built from the current data: nothing is pending for it.
                store.take_dirty(&desc.data);
            }
            let (heights, weights) = store.take_dirty(&desc.data);
            let data = &store.entry(root.as_deref(), &desc.data).data;
            let Some((_, parts)) = self.terrains.get_mut(&e.id) else {
                continue;
            };
            parts.sync_desc(data, desc, &mut self.meshes, &mut self.assets, &mut r);
            if let Some(rect) = heights {
                parts.update_heights(&mut self.meshes, data, rect);
            }
            if weights {
                parts.update_weights(r.textures, data);
            }
            for chunk in &parts.chunks {
                self.world.spawn((
                    Transform::IDENTITY,
                    GlobalTransform(*m),
                    MeshRenderer {
                        mesh: chunk.mesh,
                        color: Color([1.0; 4]),
                        material: parts.material,
                    },
                ));
            }
        }
        self.terrains.retain(|id, _| seen.contains(id));
    }

    /// A lone sphere wearing `desc` (kept under `key`, rebuilt when `desc`
    /// changes, so unsaved edits show at once), for the material preview.
    pub(crate) fn material_preview_world(
        &mut self,
        key: &str,
        desc: &olvin::scene::material::MaterialDesc,
    ) -> World {
        let meshes = &mut self.meshes;
        let sphere = *self
            .shapes
            .entry(Shape::Sphere)
            .or_insert_with(|| meshes.add(MeshData::primitive(Shape::Sphere)));
        let material = self.assets.inline_material(
            key,
            desc,
            &mut Registries {
                textures: &mut self.textures,
                materials: &mut self.materials,
                shaders: &mut self.shaders,
            },
        );
        let mut world = World::new();
        world.spawn((
            Transform::IDENTITY,
            GlobalTransform(Mat4::IDENTITY),
            MeshRenderer {
                mesh: sphere,
                color: Color([1.0; 4]),
                material: Some(material),
            },
        ));
        world
    }

    /// A model (imported on first use; `None`: it failed).
    fn model(&mut self, rel: &str) -> Option<std::sync::Arc<CachedModel>> {
        if let Some(model) = self.models.get(rel) {
            return model.clone();
        }
        let cached = self
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
                let mut statics = Vec::new();
                let mut skinned = Vec::new();
                for (i, at) in olvin::models::placements(&model) {
                    let prim = &model.meshes[i];
                    if prim.skin.is_some() && !prim.joints.is_empty() {
                        skinned.push(i);
                        continue;
                    }
                    let material = prim.material.and_then(|m| materials.get(m).copied());
                    let color = if material.is_some() {
                        [1.0; 4]
                    } else {
                        [0.8; 4]
                    };
                    let mesh = self.meshes.add(olvin::models::primitive_mesh(prim));
                    statics.push((mesh, at, color, material));
                }
                Some(std::sync::Arc::new(CachedModel {
                    data: std::sync::Arc::new(model),
                    materials,
                    statics,
                    skinned,
                }))
            });
        self.models.insert(rel.to_owned(), cached.clone());
        cached
    }

    /// Names of the clips of the model at `rel` (imports it on first use;
    /// empty when it fails or has none).
    pub fn model_clips(
        &mut self,
        assets_dir: Option<std::path::PathBuf>,
        rel: &str,
    ) -> Vec<String> {
        self.assets.set_root(assets_dir);
        self.model(rel)
            .map(|m| m.data.clips.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default()
    }

    /// Spawns a model object's parts into the preview world: static parts
    /// at their nodes, skinned parts posed by the object's [`Rig`] (rest
    /// pose, or the preview's animation advanced by `dt`).
    fn spawn_model(
        &mut self,
        id: u64,
        desc: &olvin::scene::file::ModelDesc,
        m: Mat4,
        preview: Option<&ModelPreview>,
        dt: f32,
    ) {
        let Some(model) = self.model(&desc.path) else {
            return;
        };
        for &(mesh, at, color, material) in &model.statics {
            self.world.spawn((
                Transform::IDENTITY,
                GlobalTransform(m * at),
                MeshRenderer {
                    mesh,
                    color: Color(color),
                    material,
                },
            ));
        }
        if model.skinned.is_empty() {
            return;
        }
        let mut rig = match self.rigs.remove(&id) {
            Some(rig) if rig.path == desc.path => rig,
            _ => self.build_rig(&desc.path, &model),
        };
        let setup = match preview {
            Some(p) if p.playing => RigSetup::Playing {
                animation: p
                    .clip
                    .clone()
                    .map(olvin::scene::file::AnimationDesc::Clip)
                    .or_else(|| desc.animation.clone()),
                looping: p.looping,
            },
            _ => RigSetup::Rest,
        };
        if rig.setup != setup {
            rig.animators = self.build_animators(&model, &setup);
            rig.setup = setup;
            rig.parts.iter_mut().for_each(|p| p.skin.invalidate());
        }
        if let (Some(p), RigSetup::Playing { .. }) = (preview, &rig.setup) {
            for a in &mut rig.animators {
                a.speed = p.speed;
                a.update(dt);
            }
        }
        for part in &mut rig.parts {
            if let Some(a) = rig.animators.get(part.skeleton) {
                part.skin
                    .pose(&mut self.meshes, part.mesh, a.version, &a.matrices);
            }
            self.world.spawn((
                Transform::IDENTITY,
                GlobalTransform(m),
                MeshRenderer {
                    mesh: part.mesh,
                    color: Color(part.color),
                    material: part.material,
                },
            ));
        }
        self.rigs.insert(id, rig);
    }

    fn build_rig(&mut self, path: &str, model: &CachedModel) -> Rig {
        let parts = model
            .skinned
            .iter()
            .filter_map(|&i| {
                let prim = &model.data.meshes[i];
                let skin = olvin::render::SkinnedMesh::new(
                    olvin::models::primitive_mesh(prim),
                    prim.joints.clone(),
                    prim.weights.clone(),
                    olvin::ecs::Entity::DANGLING,
                )
                .ok()?;
                let mesh = self.meshes.add(skin.bind_mesh());
                let material = prim.material.and_then(|m| model.materials.get(m).copied());
                let color = match (
                    material,
                    prim.material.and_then(|m| model.data.materials.get(m)),
                ) {
                    (None, Some(m)) => m.base_color,
                    (None, None) => [0.8; 4],
                    (Some(_), _) => [1.0; 4],
                };
                Some(RigPart {
                    mesh,
                    skin,
                    skeleton: prim.skin.unwrap_or(0),
                    color,
                    material,
                })
            })
            .collect();
        Rig {
            path: path.to_owned(),
            parts,
            animators: self.build_animators(model, &RigSetup::Rest),
            setup: RigSetup::Rest,
        }
    }

    /// One animator per skeleton of the model for `setup` (a broken one
    /// stays in the rest pose, with a warning).
    fn build_animators(&self, model: &CachedModel, setup: &RigSetup) -> Vec<olvin::anim::Animator> {
        use olvin::scene::file::{AnimGraphDesc, AnimationDesc};
        let data = &model.data;
        let graph_file = match setup {
            RigSetup::Playing {
                animation: Some(AnimationDesc::Graph(path)),
                ..
            } => self
                .assets
                .root()
                .ok_or_else(|| "no assets folder".to_owned())
                .and_then(|root| AnimGraphDesc::load(root.join(path)))
                .map_err(|e| log::warn!("{path}: {e}"))
                .ok(),
            _ => None,
        };
        (0..data.skeletons.len())
            .filter_map(|skin| {
                let rest = data.animator(skin)?;
                let RigSetup::Playing { animation, looping } = setup else {
                    return Some(rest);
                };
                let built = match animation {
                    Some(AnimationDesc::Clip(name)) => data
                        .clips
                        .iter()
                        .position(|c| &c.name == name)
                        .map(|clip| {
                            let mut graph = olvin::anim::AnimGraph::single(clip);
                            graph.states[0].looping = *looping;
                            rest.clone().with_graph(graph)
                        })
                        .ok_or_else(|| format!("no clip \"{name}\"")),
                    Some(other) => data.animator_with(skin, Some(other), graph_file.as_ref()),
                    None => Ok(rest.clone()),
                };
                Some(built.unwrap_or_else(|e| {
                    log::warn!("{}: {e}; rest pose", data.name);
                    rest
                }))
            })
            .collect()
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
    if !prefs.viewport_post {
        settings.bloom = None;
        settings.post.ao = None;
        settings.post.grade = Default::default();
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
        let stats = res.engine.prepare(
            id,
            Source::Play,
            &world,
            &mut meshes,
            regs,
            vp,
            &settings,
            &world,
        );
        state.render_stats.insert(id, stats);
        Some(Source::Play)
    } else {
        let assets_dir = state.assets_dir();
        state
            .engine_scene
            .sync(&state.scene, frame, assets_dir, &state.model_previews);
        state
            .engine_scene
            .sync_terrains(&state.scene, frame, &mut state.terrains);
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
            &p.fx.world,
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
    /// Faults of the game already shown in the console.
    faults_seen: usize,
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
        // A crash in the game stops that subsystem, never the editor.
        app.resources
            .fetch_mut::<olvin::core::Faults>()
            .set_policy(olvin::core::FaultPolicy::Isolate);
        app.add_plugin(EcsPlugin)
            .add_plugin(TransformPlugin)
            .add_plugin(PhysicsPlugin::default());
        app.insert_resource(Meshes::default())
            .insert_resource(Textures::default())
            .insert_resource(Materials::default())
            .insert_resource(Shaders::default())
            .insert_resource(RenderAssets::new(assets_dir.clone()))
            .insert_resource(settings_for(scene));
        // Scene models with their animators; `skin_meshes` queues the poses of
        // skinned meshes for the renderer (GPU skinning on the Full tier).
        if let Some(dir) = &assets_dir {
            app.insert_resource(olvin_assets::AssetManager::new(dir.clone()));
        }
        app.add_plugin(olvin::anim::AnimPlugin)
            .add_plugin(olvin::models::ModelPlugin)
            .add_system(olvin::core::Stage::PostUpdate, olvin::render::skin_meshes);
        // Scripts (the built-in behaviours and Lua; Rust game code isn't linked into the
        // editor) and the project's input map, fed from the Game view.
        let lua_dir = assets_dir.as_ref().map(|d| d.join("scripts"));
        let map = assets_dir.map(|d| olvin::input::InputMap::load_or_default(d.join("input.ron")));
        app.add_plugin(olvin::input::InputPlugin { map })
            .add_plugin(olvin::script::ScriptPlugin::default());
        // Prefabs scripts spawn with `ctx:spawn_prefab`.
        app.add_system(olvin::core::Stage::Update, |res| {
            olvin::scene_file::run_prefab_spawns(res)
        });
        // The project's Lua behaviours (`assets/scripts/**/*.lua`, library packs
        // keep theirs in subfolders). No hot reload: a session loads them once.
        if let Some(dir) = lua_dir {
            app.add_plugin(olvin::script::lua::LuaPlugin {
                dir,
                hot_reload: false,
                limits: Default::default(),
            });
        }
        olvin::scene_file::spawn(&app.resources, scene);
        app.startup();
        Self {
            app,
            paused: false,
            step: false,
            saved: scene.clone(),
            undo_len,
            faults_seen: 0,
        }
    }

    /// Subsystems of the game that crashed since the last call.
    pub fn new_faults(&mut self) -> Vec<olvin::core::Fault> {
        let faults = self.app.resources.fetch::<olvin::core::Faults>();
        let new = faults.list()[self.faults_seen.min(faults.list().len())..].to_vec();
        self.faults_seen = faults.list().len();
        new
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
            // Particle systems of the scene (the editor's play app has no RenderPlugin).
            let dt = self
                .app
                .resources
                .get::<olvin::core::Time>()
                .map_or(0.0, |t| t.delta());
            olvin::render::update_emitters(&mut self.app.resources.fetch_mut::<World>(), dt);
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
        preview.sync(&scene, 1, None, &HashMap::new());
        assert_eq!(preview.world.query::<&MeshRenderer>().iter().count(), 1);
        assert_eq!(preview.world.query::<&Camera>().iter().count(), 1);
        preview.sync(&scene, 1, None, &HashMap::new());
        assert_eq!(preview.world.len(), 2, "same frame: not rebuilt twice");
    }

    /// A project's assets folder holding the vertical slice's robot (a skinned
    /// glTF with the clips Idle and Wave).
    fn robot_assets(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("olvin_editor_rig_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("models")).unwrap();
        let src = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../apps/vertical_slice/assets/models/robot.gltf"
        );
        std::fs::copy(src, dir.join("models/robot.gltf")).unwrap();
        dir
    }

    fn robot_scene(animation: Option<olvin::scene::file::AnimationDesc>) -> SceneFile {
        let mut robot = EntityDesc::new(5, "Robot");
        robot.model = Some(olvin::scene::file::ModelDesc {
            path: "models/robot.gltf".into(),
            animation,
        });
        SceneFile {
            entities: vec![robot],
            ..Default::default()
        }
    }

    #[test]
    fn model_preview_poses_the_robot_and_stops_back_in_rest() {
        let dir = robot_assets("preview");
        let scene = robot_scene(None);
        let mut preview = ScenePreview::default();
        let mut previews = HashMap::new();
        let mut frame = 0;
        let mut step = |preview: &mut ScenePreview, previews: &HashMap<u64, ModelPreview>, dt| {
            frame += 1;
            preview.sync_dt(&scene, frame, Some(dir.clone()), previews, dt);
        };

        assert_eq!(
            preview.model_clips(Some(dir.clone()), "models/robot.gltf"),
            ["Idle", "Wave"]
        );
        // Not playing: the rest pose, posed once (two skinned primitives).
        step(&mut preview, &previews, 0.1);
        assert_eq!(preview.meshes.skin_updates_len(), 2);
        let rest = preview.rigs[&5].animators[0].matrices.clone();
        assert_eq!(preview.rigs[&5].animators[0].version, 0);
        assert_eq!(preview.world.query::<&MeshRenderer>().iter().count(), 2);
        preview.meshes.clear_skin_updates();
        step(&mut preview, &previews, 0.1);
        assert_eq!(
            preview.meshes.skin_updates_len(),
            0,
            "unchanged: no re-pose"
        );

        // Playing Wave at double speed: time advances 2 x dt and the pose moves.
        previews.insert(
            5,
            ModelPreview {
                playing: true,
                clip: Some("Wave".into()),
                speed: 2.0,
                looping: true,
            },
        );
        step(&mut preview, &previews, 0.25);
        let animator = &preview.rigs[&5].animators[0];
        assert!((animator.layers[0].state.time - 0.5).abs() < 1e-4);
        assert_ne!(animator.matrices, rest, "the pose moved");
        assert_eq!(preview.meshes.skin_updates_len(), 2);
        let first = animator.matrices.clone();
        step(&mut preview, &previews, 0.25);
        assert_ne!(
            preview.rigs[&5].animators[0].matrices, first,
            "keeps playing"
        );

        // A non-looping preview stops at the clip's end.
        previews.get_mut(&5).unwrap().looping = false;
        step(&mut preview, &previews, 0.0);
        for _ in 0..3 {
            step(&mut preview, &previews, 100.0);
        }
        let end = preview.rigs[&5].animators[0].matrices.clone();
        step(&mut preview, &previews, 1.0);
        assert_eq!(preview.rigs[&5].animators[0].matrices, end);

        // Stop: back to the rest pose.
        previews.get_mut(&5).unwrap().playing = false;
        step(&mut preview, &previews, 0.1);
        assert_eq!(preview.rigs[&5].animators[0].matrices, rest);
        // The scene itself was never touched, and rigs go with their objects.
        assert_eq!(scene, robot_scene(None));
        preview.sync_dt(&SceneFile::default(), 99, Some(dir.clone()), &previews, 0.1);
        assert!(preview.rigs.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preview_without_a_clip_runs_the_scenes_graph() {
        use olvin::scene::file::{AnimGraphDesc, AnimStateDesc, AnimationDesc};
        let dir = robot_assets("graph");
        let graph = AnimGraphDesc {
            states: vec![AnimStateDesc::clip("Wave", "Wave")],
            ..Default::default()
        };
        std::fs::create_dir_all(dir.join("anim")).unwrap();
        graph.save(dir.join("anim/robot.animgraph.ron")).unwrap();
        for animation in [
            AnimationDesc::Inline(graph.clone()),
            AnimationDesc::Graph("anim/robot.animgraph.ron".into()),
            AnimationDesc::Clip("Wave".into()),
        ] {
            let scene = robot_scene(Some(animation.clone()));
            let mut preview = ScenePreview::default();
            let previews = HashMap::from([(
                5,
                ModelPreview {
                    playing: true,
                    ..Default::default()
                },
            )]);
            preview.sync_dt(&scene, 1, Some(dir.clone()), &previews, 0.0);
            preview.sync_dt(&scene, 2, Some(dir.clone()), &previews, 0.3);
            let a = &preview.rigs[&5].animators[0];
            assert_eq!(a.layers.len(), 1, "{animation:?}");
            assert!((a.layers[0].state.time - 0.3).abs() < 1e-4, "{animation:?}");
            assert_ne!(
                a.matrices,
                a.skeleton.skin_matrices(&a.skeleton.rest_pose())
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Play mode runs the model's animation: clip, inline graph and graph file.
    #[test]
    fn play_mode_animates_skinned_models() {
        use olvin::anim::Animator;
        use olvin::scene::file::{AnimGraphDesc, AnimStateDesc, AnimationDesc};
        let dir = robot_assets("play");
        let graph = AnimGraphDesc {
            states: vec![AnimStateDesc::clip("Wave", "Wave")],
            ..Default::default()
        };
        std::fs::create_dir_all(dir.join("anim")).unwrap();
        graph.save(dir.join("anim/robot.animgraph.ron")).unwrap();
        for animation in [
            AnimationDesc::Clip("Wave".into()),
            AnimationDesc::Inline(graph.clone()),
            AnimationDesc::Graph("anim/robot.animgraph.ron".into()),
        ] {
            let scene = robot_scene(Some(animation.clone()));
            let mut play = PlaySession::start(&scene, 0, Some(dir.clone()));
            play.app
                .resources
                .fetch_mut::<olvin::core::Time>()
                .set_fixed_delta(Some(0.1));
            for _ in 0..2000 {
                if olvin::models::pending_count(&play.app.resources) == 0 {
                    break;
                }
                play.tick();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let version = |play: &PlaySession| {
                let world = play.app.resources.fetch::<World>();
                let a = world.query::<&Animator>().iter().next().map(|a| {
                    (
                        a.version,
                        a.layers.len(),
                        a.layers[0].graph.states[0].name.clone(),
                    )
                });
                a.expect("a skinned model has its animator")
            };
            let before = version(&play);
            assert_eq!(before.1, 1, "{animation:?}");
            // Poses were queued for the renderer by the skin system.
            play.app
                .resources
                .fetch_mut::<Meshes>()
                .clear_skin_updates();
            for _ in 0..5 {
                play.tick();
            }
            let after = version(&play);
            assert!(after.0 > before.0, "{animation:?}: {before:?} -> {after:?}");
            assert_eq!(
                play.app.resources.fetch::<Meshes>().skin_updates_len(),
                2,
                "{animation:?}: both primitives were re-posed"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

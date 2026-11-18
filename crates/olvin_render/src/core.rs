//! The renderer without a window: pipelines, GPU assets and per-camera data.
//! The window renderer, the editor's scene/game views and headless tests all
//! draw through this.
//!
//! Frame order for a view: `upload` + `upload_assets` (new meshes, textures,
//! materials, shaders) → `prepare` (CPU: culling, lights, shadow fit,
//! uniforms) → `render_shadows` (own depth pass into the view's shadow map)
//! → `draw` (into the caller's main pass: meshes, then the sky).

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::Range;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use olvin_ecs::World;
use olvin_scene::{Camera, Transform};
use olvin_transform::GlobalTransform;

use crate::light::{CameraInfo, LightList, MAX_LIGHTS, PackedLight, fit_shadow};
use crate::material::MaterialUniform;
use crate::shader::{DEFAULT_SURFACE_SHADER, mesh_source, sky_source};
use crate::{
    Frustum, Light, LightKind, Material, Materials, MeshData, MeshRenderer, Meshes, RenderSettings,
    RenderStats, Shaders, SkyMode, TextureData, TextureHandle, Textures, Tonemapping, Vertex,
};

const MIN_INSTANCE_CAPACITY: usize = 1024;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Bucket key's material half for "no material" (the fallback material).
const NO_MATERIAL: u32 = u32::MAX;

/// Uniforms of the main pass; mirrors `Globals` in globals.wgsl (all vec4 /
/// mat4, so std140 and WGSL layouts agree).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    shadow_view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    camera: [f32; 4],
    view_plane: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky: [f32; 4],
    ground: [f32; 4],
    fog_color: [f32; 4],
    params: [f32; 4],
    shadow_params: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    sky_ground: [f32; 4],
    sky_params: [f32; 4],
    lights: [PackedLight; MAX_LIGHTS],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    model: [[f32; 4]; 4],
    color: [f32; 4],
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    center: Vec3,
    radius: f32,
}

struct GpuTexture {
    view: wgpu::TextureView,
    mips: u32,
    /// Unique per upload, so bind groups know when to rebuild.
    generation: u64,
}

struct GpuMaterial {
    material: Material,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

/// GPU copies of one set of registries ([`Meshes`], [`Textures`],
/// [`Materials`], [`Shaders`]; handles index into them), plus the pipelines
/// of its surface shaders. Belongs to the [`RenderCore`] that uploaded it.
#[derive(Default)]
pub struct GpuMeshes {
    meshes: Vec<Option<GpuMesh>>,
    textures: Vec<Option<GpuTexture>>,
    materials: Vec<Option<GpuMaterial>>,
    /// Indexed by `ShaderHandle`; the last pipeline that compiled.
    pipelines: Vec<Option<wgpu::RenderPipeline>>,
    generation: u64,
}

impl GpuMeshes {
    /// Pipeline id of a material slot: 0 = built-in, `h + 1` = shader `h`.
    fn pipeline_of(&self, material: u32) -> u32 {
        self.materials
            .get(material as usize)
            .and_then(Option::as_ref)
            .and_then(|m| m.material.shader)
            .filter(|h| matches!(self.pipelines.get(h.0 as usize), Some(Some(_))))
            .map_or(0, |h| h.0 + 1)
    }
}

/// Cheap hasher for the (material, mesh) bucket keys.
#[derive(Default)]
struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }

    fn write_u64(&mut self, v: u64) {
        self.0 = (v ^ (v >> 29)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

fn bucket_key(material: u32, mesh: u32) -> u64 {
    ((material as u64) << 32) | mesh as u64
}

/// One instanced draw: sorted by pipeline, then material, then mesh.
struct DrawRange {
    pipeline: u32,
    material: u32,
    mesh: u32,
    instances: Range<u32>,
}

/// Instances bucketed by (material, mesh), packed into one vertex buffer.
/// Buckets, the key map and the draw list are reused, so a steady frame
/// doesn't allocate.
struct InstanceSet {
    buffer: wgpu::Buffer,
    capacity: usize,
    buckets: Vec<(u64, Vec<InstanceRaw>)>,
    lookup: HashMap<u64, u32, BuildHasherDefault<KeyHasher>>,
    /// Last key pushed and its bucket (consecutive entities often share it).
    last: Option<(u64, u32)>,
    /// (pipeline, key, bucket) of non-empty buckets, sorted.
    order: Vec<(u32, u64, u32)>,
    ranges: Vec<DrawRange>,
    count: u32,
}

impl InstanceSet {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            buffer: instance_buffer(device, MIN_INSTANCE_CAPACITY),
            capacity: MIN_INSTANCE_CAPACITY,
            buckets: Vec::new(),
            lookup: HashMap::default(),
            last: None,
            order: Vec::new(),
            ranges: Vec::new(),
            count: 0,
        }
    }

    fn begin(&mut self) {
        // Forget pairs no longer drawn once they pile up (rare reallocation).
        if self.buckets.len() > 2 * self.ranges.len() + 256 {
            self.buckets.clear();
            self.lookup.clear();
        }
        self.buckets.iter_mut().for_each(|(_, b)| b.clear());
        self.last = None;
    }

    #[inline]
    fn push(&mut self, key: u64, instance: InstanceRaw) {
        let index = match self.last {
            Some((k, i)) if k == key => i,
            _ => {
                let next = self.buckets.len() as u32;
                let i = *self.lookup.entry(key).or_insert(next);
                if i == next {
                    self.buckets.push((key, Vec::new()));
                }
                self.last = Some((key, i));
                i
            }
        };
        self.buckets[index as usize].1.push(instance);
    }

    /// Sorts the buckets into draws, packs them through `staging` (reused
    /// scratch) and uploads them.
    fn finish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        staging: &mut Vec<InstanceRaw>,
        pipeline_of: impl Fn(u32) -> u32,
    ) {
        self.order.clear();
        for (i, (key, bucket)) in self.buckets.iter().enumerate() {
            if !bucket.is_empty() {
                self.order
                    .push((pipeline_of((key >> 32) as u32), *key, i as u32));
            }
        }
        self.order.sort_unstable();
        staging.clear();
        self.ranges.clear();
        for &(pipeline, key, i) in &self.order {
            let start = staging.len() as u32;
            staging.extend_from_slice(&self.buckets[i as usize].1);
            self.ranges.push(DrawRange {
                pipeline,
                material: (key >> 32) as u32,
                mesh: key as u32,
                instances: start..staging.len() as u32,
            });
        }
        if staging.len() > self.capacity {
            self.capacity = staging.len().next_power_of_two();
            self.buffer = instance_buffer(device, self.capacity);
        }
        if !staging.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(staging));
        }
        self.count = staging.len() as u32;
    }
}

/// Per-camera state: uniforms, instance buffers, the sun's shadow map and
/// frame scratch.
pub struct ViewData {
    globals: wgpu::Buffer,
    /// Globals + shadow map + sky texture.
    bind: wgpu::BindGroup,
    main: InstanceSet,
    staging: Vec<InstanceRaw>,
    shadow_globals: wgpu::Buffer,
    shadow_bind: wgpu::BindGroup,
    shadow: InstanceSet,
    /// Attachment view and size; created on first use, kept afterwards.
    shadow_map: Option<(wgpu::TextureView, u32)>,
    shadow_active: bool,
    /// Generation of the sky texture in `bind` (None = the default).
    sky_bound: Option<u64>,
    sky_active: bool,
}

impl ViewData {
    /// Whether the last `prepare` enabled the sun's shadows (so
    /// `render_shadows` has work to do).
    pub fn shadows_active(&self) -> bool {
        self.shadow_active
    }

    /// Whether `draw` paints a sky (procedural or texture) behind the meshes.
    pub fn sky_active(&self) -> bool {
        self.sky_active
    }
}

/// 1×1 stand-ins for missing material textures: white base, flat normal,
/// white ORM (occlusion 1, factors as is), white emissive.
struct DefaultTextures {
    base: wgpu::TextureView,
    normal: wgpu::TextureView,
    orm: wgpu::TextureView,
    emissive: wgpu::TextureView,
}

pub struct RenderCore {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// The built-in mesh pipeline (default surface).
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sky_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    shadow_layout: wgpu::BindGroupLayout,
    shadow_sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    point_sampler: wgpu::Sampler,
    sky_sampler: wgpu::Sampler,
    /// Bound while a view has no shadow map yet.
    dummy_shadow: wgpu::TextureView,
    defaults: DefaultTextures,
    /// For renderers without a material.
    fallback_material: wgpu::BindGroup,
    target: (wgpu::TextureFormat, u32, wgpu::TextureFormat),
    encode_srgb: bool,
    time: Option<f32>,
    start: Instant,
}

impl RenderCore {
    /// `color` / `samples` / `depth` must match the render pass the draws go into.
    /// Non-sRGB colour targets get gamma encoding in the shader.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color: wgpu::TextureFormat,
        samples: u32,
        depth: wgpu::TextureFormat,
    ) -> Self {
        let (layout, material_layout) = create_layouts(&device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh"),
            bind_group_layouts: &[Some(&layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let target = (color, samples, depth);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh"),
            source: wgpu::ShaderSource::Wgsl(mesh_source(DEFAULT_SURFACE_SHADER).into()),
        });
        let pipeline = create_mesh_pipeline(&device, &pipeline_layout, &module, target);
        let sky_pipeline = create_sky_pipeline(&device, &layout, target);
        let (shadow_pipeline, shadow_layout) = create_shadow_pipeline(&device);
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        // Anisotropy is clamped to 1 where unsupported (WebGL2 without the extension).
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material linear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let point_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material point"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            lod_max_clamp: 0.0,
            ..Default::default()
        });
        let sky_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let dummy_shadow = shadow_texture(&device, 1);
        let solid = |rgba, srgb| {
            upload_texture(&device, &queue, &TextureData::solid(rgba, srgb), u32::MAX).view
        };
        let defaults = DefaultTextures {
            base: solid([255; 4], true),
            normal: solid([128, 128, 255, 255], false),
            orm: solid([255; 4], false),
            emissive: solid([255; 4], true),
        };
        let fallback = Material::fallback();
        let fallback_material = material_bind_group(
            &device,
            &material_layout,
            &defaults,
            (&linear_sampler, &point_sampler),
            &[],
            &material_uniform(&device, &fallback),
            &fallback,
        );
        Self {
            fallback_material,
            device,
            queue,
            pipeline,
            layout,
            material_layout,
            pipeline_layout,
            sky_pipeline,
            shadow_pipeline,
            shadow_layout,
            shadow_sampler,
            linear_sampler,
            point_sampler,
            sky_sampler,
            dummy_shadow,
            defaults,
            target,
            encode_srgb: !color.is_srgb(),
            time: None,
            start: Instant::now(),
        }
    }

    /// Seconds for surface shaders' `in.time`. Until it's called, the time
    /// since this core was created.
    pub fn set_time(&mut self, seconds: f32) {
        self.time = Some(seconds);
    }

    pub fn create_view(&self) -> ViewData {
        let uniform = |label, size| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let globals = uniform("olvin globals", size_of::<Globals>() as u64);
        let shadow_globals = uniform("olvin shadow globals", size_of::<Mat4>() as u64);
        let bind = self.main_bind_group(&globals, &self.dummy_shadow, &self.defaults.base);
        let shadow_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("olvin shadow globals"),
            layout: &self.shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shadow_globals.as_entire_binding(),
            }],
        });
        ViewData {
            globals,
            bind,
            main: InstanceSet::new(&self.device),
            staging: Vec::new(),
            shadow_globals,
            shadow_bind,
            shadow: InstanceSet::new(&self.device),
            shadow_map: None,
            shadow_active: false,
            sky_bound: None,
            sky_active: false,
        }
    }

    fn main_bind_group(
        &self,
        globals: &wgpu::Buffer,
        shadow: &wgpu::TextureView,
        sky: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("olvin globals"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(shadow),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(sky),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sky_sampler),
                },
            ],
        })
    }

    fn material_uniform(&self, material: &Material) -> wgpu::Buffer {
        material_uniform(&self.device, material)
    }

    fn material_bind_group(
        &self,
        textures: &[Option<GpuTexture>],
        uniform: &wgpu::Buffer,
        material: &Material,
    ) -> wgpu::BindGroup {
        let samplers = (&self.linear_sampler, &self.point_sampler);
        material_bind_group(
            &self.device,
            &self.material_layout,
            &self.defaults,
            samplers,
            textures,
            uniform,
            material,
        )
    }

    /// Uploads meshes added to `meshes` since the last call.
    pub fn upload(&self, gpu: &mut GpuMeshes, meshes: &mut Meshes) {
        use wgpu::util::DeviceExt;
        for (handle, data) in meshes.pending.drain(..) {
            let MeshData { vertices, indices } = &data;
            let (center, radius) = data.bounding_sphere();
            let mesh = GpuMesh {
                vertices: self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mesh vertices"),
                        contents: bytemuck::cast_slice::<Vertex, u8>(vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                indices: self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mesh indices"),
                        contents: bytemuck::cast_slice::<u32, u8>(indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                index_count: indices.len() as u32,
                center,
                radius,
            };
            put(&mut gpu.meshes, handle.0, mesh);
        }
    }

    /// Uploads what changed in the asset registries since the last call:
    /// compiles surface shaders (a failure keeps the last good pipeline and
    /// is reported by `Shaders::error`; nothing panics), uploads textures
    /// with mipmaps, and (re)builds material bind groups, including those of
    /// materials whose textures were replaced. Call it next to [`Self::upload`],
    /// before `prepare`.
    pub fn upload_assets(
        &self,
        gpu: &mut GpuMeshes,
        textures: &mut Textures,
        materials: &mut Materials,
        shaders: &mut Shaders,
    ) {
        for (handle, source) in std::mem::take(&mut shaders.pending) {
            match self.compile_surface(&source) {
                Ok(pipeline) => put(&mut gpu.pipelines, handle.0, pipeline),
                Err(e) => {
                    log::warn!("surface shader {}: {e}", handle.0);
                    shaders.set_error(handle, Some(e));
                }
            }
        }

        let max = self.device.limits().max_texture_dimension_2d;
        let mut replaced = Vec::new();
        for (handle, data) in textures.pending.drain(..) {
            gpu.generation += 1;
            let mut texture = upload_texture(&self.device, &self.queue, &data, max);
            texture.generation = gpu.generation;
            if matches!(gpu.textures.get(handle.0 as usize), Some(Some(_))) {
                replaced.push(handle);
            }
            put(&mut gpu.textures, handle.0, texture);
        }
        if !replaced.is_empty() {
            // New texture objects under existing materials: rebind those (an asset reload).
            for slot in 0..gpu.materials.len() {
                let Some(m) = &gpu.materials[slot] else {
                    continue;
                };
                if m.material
                    .textures()
                    .iter()
                    .flatten()
                    .any(|t| replaced.contains(t))
                {
                    let bind = self.material_bind_group(&gpu.textures, &m.uniform, &m.material);
                    if let Some(m) = &mut gpu.materials[slot] {
                        m.bind = bind;
                    }
                }
            }
        }

        for (handle, material) in materials.pending.drain(..) {
            let uniform = match gpu.materials.get(handle.0 as usize) {
                Some(Some(existing)) => {
                    self.queue.write_buffer(
                        &existing.uniform,
                        0,
                        bytemuck::bytes_of(&material.uniform()),
                    );
                    existing.uniform.clone()
                }
                _ => self.material_uniform(&material),
            };
            let bind = self.material_bind_group(&gpu.textures, &uniform, &material);
            put(
                &mut gpu.materials,
                handle.0,
                GpuMaterial {
                    material,
                    uniform,
                    bind,
                },
            );
        }
    }

    /// Builds a mesh pipeline from a full template source, catching wgpu's
    /// errors instead of letting them reach the device's panic handler.
    fn compile_surface(&self, source: &str) -> Result<wgpu::RenderPipeline, String> {
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("surface shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipeline =
            create_mesh_pipeline(&self.device, &self.pipeline_layout, &module, self.target);
        let validation = pollster::block_on(validation.pop());
        let internal = pollster::block_on(internal.pop());
        match validation.or(internal) {
            None => Ok(pipeline),
            Some(e) => Err(e.to_string()),
        }
    }

    /// The hot path: gathers lights (the first directional [`Light`] is the
    /// sun, up to [`MAX_LIGHTS`] others ranked by distance), fits the sun's
    /// shadow map, then one linear pass over (GlobalTransform, MeshRenderer),
    /// culled against the camera (and the shadow box) and bucketed by
    /// (material, mesh) into flat arrays, drawn sorted by (pipeline,
    /// material, mesh). Writes uniforms and instances for `view`.
    /// Call before `render_shadows` and `draw`.
    pub fn prepare(
        &self,
        view: &mut ViewData,
        gpu: &GpuMeshes,
        world: &World,
        view_proj: Mat4,
        settings: &RenderSettings,
    ) -> RenderStats {
        let frustum = Frustum::from_view_proj(view_proj);
        let cam = CameraInfo::from_view_proj(view_proj);

        // Lights.
        let mut sun: Option<(Vec3, Vec3, bool)> = None;
        let mut lights = LightList::new();
        let mut query = world.query::<(&Light, Option<&GlobalTransform>, Option<&Transform>)>();
        for (light, global, transform) in query.iter().filter(|_| settings.scene_lights) {
            let m = global
                .map(|g| g.0)
                .or_else(|| transform.map(Transform::matrix))
                .unwrap_or(Mat4::IDENTITY);
            let dir = m.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Y);
            let pos = m.w_axis.truncate();
            let score = match light.kind {
                LightKind::Directional if sun.is_none() => {
                    sun = Some((dir, light.radiance(), light.shadows));
                    continue;
                }
                LightKind::Directional => f32::MIN,
                LightKind::Point | LightKind::Spot => {
                    if light.intensity <= 0.0 || !frustum.intersects_sphere(pos, light.range) {
                        continue;
                    }
                    pos.distance(cam.eye) - light.range
                }
            };
            lights.push(light.pack(pos, dir), score);
        }
        let (sun_dir, sun_color, sun_shadows) = sun.unwrap_or((
            settings.light_dir,
            Vec3::from_slice(&settings.light_color.0[..3]),
            settings.light_shadows,
        ));
        let sun_dir = sun_dir.normalize_or(Vec3::NEG_Y);

        // Shadow map fit (and lazy (re)creation).
        let max_size = self.device.limits().max_texture_dimension_2d;
        let size = settings.shadow_map_size.clamp(16, max_size.max(16));
        let fit = if sun_shadows && sun_color != Vec3::ZERO && settings.shadow_distance > 0.0 {
            fit_shadow(&cam, sun_dir, settings.shadow_distance, size)
        } else {
            None
        };
        view.shadow_active = fit.is_some();
        let mut rebind = false;
        if fit.is_some() && view.shadow_map.as_ref().is_none_or(|(_, s)| *s != size) {
            view.shadow_map = Some((shadow_texture(&self.device, size), size));
            rebind = true;
        }
        let shadow_frustum = fit.map(|f| Frustum::from_view_proj(f.view_proj));

        // Sky (its texture lives in the view's bind group).
        let sky_texture = match settings.sky {
            SkyMode::Texture { texture, .. } => gpu
                .textures
                .get(texture.0 as usize)
                .and_then(Option::as_ref),
            _ => None,
        };
        let sky_generation = sky_texture.map(|t| t.generation);
        if rebind || sky_generation != view.sky_bound {
            let shadow = view
                .shadow_map
                .as_ref()
                .map_or(&self.dummy_shadow, |(v, _)| v);
            let sky = sky_texture.map_or(&self.defaults.base, |t| &t.view);
            view.bind = self.main_bind_group(&view.globals, shadow, sky);
            view.sky_bound = sky_generation;
        }

        // Meshes.
        view.main.begin();
        view.shadow.begin();
        let mut culled = 0;
        for (global, transform, renderer) in world
            .query::<(Option<&GlobalTransform>, &Transform, &MeshRenderer)>()
            .iter()
        {
            let i = renderer.mesh.0;
            let Some(Some(mesh)) = gpu.meshes.get(i as usize) else {
                continue;
            };
            let material = match renderer.material {
                Some(m) if matches!(gpu.materials.get(m.0 as usize), Some(Some(_))) => m.0,
                _ => NO_MATERIAL,
            };
            let model = global.map_or_else(|| transform.matrix(), |g| g.0);
            let scale = model
                .x_axis
                .truncate()
                .length()
                .max(model.y_axis.truncate().length())
                .max(model.z_axis.truncate().length());
            let (center, radius) = (model.transform_point3(mesh.center), mesh.radius * scale);
            let instance = InstanceRaw {
                model: model.to_cols_array_2d(),
                color: renderer.color.0,
            };
            if let Some(f) = &shadow_frustum
                && f.intersects_sphere(center, radius)
            {
                view.shadow.push(bucket_key(NO_MATERIAL, i), instance);
            }
            if frustum.intersects_sphere(center, radius) {
                view.main.push(bucket_key(material, i), instance);
            } else {
                culled += 1;
            }
        }
        view.main
            .finish(&self.device, &self.queue, &mut view.staging, |m| {
                gpu.pipeline_of(m)
            });
        if view.shadow_active {
            view.shadow
                .finish(&self.device, &self.queue, &mut view.staging, |_| 0);
        }

        // Uniforms.
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let fog = settings.fog;
        let shadow_view_proj = fit.map_or(Mat4::IDENTITY, |f| f.view_proj);
        let texel_world = fit.map_or(0.0, |f| f.texel_world);
        let time = self
            .time
            .unwrap_or_else(|| self.start.elapsed().as_secs_f32());
        let mut sky = SkyUniform::default();
        match settings.sky {
            SkyMode::Color => {}
            SkyMode::Procedural {
                zenith,
                horizon,
                ground,
                sun_size,
            } => {
                let r = sun_size.max(0.0).to_radians();
                let disc = r > 0.0 && sun_color != Vec3::ZERO;
                sky = SkyUniform {
                    zenith: Vec3::from(zenith).extend((r * 0.85).cos()).into(),
                    horizon: Vec3::from(horizon).extend((r * 1.15).cos()).into(),
                    ground: Vec3::from(ground).extend(0.0).into(),
                    params: [1.0, 1.0, 1.0, flag(disc)],
                };
            }
            SkyMode::Texture {
                rotation,
                intensity,
                ..
            } => {
                if let Some(t) = sky_texture {
                    sky.ground[3] = rotation;
                    sky.params = [2.0, intensity, t.mips as f32, 0.0];
                }
            }
        }
        view.sky_active = sky.params[0] > 0.5;
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            shadow_view_proj: shadow_view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            camera: cam.camera.into(),
            view_plane: cam.view_plane.into(),
            sun_dir: sun_dir.extend(flag(view.shadow_active)).into(),
            sun_color: sun_color.extend(flag(self.encode_srgb)).into(),
            sky: rgb(settings.ambient.0)
                .extend(settings.exposure.max(0.0))
                .into(),
            ground: rgb(settings.ground_ambient.0)
                .extend(lights.len as f32)
                .into(),
            fog_color: fog.map_or([0.0; 4], |f| rgb(f.color.0).extend(1.0).into()),
            params: [
                fog.map_or(0.0, |f| f.start),
                fog.map_or(1.0, |f| f.end),
                1.0 / size as f32,
                settings.shadow_distance,
            ],
            shadow_params: [
                texel_world * 1.5,
                settings.shadow_distance * 0.85,
                flag(settings.tonemapping == Tonemapping::Aces),
                time,
            ],
            sky_zenith: sky.zenith,
            sky_horizon: sky.horizon,
            sky_ground: sky.ground,
            sky_params: sky.params,
            lights: lights.lights,
        };
        self.queue
            .write_buffer(&view.globals, 0, bytemuck::bytes_of(&globals));
        if view.shadow_active {
            self.queue.write_buffer(
                &view.shadow_globals,
                0,
                bytemuck::bytes_of(&shadow_view_proj.to_cols_array_2d()),
            );
        }

        RenderStats {
            draw_calls: view.main.ranges.len() as u32,
            instances: view.main.count,
            culled,
        }
    }

    /// Records the sun's shadow-map pass (depth only) for `view`. Call after
    /// `prepare` and before the pass that `draw`s this view; does nothing
    /// when shadows are off.
    pub fn render_shadows(
        &self,
        view: &ViewData,
        gpu: &GpuMeshes,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if !view.shadow_active {
            return;
        }
        let Some((map, _)) = &view.shadow_map else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("olvin shadows"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: map,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        if view.shadow.ranges.is_empty() {
            return;
        }
        pass.set_pipeline(&self.shadow_pipeline);
        pass.set_bind_group(0, &view.shadow_bind, &[]);
        pass.set_vertex_buffer(1, view.shadow.buffer.slice(..));
        for range in &view.shadow.ranges {
            if let Some(Some(mesh)) = gpu.meshes.get(range.mesh as usize) {
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, range.instances.clone());
            }
        }
    }

    /// Records the prepared draws into a pass compatible with this core:
    /// meshes sorted by pipeline / material / mesh, then the sky (if any) on
    /// the far plane. Call it even when `prepare` reported no draw calls, so
    /// the sky still shows.
    pub fn draw(&self, view: &ViewData, gpu: &GpuMeshes, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &view.bind, &[]);
        if !view.main.ranges.is_empty() {
            pass.set_vertex_buffer(1, view.main.buffer.slice(..));
            let mut bound_pipeline = None;
            let mut bound_material = None;
            for range in &view.main.ranges {
                let Some(Some(mesh)) = gpu.meshes.get(range.mesh as usize) else {
                    continue;
                };
                if bound_pipeline != Some(range.pipeline) {
                    let pipeline = match range.pipeline {
                        0 => &self.pipeline,
                        id => match gpu.pipelines.get(id as usize - 1) {
                            Some(Some(p)) => p,
                            _ => &self.pipeline,
                        },
                    };
                    pass.set_pipeline(pipeline);
                    bound_pipeline = Some(range.pipeline);
                }
                if bound_material != Some(range.material) {
                    let bind = gpu
                        .materials
                        .get(range.material as usize)
                        .and_then(Option::as_ref)
                        .map_or(&self.fallback_material, |m| &m.bind);
                    pass.set_bind_group(1, bind, &[]);
                    bound_material = Some(range.material);
                }
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, range.instances.clone());
            }
        }
        if view.sky_active {
            pass.set_pipeline(&self.sky_pipeline);
            pass.draw(0..3, 0..1);
        }
    }
}

#[derive(Default)]
struct SkyUniform {
    zenith: [f32; 4],
    horizon: [f32; 4],
    ground: [f32; 4],
    params: [f32; 4],
}

fn material_uniform(device: &wgpu::Device, material: &Material) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("material"),
        contents: bytemuck::bytes_of(&material.uniform()),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

/// Material uniform + its textures (defaults for missing ones) + sampler.
fn material_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    defaults: &DefaultTextures,
    (linear, point): (&wgpu::Sampler, &wgpu::Sampler),
    textures: &[Option<GpuTexture>],
    uniform: &wgpu::Buffer,
    material: &Material,
) -> wgpu::BindGroup {
    let view = |handle: Option<TextureHandle>, default| {
        handle
            .and_then(|h| textures.get(h.0 as usize))
            .and_then(Option::as_ref)
            .map_or(default, |t: &GpuTexture| &t.view)
    };
    let entry = |binding, view| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    };
    let sampler = if material.point_filter { point } else { linear };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("material"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            entry(1, view(material.base_texture, &defaults.base)),
            entry(2, view(material.normal_texture, &defaults.normal)),
            entry(3, view(material.orm_texture, &defaults.orm)),
            entry(4, view(material.emissive_texture, &defaults.emissive)),
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn put<T>(slots: &mut Vec<Option<T>>, index: u32, value: T) {
    let i = index as usize;
    if slots.len() <= i {
        slots.resize_with(i + 1, || None);
    }
    slots[i] = Some(value);
}

/// Uploads a texture with its CPU mip chain. Levels larger than
/// `max_dimension` are skipped (the texture starts at the first that fits);
/// invalid data becomes 1×1 magenta.
fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    data: &TextureData,
    max_dimension: u32,
) -> GpuTexture {
    let data = if data.is_valid() {
        data.to_gpu_layout()
    } else {
        log::warn!(
            "texture {}x{} {:?}: {} bytes don't match; using magenta",
            data.width,
            data.height,
            data.format,
            data.pixels.len()
        );
        TextureData::solid([255, 0, 255, 255], true)
    };
    let chain = data.mip_chain();
    let levels: Vec<&TextureData> = std::iter::once(&data)
        .chain(&chain)
        .skip_while(|l| l.width > max_dimension || l.height > max_dimension)
        .collect();
    let base = levels[0];
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("olvin texture"),
        size: wgpu::Extent3d {
            width: base.width,
            height: base.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: data.format.gpu_format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (mip, level) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &level.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(level.width * level.format.bytes_per_pixel() as u32),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: level.width,
                height: level.height,
                depth_or_array_layers: 1,
            },
        );
    }
    GpuTexture {
        view: texture.create_view(&Default::default()),
        mips: levels.len() as u32,
        generation: 0,
    }
}

fn rgb(c: [f32; 4]) -> Vec3 {
    Vec3::new(c[0], c[1], c[2])
}

/// View-projection of the first camera in the world.
pub fn camera_view_proj(world: &World, aspect: f32) -> Option<Mat4> {
    world
        .query::<(Option<&GlobalTransform>, &Transform, &Camera)>()
        .iter()
        .next()
        .map(|(g, t, c)| {
            let pose = g.map_or(*t, GlobalTransform::to_transform);
            c.projection(aspect) * Camera::view(&pose)
        })
}

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: (capacity * size_of::<InstanceRaw>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn shadow_texture(device: &wgpu::Device, size: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("olvin shadow map"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn instance_layout(attributes: &[wgpu::VertexAttribute]) -> wgpu::VertexBufferLayout<'_> {
    wgpu::VertexBufferLayout {
        array_stride: size_of::<InstanceRaw>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes,
    }
}

/// Group 0 (view: globals, shadow map, sky) and group 1 (material).
fn create_layouts(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::BindGroupLayout) {
    let fragment = wgpu::ShaderStages::FRAGMENT;
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: fragment,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let sampler = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: fragment,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    };
    let uniform = |binding, visibility, size: usize| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size as u64),
        },
        count: None,
    };
    let globals = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("globals"),
        entries: &[
            uniform(
                0,
                wgpu::ShaderStages::VERTEX | fragment,
                size_of::<Globals>(),
            ),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: fragment,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            texture(3),
            sampler(4),
        ],
    });
    let material = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("material"),
        entries: &[
            uniform(0, fragment, size_of::<MaterialUniform>()),
            texture(1),
            texture(2),
            texture(3),
            texture(4),
            sampler(5),
        ],
    });
    (globals, material)
}

fn create_mesh_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    (format, samples, depth): (wgpu::TextureFormat, u32, wgpu::TextureFormat),
) -> wgpu::RenderPipeline {
    let vertex_attrs = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4
    ];
    let instance_attrs = wgpu::vertex_attr_array![
        4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mesh"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &vertex_attrs,
                }),
                Some(instance_layout(&instance_attrs)),
            ],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Fullscreen triangle at the far plane: depth test LessEqual, no writes.
fn create_sky_pipeline(
    device: &wgpu::Device,
    globals: &wgpu::BindGroupLayout,
    (format, samples, depth): (wgpu::TextureFormat, u32, wgpu::TextureFormat),
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sky"),
        source: wgpu::ShaderSource::Wgsl(sky_source().into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sky"),
        bind_group_layouts: &[Some(globals)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sky"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_sky"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_sky"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_shadow_pipeline(device: &wgpu::Device) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shadow.wgsl"));
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow globals"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size_of::<Mat4>() as u64),
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("shadow"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let vertex_attrs = wgpu::vertex_attr_array![0 => Float32x3];
    let instance_attrs =
        wgpu::vertex_attr_array![4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shadow"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_shadow"),
            compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &vertex_attrs,
                }),
                Some(instance_layout(&instance_attrs)),
            ],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SHADOW_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            // Slope-scaled bias; the shader adds a normal offset on top.
            bias: wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 2.0,
                clamp: 0.0,
            },
        }),
        multisample: Default::default(),
        fragment: None,
        multiview_mask: None,
        cache: None,
    });
    (pipeline, layout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, Material, ShaderHandle, TextureFormat, renderer::begin_pass};

    const SIZE: u32 = 64;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits:
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .ok()
    }

    /// Offscreen target + a core configured like the window (sRGB, 1x,
    /// Depth24Plus) or like the editor's egui pass (Rgba8Unorm, 4x MSAA,
    /// Depth32Float).
    struct Harness {
        core: RenderCore,
        gpu: GpuMeshes,
        view: ViewData,
        meshes: Meshes,
        textures: Textures,
        materials: Materials,
        shaders: Shaders,
        format: wgpu::TextureFormat,
        samples: u32,
        depth: wgpu::TextureFormat,
    }

    impl Harness {
        fn new(editor_like: bool) -> Option<Self> {
            let Some((device, queue)) = device() else {
                eprintln!("no GPU adapter: skipping");
                return None;
            };
            let (format, samples, depth) = if editor_like {
                (
                    wgpu::TextureFormat::Rgba8Unorm,
                    4,
                    wgpu::TextureFormat::Depth32Float,
                )
            } else {
                (FORMAT, 1, wgpu::TextureFormat::Depth24Plus)
            };
            let core = RenderCore::new(device, queue, format, samples, depth);
            let view = core.create_view();
            Some(Self {
                core,
                gpu: GpuMeshes::default(),
                view,
                meshes: Meshes::default(),
                textures: Textures::default(),
                materials: Materials::default(),
                shaders: Shaders::default(),
                format,
                samples,
                depth,
            })
        }

        fn texture(
            &self,
            format: wgpu::TextureFormat,
            samples: u32,
            usage: wgpu::TextureUsages,
        ) -> wgpu::TextureView {
            self.core
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: SIZE,
                        height: SIZE,
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

        /// Renders the world's camera view; returns RGB at (x, y).
        fn render(
            &mut self,
            world: &World,
            settings: &RenderSettings,
        ) -> (RenderStats, impl Fn(u32, u32) -> [u8; 3] + use<>) {
            let core = &self.core;
            core.upload(&mut self.gpu, &mut self.meshes);
            core.upload_assets(
                &mut self.gpu,
                &mut self.textures,
                &mut self.materials,
                &mut self.shaders,
            );
            let stats = core.prepare(
                &mut self.view,
                &self.gpu,
                world,
                camera_view_proj(world, 1.0).unwrap(),
                settings,
            );

            let rt = wgpu::TextureUsages::RENDER_ATTACHMENT;
            let resolved = core.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: rt | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let resolved_view = resolved.create_view(&Default::default());
            let depth = self.texture(self.depth, self.samples, rt);
            let readback = core.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (SIZE * SIZE * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = core.device.create_command_encoder(&Default::default());
            core.render_shadows(&self.view, &self.gpu, &mut encoder);
            if self.samples == 1 {
                let mut pass = begin_pass(&mut encoder, &resolved_view, &depth, settings);
                core.draw(&self.view, &self.gpu, &mut pass);
            } else {
                // A foreign pass, like egui's: MSAA resolved into a non-sRGB target.
                let msaa = self.texture(self.format, self.samples, rt);
                let [r, g, b, a] = settings.clear_color.0.map(f64::from);
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("foreign"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &msaa,
                        depth_slice: None,
                        resolve_target: Some(&resolved_view),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                            store: wgpu::StoreOp::Discard,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                core.draw(&self.view, &self.gpu, &mut pass);
            }
            encoder.copy_texture_to_buffer(
                resolved.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(SIZE * 4),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
            );
            core.queue.submit([encoder.finish()]);
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            core.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let pixels = readback.slice(..).get_mapped_range().unwrap().to_vec();
            (stats, move |x: u32, y: u32| {
                let i = ((y * SIZE + x) * 4) as usize;
                [pixels[i], pixels[i + 1], pixels[i + 2]]
            })
        }
    }

    /// Pixel column of world x on the ground when looking straight down
    /// from `height` (default 60° fov, square view).
    fn column(x: f32, height: f32) -> u32 {
        let half = height * 30f32.to_radians().tan();
        ((0.5 + x / half * 0.5) * SIZE as f32) as u32
    }

    fn top_down_camera(height: f32) -> (Transform, Camera) {
        (
            Transform::from_xyz(0.0, height, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
            Camera::default(),
        )
    }

    fn luma(c: [u8; 3]) -> u32 {
        c.iter().map(|&v| v as u32).sum()
    }

    #[test]
    fn renders_a_lit_cube_offscreen() {
        let Some(mut h) = Harness::new(false) else {
            return;
        };
        let cube = h.meshes.add(MeshData::cube(1.0));
        let mut world = World::new();
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(cube, Color::rgb(1.0, 0.0, 0.0)),
        ));
        world.spawn((Transform::from_xyz(0.0, 0.0, 3.0), Camera::default()));
        let mut settings = RenderSettings {
            light_dir: Vec3::new(0.0, 0.0, -1.0),
            clear_color: Color::rgb(0.0, 0.0, 1.0),
            ..Default::default()
        };
        let (stats, at) = h.render(&world, &settings);
        assert_eq!((stats.draw_calls, stats.instances), (1, 1));
        let lit = at(SIZE / 2, SIZE / 2);
        // Red dominates; the dielectric highlight may tint green/blue a little.
        assert!(
            lit[0] > 200 && lit[1] < 120 && lit[2] < 120 && lit[0] > lit[1] + 100,
            "lit red cube in the middle: {lit:?}"
        );
        let corner = at(1, 1);
        assert!(
            corner[2] > 200 && corner[0] < 30,
            "clear colour in the corner: {corner:?}"
        );

        // Light from behind: only ambient on the visible face.
        settings.light_dir = Vec3::new(0.0, 0.0, 1.0);
        let (_, at) = h.render(&world, &settings);
        let unlit = at(SIZE / 2, SIZE / 2);
        assert!(
            unlit[0] > 20 && unlit[0] + 80 < lit[0],
            "unlit side is dimmer but not black: {unlit:?} vs {lit:?}"
        );
    }

    #[test]
    fn red_point_light_brightens_the_floor_near_it() {
        for editor_like in [false, true] {
            let Some(mut h) = Harness::new(editor_like) else {
                return;
            };
            let plane = h.meshes.add(MeshData::plane(20.0));
            let mut world = World::new();
            world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
            world.spawn(top_down_camera(8.0));
            world.spawn((
                Transform::from_xyz(-2.0, 0.7, 0.0),
                Light::point([1.0, 0.0, 0.0], 4.0, 4.0),
            ));
            // A dark sun so only the point light paints the floor red.
            let settings = RenderSettings {
                light_color: Color::BLACK,
                ..Default::default()
            };
            let (_, at) = h.render(&world, &settings);
            let (y, near, far) = (
                SIZE / 2,
                at(column(-2.0, 8.0), SIZE / 2),
                at(column(3.0, 8.0), SIZE / 2),
            );
            assert!(
                near[0] > far[0] + 60 && near[0] > near[1] + 60,
                "editor_like={editor_like}: near {near:?} far {far:?} (row {y})"
            );
            assert!(luma(far) > 0, "ambient still lights the far floor");
        }
    }

    #[test]
    fn directional_light_casts_a_shadow() {
        for editor_like in [false, true] {
            let Some(mut h) = Harness::new(editor_like) else {
                return;
            };
            let plane = h.meshes.add(MeshData::plane(20.0));
            let cube = h.meshes.add(MeshData::cube(1.0));
            let mut world = World::new();
            world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
            world.spawn((
                Transform::from_xyz(0.0, 1.5, 0.0),
                MeshRenderer::new(cube, Color::WHITE),
            ));
            world.spawn(top_down_camera(8.0));
            // Travels +X and down at 45°: the cube's shadow lands on x ∈ [0.5, 2.5].
            let sun_dir = Vec3::new(1.0, -1.0, 0.0).normalize();
            let sun = world.spawn((
                Transform::IDENTITY.looking_at(sun_dir, Vec3::Y),
                Light::directional([1.0; 3], 1.0),
            ));
            let settings = RenderSettings::default();
            let (shadowed, open) = ((column(1.5, 8.0), SIZE / 2), (column(-2.5, 8.0), SIZE / 2));

            let (_, at) = h.render(&world, &settings);
            assert!(h.view.shadows_active());
            let (s, o) = (at(shadowed.0, shadowed.1), at(open.0, open.1));
            assert!(
                luma(s) + 150 < luma(o),
                "editor_like={editor_like}: shadowed {s:?} vs open {o:?}"
            );

            // Shadows off: the same pixel is lit like the rest of the floor.
            world.get::<&mut Light>(sun).unwrap().shadows = false;
            let (_, at) = h.render(&world, &settings);
            assert!(!h.view.shadows_active());
            let (s, o) = (at(shadowed.0, shadowed.1), at(open.0, open.1));
            assert!(
                luma(s).abs_diff(luma(o)) < 30,
                "editor_like={editor_like}: no shadow {s:?} vs {o:?}"
            );
        }
    }

    #[test]
    fn first_directional_light_replaces_the_settings_sun() {
        let Some(mut h) = Harness::new(false) else {
            return;
        };
        let plane = h.meshes.add(MeshData::plane(20.0));
        let mut world = World::new();
        world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
        world.spawn(top_down_camera(8.0));
        let settings = RenderSettings {
            light_dir: Vec3::NEG_Y,
            light_color: Color::WHITE,
            ..Default::default()
        };
        let (_, at) = h.render(&world, &settings);
        let bright = at(SIZE / 2, SIZE / 2);
        // A green sun overrides the white settings sun.
        world.spawn((
            Transform::IDENTITY.looking_at(Vec3::NEG_Y, Vec3::Z),
            Light::directional([0.0, 1.0, 0.0], 1.0),
        ));
        let (_, at) = h.render(&world, &settings);
        let green = at(SIZE / 2, SIZE / 2);
        assert!(
            green[1] > green[0] + 60 && green[0] + 60 < bright[0],
            "{green:?} vs {bright:?}"
        );
        // Solid view: scene lights off, the settings sun again.
        let solid = RenderSettings {
            scene_lights: false,
            ..settings
        };
        let (_, at) = h.render(&world, &solid);
        assert_eq!(at(SIZE / 2, SIZE / 2), bright);
    }

    // ---- Materials, textures, skies, surface shaders.

    /// No lights, no ambient, no tonemapping: pixels show material colours as is.
    fn flat_settings() -> RenderSettings {
        RenderSettings {
            light_color: Color::BLACK,
            ambient: Color::BLACK,
            ground_ambient: Color::BLACK,
            tonemapping: Tonemapping::None,
            clear_color: Color::rgb(0.0, 0.0, 1.0),
            ..Default::default()
        }
    }

    fn unlit(texture: Option<TextureHandle>) -> Material {
        Material {
            base_color: Color::WHITE,
            base_texture: texture,
            point_filter: true,
            unlit: true,
            ..Default::default()
        }
    }

    fn close(c: [u8; 3], want: [u8; 3]) -> bool {
        c.iter().zip(want).all(|(&a, b)| a.abs_diff(b) <= 6)
    }

    #[test]
    fn textured_quad_shows_texels_at_known_uvs() {
        for editor_like in [false, true] {
            let Some(mut h) = Harness::new(editor_like) else {
                return;
            };
            // 2×2: red green / blue white, rows top to bottom.
            let texture = h.textures.add(TextureData::rgba8(
                2,
                2,
                [
                    [255, 0, 0, 255],
                    [0, 255, 0, 255],
                    [0, 0, 255, 255],
                    [255; 4],
                ]
                .concat(),
                true,
            ));
            let material = h.materials.add(unlit(Some(texture)));
            let plane = h.meshes.add(MeshData::plane(4.0));
            let mut world = World::new();
            world.spawn((
                Transform::IDENTITY,
                MeshRenderer::new(plane, Color::WHITE).with_material(material),
            ));
            world.spawn(top_down_camera(8.0));
            let (_, at) = h.render(&world, &flat_settings());
            // Image top is -Z (the top of the screen), image left is -X.
            let px = |x: f32, z: f32| at(column(x, 8.0), column(z, 8.0));
            let cases = [
                ((-1.0, -1.0), [255, 0, 0]),
                ((1.0, -1.0), [0, 255, 0]),
                ((-1.0, 1.0), [0, 0, 255]),
                ((1.0, 1.0), [255, 255, 255]),
            ];
            for ((x, z), want) in cases {
                let got = px(x, z);
                assert!(
                    close(got, want),
                    "editor_like={editor_like} ({x}, {z}): {got:?}"
                );
            }
            // Outside the plane: clear colour.
            assert!(close(px(3.0, 0.0), [0, 0, 255]));
        }
    }

    #[test]
    fn roughness_and_metallic_shape_the_highlight() {
        let Some(mut h) = Harness::new(false) else {
            return;
        };
        let plane = h.meshes.add(MeshData::plane(20.0));
        let red = Color::rgb(1.0, 0.0, 0.0);
        let mut make = |roughness: f32, metallic: f32, base_color: Color| {
            h.materials.add(Material {
                base_color,
                roughness,
                metallic,
                ..Default::default()
            })
        };
        let smooth = make(0.15, 0.0, Color::WHITE);
        let rough = make(0.9, 0.0, Color::WHITE);
        let red_plastic = make(0.3, 0.0, red);
        let red_metal = make(0.3, 1.0, red);
        let settings = RenderSettings {
            light_color: Color::BLACK,
            ..Default::default()
        };
        let mut centre = |material| {
            let mut world = World::new();
            world.spawn((
                Transform::IDENTITY,
                MeshRenderer::new(plane, Color::WHITE).with_material(material),
            ));
            world.spawn(top_down_camera(8.0));
            // Straight under the camera: the mirror direction at the centre.
            world.spawn((
                Transform::from_xyz(0.0, 2.0, 0.0),
                Light::point([1.0; 3], 1.0, 10.0),
            ));
            let (_, at) = h.render(&world, &settings);
            at(SIZE / 2, SIZE / 2)
        };
        let (s, r) = (centre(smooth), centre(rough));
        assert!(luma(s) > luma(r) + 100, "smooth {s:?} vs rough {r:?}");
        // A dielectric's highlight is white; a metal's takes the base colour.
        let (plastic, metal) = (centre(red_plastic), centre(red_metal));
        assert!(
            plastic[1] > metal[1] + 80 && metal[0] > 200,
            "plastic {plastic:?} vs metal {metal:?}"
        );
    }

    #[test]
    fn emissive_shows_without_lights() {
        let Some(mut h) = Harness::new(true) else {
            return;
        };
        let material = h.materials.add(Material {
            base_color: Color::BLACK,
            emissive: [0.0, 1.0, 0.0],
            ..Default::default()
        });
        let cube = h.meshes.add(MeshData::cube(1.0));
        let mut world = World::new();
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(cube, Color::WHITE).with_material(material),
        ));
        world.spawn((Transform::from_xyz(0.0, 0.0, 3.0), Camera::default()));
        let (_, at) = h.render(&world, &flat_settings());
        let c = at(SIZE / 2, SIZE / 2);
        assert!(close(c, [0, 255, 0]), "{c:?}");
    }

    #[test]
    fn alpha_cutoff_discards() {
        let Some(mut h) = Harness::new(false) else {
            return;
        };
        // Left texel transparent, right opaque.
        let texture = h.textures.add(TextureData::rgba8(
            2,
            1,
            vec![255, 255, 255, 0, 255, 255, 255, 255],
            true,
        ));
        let material = h.materials.add(Material {
            alpha_cutoff: 0.5,
            ..unlit(Some(texture))
        });
        let plane = h.meshes.add(MeshData::plane(4.0));
        let mut world = World::new();
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(plane, Color::WHITE).with_material(material),
        ));
        world.spawn(top_down_camera(8.0));
        let (_, at) = h.render(&world, &flat_settings());
        let (cut, kept) = (
            at(column(-1.0, 8.0), SIZE / 2),
            at(column(1.0, 8.0), SIZE / 2),
        );
        assert!(close(cut, [0, 0, 255]), "discarded: clear colour {cut:?}");
        assert!(close(kept, [255, 255, 255]), "kept {kept:?}");
    }

    /// A camera at the origin looking down -Z (horizon across the middle row).
    fn horizon_world() -> World {
        let mut world = World::new();
        world.spawn((Transform::IDENTITY, Camera::default()));
        world
    }

    #[test]
    fn procedural_sky_fills_the_background() {
        for editor_like in [false, true] {
            let Some(mut h) = Harness::new(editor_like) else {
                return;
            };
            let world = horizon_world();
            let mut settings = RenderSettings {
                sky: SkyMode::Procedural {
                    zenith: [0.0, 0.0, 1.0],
                    horizon: [1.0, 0.0, 0.0],
                    ground: [0.0, 1.0, 0.0],
                    sun_size: 0.0,
                },
                ..flat_settings()
            };
            settings.clear_color = Color::BLACK;
            let (stats, at) = h.render(&world, &settings);
            assert_eq!(stats.draw_calls, 0);
            assert!(h.view.sky_active());
            let (top, middle, bottom) = (
                at(SIZE / 2, 0),
                at(SIZE / 2, SIZE / 2),
                at(SIZE / 2, SIZE - 1),
            );
            assert!(
                top[2] > top[0] + 100,
                "editor_like={editor_like} zenith-ish top {top:?}"
            );
            assert!(
                middle[0] > middle[2] + 100,
                "horizon in the middle {middle:?}"
            );
            assert!(bottom[1] > bottom[0] + 100, "ground below {bottom:?}");

            // A sun straight ahead: a white disc in the middle.
            settings.light_dir = Vec3::Z;
            settings.light_color = Color::WHITE;
            settings.sky = SkyMode::Procedural {
                zenith: [0.0, 0.0, 1.0],
                horizon: [1.0, 0.0, 0.0],
                ground: [0.0, 1.0, 0.0],
                sun_size: 5.0,
            };
            let (_, at) = h.render(&world, &settings);
            assert_eq!(at(SIZE / 2, SIZE / 2), [255, 255, 255]);
        }
    }

    #[test]
    fn equirect_texture_sky() {
        for editor_like in [false, true] {
            let Some(mut h) = Harness::new(editor_like) else {
                return;
            };
            // 4×8 HDR panorama: upper half blue 2.0, lower half red 2.0.
            let mut pixels = Vec::new();
            for row in 0..8 {
                let c = if row < 4 {
                    [0.0, 0.0, 2.0, 1.0]
                } else {
                    [2.0, 0.0, 0.0, 1.0]
                };
                for _ in 0..4 {
                    pixels.extend(c);
                }
            }
            let texture = h.textures.add(TextureData::rgba32f(4, 8, &pixels));
            let world = horizon_world();
            let mut settings = RenderSettings {
                sky: SkyMode::Texture {
                    texture,
                    rotation: 0.0,
                    intensity: 1.0,
                },
                ..flat_settings()
            };
            let (_, at) = h.render(&world, &settings);
            let (top, bottom) = (at(SIZE / 2, 2), at(SIZE / 2, SIZE - 3));
            assert!(close(top, [0, 0, 255]), "editor_like={editor_like} {top:?}");
            assert!(close(bottom, [255, 0, 0]), "{bottom:?}");
            // HDR × intensity: 2.0 × 0.25 = 0.5 linear ≈ 188 sRGB.
            settings.sky = SkyMode::Texture {
                texture,
                rotation: 0.0,
                intensity: 0.25,
            };
            let (_, at) = h.render(&world, &settings);
            let top = at(SIZE / 2, 2);
            assert!(top[2].abs_diff(188) <= 4 && top[0] < 5, "{top:?}");
        }
    }

    #[test]
    fn triplanar_projects_in_world_space() {
        let Some(mut h) = Harness::new(false) else {
            return;
        };
        // Red | green.
        let texture = h.textures.add(TextureData::rgba8(
            2,
            1,
            vec![255, 0, 0, 255, 0, 255, 0, 255],
            true,
        ));
        let cube = h.meshes.add(MeshData::cube(4.0));
        let triplanar = h.materials.add(Material {
            triplanar: true,
            // One repeat per 2 m: red on x ∈ (0, 1), green on (1, 2) and (-1, 0).
            uv_scale: [0.5, 0.5],
            ..unlit(Some(texture))
        });
        let mesh_uv = h.materials.add(unlit(Some(texture)));
        let mut world = World::new();
        let e = world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(cube, Color::WHITE).with_material(triplanar),
        ));
        world.spawn(top_down_camera(8.0));
        let settings = flat_settings();
        // The top face is 6 m from the camera.
        let (_, at) = h.render(&world, &settings);
        let px = |x: f32| at(column(x, 6.0), SIZE / 2);
        assert!(close(px(0.5), [255, 0, 0]), "{:?}", px(0.5));
        assert!(close(px(1.5), [0, 255, 0]), "{:?}", px(1.5));
        assert!(close(px(-0.5), [0, 255, 0]), "{:?}", px(-0.5));
        // Mesh UVs instead: the face maps the texture once (left red, right green).
        world.get::<&mut MeshRenderer>(e).unwrap().material = Some(mesh_uv);
        let (_, at) = h.render(&world, &settings);
        let px = |x: f32| at(column(x, 6.0), SIZE / 2);
        assert!(close(px(0.5), [0, 255, 0]) && close(px(-0.5), [255, 0, 0]));
    }

    #[test]
    fn normal_map_tilts_towards_tangent_and_image_up() {
        let Some(mut h) = Harness::new(false) else {
            return;
        };
        let plane = h.meshes.add(MeshData::plane(20.0));
        let encode = |n: Vec3| {
            let n = n.normalize();
            [n.x, n.y, n.z].map(|c| ((c * 0.5 + 0.5) * 255.0).round() as u8)
        };
        let mut map = |n: Vec3| {
            let [r, g, b] = encode(n);
            let t = h
                .textures
                .add(TextureData::rgba8(1, 1, vec![r, g, b, 255], false));
            h.materials.add(Material {
                base_color: Color::WHITE,
                normal_texture: Some(t),
                ..Default::default()
            })
        };
        // Tangent space: +x = +u = world +X; +y = image up = world -Z on the plane.
        let towards_u = map(Vec3::new(1.0, 0.0, 1.0));
        let towards_up = map(Vec3::new(0.0, 1.0, 1.0));
        let flat = h.materials.add(Material {
            base_color: Color::WHITE,
            ..Default::default()
        });
        let mut lit = |material, light_dir: Vec3| {
            let mut world = World::new();
            world.spawn((
                Transform::IDENTITY,
                MeshRenderer::new(plane, Color::WHITE).with_material(material),
            ));
            world.spawn(top_down_camera(8.0));
            let settings = RenderSettings {
                light_dir,
                ambient: Color::BLACK,
                ground_ambient: Color::BLACK,
                ..Default::default()
            };
            let (_, at) = h.render(&world, &settings);
            luma(at(column(-2.0, 8.0), column(-2.0, 8.0)))
        };
        // Grazing light from +X (travelling -X) and from -Z (travelling +Z).
        let from_x = Vec3::new(-1.0, -0.15, 0.0);
        let from_neg_z = Vec3::new(0.0, -0.15, 1.0);
        let (flat_x, tilted_x) = (lit(flat, from_x), lit(towards_u, from_x));
        assert!(
            tilted_x > flat_x + 150,
            "towards +u: {tilted_x} vs {flat_x}"
        );
        let (flat_z, tilted_z) = (lit(flat, from_neg_z), lit(towards_up, from_neg_z));
        assert!(
            tilted_z > flat_z + 150,
            "towards image up: {tilted_z} vs {flat_z}"
        );
    }

    const MAGENTA_SURFACE: &str = "fn surface(in: SurfaceInput) -> SurfaceOutput {
    var out = default_surface(in);
    out.color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.emissive = vec3<f32>(1.0, 0.0, 1.0);
    return out;
}
";

    /// A cube drawn with `shader`; returns the centre pixel.
    fn shaded_cube(h: &mut Harness, shader: ShaderHandle) -> [u8; 3] {
        let material = h.materials.add(Material {
            shader: Some(shader),
            ..Default::default()
        });
        let cube = h.meshes.add(MeshData::cube(1.0));
        let mut world = World::new();
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(cube, Color::WHITE).with_material(material),
        ));
        // A plain cube too, so both pipelines draw in one pass.
        world.spawn((
            Transform::from_xyz(5.0, 0.0, 0.0),
            MeshRenderer::new(cube, Color::WHITE),
        ));
        world.spawn((Transform::from_xyz(0.0, 0.0, 3.0), Camera::default()));
        let settings = RenderSettings {
            light_dir: Vec3::NEG_Z,
            ..flat_settings()
        };
        let (_, at) = h.render(&world, &settings);
        at(SIZE / 2, SIZE / 2)
    }

    #[test]
    fn custom_surface_shader_outputs_its_colour() {
        for editor_like in [false, true] {
            let Some(mut h) = Harness::new(editor_like) else {
                return;
            };
            let shader = h.shaders.add(MAGENTA_SURFACE.into());
            assert_eq!(h.shaders.error(shader), None);
            let c = shaded_cube(&mut h, shader);
            assert!(close(c, [255, 0, 255]), "editor_like={editor_like}: {c:?}");
        }
    }

    #[test]
    fn broken_surface_shader_reports_and_keeps_the_last_good_pipeline() {
        let Some(mut h) = Harness::new(false) else {
            return;
        };
        let broken = "fn surface(in: SurfaceInput) -> SurfaceOutput {\n    return 1.0;\n}\n";
        // Never compiled: the built-in pipeline draws instead, no panic.
        let shader = h.shaders.add(broken.into());
        let err = h.shaders.error(shader).expect("an error");
        assert!(
            err.contains("return 1.0") && !err.contains("template"),
            "{err}"
        );
        let c = shaded_cube(&mut h, shader);
        // No lights in these settings: the built-in grey material is black.
        assert!(close(c, [0, 0, 0]), "built-in look: {c:?}");
        // Good, then broken again: the good pipeline stays.
        h.shaders.set(shader, MAGENTA_SURFACE.into());
        assert!(close(shaded_cube(&mut h, shader), [255, 0, 255]));
        h.shaders.set(shader, broken.into());
        assert!(h.shaders.error(shader).is_some());
        assert!(close(shaded_cube(&mut h, shader), [255, 0, 255]));
    }

    #[test]
    fn mip_chain_reaches_the_gpu_and_big_textures_shrink_to_fit() {
        let Some((device, queue)) = device() else {
            return;
        };
        let data = TextureData::rgba8(8, 4, vec![200; 8 * 4 * 4], true);
        let t = upload_texture(&device, &queue, &data, u32::MAX);
        assert_eq!(t.mips, TextureData::mip_count(8, 4));
        assert_eq!(t.view.texture().mip_level_count(), 4);
        // Over the limit: starts at the first level that fits.
        let t = upload_texture(&device, &queue, &data, 2);
        assert_eq!(t.mips, 2);
        assert_eq!(t.view.texture().width(), 2);
        // Bad data: a 1×1 stand-in instead of a panic.
        let bad = TextureData {
            format: TextureFormat::Rgba16Float,
            ..data
        };
        assert_eq!(upload_texture(&device, &queue, &bad, u32::MAX).mips, 1);
    }

    #[test]
    fn fog_fades_into_the_sky_at_the_horizon() {
        let Some(mut h) = Harness::new(false) else {
            return;
        };
        let plane = h.meshes.add(MeshData::plane(400.0));
        let mut world = World::new();
        world.spawn((
            Transform::from_xyz(0.0, -1.0, 0.0),
            MeshRenderer::new(plane, Color::WHITE),
        ));
        world.spawn((Transform::IDENTITY, Camera::default()));
        let settings = RenderSettings {
            sky: SkyMode::Procedural {
                zenith: [0.0, 0.0, 1.0],
                horizon: [1.0, 0.0, 0.0],
                ground: [0.0, 1.0, 0.0],
                sun_size: 0.0,
            },
            fog: Some(crate::Fog {
                color: Color::rgb(0.0, 1.0, 0.0),
                start: 0.0,
                end: 1.0,
            }),
            ..flat_settings()
        };
        let (_, at) = h.render(&world, &settings);
        // The floor just below the horizon is fully fogged: horizon red, not fog green.
        let c = at(SIZE / 2, SIZE / 2 + 4);
        assert!(close(c, [255, 0, 0]), "{c:?}");
    }
}

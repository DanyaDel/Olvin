//! The renderer without a window: pipelines, GPU assets and per-camera data.
//! The window renderer, the editor's scene/game views and headless tests all
//! draw through this.
//!
//! Frame order for a view: `upload` + `upload_assets` (new meshes, textures,
//! materials, shaders) → `prepare` (CPU: culling, lights, shadow fit,
//! uniforms) → `render_shadows` (own depth pass into the view's shadow map)
//! → `draw` (into the caller's main pass: opaque meshes, the sky, then the
//! alpha-blended meshes sorted back to front).

mod assets;
mod frame;
mod init;
mod instancing;
mod shadows;
mod skin;
#[cfg(test)]
mod tests;

use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use olvin_ecs::World;
use olvin_scene::{Camera, Transform};
use olvin_transform::GlobalTransform;

use crate::light::{MAX_CASCADES, MAX_LIGHTS, MAX_LOCAL_TILES, PackedLight};
use crate::{BlendMode, Material, RenderTier, TextureHandle};
use instancing::{BlendSet, InstanceSet};
use shadows::Cascade;

pub(crate) const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Bucket key's material half for "no material" (the fallback material).
const NO_MATERIAL: u32 = u32::MAX;

/// Uniforms of the main pass; mirrors `Globals` in globals.wgsl (all vec4 /
/// mat4, so std140 and WGSL layouts agree).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Globals {
    view_proj: [[f32; 4]; 4],
    shadow_view_proj: [[[f32; 4]; 4]; MAX_CASCADES],
    local_view_proj: [[[f32; 4]; 4]; MAX_LOCAL_TILES],
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
    shadow_splits: [f32; 4],
    shadow_offsets: [f32; 4],
    shadow_cfg: [f32; 4],
    shadow_texel: [f32; 4],
    shadow_depth: [f32; 4],
    local_cfg: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    sky_ground: [f32; 4],
    sky_params: [f32; 4],
    height_fog_color: [f32; 4],
    height_fog: [f32; 4],
    lights: [PackedLight; MAX_LIGHTS],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct InstanceRaw {
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

/// A compiled surface shader: its opaque pipeline, and the alpha-blended
/// one, built the first time a blended material uses the shader.
struct SurfacePipeline {
    module: wgpu::ShaderModule,
    opaque: wgpu::RenderPipeline,
    blend: Option<wgpu::RenderPipeline>,
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
    pipelines: Vec<Option<SurfacePipeline>>,
    /// GPU-skinned meshes by mesh slot (Full tier, see `skin.rs`).
    skins: std::collections::HashMap<u32, skin::GpuSkin>,
    generation: u64,
}

impl GpuMeshes {
    /// An uploaded texture and its generation (changes when it's replaced),
    /// for the quad renderers (particles, sprites).
    pub(crate) fn texture_view(&self, handle: TextureHandle) -> Option<(&wgpu::TextureView, u64)> {
        self.textures
            .get(handle.0 as usize)
            .and_then(Option::as_ref)
            .map(|t| (&t.view, t.generation))
    }

    fn is_blended(&self, material: u32) -> bool {
        self.materials
            .get(material as usize)
            .and_then(Option::as_ref)
            .is_some_and(|m| m.material.blend == BlendMode::Alpha)
    }

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

/// Per-camera state: uniforms, instance buffers, the sun's shadow map and
/// frame scratch.
pub struct ViewData {
    globals: wgpu::Buffer,
    /// Globals + shadow map + sky texture.
    bind: wgpu::BindGroup,
    main: InstanceSet,
    blend: BlendSet,
    staging: Vec<InstanceRaw>,
    /// Created on demand, kept afterwards; the first `cascade_count` are live.
    cascades: Vec<Cascade>,
    cascade_count: usize,
    /// Side of one cascade's tile in the shadow map, in texels (the map is a
    /// 2x2 atlas of tiles when there is more than one cascade).
    shadow_tile: u32,
    /// Attachment view and size; created on first use, kept afterwards.
    shadow_map: Option<(wgpu::TextureView, u32)>,
    /// The spot / point light atlas (4x4 tiles) and its size, the same way.
    local_map: Option<(wgpu::TextureView, u32)>,
    /// One per atlas tile; the first `local_count` are live.
    local_tiles: Vec<Cascade>,
    local_count: usize,
    /// Side of one atlas tile, in texels.
    local_tile: u32,
    /// Generation of the sky texture in `bind` (None = the default).
    sky_bound: Option<u64>,
    sky_active: bool,
}

impl ViewData {
    /// Whether the last `prepare` enabled the sun's shadows (so
    /// `render_shadows` has work to do).
    pub fn shadows_active(&self) -> bool {
        self.cascade_count > 0
    }

    /// Atlas tiles of spot / point shadows the last `prepare` rendered
    /// (a spot light takes one, a point light six).
    pub fn local_shadow_tiles(&self) -> usize {
        self.local_count
    }

    /// Cascades of the sun's shadow the last `prepare` rendered (0 = off).
    pub fn shadow_cascades(&self) -> usize {
        self.cascade_count
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
    /// The same, alpha-blended without depth writes.
    blend_pipeline: wgpu::RenderPipeline,
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
    /// Writes untonemapped linear light for a post-processing chain.
    hdr_output: bool,
    tier: RenderTier,
    /// The skinning compute pipeline, built on first use.
    skin_pipeline: std::sync::OnceLock<skin::SkinPipeline>,
    /// Whether the mesh shaders can read raw shadow depths (PCSS): native
    /// APIs only, decided when the shaders were compiled.
    pcss_shader: bool,
    time: Option<f32>,
    start: Instant,
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

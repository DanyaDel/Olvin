//! Renderer on wgpu: Vulkan / Metal / DX12 / GLES 3 / WebGL2 / WebGPU.
//! Reads the ECS world and a [`SurfaceTarget`](olvin_core::SurfaceTarget);
//! knows nothing about the windowing library.

pub mod core;
mod frustum;
mod gl_check;
mod light;
mod material;
mod mesh;
mod overlay;
mod particles;
mod particles_gpu;
mod pipelines;
mod post;
mod quad;
mod renderer;
mod retro;
mod shader;
mod skinning;
mod sprite;
mod texture;

pub use core::{GpuMeshes, RenderCore, ViewData, camera_view_proj};
pub use frustum::Frustum;
pub use gl_check::{GlCheck, GlCheckInput, check_gl};
pub use light::{
    DEFAULT_LIGHT_SIZE, DEFAULT_SHADOW_BIAS, Light, LightKind, MAX_CASCADES, MAX_LIGHTS,
    MAX_LOCAL_TILES,
};
pub use material::{BlendMode, Material, MaterialHandle, Materials};
pub use mesh::{MeshData, MeshHandle, Meshes, Vertex};
pub use overlay::{Overlay, OverlayQuad, OverlayRenderer, begin_overlay_pass};
pub use particles::{
    Burst, EmitShape, EmitterParams, FlipBook, MAX_GPU_PARTICLES, ParticleEmitter, ParticleMode,
    ParticleRenderer, SimSpace, gpu_particles_available, soft_circle_texture, update_emitters,
};
pub use particles_gpu::{GpuParticle, device_limits as full_tier_limits};
pub use post::{
    AmbientOcclusion, ColorGrade, HDR_FORMAT, PostProcess, PostSettings, identity_lut, lut_strip,
};
pub use quad::{QuadBlend, QuadFilter};
pub use renderer::{
    RenderInfo, RenderStats, RenderTier, Renderer, begin_pass, begin_scene_pass, tier_of,
};
pub use retro::RetroSettings;
pub use shader::{DEFAULT_SURFACE_SHADER, ShaderHandle, Shaders, validate_surface_shader};
pub use skinning::{SkinnedMesh, skin_meshes};
pub use sprite::{
    AtlasRegion, Camera2d, Space2d, Sprite, SpriteAtlas, SpriteRenderer, TileSet, Tilemap,
    screen_view_proj,
};
pub use texture::{TextureData, TextureFormat, TextureHandle, Textures};
/// The wgpu version the engine renders with (the editor shares it).
pub use wgpu;

use glam::Vec3;
use olvin_core::{App, Plugin, Stage};
use olvin_ecs::EcsPlugin;

/// Linear RGBA.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Color(pub [f32; 4]);

impl Color {
    pub const WHITE: Self = Self([1.0; 4]);
    pub const BLACK: Self = Self([0.0, 0.0, 0.0, 1.0]);

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self([r, g, b, 1.0])
    }
}

/// Draws `mesh` at the entity's [`Transform`](olvin_scene::Transform).
#[derive(Clone, Copy, Debug)]
pub struct MeshRenderer {
    pub mesh: MeshHandle,
    /// Instance colour; multiplies the material's base colour.
    pub color: Color,
    /// `None`: the instance colour with default PBR factors ([`Material::fallback`]).
    pub material: Option<MaterialHandle>,
}

impl MeshRenderer {
    pub fn new(mesh: MeshHandle, color: Color) -> Self {
        Self {
            mesh,
            color,
            material: None,
        }
    }

    pub fn with_material(mut self, material: MaterialHandle) -> Self {
        self.material = Some(material);
        self
    }
}

/// What's drawn behind everything (and reflected by shiny surfaces).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SkyMode {
    /// `RenderSettings::clear_color`; reflections see the hemisphere ambient.
    #[default]
    Color,
    /// Gradient with a sun disc towards the first directional light. Linear colours.
    Procedural {
        zenith: [f32; 3],
        horizon: [f32; 3],
        ground: [f32; 3],
        /// Angular radius of the sun disc in degrees (0 = no disc).
        sun_size: f32,
    },
    /// Equirectangular panorama (HDR or sRGB); u = 0.5 looks down -Z.
    Texture {
        texture: TextureHandle,
        /// Around +Y, radians.
        rotation: f32,
        intensity: f32,
    },
}

/// Linear distance fog, applied before tonemapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    pub color: Color,
    /// View depth where fog starts.
    pub start: f32,
    /// View depth where everything is fog colour.
    pub end: f32,
}

/// Exponential height fog, applied before tonemapping next to [`Fog`]: the
/// air is densest at `height` and thins out upwards, so valleys fill with
/// mist and the camera looking down through it sees more than looking along
/// it. Works on every tier (it lives in the mesh shader). Like [`Fog`] it
/// fades into the sky's horizon colour when a sky is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightFog {
    pub color: Color,
    /// Fog per metre at `height`; 0.02 sees about 50 m.
    pub density: f32,
    /// How fast the density drops per metre above `height` (0 = uniform).
    pub falloff: f32,
    /// World altitude (Y) where the density equals `density`.
    pub height: f32,
}

impl Default for HeightFog {
    fn default() -> Self {
        Self {
            color: Color::rgb(0.5, 0.6, 0.7),
            density: 0.02,
            falloff: 0.15,
            height: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tonemapping {
    /// Clamp; what you write is what you get.
    None,
    /// ACES filmic (Narkowicz fit), like Unity's ACES.
    #[default]
    Aces,
}

/// How the sun's shadow map is filtered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadowFilter {
    /// One hardware-filtered tap: crisp, cheapest.
    Hard,
    /// 3×3 hardware-filtered taps (PCF), everywhere.
    #[default]
    Pcf,
    /// 16 taps over a rotated disc: smooth penumbrae at large radii. Needs
    /// [`RenderTier::Full`]; Compat hardware falls back to `Pcf`.
    Soft,
    /// Percentage-closer soft shadows for the sun: a blocker search sizes the
    /// filter, so penumbrae widen with the distance from the occluder (see
    /// `Light::size`). Local lights filter like `Soft`. Needs
    /// [`RenderTier::Full`]; Compat hardware falls back to `Pcf`.
    Pcss,
}

/// Glow around bright areas (post-processing, Full tier only).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bloom {
    /// Scene brightness (linear, before exposure) where glow starts.
    pub threshold: f32,
    /// How much of the blurred bright areas is added back.
    pub intensity: f32,
}

impl Default for Bloom {
    fn default() -> Self {
        Self {
            threshold: 1.0,
            intensity: 0.3,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RenderSettings {
    /// Fifo when on: no tearing, and the GPU idles instead of burning battery.
    pub vsync: bool,
    /// Prefer the integrated GPU.
    pub low_power: bool,
    pub clear_color: Color,
    /// Background; anything but `Color` also tints reflections and fog.
    pub sky: SkyMode,
    /// Direction the light travels. Fallback sun: the first directional
    /// [`Light`] in the world replaces it.
    pub light_dir: Vec3,
    /// Linear RGB × intensity of the fallback sun.
    pub light_color: Color,
    /// Whether the fallback sun casts shadows.
    pub light_shadows: bool,
    /// Sky ambient: what surfaces facing up receive (hemisphere ambient).
    pub ambient: Color,
    /// Ground ambient: what surfaces facing down receive.
    pub ground_ambient: Color,
    /// Multiplies the lit colour before tonemapping.
    pub exposure: f32,
    pub tonemapping: Tonemapping,
    pub fog: Option<Fog>,
    /// Exponential height fog, on top of [`Self::fog`].
    pub height_fog: Option<HeightFog>,
    /// Bloom; needs the post-processing chain of [`RenderTier::Full`] in the
    /// window renderer, ignored elsewhere.
    pub bloom: Option<Bloom>,
    /// FXAA and the other post-processing passes (Full tier, window renderer).
    pub post: PostSettings,
    /// How far from the camera the sun's shadows reach (metres).
    pub shadow_distance: f32,
    /// Shadow map resolution (square); clamped to the device's limit.
    pub shadow_map_size: u32,
    /// Cascades the sun's shadow map is split into (1 to [`MAX_CASCADES`]):
    /// sharp shadows near the camera, coarse far away. They share the map as
    /// tiles, so more cascades means smaller tiles. Compat hardware uses at
    /// most 2.
    pub shadow_cascades: u32,
    pub shadow_filter: ShadowFilter,
    /// Filter reach in shadow texels (1 = the classic 3×3 PCF kernel).
    pub shadow_filter_radius: f32,
    /// Angular diameter of the fallback sun in degrees, for
    /// [`ShadowFilter::Pcss`] (the sun `Light` has its own `size`).
    pub shadow_light_size: f32,
    /// Most point + spot lights' shadow maps are drawn for: spot shadows
    /// cost one tile each, point shadows six. Compat hardware halves them.
    pub max_spot_shadows: u32,
    pub max_point_shadows: u32,
    /// Side of the atlas holding every spot / point light tile (4×4 tiles);
    /// clamped to the device's limit.
    pub local_shadow_size: u32,
    /// Use `Light` components; off = only the fallback sun (editor's Solid view).
    pub scene_lights: bool,
    /// The retro (PS1) look; materials can opt in or out one by one.
    pub retro: RetroSettings,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            vsync: true,
            low_power: false,
            clear_color: Color::rgb(0.02, 0.02, 0.03),
            sky: SkyMode::Color,
            light_dir: Vec3::new(-0.4, -1.0, -0.3),
            light_color: Color::WHITE,
            light_shadows: false,
            ambient: Color::rgb(0.15, 0.15, 0.18),
            ground_ambient: Color::rgb(0.07, 0.065, 0.06),
            exposure: 1.0,
            tonemapping: Tonemapping::Aces,
            fog: None,
            height_fog: None,
            bloom: None,
            post: PostSettings::default(),
            shadow_distance: 60.0,
            shadow_map_size: 2048,
            shadow_cascades: 4,
            shadow_filter: ShadowFilter::Pcf,
            shadow_filter_radius: 1.0,
            shadow_light_size: 3.0,
            max_spot_shadows: 4,
            max_point_shadows: 2,
            local_shadow_size: 2048,
            scene_lights: true,
            retro: RetroSettings::default(),
        }
    }
}

#[derive(Default)]
pub struct RenderPlugin {
    pub settings: RenderSettings,
}

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(RenderDataPlugin)
            .insert_resource(self.settings.clone())
            .init_resource::<RenderStats>()
            .add_system(Stage::Startup, renderer::init)
            .add_system(Stage::Render, renderer::render);
    }
}

/// The part of the renderer that needs no GPU: the asset registries
/// (`Meshes`, `Textures`, `Materials`, `Shaders`), `AnimPlugin` with CPU
/// skinning and the particle simulation. `RenderPlugin` adds it; headless
/// games and tests add it alone, so scenes spawn and skinned meshes pose.
pub struct RenderDataPlugin;

impl Plugin for RenderDataPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(EcsPlugin)
            .add_plugin(olvin_transform::TransformPlugin)
            .add_plugin(olvin_anim::AnimPlugin)
            .init_resource::<Meshes>()
            .init_resource::<Textures>()
            .init_resource::<Materials>()
            .init_resource::<Shaders>()
            .init_resource::<Overlay>()
            .add_system(Stage::First, overlay::clear)
            .add_system(Stage::PostUpdate, particles::update)
            .add_system(Stage::PostUpdate, skinning::skin_meshes);
    }
}

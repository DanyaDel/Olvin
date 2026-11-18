//! Renderer on wgpu: Vulkan / Metal / DX12 / GLES 3 / WebGL2 / WebGPU.
//! Reads the ECS world and a [`SurfaceTarget`](olvin_core::SurfaceTarget);
//! knows nothing about the windowing library.

pub mod core;
mod frustum;
mod light;
mod material;
mod mesh;
mod renderer;
mod shader;
mod texture;

pub use core::{GpuMeshes, RenderCore, ViewData, camera_view_proj};
pub use frustum::Frustum;
pub use light::{Light, LightKind, MAX_LIGHTS};
pub use material::{Material, MaterialHandle, Materials};
pub use mesh::{MeshData, MeshHandle, Meshes, Vertex};
pub use renderer::{RenderInfo, RenderStats, RenderTier, Renderer, begin_pass, tier_of};
pub use shader::{DEFAULT_SURFACE_SHADER, ShaderHandle, Shaders, validate_surface_shader};
pub use texture::{TextureData, TextureFormat, TextureHandle, Textures};
/// The wgpu version the engine renders with (the editor shares it).
pub use wgpu;

use glam::Vec3;
use olvin_core::{App, Plugin, Stage};
use olvin_ecs::EcsPlugin;

/// Linear RGBA.
#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tonemapping {
    /// Clamp; what you write is what you get.
    None,
    /// ACES filmic (Narkowicz fit), like Unity's ACES.
    #[default]
    Aces,
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
    /// How far from the camera the sun's shadows reach (metres).
    pub shadow_distance: f32,
    /// Shadow map resolution (square); clamped to the device's limit.
    pub shadow_map_size: u32,
    /// Use `Light` components; off = only the fallback sun (editor's Solid view).
    pub scene_lights: bool,
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
            shadow_distance: 60.0,
            shadow_map_size: 2048,
            scene_lights: true,
        }
    }
}

#[derive(Default)]
pub struct RenderPlugin {
    pub settings: RenderSettings,
}

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(EcsPlugin)
            .add_plugin(olvin_transform::TransformPlugin)
            .insert_resource(self.settings.clone())
            .init_resource::<Meshes>()
            .init_resource::<Textures>()
            .init_resource::<Materials>()
            .init_resource::<Shaders>()
            .init_resource::<RenderStats>()
            .add_system(Stage::Startup, renderer::init)
            .add_system(Stage::Render, renderer::render);
    }
}

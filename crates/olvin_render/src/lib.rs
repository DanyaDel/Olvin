//! Renderer on wgpu: Vulkan / Metal / DX12 / GLES 3 / WebGL2 / WebGPU.
//! Reads the ECS world and a [`SurfaceTarget`](olvin_core::SurfaceTarget);
//! knows nothing about the windowing library.

mod frustum;
mod mesh;
mod renderer;

pub use frustum::Frustum;
pub use mesh::{MeshData, MeshHandle, Meshes, Vertex};
pub use renderer::{RenderInfo, RenderStats, RenderTier, Renderer};

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
    pub color: Color,
}

#[derive(Clone, Debug)]
pub struct RenderSettings {
    /// Fifo when on: no tearing, and the GPU idles instead of burning battery.
    pub vsync: bool,
    /// Prefer the integrated GPU.
    pub low_power: bool,
    pub clear_color: Color,
    /// Direction the light travels.
    pub light_dir: Vec3,
    /// Linear RGB × intensity of the sun.
    pub light_color: Color,
    pub ambient: Color,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            vsync: true,
            low_power: false,
            clear_color: Color::rgb(0.02, 0.02, 0.03),
            light_dir: Vec3::new(-0.4, -1.0, -0.3),
            light_color: Color::WHITE,
            ambient: Color::rgb(0.15, 0.15, 0.18),
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
            .insert_resource(self.settings.clone())
            .init_resource::<Meshes>()
            .init_resource::<RenderStats>()
            .add_system(Stage::Startup, renderer::init)
            .add_system(Stage::Render, renderer::render);
    }
}

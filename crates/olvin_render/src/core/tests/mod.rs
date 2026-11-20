//! Offscreen rendering tests: a [`Harness`] draws a world into a small
//! target and reads the pixels back.

mod blend;
mod lighting;
mod materials;
mod post;

use super::*;
use crate::{
    BlendMode, Color, Light, Material, Materials, MeshData, MeshRenderer, Meshes, RenderSettings,
    RenderStats, RenderTier, ShaderHandle, Shaders, ShadowFilter, SkyMode, TextureData,
    TextureFormat, TextureHandle, Textures, Tonemapping, renderer::begin_pass,
};

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

fn close(c: [u8; 3], want: [u8; 3]) -> bool {
    c.iter().zip(want).all(|(&a, b)| a.abs_diff(b) <= 6)
}

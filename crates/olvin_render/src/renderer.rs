use std::error::Error;
use std::sync::Arc;

use olvin_core::{AppExit, Resources, SurfaceTarget, WindowHandleSource};
use olvin_ecs::World;

use crate::core::{GpuMeshes, RenderCore, ViewData, camera_view_proj};
use crate::{Meshes, RenderSettings};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

/// Feature level picked from the adapter. `Compat` = GLES 3 / WebGL2 class
/// hardware; `Full` = Vulkan / Metal / DX12 class. Future GPU-driven paths
/// (compute culling, indirect draws) are only enabled on `Full`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderTier {
    Compat,
    Full,
}

#[derive(Clone, Debug)]
pub struct RenderInfo {
    pub adapter: String,
    pub backend: String,
    pub tier: RenderTier,
}

/// Per-frame counters, overwritten every frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub draw_calls: u32,
    pub instances: u32,
    pub culled: u32,
}

/// The window renderer: a [`RenderCore`] plus the surface it presents to.
pub struct Renderer {
    pub core: RenderCore,
    gpu: GpuMeshes,
    view: ViewData,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::TextureView,
}

/// `Stage::Startup`: creates the GPU device once a surface exists.
pub(crate) fn init(res: &mut Resources) {
    let Some((handle, size)) = res.get::<SurfaceTarget>().map(|t| (t.handle.clone(), t.size)) else {
        log::warn!("no SurfaceTarget: renderer disabled");
        return;
    };
    let settings = res.fetch::<RenderSettings>().clone();
    match pollster::block_on(Renderer::new(handle, size, &settings)) {
        Ok((renderer, info)) => {
            log::info!("renderer: {} via {}, tier {:?}", info.adapter, info.backend, info.tier);
            res.insert(renderer);
            res.insert(info);
        }
        Err(err) => {
            log::error!("renderer init failed: {err}");
            res.fetch_mut::<AppExit>().requested = true;
        }
    }
}

/// `Stage::Render`: uploads new meshes, culls, draws.
pub(crate) fn render(res: &mut Resources) {
    let Some(mut renderer) = res.get_mut::<Renderer>() else { return };
    let renderer = &mut *renderer;
    renderer.core.upload(&mut renderer.gpu, &mut res.fetch_mut::<Meshes>());
    let size = res.fetch::<SurfaceTarget>().size;
    let stats = renderer.frame(&res.fetch::<World>(), size, &res.fetch::<RenderSettings>());
    *res.fetch_mut::<crate::RenderStats>() = stats;
}

/// The adapter's feature level.
pub fn tier_of(adapter: &wgpu::Adapter) -> RenderTier {
    if adapter.get_downlevel_capabilities().is_webgpu_compliant() && adapter.get_info().backend != wgpu::Backend::Gl {
        RenderTier::Full
    } else {
        RenderTier::Compat
    }
}

impl Renderer {
    async fn new(
        handle: Arc<dyn WindowHandleSource>,
        size: (u32, u32),
        settings: &RenderSettings,
    ) -> Result<(Self, RenderInfo), Box<dyn Error>> {
        // GLES needs the display connection up front (Wayland in particular).
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(handle.clone())));
        let surface = instance.create_surface(handle)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: if settings.low_power {
                    wgpu::PowerPreference::LowPower
                } else {
                    wgpu::PowerPreference::HighPerformance
                },
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await?;
        let adapter_info = adapter.get_info();
        let info = RenderInfo {
            adapter: adapter_info.name.clone(),
            backend: format!("{:?}", adapter_info.backend),
            tier: tier_of(&adapter),
        };

        // Ask only for WebGL2-level limits: whatever runs here runs everywhere.
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("olvin"),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await?;

        let (width, height) = (size.0.max(1), size.1.max(1));
        let mut config =
            surface.get_default_config(&adapter, width, height).ok_or("surface is not supported by the adapter")?;
        let caps = surface.get_capabilities(&adapter);
        if let Some(srgb) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = srgb;
        }
        config.present_mode = if settings.vsync {
            wgpu::PresentMode::Fifo
        } else {
            [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
                .into_iter()
                .find(|m| caps.present_modes.contains(m))
                .unwrap_or(wgpu::PresentMode::Fifo)
        };
        config.desired_maximum_frame_latency = 2;
        surface.configure(&device, &config);

        let depth = create_depth(&device, width, height);
        let core = RenderCore::new(device, queue, config.format, 1, DEPTH_FORMAT);
        let view = core.create_view();
        Ok((Self { core, gpu: GpuMeshes::default(), view, surface, config, depth }, info))
    }

    fn resize(&mut self, (width, height): (u32, u32)) {
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.core.device, &self.config);
        self.depth = create_depth(&self.core.device, width, height);
    }

    fn frame(&mut self, world: &World, size: (u32, u32), settings: &RenderSettings) -> crate::RenderStats {
        if size.0 == 0 || size.1 == 0 {
            return Default::default(); // minimized
        }
        if size != (self.config.width, self.config.height) {
            self.resize(size);
        }
        let stats = match camera_view_proj(world, size.0 as f32 / size.1 as f32) {
            Some(view_proj) => self.core.prepare(&mut self.view, &self.gpu, world, view_proj, settings),
            None => Default::default(),
        };

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.core.device, &self.config);
                return Default::default();
            }
            other => {
                log::debug!("skipping frame: {other:?}");
                return Default::default();
            }
        };
        let target = frame.texture.create_view(&Default::default());
        let mut encoder = self.core.device.create_command_encoder(&Default::default());
        if stats.draw_calls > 0 {
            self.core.render_shadows(&self.view, &self.gpu, &mut encoder);
        }
        {
            let mut pass = begin_pass(&mut encoder, &target, &self.depth, settings);
            if stats.draw_calls > 0 {
                self.core.draw(&self.view, &self.gpu, &mut pass);
            }
        }
        self.core.queue.submit([encoder.finish()]);
        self.core.queue.present(frame);
        stats
    }
}

/// A pass that clears colour (to the settings' clear colour) and depth.
pub fn begin_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    settings: &RenderSettings,
) -> wgpu::RenderPass<'a> {
    let [r, g, b, a] = settings.clear_color.0.map(f64::from);
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("main"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: color,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
            stencil_ops: None,
        }),
        ..Default::default()
    })
}

pub fn create_depth(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

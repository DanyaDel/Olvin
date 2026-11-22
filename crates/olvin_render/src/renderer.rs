use std::error::Error;
use std::sync::Arc;

use olvin_core::{AppExit, Resources, SurfaceTarget, WindowHandleSource};
use olvin_ecs::World;

use crate::core::{GpuMeshes, RenderCore, ViewData, camera_view_proj};
use crate::overlay::{Overlay, OverlayRenderer, begin_overlay_pass};
use crate::particles::ParticleRenderer;
use crate::post::{HDR_FORMAT, PostProcess};
use crate::sprite::SpriteRenderer;
use crate::{Materials, Meshes, RenderSettings, Shaders, Textures};

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
    /// Triangles drawn by the instanced mesh draws (main + blended).
    pub triangles: u64,
}

/// The window renderer: a [`RenderCore`] plus the surface it presents to.
pub struct Renderer {
    pub core: RenderCore,
    gpu: GpuMeshes,
    view: ViewData,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::TextureView,
    /// Full tier: the scene goes into an HDR target and through this chain.
    post: Option<PostProcess>,
    /// Particles and 2D sprites, drawn after the meshes in the same pass.
    particles: ParticleRenderer,
    sprites: SpriteRenderer,
    /// Screen overlay (game UI), drawn on the final image after post.
    overlay: OverlayRenderer,
}

/// `Stage::Startup`: creates the GPU device once a surface exists.
pub(crate) fn init(res: &mut Resources) {
    let Some((handle, size)) = res
        .get::<SurfaceTarget>()
        .map(|t| (t.handle.clone(), t.size))
    else {
        log::warn!("no SurfaceTarget: renderer disabled");
        return;
    };
    let settings = res.fetch::<RenderSettings>().clone();
    let wayland = crate::gl_check::is_wayland(&handle);
    match pollster::block_on(Renderer::new(handle, size, &settings)) {
        Ok((renderer, info)) => {
            log::info!(
                "renderer: {} via {}, tier {:?}",
                info.adapter,
                info.backend,
                info.tier
            );
            res.insert(renderer);
            res.insert(info);
        }
        Err(err) => {
            log::error!("renderer init failed: {err}");
            let backend = std::env::var("WGPU_BACKEND").ok();
            if let Some(hint) = crate::gl_check::failure_hint(wayland, backend.as_deref()) {
                log::error!("{hint}");
            }
            res.fetch_mut::<AppExit>().requested = true;
        }
    }
}

/// `Stage::Render`: uploads new meshes / textures / materials / shaders, culls, draws.
pub(crate) fn render(res: &mut Resources) {
    let Some(mut renderer) = res.get_mut::<Renderer>() else {
        return;
    };
    let renderer = &mut *renderer;
    renderer
        .core
        .upload(&mut renderer.gpu, &mut res.fetch_mut::<Meshes>());
    if let (Some(mut textures), Some(mut materials), Some(mut shaders)) = (
        res.get_mut::<Textures>(),
        res.get_mut::<Materials>(),
        res.get_mut::<Shaders>(),
    ) {
        renderer.core.upload_assets(
            &mut renderer.gpu,
            &mut textures,
            &mut materials,
            &mut shaders,
        );
    }
    if let Some(time) = res.get::<olvin_core::Time>() {
        renderer.core.set_time(time.elapsed() as f32);
    }
    let size = res.fetch::<SurfaceTarget>().size;
    let empty = Overlay::default();
    let overlay = res.get::<Overlay>();
    let stats = renderer.frame(
        &res.fetch::<World>(),
        size,
        &res.fetch::<RenderSettings>(),
        overlay.as_deref().unwrap_or(&empty),
    );
    drop(overlay);
    *res.fetch_mut::<crate::RenderStats>() = stats;
}

/// The adapter's feature level.
pub fn tier_of(adapter: &wgpu::Adapter) -> RenderTier {
    if adapter.get_downlevel_capabilities().is_webgpu_compliant()
        && adapter.get_info().backend != wgpu::Backend::Gl
    {
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
        let power = if settings.low_power {
            wgpu::PowerPreference::LowPower
        } else {
            wgpu::PowerPreference::HighPerformance
        };
        // GLES needs the display connection up front (Wayland in particular).
        let open = async |backends: Option<wgpu::Backends>| {
            let mut desc = wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(
                handle.clone(),
            ));
            if let Some(b) = backends {
                desc.backends = b;
            }
            let instance = wgpu::Instance::new(desc);
            let surface = instance.create_surface(handle.clone())?;
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: power,
                    force_fallback_adapter: false,
                    compatible_surface: Some(&surface),
                    ..Default::default()
                })
                .await?;
            Ok::<_, Box<dyn Error>>((surface, adapter))
        };
        let (mut surface, mut adapter) = open(None).await?;
        let first = adapter.get_info();
        let verdict = crate::gl_check::check_gl(&crate::GlCheckInput {
            backend: first.backend,
            vendor: first.vendor,
            name: &first.name,
            driver: &first.driver,
            driver_info: &first.driver_info,
            wayland: crate::gl_check::is_wayland(&handle),
            backend_forced: crate::gl_check::backend_forced(),
        });
        match verdict {
            crate::GlCheck::Fine => {}
            crate::GlCheck::Warn => log::warn!("{}", crate::gl_check::warning(&first.name)),
            crate::GlCheck::UseVulkan => {
                log::warn!(
                    "OpenGL on the proprietary NVIDIA driver under Wayland does not present; \
                     switching to Vulkan"
                );
                match open(Some(wgpu::Backends::VULKAN)).await {
                    Ok((s, a)) => (surface, adapter) = (s, a),
                    Err(e) => {
                        log::warn!(
                            "Vulkan is not available ({e}). {}",
                            crate::gl_check::warning(&first.name)
                        );
                    }
                }
            }
        }
        let adapter_info = adapter.get_info();
        let info = RenderInfo {
            adapter: adapter_info.name.clone(),
            backend: format!("{:?}", adapter_info.backend),
            tier: tier_of(&adapter),
        };

        // WebGL2-level limits (whatever runs here runs everywhere), plus compute
        // and storage buffers on native adapters for the GPU particles.
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("olvin"),
                required_limits: crate::full_tier_limits(&adapter),
                ..Default::default()
            })
            .await?;

        let (width, height) = (size.0.max(1), size.1.max(1));
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or("surface is not supported by the adapter")?;
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
        let post =
            (info.tier == RenderTier::Full).then(|| PostProcess::new(&device, config.format));
        let scene_format = if post.is_some() {
            HDR_FORMAT
        } else {
            config.format
        };
        let mut core = RenderCore::new(device, queue, scene_format, 1, DEPTH_FORMAT);
        core.set_tier(info.tier);
        core.set_hdr_output(post.is_some());
        let mut particles =
            ParticleRenderer::new(&core.device, &core.queue, scene_format, 1, DEPTH_FORMAT);
        let mut sprites =
            SpriteRenderer::new(&core.device, &core.queue, scene_format, 1, DEPTH_FORMAT);
        particles.set_hdr_output(post.is_some());
        sprites.set_hdr_output(post.is_some());
        let overlay = OverlayRenderer::new(&core.device, &core.queue, config.format);
        let view = core.create_view();
        Ok((
            Self {
                core,
                gpu: GpuMeshes::default(),
                view,
                surface,
                config,
                depth,
                post,
                particles,
                sprites,
                overlay,
            },
            info,
        ))
    }

    fn resize(&mut self, (width, height): (u32, u32)) {
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.core.device, &self.config);
        self.depth = create_depth(&self.core.device, width, height);
    }

    fn frame(
        &mut self,
        world: &World,
        size: (u32, u32),
        settings: &RenderSettings,
        overlay: &Overlay,
    ) -> crate::RenderStats {
        if size.0 == 0 || size.1 == 0 {
            return Default::default(); // minimized
        }
        if size != (self.config.width, self.config.height) {
            self.resize(size);
        }
        olvin_core::profile_scope!("render frame");
        let view_proj = camera_view_proj(world, size.0 as f32 / size.1 as f32);
        let stats = {
            olvin_core::profile_scope!("render prepare meshes");
            match view_proj {
                Some(view_proj) => {
                    self.core
                        .prepare(&mut self.view, &self.gpu, world, view_proj, settings)
                }
                None => Default::default(),
            }
        };

        let overlay_draws = {
            olvin_core::profile_scope!("render prepare particles, sprites, overlay");
            match view_proj {
                Some(vp) => {
                    self.particles.prepare(
                        &self.core.device,
                        &self.core.queue,
                        &self.gpu,
                        world,
                        vp,
                    );
                }
                None => self.particles.clear(),
            }
            self.sprites
                .prepare(&self.core.device, &self.core.queue, &self.gpu, world, size);
            self.overlay.prepare(
                &self.core.device,
                &self.core.queue,
                &self.gpu,
                overlay,
                size,
            )
        };
        let acquired = {
            olvin_core::profile_scope!("render acquire surface");
            self.surface.get_current_texture()
        };
        let frame = match acquired {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
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
            olvin_core::profile_scope!("render shadow pass");
            self.core
                .render_shadows(&self.view, &self.gpu, &mut encoder);
        }
        olvin_core::profile_scope!("render scene passes");
        if let Some(post) = &mut self.post {
            post.resize(&self.core.device, size);
            post.sync_lut(&self.core.device, &self.gpu, settings);
            let hdr = post.hdr_view().expect("resized above");
            let depth = post.depth_view().expect("resized above");
            let mut pass = begin_scene_pass(
                &mut encoder,
                hdr,
                depth,
                settings,
                settings.post.ao.is_some(),
            );
            if view_proj.is_some() {
                self.core.draw(&self.view, &self.gpu, &mut pass);
            }
            self.particles.draw(&mut pass);
            self.sprites.draw(&mut pass);
            drop(pass);
            let vp = view_proj.unwrap_or(glam::Mat4::IDENTITY);
            post.run(&self.core.queue, &mut encoder, &target, settings, vp);
        } else {
            let mut pass = begin_pass(&mut encoder, &target, &self.depth, settings);
            // Even without meshes: the sky.
            if view_proj.is_some() {
                self.core.draw(&self.view, &self.gpu, &mut pass);
            }
            self.particles.draw(&mut pass);
            self.sprites.draw(&mut pass);
        }
        if overlay_draws > 0 {
            // Its own pass: after tonemapping, no depth, over the final image.
            let mut pass = begin_overlay_pass(&mut encoder, &target);
            self.overlay.draw(&mut pass);
        }
        {
            olvin_core::profile_scope!("render submit and present");
            self.core.queue.submit([encoder.finish()]);
            self.core.queue.present(frame);
        }
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
    begin_scene_pass(encoder, color, depth, settings, false)
}

/// [`begin_pass`] that can keep the depth for the post-processing passes
/// (`keep_depth`; they read it for ambient occlusion).
pub fn begin_scene_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    settings: &RenderSettings,
    keep_depth: bool,
) -> wgpu::RenderPass<'a> {
    let [r, g, b, a] = settings.clear_color.0.map(f64::from);
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("main"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: color,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: if keep_depth {
                    wgpu::StoreOp::Store
                } else {
                    wgpu::StoreOp::Discard
                },
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    })
}

pub fn create_depth(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

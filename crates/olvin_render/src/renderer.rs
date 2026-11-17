use std::error::Error;
use std::ops::Range;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use olvin_core::{AppExit, Resources, SurfaceTarget, WindowHandleSource};
use olvin_ecs::World;
use olvin_scene::{Camera, Transform};

use crate::{Frustum, MeshData, MeshRenderer, Meshes, RenderSettings, Vertex};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const MIN_INSTANCE_CAPACITY: usize = 1024;

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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 4],
    ambient: [f32; 4],
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

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    meshes: Vec<Option<GpuMesh>>,
    // Frame scratch, reused so the steady state allocates nothing.
    buckets: Vec<Vec<InstanceRaw>>,
    staging: Vec<InstanceRaw>,
    ranges: Vec<(usize, Range<u32>)>,
}

/// `Stage::Startup`: creates the GPU device once a surface exists.
pub(crate) fn init(res: &mut Resources) {
    let Some((handle, size)) = res.get::<SurfaceTarget>().map(|t| (t.handle.clone(), t.size))
    else {
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
    renderer.upload_pending(&mut res.fetch_mut::<Meshes>());
    let size = res.fetch::<SurfaceTarget>().size;
    let stats = renderer.frame(&res.fetch::<World>(), size, &res.fetch::<RenderSettings>());
    *res.fetch_mut::<RenderStats>() = stats;
}

impl Renderer {
    async fn new(
        handle: Arc<dyn WindowHandleSource>,
        size: (u32, u32),
        settings: &RenderSettings,
    ) -> Result<(Self, RenderInfo), Box<dyn Error>> {
        // GLES needs the display connection up front (Wayland in particular).
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(handle.clone())),
        );
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
        let tier = if adapter.get_downlevel_capabilities().is_webgpu_compliant()
            && adapter_info.backend != wgpu::Backend::Gl
        {
            RenderTier::Full
        } else {
            RenderTier::Compat
        };
        let info = RenderInfo {
            adapter: adapter_info.name.clone(),
            backend: format!("{:?}", adapter_info.backend),
            tier,
        };

        // Ask only for WebGL2-level limits: whatever runs here runs everywhere.
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("olvin"),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
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
        let (pipeline, globals, globals_bind) = create_pipeline(&device, config.format);
        let instances = create_instance_buffer(&device, MIN_INSTANCE_CAPACITY);

        let renderer = Self {
            device,
            queue,
            surface,
            config,
            depth,
            pipeline,
            globals,
            globals_bind,
            instances,
            instance_capacity: MIN_INSTANCE_CAPACITY,
            meshes: Vec::new(),
            buckets: Vec::new(),
            staging: Vec::new(),
            ranges: Vec::new(),
        };
        Ok((renderer, info))
    }

    fn upload_pending(&mut self, meshes: &mut Meshes) {
        use wgpu::util::DeviceExt;
        for (handle, data) in meshes.pending.drain(..) {
            let MeshData { vertices, indices } = &data;
            let (center, radius) = data.bounding_sphere();
            let gpu = GpuMesh {
                vertices: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh vertices"),
                    contents: bytemuck::cast_slice::<Vertex, u8>(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                indices: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh indices"),
                    contents: bytemuck::cast_slice::<u32, u8>(indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                index_count: indices.len() as u32,
                center,
                radius,
            };
            let i = handle.0 as usize;
            if self.meshes.len() <= i {
                self.meshes.resize_with(i + 1, || None);
                self.buckets.resize_with(i + 1, Vec::new);
            }
            self.meshes[i] = Some(gpu);
        }
    }

    fn resize(&mut self, (width, height): (u32, u32)) {
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth = create_depth(&self.device, width, height);
    }

    /// The hot path: one linear pass over (Transform, MeshRenderer), bucketed
    /// by mesh into flat arrays, one instanced draw per mesh.
    fn prepare_instances(&mut self, world: &World, view_proj: Mat4) -> u32 {
        let frustum = Frustum::from_view_proj(view_proj);
        self.buckets.iter_mut().for_each(Vec::clear);
        let mut culled = 0;

        for (transform, renderer) in world.query::<(&Transform, &MeshRenderer)>().iter() {
            let i = renderer.mesh.0 as usize;
            let Some(Some(mesh)) = self.meshes.get(i) else { continue };
            let model = transform.matrix();
            let center = model.transform_point3(mesh.center);
            let radius = mesh.radius * transform.scale.abs().max_element();
            if !frustum.intersects_sphere(center, radius) {
                culled += 1;
                continue;
            }
            self.buckets[i].push(InstanceRaw {
                model: model.to_cols_array_2d(),
                color: renderer.color.0,
            });
        }

        self.staging.clear();
        self.ranges.clear();
        for (i, bucket) in self.buckets.iter().enumerate() {
            if !bucket.is_empty() {
                let start = self.staging.len() as u32;
                self.staging.extend_from_slice(bucket);
                self.ranges.push((i, start..self.staging.len() as u32));
            }
        }

        if self.staging.len() > self.instance_capacity {
            self.instance_capacity = self.staging.len().next_power_of_two();
            self.instances = create_instance_buffer(&self.device, self.instance_capacity);
        }
        if !self.staging.is_empty() {
            self.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&self.staging));
        }
        culled
    }

    fn frame(&mut self, world: &World, size: (u32, u32), settings: &RenderSettings) -> RenderStats {
        if size.0 == 0 || size.1 == 0 {
            return RenderStats::default(); // minimized
        }
        if size != (self.config.width, self.config.height) {
            self.resize(size);
        }

        let camera = world
            .query::<(&Transform, &Camera)>()
            .iter()
            .next()
            .map(|(t, c)| c.projection(size.0 as f32 / size.1 as f32) * Camera::view(t));
        let culled = match camera {
            Some(view_proj) => {
                let culled = self.prepare_instances(world, view_proj);
                let globals = Globals {
                    view_proj: view_proj.to_cols_array_2d(),
                    light_dir: settings.light_dir.normalize_or_zero().extend(0.0).into(),
                    ambient: settings.ambient.0,
                };
                self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
                culled
            }
            None => {
                self.ranges.clear();
                0
            }
        };

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return RenderStats::default();
            }
            other => {
                log::debug!("skipping frame: {other:?}");
                return RenderStats::default();
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let [r, g, b, a] = settings.clear_color.0.map(f64::from);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.globals_bind, &[]);
            pass.set_vertex_buffer(1, self.instances.slice(..));
            for (i, range) in &self.ranges {
                let mesh = self.meshes[*i].as_ref().unwrap();
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, range.clone());
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);

        RenderStats {
            draw_calls: self.ranges.len() as u32,
            instances: self.staging.len() as u32,
            culled,
        }
    }
}

fn create_depth(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
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

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: (capacity * size_of::<InstanceRaw>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::Buffer, wgpu::BindGroup) {
    let shader = device.create_shader_module(wgpu::include_wgsl!("mesh.wgsl"));

    let globals = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("globals"),
        size: size_of::<Globals>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("globals"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size_of::<Globals>() as u64),
            },
            count: None,
        }],
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("globals"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("mesh"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });

    let vertex_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
    let instance_attrs = wgpu::vertex_attr_array![
        2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4
    ];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mesh"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &vertex_attrs,
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<InstanceRaw>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &instance_attrs,
                }),
            ],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
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
    });
    (pipeline, globals, bind)
}

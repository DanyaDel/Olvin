//! The renderer without a window: pipelines, GPU meshes and per-camera data.
//! The window renderer, the editor's scene/game views and headless tests all
//! draw through this.
//!
//! Frame order for a view: `prepare` (CPU: culling, lights, shadow fit,
//! uniforms) → `render_shadows` (own depth pass into the view's shadow map)
//! → `draw` (into the caller's main pass, which samples the shadow map).

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use olvin_ecs::World;
use olvin_scene::{Camera, Transform};
use olvin_transform::GlobalTransform;

use crate::light::{CameraInfo, LightList, MAX_LIGHTS, PackedLight, fit_shadow};
use crate::{
    Frustum, Light, LightKind, MeshData, MeshRenderer, Meshes, RenderSettings, RenderStats,
    Tonemapping, Vertex,
};

const MIN_INSTANCE_CAPACITY: usize = 1024;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Uniforms of the main pass; mirrors `Globals` in mesh.wgsl (all vec4 /
/// mat4, so std140 and WGSL layouts agree).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    shadow_view_proj: [[f32; 4]; 4],
    camera: [f32; 4],
    view_plane: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky: [f32; 4],
    ground: [f32; 4],
    fog_color: [f32; 4],
    params: [f32; 4],
    shadow_params: [f32; 4],
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

/// GPU copies of one [`Meshes`] registry (handles index into it).
#[derive(Default)]
pub struct GpuMeshes {
    meshes: Vec<Option<GpuMesh>>,
}

/// Instances bucketed by mesh, packed into one vertex buffer.
struct InstanceSet {
    buffer: wgpu::Buffer,
    capacity: usize,
    buckets: Vec<Vec<InstanceRaw>>,
    ranges: Vec<(usize, Range<u32>)>,
    count: u32,
}

impl InstanceSet {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            buffer: instance_buffer(device, MIN_INSTANCE_CAPACITY),
            capacity: MIN_INSTANCE_CAPACITY,
            buckets: Vec::new(),
            ranges: Vec::new(),
            count: 0,
        }
    }

    fn begin(&mut self, meshes: usize) {
        if self.buckets.len() < meshes {
            self.buckets.resize_with(meshes, Vec::new);
        }
        self.buckets.iter_mut().for_each(Vec::clear);
    }

    /// Packs the buckets through `staging` (reused scratch) and uploads them.
    fn finish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        staging: &mut Vec<InstanceRaw>,
    ) {
        staging.clear();
        self.ranges.clear();
        for (i, bucket) in self.buckets.iter().enumerate() {
            if !bucket.is_empty() {
                let start = staging.len() as u32;
                staging.extend_from_slice(bucket);
                self.ranges.push((i, start..staging.len() as u32));
            }
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

    fn draw(&self, gpu: &GpuMeshes, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(1, self.buffer.slice(..));
        for (i, range) in &self.ranges {
            let Some(Some(mesh)) = gpu.meshes.get(*i) else {
                continue;
            };
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, range.clone());
        }
    }
}

/// Per-camera state: uniforms, instance buffers, the sun's shadow map and
/// frame scratch.
pub struct ViewData {
    globals: wgpu::Buffer,
    /// Globals + shadow map (the dummy until shadows are first used).
    bind: wgpu::BindGroup,
    main: InstanceSet,
    staging: Vec<InstanceRaw>,
    shadow_globals: wgpu::Buffer,
    shadow_bind: wgpu::BindGroup,
    shadow: InstanceSet,
    /// Attachment view and size; created on first use, kept afterwards.
    shadow_map: Option<(wgpu::TextureView, u32)>,
    shadow_active: bool,
}

impl ViewData {
    /// Whether the last `prepare` enabled the sun's shadows (so
    /// `render_shadows` has work to do).
    pub fn shadows_active(&self) -> bool {
        self.shadow_active
    }
}

pub struct RenderCore {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    shadow_pipeline: wgpu::RenderPipeline,
    shadow_layout: wgpu::BindGroupLayout,
    shadow_sampler: wgpu::Sampler,
    /// Bound while a view has no shadow map yet.
    dummy_shadow: wgpu::TextureView,
    encode_srgb: bool,
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
        let (pipeline, layout) = create_pipeline(&device, color, samples, depth);
        let (shadow_pipeline, shadow_layout) = create_shadow_pipeline(&device);
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let dummy_shadow = shadow_texture(&device, 1);
        Self {
            device,
            queue,
            pipeline,
            layout,
            shadow_pipeline,
            shadow_layout,
            shadow_sampler,
            dummy_shadow,
            encode_srgb: !color.is_srgb(),
        }
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
        let bind = self.main_bind_group(&globals, &self.dummy_shadow);
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
        }
    }

    fn main_bind_group(
        &self,
        globals: &wgpu::Buffer,
        shadow: &wgpu::TextureView,
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
            ],
        })
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
            let i = handle.0 as usize;
            if gpu.meshes.len() <= i {
                gpu.meshes.resize_with(i + 1, || None);
            }
            gpu.meshes[i] = Some(mesh);
        }
    }

    /// The hot path: gathers lights (the first directional [`Light`] is the
    /// sun, up to [`MAX_LIGHTS`] others ranked by distance), fits the sun's
    /// shadow map, then one linear pass over (GlobalTransform, MeshRenderer),
    /// culled against the camera (and the shadow box) and bucketed by mesh
    /// into flat arrays. Writes uniforms and instances for `view`.
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
        if fit.is_some() && view.shadow_map.as_ref().is_none_or(|(_, s)| *s != size) {
            let texture = shadow_texture(&self.device, size);
            view.bind = self.main_bind_group(&view.globals, &texture);
            view.shadow_map = Some((texture, size));
        }
        let shadow_frustum = fit.map(|f| Frustum::from_view_proj(f.view_proj));

        // Meshes.
        view.main.begin(gpu.meshes.len());
        view.shadow.begin(gpu.meshes.len());
        let mut culled = 0;
        for (global, transform, renderer) in world
            .query::<(Option<&GlobalTransform>, &Transform, &MeshRenderer)>()
            .iter()
        {
            let i = renderer.mesh.0 as usize;
            let Some(Some(mesh)) = gpu.meshes.get(i) else {
                continue;
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
                view.shadow.buckets[i].push(instance);
            }
            if frustum.intersects_sphere(center, radius) {
                view.main.buckets[i].push(instance);
            } else {
                culled += 1;
            }
        }
        view.main
            .finish(&self.device, &self.queue, &mut view.staging);
        if view.shadow_active {
            view.shadow
                .finish(&self.device, &self.queue, &mut view.staging);
        }

        // Uniforms.
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let fog = settings.fog;
        let shadow_view_proj = fit.map_or(Mat4::IDENTITY, |f| f.view_proj);
        let texel_world = fit.map_or(0.0, |f| f.texel_world);
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            shadow_view_proj: shadow_view_proj.to_cols_array_2d(),
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
                0.0,
            ],
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
        view.shadow.draw(gpu, &mut pass);
    }

    /// Records the prepared draws into a pass compatible with this core.
    pub fn draw(&self, view: &ViewData, gpu: &GpuMeshes, pass: &mut wgpu::RenderPass<'_>) {
        if view.main.ranges.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &view.bind, &[]);
        view.main.draw(gpu, pass);
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

fn create_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    samples: u32,
    depth: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::include_wgsl!("mesh.wgsl"));
    let fragment = wgpu::ShaderStages::FRAGMENT;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("globals"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | fragment,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<Globals>() as u64),
                },
                count: None,
            },
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
        ],
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
    (pipeline, layout)
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
        wgpu::vertex_attr_array![2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4];
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
    use crate::{Color, renderer::begin_pass};

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
            MeshRenderer {
                mesh: cube,
                color: Color::rgb(1.0, 0.0, 0.0),
            },
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
            world.spawn((
                Transform::IDENTITY,
                MeshRenderer {
                    mesh: plane,
                    color: Color::WHITE,
                },
            ));
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
            world.spawn((
                Transform::IDENTITY,
                MeshRenderer {
                    mesh: plane,
                    color: Color::WHITE,
                },
            ));
            world.spawn((
                Transform::from_xyz(0.0, 1.5, 0.0),
                MeshRenderer {
                    mesh: cube,
                    color: Color::WHITE,
                },
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
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer {
                mesh: plane,
                color: Color::WHITE,
            },
        ));
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
}

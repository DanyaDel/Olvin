//! The renderer without a window: pipelines, GPU meshes and per-camera data.
//! The window renderer, the editor's scene/game views and headless tests all
//! draw through this.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use olvin_ecs::World;
use olvin_scene::{Camera, Transform};
use olvin_transform::GlobalTransform;

use crate::{Frustum, MeshData, MeshRenderer, Meshes, RenderSettings, RenderStats, Vertex};

const MIN_INSTANCE_CAPACITY: usize = 1024;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    /// xyz: direction the light travels; w: 1 to encode sRGB in the shader.
    light_dir: [f32; 4],
    ambient: [f32; 4],
    light_color: [f32; 4],
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

/// Per-camera state: uniforms, the instance buffer and frame scratch.
pub struct ViewData {
    globals: wgpu::Buffer,
    bind: wgpu::BindGroup,
    instances: wgpu::Buffer,
    capacity: usize,
    buckets: Vec<Vec<InstanceRaw>>,
    staging: Vec<InstanceRaw>,
    ranges: Vec<(usize, Range<u32>)>,
}

pub struct RenderCore {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
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
        Self { device, queue, pipeline, layout, encode_srgb: !color.is_srgb() }
    }

    pub fn create_view(&self) -> ViewData {
        let globals = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("olvin globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("olvin globals"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        ViewData {
            globals,
            bind,
            instances: instance_buffer(&self.device, MIN_INSTANCE_CAPACITY),
            capacity: MIN_INSTANCE_CAPACITY,
            buckets: Vec::new(),
            staging: Vec::new(),
            ranges: Vec::new(),
        }
    }

    /// Uploads meshes added to `meshes` since the last call.
    pub fn upload(&self, gpu: &mut GpuMeshes, meshes: &mut Meshes) {
        use wgpu::util::DeviceExt;
        for (handle, data) in meshes.pending.drain(..) {
            let MeshData { vertices, indices } = &data;
            let (center, radius) = data.bounding_sphere();
            let mesh = GpuMesh {
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
            if gpu.meshes.len() <= i {
                gpu.meshes.resize_with(i + 1, || None);
            }
            gpu.meshes[i] = Some(mesh);
        }
    }

    /// The hot path: one linear pass over (GlobalTransform, MeshRenderer),
    /// frustum-culled and bucketed by mesh into flat arrays; writes uniforms
    /// and instances for `view`. Call before `draw`.
    pub fn prepare(
        &self,
        view: &mut ViewData,
        gpu: &GpuMeshes,
        world: &World,
        view_proj: Mat4,
        settings: &RenderSettings,
    ) -> RenderStats {
        let frustum = Frustum::from_view_proj(view_proj);
        if view.buckets.len() < gpu.meshes.len() {
            view.buckets.resize_with(gpu.meshes.len(), Vec::new);
        }
        view.buckets.iter_mut().for_each(Vec::clear);
        let mut culled = 0;

        for (global, transform, renderer) in world.query::<(Option<&GlobalTransform>, &Transform, &MeshRenderer)>().iter() {
            let i = renderer.mesh.0 as usize;
            let Some(Some(mesh)) = gpu.meshes.get(i) else { continue };
            let model = global.map_or_else(|| transform.matrix(), |g| g.0);
            let scale = model.x_axis.truncate().length().max(model.y_axis.truncate().length()).max(model.z_axis.truncate().length());
            if !frustum.intersects_sphere(model.transform_point3(mesh.center), mesh.radius * scale) {
                culled += 1;
                continue;
            }
            view.buckets[i].push(InstanceRaw { model: model.to_cols_array_2d(), color: renderer.color.0 });
        }

        view.staging.clear();
        view.ranges.clear();
        for (i, bucket) in view.buckets.iter().enumerate() {
            if !bucket.is_empty() {
                let start = view.staging.len() as u32;
                view.staging.extend_from_slice(bucket);
                view.ranges.push((i, start..view.staging.len() as u32));
            }
        }
        if view.staging.len() > view.capacity {
            view.capacity = view.staging.len().next_power_of_two();
            view.instances = instance_buffer(&self.device, view.capacity);
        }
        if !view.staging.is_empty() {
            self.queue.write_buffer(&view.instances, 0, bytemuck::cast_slice(&view.staging));
        }
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            light_dir: settings.light_dir.normalize_or_zero().extend(if self.encode_srgb { 1.0 } else { 0.0 }).into(),
            ambient: settings.ambient.0,
            light_color: settings.light_color.0,
        };
        self.queue.write_buffer(&view.globals, 0, bytemuck::bytes_of(&globals));

        RenderStats { draw_calls: view.ranges.len() as u32, instances: view.staging.len() as u32, culled }
    }

    /// Records the prepared draws into a pass compatible with this core.
    pub fn draw(&self, view: &ViewData, gpu: &GpuMeshes, pass: &mut wgpu::RenderPass<'_>) {
        if view.ranges.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &view.bind, &[]);
        pass.set_vertex_buffer(1, view.instances.slice(..));
        for (i, range) in &view.ranges {
            let Some(Some(mesh)) = gpu.meshes.get(*i) else { continue };
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, range.clone());
        }
    }
}

/// View-projection of the first camera in the world.
pub fn camera_view_proj(world: &World, aspect: f32) -> Option<Mat4> {
    world.query::<(Option<&GlobalTransform>, &Transform, &Camera)>().iter().next().map(|(g, t, c)| {
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

fn create_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    samples: u32,
    depth: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::include_wgsl!("mesh.wgsl"));
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
        primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
        }),
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
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    #[test]
    fn renders_a_lit_cube_offscreen() {
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let core = RenderCore::new(device.clone(), queue.clone(), FORMAT, 1, wgpu::TextureFormat::Depth24Plus);
        let mut gpu = GpuMeshes::default();
        let mut view = core.create_view();
        let mut meshes = Meshes::default();
        let cube = meshes.add(MeshData::cube(1.0));
        core.upload(&mut gpu, &mut meshes);

        let mut world = World::new();
        world.spawn((Transform::IDENTITY, MeshRenderer { mesh: cube, color: Color::rgb(1.0, 0.0, 0.0) }));
        world.spawn((Transform::from_xyz(0.0, 0.0, 3.0), Camera::default()));
        let settings = RenderSettings { light_dir: Vec3::new(0.0, 0.0, -1.0), clear_color: Color::rgb(0.0, 0.0, 1.0), ..Default::default() };
        let stats = core.prepare(&mut view, &gpu, &world, camera_view_proj(&world, 1.0).unwrap(), &settings);
        assert_eq!((stats.draw_calls, stats.instances), (1, 1));

        let texture = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        let depth = texture(wgpu::TextureFormat::Depth24Plus, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (SIZE * SIZE * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let (cv, dv) = (color.create_view(&Default::default()), depth.create_view(&Default::default()));
            let mut pass = begin_pass(&mut encoder, &cv, &dv, &settings);
            core.draw(&view, &gpu, &mut pass);
        }
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SIZE * 4), rows_per_image: None },
            },
            wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixels = readback.slice(..).get_mapped_range().unwrap().to_vec();
        let at = |x: u32, y: u32| {
            let i = ((y * SIZE + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2]]
        };
        let center = at(SIZE / 2, SIZE / 2);
        assert!(center[0] > 200 && center[1] < 30 && center[2] < 30, "lit red cube in the middle: {center:?}");
        let corner = at(1, 1);
        assert!(corner[2] > 200 && corner[0] < 30, "clear colour in the corner: {corner:?}");
    }
}

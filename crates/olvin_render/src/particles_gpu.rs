//! GPU particles (Full tier): the simulation of [`ParticleEmitter`]s in
//! `ParticleMode::Gpu`, as a compute pass over a storage buffer, drawn
//! straight from that buffer (no readback, no per-particle CPU work).
//!
//! The emitter schedules (see `GpuStep`: dt and number of births per frame);
//! [`GpuSim::prepare`] runs the steps it has not seen yet, one dispatch each.
//! Births go to the next slots of a ring buffer, so no free list is needed; a
//! zeroed slot is dead. The renderer owns one `GpuSim`; a renderer per view
//! simulates identical copies (the steps carry their own seeds).

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4Swizzles};
use olvin_ecs::Entity;

use crate::core::GpuMeshes;
use crate::particles::{
    EmitShape, EmitterParams, FlipBook, GpuStep, MAX_GPU_PARTICLES, ParticleEmitter, SimSpace,
};
use crate::quad::QuadBlend;

const WORKGROUP: u32 = 64;
/// Bytes between the steps in an emitter's simulation uniform (dynamic offsets).
const STEP_STRIDE: u64 = 256;
const MAX_STEPS: u64 = 32;
const LUT: usize = 32;
const WHITE: u32 = u32::MAX;

/// One slot of the particle buffer (64 bytes, mirrored by both shaders).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuParticle {
    pub pos: [f32; 3],
    pub age: f32,
    pub vel: [f32; 3],
    pub life: f32,
    pub rot: f32,
    pub spin: f32,
    pub size: f32,
    pub pad: f32,
    pub tint: [f32; 4],
}

impl GpuParticle {
    pub fn is_alive(&self) -> bool {
        self.age < self.life
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SimParams {
    matrix: [[f32; 4]; 4],
    gravity_keep: [f32; 4],
    step: [f32; 4],
    counts: [u32; 4],
    shape: [f32; 4],
    half: [f32; 4],
    life_speed: [f32; 4],
    size_rot: [f32; 4],
    spin: [f32; 4],
    color_a: [f32; 4],
    color_b: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DrawParams {
    view_proj: [[f32; 4]; 4],
    matrix: [[f32; 4]; 4],
    right: [f32; 4],
    up: [f32; 4],
    flags: [f32; 4],
    color_lut: [[f32; 4]; LUT],
    size_lut: [[f32; 4]; LUT / 4],
}

/// Whether `device` can run the simulation: compute shaders and storage
/// buffers in both the compute and the vertex stage (not on GL / WebGL, and
/// not on a device created with only the downlevel limits).
pub fn supported(device: &wgpu::Device) -> bool {
    let l = device.limits();
    device.adapter_info().backend != wgpu::Backend::Gl
        && l.max_storage_buffers_per_shader_stage >= 2
        && l.max_compute_invocations_per_workgroup >= WORKGROUP
        && l.max_compute_workgroup_size_x >= WORKGROUP
        && l.max_compute_workgroups_per_dimension >= MAX_GPU_PARTICLES / WORKGROUP
        && l.max_storage_buffer_binding_size >= u64::from(MAX_GPU_PARTICLES) * 64
}

/// Device limits for `adapter`: WebGL2-level (whatever runs here runs
/// everywhere), plus what the GPU particle simulation needs when the adapter
/// is a native one (Full tier). Compat adapters get the plain downlevel set.
pub fn device_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let have = adapter.limits();
    let mut limits = wgpu::Limits::downlevel_webgl2_defaults().using_resolution(have.clone());
    if adapter.get_info().backend != wgpu::Backend::Gl {
        let want = wgpu::Limits::default();
        limits.max_storage_buffers_per_shader_stage = want
            .max_storage_buffers_per_shader_stage
            .min(have.max_storage_buffers_per_shader_stage);
        limits.max_storage_buffer_binding_size = want
            .max_storage_buffer_binding_size
            .min(have.max_storage_buffer_binding_size);
        limits.max_compute_workgroup_storage_size = want
            .max_compute_workgroup_storage_size
            .min(have.max_compute_workgroup_storage_size);
        limits.max_compute_invocations_per_workgroup = want
            .max_compute_invocations_per_workgroup
            .min(have.max_compute_invocations_per_workgroup);
        limits.max_compute_workgroup_size_x = want
            .max_compute_workgroup_size_x
            .min(have.max_compute_workgroup_size_x);
        limits.max_compute_workgroup_size_y = want
            .max_compute_workgroup_size_y
            .min(have.max_compute_workgroup_size_y);
        limits.max_compute_workgroup_size_z = want
            .max_compute_workgroup_size_z
            .min(have.max_compute_workgroup_size_z);
        limits.max_compute_workgroups_per_dimension = want
            .max_compute_workgroups_per_dimension
            .min(have.max_compute_workgroups_per_dimension);
    }
    limits
}

struct GpuEmitter {
    max: u32,
    epoch: u32,
    /// Sequence number of the last step run.
    applied: u64,
    /// The next birth slot.
    head: u32,
    particles: wgpu::Buffer,
    sim: wgpu::Buffer,
    draw: wgpu::Buffer,
    sim_bind: wgpu::BindGroup,
    draw_bind: wgpu::BindGroup,
    blend: QuadBlend,
    texture: Option<(u32, u64)>,
}

/// The compute pipeline, the draw pipelines and the state of every GPU emitter
/// of one renderer.
pub(crate) struct GpuSim {
    compute: wgpu::ComputePipeline,
    sim_layout: wgpu::BindGroupLayout,
    draw_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    alpha: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    binds: HashMap<(u32, u64), wgpu::BindGroup>,
    emitters: HashMap<Entity, GpuEmitter>,
    /// Emitters to draw, in order, from the last `prepare`.
    drawn: Vec<Entity>,
    encode_srgb: bool,
}

impl GpuSim {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: wgpu::TextureFormat,
        samples: u32,
        depth: wgpu::TextureFormat,
    ) -> Self {
        let sim_module = device.create_shader_module(wgpu::include_wgsl!("particles_sim.wgsl"));
        let draw_module = device.create_shader_module(wgpu::include_wgsl!("particles_draw.wgsl"));
        let buffer_entry = |binding: u32,
                            visibility: wgpu::ShaderStages,
                            ty: wgpu::BufferBindingType,
                            dynamic: bool| {
            wgpu::BindGroupLayoutEntry {
                binding,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty,
                    has_dynamic_offset: dynamic,
                    min_binding_size: None,
                },
                count: None,
            }
        };
        let sim_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle sim"),
            entries: &[
                buffer_entry(
                    0,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Uniform,
                    true,
                ),
                buffer_entry(
                    1,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: false },
                    false,
                ),
            ],
        });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle draw"),
            entries: &[
                buffer_entry(
                    0,
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                    false,
                ),
                buffer_entry(
                    1,
                    wgpu::ShaderStages::VERTEX,
                    wgpu::BufferBindingType::Storage { read_only: true },
                    false,
                ),
            ],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let compute_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle sim"),
            bind_group_layouts: &[Some(&sim_layout)],
            immediate_size: 0,
        });
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particle sim"),
            layout: Some(&compute_layout),
            module: &sim_module,
            entry_point: Some("cs_sim"),
            compilation_options: Default::default(),
            cache: None,
        });
        let draw_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle draw"),
            bind_group_layouts: &[Some(&draw_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let pipeline = |blend: QuadBlend| {
            let target = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: match blend {
                        QuadBlend::Alpha => wgpu::BlendFactor::OneMinusSrcAlpha,
                        QuadBlend::Additive => wgpu::BlendFactor::One,
                    },
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("gpu particles"),
                layout: Some(&draw_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &draw_module,
                    entry_point: Some("vs_particle"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &draw_module,
                    entry_point: Some("fs_particle"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color,
                        blend: Some(target),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("particles"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            lod_max_clamp: 32.0,
            ..Default::default()
        });
        let white = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("particle white"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            white.as_image_copy(),
            &[255; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        Self {
            compute,
            sim_layout,
            draw_layout,
            texture_layout,
            alpha: pipeline(QuadBlend::Alpha),
            additive: pipeline(QuadBlend::Additive),
            sampler,
            white: white.create_view(&Default::default()),
            binds: HashMap::new(),
            emitters: HashMap::new(),
            drawn: Vec::new(),
            encode_srgb: !color.is_srgb(),
        }
    }

    pub fn set_hdr_output(&mut self, on: bool, color: wgpu::TextureFormat) {
        self.encode_srgb = !on && !color.is_srgb();
    }

    pub fn clear(&mut self) {
        self.drawn.clear();
    }

    /// Slots drawn by the last `prepare`.
    pub fn slots(&self) -> usize {
        self.drawn
            .iter()
            .filter_map(|e| self.emitters.get(e))
            .map(|g| g.max as usize)
            .sum()
    }

    pub fn draw_calls(&self) -> usize {
        self.drawn.len()
    }

    fn create(&self, device: &wgpu::Device, max: u32, epoch: u32) -> GpuEmitter {
        let particles = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu particles"),
            size: u64::from(max) * size_of::<GpuParticle>() as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            // Zeroed: every slot starts dead.
            mapped_at_creation: false,
        });
        let sim = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu particle steps"),
            size: STEP_STRIDE * MAX_STEPS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let draw = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu particle draw"),
            size: size_of::<DrawParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sim_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu particle sim"),
            layout: &self.sim_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &sim,
                        offset: 0,
                        size: wgpu::BufferSize::new(size_of::<SimParams>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: particles.as_entire_binding(),
                },
            ],
        });
        let draw_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu particle draw"),
            layout: &self.draw_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: draw.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: particles.as_entire_binding(),
                },
            ],
        });
        GpuEmitter {
            max,
            epoch,
            applied: 0,
            head: 0,
            particles,
            sim,
            draw,
            sim_bind,
            draw_bind,
            blend: QuadBlend::Alpha,
            texture: None,
        }
    }

    /// Runs the steps the emitters queued since the last call and readies the
    /// draws for the camera. `emitters`: (entity, world matrix, emitter) of
    /// every GPU-mode emitter.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &GpuMeshes,
        emitters: impl Iterator<Item = (Entity, Mat4, &'a ParticleEmitter)>,
        view_proj: Mat4,
    ) {
        self.drawn.clear();
        let mut seen = Vec::new();
        // (emitter, step index): dispatches to record, in order.
        let mut dispatches: Vec<(Entity, u32)> = Vec::new();
        for (entity, matrix, emitter) in emitters {
            let params = &emitter.params;
            let max = params.max_particles.clamp(1, MAX_GPU_PARTICLES);
            let stale = self
                .emitters
                .get(&entity)
                .is_none_or(|g| g.max != max || g.epoch != emitter.epoch());
            if stale {
                let g = self.create(device, max, emitter.epoch());
                self.emitters.insert(entity, g);
            }
            seen.push(entity);
            let g = self.emitters.get_mut(&entity).expect("just inserted");
            let steps: Vec<GpuStep> = emitter.steps_after(g.applied).copied().collect();
            for (n, step) in steps.iter().enumerate().take(MAX_STEPS as usize) {
                let births = step.births.min(max);
                let sim = sim_params(params, step, g.head, births, max);
                queue.write_buffer(&g.sim, n as u64 * STEP_STRIDE, bytemuck::bytes_of(&sim));
                g.head = (g.head + births) % max;
                dispatches.push((entity, n as u32));
            }
            if let Some(last) = steps.last() {
                g.applied = last.seq;
            }

            // Texture bind group (cached by texture and its generation).
            let key = match emitter.texture {
                None => Some((WHITE, 0)),
                Some(h) => gpu.texture_view(h).map(|(_, generation)| (h.0, generation)),
            };
            g.blend = params.blend;
            g.texture = key;
            if let Some(key) = key
                && !self.binds.contains_key(&key)
            {
                let view = match emitter.texture.and_then(|h| gpu.texture_view(h)) {
                    Some((view, _)) => view,
                    None => &self.white,
                };
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("particle texture"),
                    layout: &self.texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                self.binds.insert(key, bind);
            }
            let draw = draw_params(params, matrix, view_proj, self.encode_srgb);
            queue.write_buffer(&g.draw, 0, bytemuck::bytes_of(&draw));
            if key.is_some() {
                self.drawn.push(entity);
            }
        }
        self.emitters.retain(|e, _| seen.contains(e));

        if dispatches.is_empty() {
            return;
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu particles"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gpu particle sim"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compute);
            for (entity, n) in dispatches {
                let g = &self.emitters[&entity];
                pass.set_bind_group(0, &g.sim_bind, &[n * STEP_STRIDE as u32]);
                pass.dispatch_workgroups(g.max.div_ceil(WORKGROUP), 1, 1);
            }
        }
        queue.submit([encoder.finish()]);
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let mut blend = None;
        for entity in &self.drawn {
            let Some(g) = self.emitters.get(entity) else {
                continue;
            };
            let Some(tex) = g.texture.and_then(|k| self.binds.get(&k)) else {
                continue;
            };
            if blend != Some(g.blend) {
                blend = Some(g.blend);
                pass.set_pipeline(match g.blend {
                    QuadBlend::Alpha => &self.alpha,
                    QuadBlend::Additive => &self.additive,
                });
            }
            pass.set_bind_group(0, &g.draw_bind, &[]);
            pass.set_bind_group(1, tex, &[]);
            pass.draw(0..6, 0..g.max);
        }
    }

    /// The slots of `entity`'s emitter, read back (tests, tools: stalls the GPU).
    pub fn read(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        entity: Entity,
    ) -> Option<Vec<GpuParticle>> {
        let g = self.emitters.get(&entity)?;
        let size = g.particles.size();
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu particles readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&g.particles, 0, &staging, 0, size);
        queue.submit([encoder.finish()]);
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        let out = bytemuck::cast_slice(&staging.slice(..).get_mapped_range().ok()?).to_vec();
        Some(out)
    }
}

fn sim_params(
    params: &EmitterParams,
    step: &GpuStep,
    head: u32,
    births: u32,
    max: u32,
) -> SimParams {
    let (kind, radius, cos, flag, half) = match params.shape {
        EmitShape::Point => (0.0, 0.0, 1.0, 0.0, Vec3::ZERO),
        EmitShape::Sphere { radius, surface } => (1.0, radius, 1.0, f32::from(surface), Vec3::ZERO),
        EmitShape::Hemisphere { radius, surface } => {
            (2.0, radius, 1.0, f32::from(surface), Vec3::ZERO)
        }
        EmitShape::Cone { angle_deg, radius } => {
            (3.0, radius, angle_deg.to_radians().cos(), 0.0, Vec3::ZERO)
        }
        EmitShape::Box { half_extents } => (4.0, 0.0, 1.0, 0.0, half_extents),
        EmitShape::Circle { radius, edge } => (5.0, radius, 1.0, f32::from(edge), Vec3::ZERO),
    };
    let local = f32::from(params.space == SimSpace::Local);
    let a = params.color;
    let b = params.color_max.unwrap_or(a);
    SimParams {
        matrix: step.matrix.to_cols_array_2d(),
        gravity_keep: [
            params.gravity.x,
            params.gravity.y,
            params.gravity.z,
            (1.0 - params.drag * step.dt).max(0.0),
        ],
        step: [step.dt, 0.0, 0.0, 0.0],
        counts: [
            head % max,
            births,
            params.seed ^ (step.seq as u32).wrapping_mul(0x9E37_79B1),
            max,
        ],
        shape: [kind, radius, cos, flag],
        half: [half.x, half.y, half.z, local],
        life_speed: [
            params.lifetime[0],
            params.lifetime[1],
            params.speed[0],
            params.speed[1],
        ],
        size_rot: [
            params.size[0],
            params.size[1],
            params.rotation[0],
            params.rotation[1],
        ],
        spin: [params.spin[0], params.spin[1], 0.0, 0.0],
        color_a: a,
        color_b: b,
    }
}

fn draw_params(
    params: &EmitterParams,
    matrix: Mat4,
    view_proj: Mat4,
    encode_srgb: bool,
) -> DrawParams {
    // Rows of view-proj are the camera's axes scaled by the projection.
    let right = view_proj.row(0).xyz().normalize_or_zero();
    let up = view_proj.row(1).xyz().normalize_or_zero();
    let mut color_lut = [[0.0; 4]; LUT];
    let mut sizes = [0.0f32; LUT];
    for i in 0..LUT {
        let t = i as f32 / (LUT - 1) as f32;
        color_lut[i] = params.life_color(t);
        sizes[i] = params.size_at(t);
    }
    let size_lut = std::array::from_fn(|i| {
        [
            sizes[i * 4],
            sizes[i * 4 + 1],
            sizes[i * 4 + 2],
            sizes[i * 4 + 3],
        ]
    });
    let (cols, rows) = match params.flip_book {
        Some(FlipBook { columns, rows }) if columns * rows > 0 => (columns as f32, rows as f32),
        _ => (0.0, 0.0),
    };
    DrawParams {
        view_proj: view_proj.to_cols_array_2d(),
        matrix: matrix.to_cols_array_2d(),
        right: right.extend(0.0).to_array(),
        up: up.extend(0.0).to_array(),
        flags: [
            f32::from(encode_srgb),
            f32::from(params.space == SimSpace::Local),
            cols,
            rows,
        ],
        color_lut,
        size_lut,
    }
}

/// GPU tests need a native (Vulkan / Metal / DX12) adapter with compute; on
/// llvmpipe GL (the CI default) they print why and pass. Run them with the
/// llvmpipe variables unset to exercise the simulation.
#[cfg(test)]
mod tests {
    use glam::Mat4;
    use olvin_ecs::World;
    use olvin_transform::GlobalTransform;

    use super::*;
    use crate::particles::{
        Burst, EmitterParams, ParticleEmitter, ParticleMode, ParticleRenderer, update_emitters,
    };

    const SIZE: u32 = 64;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
    const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let info = adapter.get_info();
        if info.backend == wgpu::Backend::Gl {
            eprintln!(
                "GPU particles: skipped on {} (GL has no compute; unset the llvmpipe env to run)",
                info.name
            );
            return None;
        }
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: device_limits(&adapter),
            ..Default::default()
        }))
        .ok()
    }

    fn renderer(device: &wgpu::Device, queue: &wgpu::Queue) -> ParticleRenderer {
        ParticleRenderer::new(device, queue, FORMAT, 1, DEPTH)
    }

    /// Looking down -Z at 8 x 8 world units.
    fn view_proj() -> Mat4 {
        glam::camera::rh::proj::directx::orthographic(-4.0, 4.0, -4.0, 4.0, -10.0, 10.0)
    }

    fn gpu_emitter(params: EmitterParams) -> ParticleEmitter {
        let mut e = ParticleEmitter::new(params);
        e.force_mode(ParticleMode::Gpu);
        e
    }

    #[test]
    fn the_downlevel_device_cannot_compute_and_the_full_one_can() {
        let Some((device, queue)) = device() else {
            return;
        };
        assert!(supported(&device));
        assert!(renderer(&device, &queue).gpu_enabled());
    }

    #[test]
    fn a_compat_device_has_no_gpu_simulation_and_draws_on_the_cpu() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits:
                    wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            }))
        else {
            return;
        };
        // The WebGL2 limits have no storage buffers or compute: whatever the backend.
        assert!(!supported(&device));
        assert!(!renderer(&device, &queue).gpu_enabled());
        let px = draw_one(&device, &queue, ParticleMode::Cpu);
        assert!(px[(SIZE * SIZE / 2 + SIZE / 2) as usize][0] > 200);
    }

    #[test]
    fn tens_of_thousands_of_particles_run_on_the_gpu() {
        let Some((device, queue)) = device() else {
            return;
        };
        let params = EmitterParams {
            rate: 30_000.0,
            lifetime: [1.0, 1.0],
            max_particles: 60_000,
            ..Default::default()
        };
        let mut world = World::new();
        let e = world.spawn((GlobalTransform(Mat4::IDENTITY), gpu_emitter(params)));
        let mut r = renderer(&device, &queue);
        let textures = GpuMeshes::default();
        for _ in 0..90 {
            update_emitters(&mut world, 1.0 / 60.0);
            r.prepare(&device, &queue, &textures, &world, view_proj());
        }
        let alive = r
            .read_gpu(&device, &queue, e)
            .unwrap()
            .iter()
            .filter(|p| p.is_alive())
            .count();
        assert!((29_000..=31_000).contains(&alive), "{alive}");
    }

    #[test]
    fn cpu_and_gpu_agree_on_counts_and_lifetimes() {
        let Some((device, queue)) = device() else {
            return;
        };
        let params = EmitterParams {
            rate: 600.0,
            lifetime: [0.5, 1.0],
            max_particles: 4000,
            ..Default::default()
        };
        let mut world = World::new();
        world.spawn((
            GlobalTransform(Mat4::IDENTITY),
            ParticleEmitter::new(params.clone()),
        ));
        let gpu = world.spawn((GlobalTransform(Mat4::IDENTITY), gpu_emitter(params)));
        let mut r = renderer(&device, &queue);
        let textures = GpuMeshes::default();
        for _ in 0..180 {
            update_emitters(&mut world, 1.0 / 60.0);
            r.prepare(&device, &queue, &textures, &world, view_proj());
        }
        let cpu_lives = world
            .query::<&ParticleEmitter>()
            .iter()
            .find(|e| e.mode() == ParticleMode::Cpu)
            .unwrap()
            .live_lives();
        let slots = r.read_gpu(&device, &queue, gpu).unwrap();
        let alive: Vec<_> = slots.iter().filter(|p| p.is_alive()).collect();
        // Steady state: rate x mean lifetime = 450 alive.
        let (c, g) = (cpu_lives.len() as f32, alive.len() as f32);
        assert!((c - 450.0).abs() < 45.0, "cpu {c}");
        assert!((g - 450.0).abs() < 45.0, "gpu {g}");
        assert!((c - g).abs() / c < 0.12, "cpu {c} vs gpu {g}");
        // Lifetimes: uniform in [0.5, 1], length-biased among the living.
        assert!(
            alive
                .iter()
                .all(|p| (0.5..=1.0).contains(&p.life) && p.age >= 0.0)
        );
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        let gpu_life = mean(&alive.iter().map(|p| p.life).collect::<Vec<_>>());
        let cpu_life = mean(&cpu_lives);
        assert!(
            (gpu_life - cpu_life).abs() < 0.04,
            "{gpu_life} vs {cpu_life}"
        );
        assert!((gpu_life - 0.778).abs() < 0.04, "{gpu_life}");
    }

    #[test]
    fn bursts_are_exact_and_particles_die() {
        let Some((device, queue)) = device() else {
            return;
        };
        let params = EmitterParams {
            rate: 0.0,
            looping: false,
            bursts: vec![Burst {
                time: 0.0,
                count: 300,
            }],
            duration: 0.1,
            lifetime: [0.2, 0.2],
            max_particles: 1000,
            ..Default::default()
        };
        let mut world = World::new();
        let e = world.spawn((GlobalTransform(Mat4::IDENTITY), gpu_emitter(params)));
        let mut r = renderer(&device, &queue);
        let textures = GpuMeshes::default();
        update_emitters(&mut world, 0.016);
        r.prepare(&device, &queue, &textures, &world, view_proj());
        let alive = |r: &ParticleRenderer| {
            r.read_gpu(&device, &queue, e)
                .unwrap()
                .iter()
                .filter(|p| p.is_alive())
                .count()
        };
        assert_eq!(alive(&r), 300);
        assert!(!world.get::<&ParticleEmitter>(e).unwrap().is_done());
        for _ in 0..30 {
            update_emitters(&mut world, 0.016);
            r.prepare(&device, &queue, &textures, &world, view_proj());
        }
        assert_eq!(alive(&r), 0);
        assert!(world.get::<&ParticleEmitter>(e).unwrap().is_done());
    }

    /// A drawn pixel row: one big particle, GPU and CPU alike.
    fn draw_one(device: &wgpu::Device, queue: &wgpu::Queue, mode: ParticleMode) -> Vec<[u8; 3]> {
        let params = EmitterParams {
            rate: 0.0,
            looping: false,
            bursts: vec![Burst {
                time: 0.0,
                count: 1,
            }],
            shape: crate::EmitShape::Point,
            speed: [0.0, 0.0],
            gravity: Vec3::ZERO,
            lifetime: [10.0, 10.0],
            size: [4.0, 4.0],
            color_over_life: Vec::new(),
            color: [1.0, 0.5, 0.0, 1.0],
            ..Default::default()
        };
        let mut e = ParticleEmitter::new(params);
        e.force_mode(mode);
        let mut world = World::new();
        world.spawn((GlobalTransform(Mat4::IDENTITY), e));
        let mut r = renderer(device, queue);
        update_emitters(&mut world, 0.016);
        r.prepare(device, queue, &GpuMeshes::default(), &world, view_proj());
        let extent = wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        };
        let make = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = make(
            FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = make(DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let (cv, dv) = (
                color.create_view(&Default::default()),
                depth.create_view(&Default::default()),
            );
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &cv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &dv,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            r.draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: None,
                },
            },
            extent,
        );
        queue.submit([encoder.finish()]);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = readback.slice(..).get_mapped_range().unwrap().to_vec();
        data.chunks(4).map(|p| [p[0], p[1], p[2]]).collect()
    }

    #[test]
    fn a_gpu_particle_draws_like_a_cpu_one() {
        let Some((device, queue)) = device() else {
            return;
        };
        let at = |px: &[[u8; 3]], x: u32, y: u32| px[(y * SIZE + x) as usize];
        let gpu = draw_one(&device, &queue, ParticleMode::Gpu);
        let cpu = draw_one(&device, &queue, ParticleMode::Cpu);
        // A 4 x 4 square in the middle of 8 x 8 units: half the image.
        let c = SIZE / 2;
        assert_eq!(at(&gpu, c, c), at(&cpu, c, c));
        let mid = at(&gpu, c, c);
        assert!(
            mid[0] > 200 && mid[1] > 100 && mid[1] < 200 && mid[2] < 10,
            "{mid:?}"
        );
        assert_eq!(at(&gpu, 1, 1), [0, 0, 0]);
        assert_eq!(at(&cpu, 1, 1), [0, 0, 0]);
        let lit = |px: &[[u8; 3]]| px.iter().filter(|p| p[0] > 100).count();
        assert_eq!(lit(&gpu), lit(&cpu));
        assert!(lit(&gpu) > (SIZE * SIZE) as usize / 5);
    }
}

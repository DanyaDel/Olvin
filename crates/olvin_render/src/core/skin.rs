//! GPU skinning (Full tier): a compute pass poses each skinned mesh's bind
//! vertices with the animator's joint matrices into the mesh's own vertex
//! buffer, so every later pass (main, blend, shadows, custom surface
//! shaders) draws the posed mesh without knowing about skinning. Compat
//! devices (no usable compute / storage buffers) pose on the CPU instead,
//! see `RenderCore::upload`.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};

use crate::skinning::{SkinSource, SkinUpdate};
use crate::{RenderTier, Vertex};

use super::assets::put;
use super::{GpuMesh, GpuMeshes, RenderCore};

const WORKGROUP: u32 = 64;
/// Storage buffers the skinning shader binds.
const STORAGE_BUFFERS: u32 = 4;

pub(super) struct SkinPipeline {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkinParams {
    vertex_count: u32,
    joint_count: u32,
    pad: [u32; 2],
}

/// One skinned mesh on the GPU: its inputs, the joint palette, and the bind
/// group that ties them to the mesh slot's vertex buffer.
pub(super) struct GpuSkin {
    pub(super) source: Arc<SkinSource>,
    palette: wgpu::Buffer,
    joints: usize,
    bind: wgpu::BindGroup,
    groups: u32,
}

impl RenderCore {
    /// Whether skinned meshes are posed by a compute pass: the Full tier on a
    /// device with compute and storage buffers. Otherwise (Compat, or limits
    /// below WebGPU's defaults) they are posed on the CPU.
    pub fn gpu_skinning(&self) -> bool {
        let l = self.device.limits();
        self.tier == RenderTier::Full
            && l.max_storage_buffers_per_shader_stage >= STORAGE_BUFFERS
            && l.max_compute_workgroup_size_x >= WORKGROUP
            && l.max_compute_invocations_per_workgroup >= WORKGROUP
            && l.max_compute_workgroups_per_dimension > 0
    }

    fn skin_pipeline(&self) -> &SkinPipeline {
        self.skin_pipeline.get_or_init(|| {
            let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            };
            let layout = self
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("olvin skinning"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: wgpu::BufferSize::new(
                                    size_of::<SkinParams>() as u64
                                ),
                            },
                            count: None,
                        },
                        storage(1, true),
                        storage(2, true),
                        storage(3, true),
                        storage(4, false),
                    ],
                });
            let module = self
                .device
                .create_shader_module(wgpu::include_wgsl!("../skin.wgsl"));
            let pipeline_layout =
                self.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("olvin skinning"),
                        bind_group_layouts: &[Some(&layout)],
                        immediate_size: 0,
                    });
            let pipeline = self
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("olvin skinning"),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some("cs_skin"),
                    compilation_options: Default::default(),
                    cache: None,
                });
            SkinPipeline { pipeline, layout }
        })
    }

    /// Whether `source` fits the GPU path's limits (else it is posed on the CPU).
    fn skin_fits(&self, source: &SkinSource) -> bool {
        let l = self.device.limits();
        let n = source.bind.vertices.len() as u64;
        n > 0
            && !source.bind.indices.is_empty()
            && n.div_ceil(WORKGROUP as u64) <= l.max_compute_workgroups_per_dimension as u64
            && n * size_of::<Vertex>() as u64 <= l.max_storage_buffer_binding_size
            && n * size_of::<Vertex>() as u64 <= l.max_buffer_size
    }

    /// Builds the GPU side of a skinned mesh and swaps the mesh slot's
    /// vertex buffer for the skinning output (vertex + storage usage).
    fn create_gpu_skin(
        &self,
        gpu: &mut GpuMeshes,
        handle: crate::MeshHandle,
        source: &Arc<SkinSource>,
        joints: usize,
    ) {
        use wgpu::util::DeviceExt;
        let buf = |label, contents: &[u8], usage| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                })
        };
        let storage = wgpu::BufferUsages::STORAGE;
        let bind_pose = buf(
            "skin bind pose",
            bytemuck::cast_slice::<Vertex, u8>(&source.bind.vertices),
            storage,
        );
        let influences: Vec<[u32; 4]> = source
            .joints
            .iter()
            .zip(&source.weights)
            .flat_map(|(j, w)| [j.map(u32::from), w.map(f32::to_bits)])
            .collect();
        let influences = buf(
            "skin influences",
            bytemuck::cast_slice(&influences),
            storage,
        );
        let palette = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skin joint matrices"),
            size: (joints.max(1) * size_of::<glam::Mat4>()) as u64,
            usage: storage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vertex_count = source.bind.vertices.len() as u32;
        let params = buf(
            "skin params",
            bytemuck::bytes_of(&SkinParams {
                vertex_count,
                joint_count: joints as u32,
                pad: [0; 2],
            }),
            wgpu::BufferUsages::UNIFORM,
        );
        let vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skinned mesh vertices"),
            size: u64::from(vertex_count) * size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_SRC | storage,
            mapped_at_creation: false,
        });
        let pipeline = self.skin_pipeline();
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("olvin skinning"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: bind_pose.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: influences.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: palette.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: vertices.as_entire_binding(),
                },
            ],
        });
        let indices = buf(
            "mesh indices",
            bytemuck::cast_slice::<u32, u8>(&source.bind.indices),
            wgpu::BufferUsages::INDEX,
        );
        let (center, radius) = source.bind.bounding_sphere();
        put(
            &mut gpu.meshes,
            handle.0,
            GpuMesh {
                vertices,
                indices,
                index_count: source.bind.indices.len() as u32,
                center,
                radius,
            },
        );
        gpu.skins.insert(
            handle.0,
            GpuSkin {
                source: source.clone(),
                palette,
                joints,
                bind,
                groups: vertex_count.div_ceil(WORKGROUP),
            },
        );
    }

    /// Poses `updates` in one compute pass (submitted right away, so it runs
    /// before any render pass recorded later). Meshes that do not fit the
    /// GPU path are posed on the CPU.
    pub(super) fn skin_on_gpu(&self, gpu: &mut GpuMeshes, updates: Vec<SkinUpdate>) {
        if updates.is_empty() {
            return;
        }
        let mut dispatch = Vec::with_capacity(updates.len());
        for u in updates {
            if !self.skin_fits(&u.source) {
                self.upload_mesh(gpu, u.handle, &u.source.skin(&u.matrices));
                continue;
            }
            let joints = u.matrices.len();
            let fresh = gpu
                .skins
                .get(&u.handle.0)
                .is_none_or(|s| !Arc::ptr_eq(&s.source, &u.source) || s.joints != joints);
            if fresh {
                self.create_gpu_skin(gpu, u.handle, &u.source, joints);
            }
            let Some(skin) = gpu.skins.get(&u.handle.0) else {
                continue;
            };
            if joints > 0 {
                self.queue
                    .write_buffer(&skin.palette, 0, bytemuck::cast_slice(&u.matrices));
            }
            // The culling sphere follows the pose (conservative, per joint).
            if let Some(Some(mesh)) = gpu.meshes.get_mut(u.handle.0 as usize) {
                (mesh.center, mesh.radius) = u.source.posed_bounds(&u.matrices);
            }
            dispatch.push(u.handle.0);
        }
        if dispatch.is_empty() {
            return;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("olvin skinning"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("olvin skinning"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.skin_pipeline().pipeline);
            for slot in dispatch {
                if let Some(skin) = gpu.skins.get(&slot) {
                    pass.set_bind_group(0, &skin.bind, &[]);
                    pass.dispatch_workgroups(skin.groups, 1, 1);
                }
            }
        }
        self.queue.submit([encoder.finish()]);
    }
}

//! The sun's cascaded shadow map: per-cascade cameras and the depth pass.

use glam::Mat4;

use super::instancing::InstanceSet;
use super::{GpuMeshes, RenderCore, ViewData};

/// One cascade of the sun's shadow map: its camera and its casters.
pub(super) struct Cascade {
    /// The cascade's tile-local view-projection.
    pub(super) globals: wgpu::Buffer,
    bind: wgpu::BindGroup,
    pub(super) casters: InstanceSet,
}

impl RenderCore {
    pub(super) fn create_cascade(&self) -> Cascade {
        let globals = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("olvin shadow globals"),
            size: size_of::<Mat4>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("olvin shadow globals"),
            layout: &self.shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        Cascade {
            globals,
            bind,
            casters: InstanceSet::new(&self.device),
        }
    }

    /// Records the sun's shadow-map pass (depth only, one viewport per
    /// cascade) for `view`. Call after `prepare` and before the pass that
    /// `draw`s this view; does nothing when shadows are off.
    pub fn render_shadows(
        &self,
        view: &ViewData,
        gpu: &GpuMeshes,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if view.cascade_count == 0 {
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
        pass.set_pipeline(&self.shadow_pipeline);
        let tile = view.shadow_tile as f32;
        for (c, cascade) in view.cascades.iter().take(view.cascade_count).enumerate() {
            if cascade.casters.ranges.is_empty() {
                continue;
            }
            // Cascade c's tile: column c % 2, row c / 2 (the whole map for one).
            let (col, row) = if view.cascade_count > 1 {
                ((c % 2) as f32, (c / 2) as f32)
            } else {
                (0.0, 0.0)
            };
            pass.set_viewport(col * tile, row * tile, tile, tile, 0.0, 1.0);
            pass.set_bind_group(0, &cascade.bind, &[]);
            pass.set_vertex_buffer(1, cascade.casters.buffer.slice(..));
            for range in &cascade.casters.ranges {
                if let Some(Some(mesh)) = gpu.meshes.get(range.mesh as usize) {
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, range.instances.clone());
                }
            }
        }
    }
}

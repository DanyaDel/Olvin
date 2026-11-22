//! The sun's cascaded shadow map: per-cascade cameras and the depth pass.

use glam::{Mat4, Vec3};
use olvin_ecs::{Entity, World};
use olvin_scene::Transform;
use olvin_transform::GlobalTransform;

use super::instancing::InstanceSet;
use super::{GpuMeshes, RenderCore, ViewData};
use crate::light::{
    Candidate, LocalFit, MAX_LOCAL_TILES, assign_tiles, atlas_matrix, fit_point, fit_spot,
    importance,
};
use crate::{Frustum, Light, LightKind, RenderSettings, RenderTier};

/// One cascade of the sun's shadow map: its camera and its casters.
pub(super) struct Cascade {
    /// The cascade's tile-local view-projection.
    pub(super) globals: wgpu::Buffer,
    bind: wgpu::BindGroup,
    pub(super) casters: InstanceSet,
}

/// The point / spot lights that get shadow tiles this frame.
pub(super) struct LocalPlan {
    /// Side of the atlas and of one tile, in texels.
    pub size: u32,
    pub tile: u32,
    /// Tiles in use (they are handed out from 0).
    pub count: usize,
    /// (light, first tile, texel size per metre, point light).
    pub lights: Vec<(Entity, usize, f32, bool)>,
    /// Per tile: the tile-local view-projection (the depth pass) and the
    /// one into the atlas (sampling).
    pub view_proj: [Mat4; MAX_LOCAL_TILES],
    pub atlas: [Mat4; MAX_LOCAL_TILES],
    pub frusta: [Option<Frustum>; MAX_LOCAL_TILES],
}

impl LocalPlan {
    /// The tile data of `light`: (first tile, texel per metre, point).
    pub fn of(&self, light: Entity) -> Option<(usize, f32, bool)> {
        self.lights
            .iter()
            .find(|l| l.0 == light)
            .map(|&(_, first, k, point)| (first, k, point))
    }
}

impl RenderCore {
    /// Picks the spot / point lights that cast shadows (the most important
    /// within the settings' caps, fewer on Compat) and fits their cameras.
    pub(super) fn plan_local_shadows(
        &self,
        world: &World,
        settings: &RenderSettings,
        eye: Vec3,
        frustum: &Frustum,
    ) -> LocalPlan {
        let max = self.device.limits().max_texture_dimension_2d.max(16);
        let size = (settings.local_shadow_size.clamp(64, max) / 4) * 4;
        let tile = size / 4;
        let mut plan = LocalPlan {
            size,
            tile,
            count: 0,
            lights: Vec::new(),
            view_proj: [Mat4::IDENTITY; MAX_LOCAL_TILES],
            atlas: [Mat4::IDENTITY; MAX_LOCAL_TILES],
            frusta: std::array::from_fn(|_| None),
        };
        let (spots, points) = (
            settings.max_spot_shadows as usize,
            settings.max_point_shadows as usize,
        );
        let (spots, points) = match self.tier {
            RenderTier::Compat => (spots.min(2), points.min(1)),
            RenderTier::Full => (spots, points),
        };
        if !settings.scene_lights || spots + points == 0 {
            return plan;
        }
        let mut found = Vec::new();
        let mut cands = Vec::new();
        let mut query =
            world.query::<(Entity, &Light, Option<&GlobalTransform>, Option<&Transform>)>();
        for (entity, light, global, transform) in query.iter() {
            if light.kind == LightKind::Directional
                || !light.shadows
                || light.intensity <= 0.0
                || light.range <= 0.0
            {
                continue;
            }
            let m = global
                .map(|g| g.0)
                .or_else(|| transform.map(Transform::matrix))
                .unwrap_or(Mat4::IDENTITY);
            let pos = m.w_axis.truncate();
            if !frustum.intersects_sphere(pos, light.range) {
                continue;
            }
            let dir = m.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Y);
            cands.push(Candidate {
                point: light.kind == LightKind::Point,
                importance: importance(light, pos, eye),
            });
            found.push((entity, *light, pos, dir));
        }
        for (i, first) in assign_tiles(&cands, spots, points) {
            let (entity, light, pos, dir) = found[i];
            let fits: Vec<LocalFit> = if light.kind == LightKind::Point {
                fit_point(pos, light.range, tile).to_vec()
            } else {
                fit_spot(pos, dir, light.spot_angle_deg, light.range, tile)
                    .into_iter()
                    .collect()
            };
            // A degenerate light (no usable camera) simply stays unshadowed.
            let Some(k) = fits.first().map(|f| f.texel_per_metre) else {
                continue;
            };
            for (n, fit) in fits.iter().enumerate() {
                plan.view_proj[first + n] = fit.view_proj;
                plan.atlas[first + n] = atlas_matrix(fit.view_proj, first + n);
                plan.frusta[first + n] = Some(Frustum::from_view_proj(fit.view_proj));
            }
            plan.count = plan.count.max(first + fits.len());
            plan.lights
                .push((entity, first, k, light.kind == LightKind::Point));
        }
        plan
    }

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
        self.render_local_shadows(view, gpu, encoder);
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

    /// The spot / point light atlas: one viewport per tile, same depth pass.
    fn render_local_shadows(
        &self,
        view: &ViewData,
        gpu: &GpuMeshes,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if view.local_count == 0 {
            return;
        }
        let Some((map, _)) = &view.local_map else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("olvin local shadows"),
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
        let tile = view.local_tile as f32;
        for (t, local) in view.local_tiles.iter().take(view.local_count).enumerate() {
            if local.casters.ranges.is_empty() {
                continue;
            }
            let (col, row) = ((t % 4) as f32, (t / 4) as f32);
            pass.set_viewport(col * tile, row * tile, tile, tile, 0.0, 1.0);
            pass.set_bind_group(0, &local.bind, &[]);
            pass.set_vertex_buffer(1, local.casters.buffer.slice(..));
            for range in &local.casters.ranges {
                if let Some(Some(mesh)) = gpu.meshes.get(range.mesh as usize) {
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, range.instances.clone());
                }
            }
        }
    }
}

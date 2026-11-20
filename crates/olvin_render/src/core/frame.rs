//! Per-frame work: `prepare` (lights, shadow fit, culling, batching,
//! uniforms) and `draw` (opaque meshes, sky, blended meshes).

use glam::{Mat4, Vec3};
use olvin_ecs::World;
use olvin_scene::Transform;
use olvin_transform::GlobalTransform;

use crate::light::{CameraInfo, LightList, MAX_CASCADES, fit_cascades};
use crate::pipelines::shadow_texture;
use crate::{
    Frustum, Light, LightKind, MeshRenderer, RenderSettings, RenderStats, RenderTier, ShadowFilter,
    SkyMode, Tonemapping,
};

use super::instancing::{BlendItem, DrawRange, bucket_key};
use super::{Globals, GpuMeshes, InstanceRaw, NO_MATERIAL, RenderCore, ViewData};

impl RenderCore {
    /// The hot path: gathers lights (the first directional [`Light`] is the
    /// sun, up to [`MAX_LIGHTS`] others ranked by distance), fits the sun's
    /// shadow cascades, then one linear pass over (GlobalTransform,
    /// MeshRenderer), culled against the camera (and the cascade boxes) and
    /// bucketed by (material, mesh) into flat arrays, drawn sorted by
    /// (pipeline, material, mesh). Alpha-blended materials go to a separate
    /// list sorted back to front by view depth. Writes uniforms and
    /// instances for `view`. Call before `render_shadows` and `draw`.
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

        // Shadow cascades (the map is created lazily, an atlas of tiles for
        // more than one cascade).
        let max_size = self.device.limits().max_texture_dimension_2d;
        let size = settings.shadow_map_size.clamp(16, max_size.max(16));
        let max_cascades = match self.tier {
            RenderTier::Compat => 2,
            RenderTier::Full => MAX_CASCADES,
        };
        let wanted = (settings.shadow_cascades as usize).clamp(1, max_cascades);
        let tile = if wanted > 1 { size / 2 } else { size };
        let fit = if sun_shadows && sun_color != Vec3::ZERO && settings.shadow_distance > 0.0 {
            fit_cascades(&cam, sun_dir, settings.shadow_distance, wanted, tile)
        } else {
            None
        };
        let count = fit.map_or(0, |f| f.count);
        view.cascade_count = count;
        view.shadow_tile = tile;
        let mut rebind = false;
        if count > 0 && view.shadow_map.as_ref().is_none_or(|(_, s)| *s != size) {
            view.shadow_map = Some((shadow_texture(&self.device, size), size));
            rebind = true;
        }
        while view.cascades.len() < count {
            view.cascades.push(self.create_cascade());
        }
        let cascade_frusta: [Option<Frustum>; MAX_CASCADES] = std::array::from_fn(|c| {
            fit.filter(|f| c < f.count)
                .map(|f| Frustum::from_view_proj(f.view_proj[c]))
        });

        // Sky (its texture lives in the view's bind group).
        let sky_texture = match settings.sky {
            SkyMode::Texture { texture, .. } => gpu
                .textures
                .get(texture.0 as usize)
                .and_then(Option::as_ref),
            _ => None,
        };
        let sky_generation = sky_texture.map(|t| t.generation);
        if rebind || sky_generation != view.sky_bound {
            let shadow = view
                .shadow_map
                .as_ref()
                .map_or(&self.dummy_shadow, |(v, _)| v);
            let sky = sky_texture.map_or(&self.defaults.base, |t| &t.view);
            view.bind = self.main_bind_group(&view.globals, shadow, sky);
            view.sky_bound = sky_generation;
        }

        // Meshes.
        view.main.begin();
        view.blend.items.clear();
        for cascade in &mut view.cascades {
            cascade.casters.begin();
        }
        let mut culled = 0;
        for (global, transform, renderer) in world
            .query::<(Option<&GlobalTransform>, &Transform, &MeshRenderer)>()
            .iter()
        {
            let i = renderer.mesh.0;
            let Some(Some(mesh)) = gpu.meshes.get(i as usize) else {
                continue;
            };
            let material = match renderer.material {
                Some(m) if matches!(gpu.materials.get(m.0 as usize), Some(Some(_))) => m.0,
                _ => NO_MATERIAL,
            };
            let blended = gpu.is_blended(material);
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
            // Blended meshes don't cast shadows.
            if !blended {
                for (cascade, f) in view.cascades.iter_mut().zip(&cascade_frusta) {
                    if let Some(f) = f
                        && f.intersects_sphere(center, radius)
                    {
                        cascade.casters.push(bucket_key(NO_MATERIAL, i), instance);
                    }
                }
            }
            if !frustum.intersects_sphere(center, radius) {
                culled += 1;
            } else if blended {
                view.blend.items.push(BlendItem {
                    depth: cam.view_plane.truncate().dot(center) + cam.view_plane.w,
                    key: bucket_key(material, i),
                    instance,
                });
            } else {
                view.main.push(bucket_key(material, i), instance);
            }
        }
        view.main
            .finish(&self.device, &self.queue, &mut view.staging, |m| {
                gpu.pipeline_of(m)
            });
        view.blend
            .finish(&self.device, &self.queue, &mut view.staging, |m| {
                gpu.pipeline_of(m)
            });
        for cascade in view.cascades.iter_mut().take(count) {
            cascade
                .casters
                .finish(&self.device, &self.queue, &mut view.staging, |_| 0);
        }

        // Uniforms.
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let fog = settings.fog;
        // Sampling matrices land in the cascade's tile of the atlas.
        let mut shadow_view_proj = [Mat4::IDENTITY; MAX_CASCADES];
        let mut shadow_offsets = [0.0; 4];
        let mut shadow_splits = [f32::MAX; 4];
        if let Some(f) = fit {
            for c in 0..f.count {
                shadow_view_proj[c] = if f.count > 1 {
                    let (col, row) = ((c % 2) as f32, (c / 2) as f32);
                    Mat4::from_translation(Vec3::new(col - 0.5, 0.5 - row, 0.0))
                        * Mat4::from_scale(Vec3::new(0.5, 0.5, 1.0))
                        * f.view_proj[c]
                } else {
                    f.view_proj[c]
                };
                shadow_offsets[c] = f.texel_world[c] * 1.5;
                shadow_splits[c] = f.splits[c];
            }
        }
        let filter = match settings.shadow_filter {
            ShadowFilter::Hard => 0.0,
            ShadowFilter::Pcf => 1.0,
            ShadowFilter::Soft if self.tier == RenderTier::Full => 2.0,
            ShadowFilter::Soft => 1.0,
        };
        let time = self
            .time
            .unwrap_or_else(|| self.start.elapsed().as_secs_f32());
        let mut sky = SkyUniform::default();
        match settings.sky {
            SkyMode::Color => {}
            SkyMode::Procedural {
                zenith,
                horizon,
                ground,
                sun_size,
            } => {
                let r = sun_size.max(0.0).to_radians();
                let disc = r > 0.0 && sun_color != Vec3::ZERO;
                sky = SkyUniform {
                    zenith: Vec3::from(zenith).extend((r * 0.85).cos()).into(),
                    horizon: Vec3::from(horizon).extend((r * 1.15).cos()).into(),
                    ground: Vec3::from(ground).extend(0.0).into(),
                    params: [1.0, 1.0, 1.0, flag(disc)],
                };
            }
            SkyMode::Texture {
                rotation,
                intensity,
                ..
            } => {
                if let Some(t) = sky_texture {
                    sky.ground[3] = rotation;
                    sky.params = [2.0, intensity, t.mips as f32, 0.0];
                }
            }
        }
        view.sky_active = sky.params[0] > 0.5;
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            shadow_view_proj: shadow_view_proj.map(|m| m.to_cols_array_2d()),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            camera: cam.camera.into(),
            view_plane: cam.view_plane.into(),
            sun_dir: sun_dir.extend(flag(count > 0)).into(),
            sun_color: sun_color
                .extend(flag(self.encode_srgb && !self.hdr_output))
                .into(),
            sky: rgb(settings.ambient.0)
                .extend(if self.hdr_output {
                    1.0
                } else {
                    settings.exposure.max(0.0)
                })
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
                flag(self.hdr_output),
                settings.shadow_distance * 0.85,
                flag(settings.tonemapping == Tonemapping::Aces && !self.hdr_output),
                time,
            ],
            shadow_splits,
            shadow_offsets,
            shadow_cfg: [
                count as f32,
                filter,
                settings.shadow_filter_radius.max(0.0),
                0.0,
            ],
            sky_zenith: sky.zenith,
            sky_horizon: sky.horizon,
            sky_ground: sky.ground,
            sky_params: sky.params,
            lights: lights.lights,
        };
        self.queue
            .write_buffer(&view.globals, 0, bytemuck::bytes_of(&globals));
        if let Some(f) = fit {
            for (cascade, m) in view.cascades.iter().zip(&f.view_proj).take(f.count) {
                self.queue
                    .write_buffer(&cascade.globals, 0, bytemuck::bytes_of(m));
            }
        }

        RenderStats {
            draw_calls: (view.main.ranges.len() + view.blend.ranges.len()) as u32,
            instances: view.main.count + view.blend.count,
            culled,
        }
    }

    /// Records the prepared draws into a pass compatible with this core:
    /// opaque meshes sorted by pipeline / material / mesh, then the sky (if
    /// any) on the far plane, then the alpha-blended meshes back to front.
    /// Call it even when `prepare` reported no draw calls, so the sky still
    /// shows.
    pub fn draw(&self, view: &ViewData, gpu: &GpuMeshes, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &view.bind, &[]);
        self.draw_ranges(gpu, pass, &view.main.buffer, &view.main.ranges, false);
        if view.sky_active {
            pass.set_pipeline(&self.sky_pipeline);
            pass.draw(0..3, 0..1);
        }
        self.draw_ranges(gpu, pass, &view.blend.buffer, &view.blend.ranges, true);
    }

    fn draw_ranges(
        &self,
        gpu: &GpuMeshes,
        pass: &mut wgpu::RenderPass<'_>,
        instances: &wgpu::Buffer,
        ranges: &[DrawRange],
        blend: bool,
    ) {
        if ranges.is_empty() {
            return;
        }
        pass.set_vertex_buffer(1, instances.slice(..));
        let mut bound_pipeline = None;
        let mut bound_material = None;
        for range in ranges {
            let Some(Some(mesh)) = gpu.meshes.get(range.mesh as usize) else {
                continue;
            };
            if bound_pipeline != Some(range.pipeline) {
                let builtin = if blend {
                    &self.blend_pipeline
                } else {
                    &self.pipeline
                };
                let pipeline = match range.pipeline {
                    0 => builtin,
                    id => match gpu.pipelines.get(id as usize - 1) {
                        Some(Some(p)) if blend => p.blend.as_ref().unwrap_or(builtin),
                        Some(Some(p)) => &p.opaque,
                        _ => builtin,
                    },
                };
                pass.set_pipeline(pipeline);
                bound_pipeline = Some(range.pipeline);
            }
            if bound_material != Some(range.material) {
                let bind = gpu
                    .materials
                    .get(range.material as usize)
                    .and_then(Option::as_ref)
                    .map_or(&self.fallback_material, |m| &m.bind);
                pass.set_bind_group(1, bind, &[]);
                bound_material = Some(range.material);
            }
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, range.instances.clone());
        }
    }
}

#[derive(Default)]
struct SkyUniform {
    zenith: [f32; 4],
    horizon: [f32; 4],
    ground: [f32; 4],
    params: [f32; 4],
}

fn rgb(c: [f32; 4]) -> Vec3 {
    Vec3::new(c[0], c[1], c[2])
}

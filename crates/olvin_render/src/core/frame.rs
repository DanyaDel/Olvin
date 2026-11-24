//! Per-frame work: `prepare` (lights, shadow fit, culling, batching,
//! uniforms) and `draw` (opaque meshes, sky, blended meshes).

use glam::{Mat4, Vec3};
use olvin_ecs::{Entity, World};
use olvin_scene::Transform;
use olvin_transform::GlobalTransform;

use crate::light::{CameraInfo, DEFAULT_SHADOW_BIAS, LightList, MAX_CASCADES, fit_cascades};
use crate::pipelines::shadow_texture;
use crate::{
    Fog, Frustum, Light, LightKind, MeshRenderer, RenderSettings, RenderStats, RenderTier,
    RetroSettings, ShadowFilter, SkyMode, Tonemapping,
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
        let local = self.plan_local_shadows(world, settings, cam.eye, &frustum);
        let mut sun: Option<(Vec3, Vec3, bool, f32, f32)> = None;
        let mut lights = LightList::new();
        // Lights that did not make the best sixteen, for the clusters.
        let mut extra: Vec<(crate::light::PackedLight, f32)> = Vec::new();
        let mut query =
            world.query::<(Entity, &Light, Option<&GlobalTransform>, Option<&Transform>)>();
        for (entity, light, global, transform) in query.iter().filter(|_| settings.scene_lights) {
            let m = global
                .map(|g| g.0)
                .or_else(|| transform.map(Transform::matrix))
                .unwrap_or(Mat4::IDENTITY);
            let dir = m.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Y);
            let pos = m.w_axis.truncate();
            let score = match light.kind {
                LightKind::Directional if sun.is_none() => {
                    sun = Some((
                        dir,
                        light.radiance(),
                        light.shadows,
                        light.size,
                        light.shadow_bias,
                    ));
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
            let mut packed = light.pack(pos, dir);
            if let Some((first, texel, point)) = local.of(entity) {
                packed.shadow = [(first + 1) as f32, light.shadow_bias, texel, flag(point)];
            }
            if let Some(left_out) = lights.push(packed, score) {
                extra.push(left_out);
            }
        }
        extra.sort_by(|a, b| a.1.total_cmp(&b.1));
        let (sun_dir, sun_color, sun_shadows, sun_size, sun_bias) = sun.unwrap_or((
            settings.light_dir,
            Vec3::from_slice(&settings.light_color.0[..3]),
            settings.light_shadows,
            settings.shadow_light_size,
            DEFAULT_SHADOW_BIAS,
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
        view.local_count = local.count;
        view.local_tile = local.tile;
        if local.count > 0
            && view
                .local_map
                .as_ref()
                .is_none_or(|(_, s)| *s != local.size)
        {
            view.local_map = Some((shadow_texture(&self.device, local.size), local.size));
            rebind = true;
        }
        while view.local_tiles.len() < local.count {
            view.local_tiles.push(self.create_cascade());
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
            let local_map = view
                .local_map
                .as_ref()
                .map_or(&self.dummy_shadow, |(v, _)| v);
            let sky = sky_texture.map_or(&self.defaults.base, |t| &t.view);
            view.bind = self.main_bind_group(&view.globals, shadow, local_map, sky, &view.clusters);
            view.sky_bound = sky_generation;
        }

        // Meshes.
        view.main.begin();
        view.blend.items.clear();
        for cascade in view.cascades.iter_mut().chain(&mut view.local_tiles) {
            cascade.casters.begin();
        }
        let mut culled = 0;
        let retro = &settings.retro;
        view.retro_nearest = retro.force_nearest;
        view.retro_global = retro.enabled;
        // Projected size of a metre at depth 1, in virtual-screen pixels.
        let retro_px = Vec3::new(view_proj.x_axis.y, view_proj.y_axis.y, view_proj.z_axis.y)
            .length()
            * retro.snap_resolution[1] as f32
            * 0.5;
        // How much of the screen's height a metre at depth 1 covers (half-height units).
        let lod_scale =
            Vec3::new(view_proj.x_axis.y, view_proj.y_axis.y, view_proj.z_axis.y).length();
        for (global, transform, renderer) in world
            .query::<(Option<&GlobalTransform>, &Transform, &MeshRenderer)>()
            .iter()
        {
            let base = renderer.mesh.0;
            let Some(Some(mesh)) = gpu.meshes.get(base as usize) else {
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
            // A simpler copy for objects that look small; the base mesh's
            // sphere decides (so the choice does not flicker with the copy).
            let i = match gpu.lods.get(&base) {
                Some(lods) => {
                    let depth =
                        (cam.view_plane.truncate().dot(center) + cam.view_plane.w).max(0.01);
                    let level =
                        crate::lod::pick(base, lods, radius * lod_scale / depth, settings.lod_bias);
                    match gpu.meshes.get(level as usize) {
                        Some(Some(_)) => level,
                        _ => base,
                    }
                }
                None => base,
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
                for (tile, f) in view.local_tiles.iter_mut().zip(&local.frusta) {
                    if let Some(f) = f
                        && f.intersects_sphere(center, radius)
                    {
                        tile.casters.push(bucket_key(NO_MATERIAL, i), instance);
                    }
                }
            }
            let depth = cam.view_plane.truncate().dot(center) + cam.view_plane.w;
            if !frustum.intersects_sphere(center, radius)
                || (gpu.material_retro(material, retro.enabled)
                    && retro_culled(retro, depth, radius, retro_px))
            {
                culled += 1;
            } else if blended {
                view.blend.items.push(BlendItem {
                    depth,
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
        let local_count = local.count;
        for cascade in view
            .cascades
            .iter_mut()
            .take(count)
            .chain(view.local_tiles.iter_mut().take(local_count))
        {
            cascade
                .casters
                .finish(&self.device, &self.queue, &mut view.staging, |_| 0);
        }

        // Lights past the best sixteen: sorted into view clusters.
        let cluster_lights = if extra.is_empty() {
            0
        } else {
            let packed: Vec<_> = extra.iter().map(|(l, _)| *l).collect();
            let set = crate::cluster::build(&packed, &view_proj, cam.view_plane);
            if set.dropped > 0 {
                log::warn!(
                    "{} lights do not fit the light clusters (at most {} lights and {} list entries)",
                    set.dropped,
                    crate::MAX_CLUSTER_LIGHTS,
                    crate::cluster::MAX_INDICES
                );
            }
            view.clusters.upload(&self.queue, &set);
            set.lights.len()
        };

        // Uniforms.
        let mut fog = settings.fog;
        if retro.enabled && retro.draw_distance > 0.0 {
            // The fog completes at the draw distance, so the cut is hidden
            // (without fog: the background colour).
            let end = retro.draw_distance;
            fog = Some(match fog {
                Some(f) => Fog {
                    end: f.end.min(end),
                    start: if f.start < end { f.start } else { end * 0.5 },
                    ..f
                },
                None => Fog {
                    color: settings.clear_color,
                    start: end * 0.5,
                    end,
                },
            });
        }
        let height_fog = settings.height_fog;
        // Sampling matrices land in the cascade's tile of the atlas.
        let mut shadow_view_proj = [Mat4::IDENTITY; MAX_CASCADES];
        let mut shadow_offsets = [0.0; 4];
        let mut shadow_texel = [0.0; 4];
        let mut shadow_depth = [1.0; 4];
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
                shadow_offsets[c] = f.texel_world[c] * sun_bias;
                shadow_texel[c] = f.texel_world[c];
                shadow_depth[c] = f.depth_world[c];
                shadow_splits[c] = f.splits[c];
            }
        }
        let filter = match settings.shadow_filter {
            ShadowFilter::Hard => 0.0,
            ShadowFilter::Pcf => 1.0,
            ShadowFilter::Soft if self.tier == RenderTier::Full => 2.0,
            ShadowFilter::Pcss if self.tier == RenderTier::Full && self.pcss_shader => 3.0,
            ShadowFilter::Soft | ShadowFilter::Pcss => 1.0,
        };
        let sun_tan = (sun_size.clamp(0.0, 60.0).to_radians() * 0.5).tan();
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
            local_view_proj: local.atlas.map(|m| m.to_cols_array_2d()),
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
                sun_tan,
            ],
            shadow_texel,
            shadow_depth,
            local_cfg: [1.0 / local.size as f32, 0.0, 0.0, 0.0],
            sky_zenith: sky.zenith,
            sky_horizon: sky.horizon,
            sky_ground: sky.ground,
            sky_params: sky.params,
            height_fog_color: height_fog.map_or([0.0; 4], |f| rgb(f.color.0).extend(1.0).into()),
            height_fog: height_fog.map_or([0.0; 4], |f| {
                [f.density.max(0.0), f.falloff.max(0.0), f.height, 0.0]
            }),
            retro_a: [
                flag(retro.enabled),
                retro.snap_resolution[0] as f32,
                retro.snap_resolution[1] as f32,
                retro.snap_grid_m.max(0.0),
            ],
            retro_b: [
                retro.affine_strength.clamp(0.0, 1.0),
                retro.affine_fade_near.max(0.0),
                // The post chain quantises the final image instead.
                if self.hdr_output {
                    0.0
                } else {
                    retro.color_levels()
                },
                retro.dither_strength.max(0.0),
            ],
            retro_c: [
                flag(retro.vertex_lighting),
                flag(retro.shadows),
                retro.draw_distance.max(0.0),
                flag(retro.dissolve),
            ],
            cluster_cfg: [
                cluster_lights as f32,
                crate::cluster::slice_scale(),
                0.0,
                0.0,
            ],
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

        for (tile, m) in view
            .local_tiles
            .iter()
            .zip(&local.view_proj)
            .take(local_count)
        {
            self.queue
                .write_buffer(&tile.globals, 0, bytemuck::bytes_of(m));
        }

        let triangles = view
            .main
            .ranges
            .iter()
            .chain(&view.blend.ranges)
            .filter_map(|r| {
                let mesh = gpu.meshes.get(r.mesh as usize)?.as_ref()?;
                let instances = r.instances.end - r.instances.start;
                Some(u64::from(mesh.index_count / 3) * u64::from(instances))
            })
            .sum();
        RenderStats {
            triangles,
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
        self.draw_ranges(view, gpu, pass, &view.main.buffer, &view.main.ranges, false);
        if view.sky_active {
            pass.set_pipeline(&self.sky_pipeline);
            pass.draw(0..3, 0..1);
        }
        self.draw_ranges(
            view,
            gpu,
            pass,
            &view.blend.buffer,
            &view.blend.ranges,
            true,
        );
    }

    fn draw_ranges(
        &self,
        view: &ViewData,
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
        let (retro_nearest, retro_global) = (view.retro_nearest, view.retro_global);
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
                    .map_or(&self.fallback_material, |m| {
                        if retro_nearest
                            && m.material.retro.unwrap_or(retro_global)
                            && !m.material.point_filter
                        {
                            &m.bind_nearest
                        } else {
                            &m.bind
                        }
                    });
                pass.set_bind_group(1, bind, &[]);
                bound_material = Some(range.material);
            }
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, range.instances.clone());
        }
    }
}

/// Object-level retro culling: past the draw distance (even the nearest
/// point of the bounding sphere), or too small on the virtual screen.
/// `px_per_metre` is the projected size of one metre at depth 1.
fn retro_culled(retro: &RetroSettings, depth: f32, radius: f32, px_per_metre: f32) -> bool {
    (retro.draw_distance > 0.0 && depth - radius > retro.draw_distance)
        || (retro.cull_small_px > 0.0
            && retro.snap_resolution[1] > 0
            && depth > radius
            && radius * px_per_metre / depth < retro.cull_small_px)
}

#[derive(Default)]
struct SkyUniform {
    zenith: [f32; 4],
    horizon: [f32; 4],
    ground: [f32; 4],
    params: [f32; 4],
}

fn flag(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

fn rgb(c: [f32; 4]) -> Vec3 {
    Vec3::new(c[0], c[1], c[2])
}

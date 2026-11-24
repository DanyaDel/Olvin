//! Clustered lighting: lights past the sixteen best ones are sorted into a
//! grid of view-space cells (16 × 9 tiles of the screen, 24 depth slices),
//! so a fragment loops only over the few lights that can reach it.
//!
//! The sixteen best lights stay in the uniform buffer and are tried by every
//! fragment as before (they also carry the shadows). The rest, up to
//! [`MAX_CLUSTER_LIGHTS`], go into three textures read with `textureLoad`
//! (no storage buffers, so Compat devices work too): the lights, a grid of
//! `offset << 8 | count` per cluster, and the lists of light indices.
//! `globals.wgsl` repeats the constants below.

use bytemuck::cast_slice;
use glam::{Mat4, Vec3, Vec4};

use crate::light::PackedLight;

pub(crate) const TILES_X: usize = 16;
pub(crate) const TILES_Y: usize = 9;
pub(crate) const SLICES: usize = 24;
pub(crate) const CLUSTERS: usize = TILES_X * TILES_Y * SLICES;

/// Lights the cluster path holds beyond the uniform buffer's sixteen.
pub const MAX_CLUSTER_LIGHTS: usize = 1024;
/// The most lights one cluster lists.
const MAX_PER_CLUSTER: usize = 255;
/// Width of the three data textures, in texels.
pub(crate) const TEX_W: u32 = 1024;
/// Rows of the light texture: four texels per light.
pub(crate) const LIGHT_ROWS: u32 = (MAX_CLUSTER_LIGHTS as u32 * 4).div_ceil(TEX_W);
/// Rows of the grid texture.
pub(crate) const GRID_ROWS: u32 = (CLUSTERS as u32).div_ceil(TEX_W);
/// Rows of the index texture, which bounds the sum of all lists.
pub(crate) const INDEX_ROWS: u32 = 64;
pub(crate) const MAX_INDICES: usize = (INDEX_ROWS * TEX_W) as usize;

/// Depth where slice 0 starts (shallower fragments use it too) and where the
/// last slice ends (deeper ones use the last).
const NEAR: f32 = 0.1;
const FAR: f32 = 250.0;

/// `k` in `slice = floor(log2(depth / NEAR) * k)`.
pub(crate) fn slice_scale() -> f32 {
    SLICES as f32 / (FAR / NEAR).log2()
}

/// What goes to the GPU for one view.
#[derive(Default)]
pub(crate) struct ClusterSet {
    /// The lights, 16 floats each (the [`PackedLight`] layout).
    pub lights: Vec<PackedLight>,
    /// `offset << 8 | count` per cluster, `CLUSTERS` long.
    pub grid: Vec<u32>,
    pub index: Vec<u32>,
    /// Lights that did not fit (past the cap or the index texture).
    pub dropped: usize,
}

fn slice_of(depth: f32) -> usize {
    let s = (depth.max(NEAR) / NEAR).log2() * slice_scale();
    (s.floor().max(0.0) as usize).min(SLICES - 1)
}

/// The inclusive cluster ranges (x, y, z) a light can reach, or `None` when
/// it is entirely behind the camera.
fn reach(light: &PackedLight, view_proj: &Mat4, view_plane: Vec4) -> Option<[(usize, usize); 3]> {
    let range = light.position_range[3];
    let center = Vec3::from_slice(&light.position_range[..3]);
    if range <= 0.0 {
        // A directional light is everywhere.
        return Some([(0, TILES_X - 1), (0, TILES_Y - 1), (0, SLICES - 1)]);
    }
    let depth = view_plane.truncate().dot(center) + view_plane.w;
    if depth + range < NEAR {
        return None;
    }
    let z = (slice_of(depth - range), slice_of(depth + range));
    // Screen rectangle of the sphere's bounding cube; a corner behind the
    // camera makes it the whole screen.
    let (mut lo, mut hi) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
    let mut whole = false;
    for corner in 0..8 {
        let offset = Vec3::new(
            if corner & 1 == 0 { -range } else { range },
            if corner & 2 == 0 { -range } else { range },
            if corner & 4 == 0 { -range } else { range },
        );
        let clip = *view_proj * (center + offset).extend(1.0);
        if clip.w < 1e-4 {
            whole = true;
            break;
        }
        let ndc = glam::Vec2::new(clip.x, clip.y) / clip.w;
        lo = lo.min(ndc);
        hi = hi.max(ndc);
    }
    let (x, y) = if whole {
        ((0, TILES_X - 1), (0, TILES_Y - 1))
    } else {
        if hi.x < -1.0 || lo.x > 1.0 || hi.y < -1.0 || lo.y > 1.0 {
            return None;
        }
        let tile = |v: f32, n: usize| ((v * n as f32).floor().max(0.0) as usize).min(n - 1);
        (
            (
                tile(lo.x * 0.5 + 0.5, TILES_X),
                tile(hi.x * 0.5 + 0.5, TILES_X),
            ),
            // Screen y runs down while NDC y runs up.
            (
                tile(0.5 - hi.y * 0.5, TILES_Y),
                tile(0.5 - lo.y * 0.5, TILES_Y),
            ),
        )
    };
    Some([x, y, z])
}

fn cluster_id(x: usize, y: usize, z: usize) -> usize {
    (z * TILES_Y + y) * TILES_X + x
}

/// Sorts `lights` (best first) into clusters for the camera `view_proj` /
/// `view_plane` (view depth of p = `dot(xyz, p) + w`).
pub(crate) fn build(lights: &[PackedLight], view_proj: &Mat4, view_plane: Vec4) -> ClusterSet {
    let mut set = ClusterSet {
        grid: vec![0; CLUSTERS],
        ..Default::default()
    };
    let mut counts = vec![0u16; CLUSTERS];
    let mut accepted: Vec<(PackedLight, [(usize, usize); 3])> = Vec::new();
    let mut total = 0;
    for light in lights.iter().take(MAX_CLUSTER_LIGHTS) {
        let Some(r) = reach(light, view_proj, view_plane) else {
            continue;
        };
        let cost = (r[0].1 - r[0].0 + 1) * (r[1].1 - r[1].0 + 1) * (r[2].1 - r[2].0 + 1);
        if total + cost > MAX_INDICES {
            set.dropped += 1;
            continue;
        }
        total += cost;
        accepted.push((*light, r));
    }
    set.dropped += lights.len().saturating_sub(MAX_CLUSTER_LIGHTS);
    for (_, r) in &accepted {
        for z in r[2].0..=r[2].1 {
            for y in r[1].0..=r[1].1 {
                for x in r[0].0..=r[0].1 {
                    let c = cluster_id(x, y, z);
                    counts[c] = (counts[c] + 1).min(MAX_PER_CLUSTER as u16);
                }
            }
        }
    }
    let mut next = vec![0u32; CLUSTERS];
    let mut offset = 0u32;
    for c in 0..CLUSTERS {
        set.grid[c] = offset << 8 | u32::from(counts[c]);
        next[c] = offset;
        offset += u32::from(counts[c]);
    }
    set.index = vec![0; offset as usize];
    let mut filled = vec![0u16; CLUSTERS];
    for (i, (light, r)) in accepted.iter().enumerate() {
        set.lights.push(*light);
        for z in r[2].0..=r[2].1 {
            for y in r[1].0..=r[1].1 {
                for x in r[0].0..=r[0].1 {
                    let c = cluster_id(x, y, z);
                    if filled[c] < counts[c] {
                        set.index[(next[c] + u32::from(filled[c])) as usize] = i as u32;
                        filled[c] += 1;
                    }
                }
            }
        }
    }
    set
}

/// The three data textures of a view.
pub(crate) struct ClusterTextures {
    lights: wgpu::Texture,
    grid: wgpu::Texture,
    index: wgpu::Texture,
    pub lights_view: wgpu::TextureView,
    pub grid_view: wgpu::TextureView,
    pub index_view: wgpu::TextureView,
}

impl ClusterTextures {
    pub fn new(device: &wgpu::Device) -> Self {
        let make = |label, format, rows| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: TEX_W,
                    height: rows,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let lights = make(
            "cluster lights",
            wgpu::TextureFormat::Rgba32Float,
            LIGHT_ROWS,
        );
        let grid = make("cluster grid", wgpu::TextureFormat::R32Uint, GRID_ROWS);
        let index = make("cluster index", wgpu::TextureFormat::R32Uint, INDEX_ROWS);
        Self {
            lights_view: lights.create_view(&Default::default()),
            grid_view: grid.create_view(&Default::default()),
            index_view: index.create_view(&Default::default()),
            lights,
            grid,
            index,
        }
    }

    /// Writes the rows `set` uses.
    pub fn upload(&self, queue: &wgpu::Queue, set: &ClusterSet) {
        let write = |texture: &wgpu::Texture, bytes: &[u8], bytes_per_texel: u32, texels: usize| {
            let rows = (texels as u32).div_ceil(TEX_W).max(1);
            let row_bytes = (TEX_W * bytes_per_texel) as usize;
            let mut padded = bytes.to_vec();
            padded.resize(rows as usize * row_bytes, 0);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &padded,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes as u32),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: TEX_W,
                    height: rows,
                    depth_or_array_layers: 1,
                },
            );
        };
        write(
            &self.lights,
            cast_slice(&set.lights),
            16,
            set.lights.len() * 4,
        );
        write(&self.grid, cast_slice(&set.grid), 4, CLUSTERS);
        write(&self.index, cast_slice(&set.index), 4, set.index.len());
    }
}

#[cfg(test)]
mod tests {
    use glam::camera::rh::{proj::directx, view};

    use super::*;

    fn camera() -> (Mat4, Vec4) {
        let eye = Vec3::ZERO;
        let v = view::look_to_mat4(eye, Vec3::NEG_Z, Vec3::Y);
        let p = directx::perspective(1.0, 16.0 / 9.0, 0.1, 250.0);
        // View depth of p = -(view * p).z
        let row = Vec4::new(-v.x_axis.z, -v.y_axis.z, -v.z_axis.z, -v.w_axis.z);
        (p * v, row)
    }

    fn point(pos: Vec3, range: f32) -> PackedLight {
        PackedLight {
            position_range: [pos.x, pos.y, pos.z, range],
            ..Default::default()
        }
    }

    fn listed(set: &ClusterSet, x: usize, y: usize, z: usize) -> Vec<u32> {
        let rec = set.grid[cluster_id(x, y, z)];
        let (offset, count) = ((rec >> 8) as usize, (rec & 255) as usize);
        set.index[offset..offset + count].to_vec()
    }

    #[test]
    fn a_light_ahead_is_in_the_middle_clusters_only() {
        let (vp, plane) = camera();
        let set = build(&[point(Vec3::new(0.0, 0.0, -10.0), 1.0)], &vp, plane);
        assert_eq!(set.lights.len(), 1);
        let z = slice_of(10.0);
        assert_eq!(listed(&set, TILES_X / 2, TILES_Y / 2, z), [0]);
        assert!(
            listed(&set, 0, 0, z).is_empty(),
            "the corner is far from it"
        );
        assert!(
            listed(&set, TILES_X / 2, TILES_Y / 2, 0).is_empty(),
            "too near"
        );
        assert!(
            listed(&set, TILES_X / 2, TILES_Y / 2, SLICES - 1).is_empty(),
            "too far"
        );
    }

    #[test]
    fn a_light_to_the_left_is_on_the_left_of_the_grid() {
        let (vp, plane) = camera();
        let set = build(&[point(Vec3::new(-6.0, 0.0, -10.0), 1.0)], &vp, plane);
        let z = slice_of(10.0);
        let left: usize = (0..TILES_X / 2)
            .map(|x| listed(&set, x, TILES_Y / 2, z).len())
            .sum();
        let right: usize = (TILES_X / 2..TILES_X)
            .map(|x| listed(&set, x, TILES_Y / 2, z).len())
            .sum();
        assert!(left > 0 && right == 0, "left {left}, right {right}");
    }

    #[test]
    fn higher_on_screen_means_the_first_rows() {
        let (vp, plane) = camera();
        let set = build(&[point(Vec3::new(0.0, 4.0, -10.0), 0.5)], &vp, plane);
        let z = slice_of(10.0);
        let top: usize = (0..TILES_Y / 2)
            .map(|y| listed(&set, TILES_X / 2, y, z).len())
            .sum();
        let bottom: usize = (TILES_Y / 2..TILES_Y)
            .map(|y| listed(&set, TILES_X / 2, y, z).len())
            .sum();
        assert!(top > 0 && bottom == 0, "top {top}, bottom {bottom}");
    }

    #[test]
    fn behind_the_camera_is_skipped_and_a_directional_light_is_everywhere() {
        let (vp, plane) = camera();
        let set = build(
            &[
                point(Vec3::new(0.0, 0.0, 20.0), 2.0),
                point(Vec3::ZERO, 0.0),
            ],
            &vp,
            plane,
        );
        assert_eq!(set.lights.len(), 1, "only the directional one");
        assert!(set.grid.iter().all(|r| r & 255 == 1));
    }

    #[test]
    fn a_light_around_the_camera_reaches_every_tile() {
        let (vp, plane) = camera();
        let set = build(&[point(Vec3::ZERO, 5.0)], &vp, plane);
        for y in 0..TILES_Y {
            for x in 0..TILES_X {
                assert_eq!(listed(&set, x, y, 0), [0], "tile {x},{y}");
            }
        }
    }

    #[test]
    fn lists_hold_at_most_255_and_the_rest_are_not_lost_silently() {
        let (vp, plane) = camera();
        let lights: Vec<PackedLight> = (0..300)
            .map(|_| point(Vec3::new(0.0, 0.0, -10.0), 1.0))
            .collect();
        let set = build(&lights, &vp, plane);
        let z = slice_of(10.0);
        assert_eq!(listed(&set, TILES_X / 2, TILES_Y / 2, z).len(), 255);
        assert_eq!(set.lights.len(), 300);
    }

    #[test]
    fn the_index_texture_bounds_the_work() {
        let (vp, plane) = camera();
        // Directional lights cost every cluster each: 3456 of them per light.
        let lights = vec![point(Vec3::ZERO, 0.0); 100];
        let set = build(&lights, &vp, plane);
        assert!(set.index.len() <= MAX_INDICES);
        assert!(set.dropped > 0 && set.lights.len() + set.dropped == 100);
    }
}

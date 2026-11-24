//! Runtime lights and the CPU side of lighting: packing lights into the
//! uniform buffer and fitting the directional shadow map to the camera.

use bytemuck::{Pod, Zeroable};
use glam::camera::rh::{proj::directx, view};
use glam::{Mat4, Vec3, Vec4};

/// Max lights the fragment shader loops over (packed into the uniform buffer).
pub const MAX_LIGHTS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

/// A light, placed by the entity's `GlobalTransform` (or `Transform`).
/// Lights shine along the entity's forward (-Z).
///
/// The first `Directional` light in the world is the sun: it replaces
/// `RenderSettings::light_dir/light_color` and, with `shadows`, casts the
/// directional shadow map. Point and spot lights cast shadows with `shadows`
/// too, but only the most important few (`RenderSettings::max_spot_shadows` /
/// `max_point_shadows`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    /// Linear RGB.
    pub color: [f32; 3],
    pub intensity: f32,
    /// Point / spot: distance where the light fades to zero.
    pub range: f32,
    /// Spot: full cone angle in degrees (like Unity's Spot Angle).
    pub spot_angle_deg: f32,
    /// Casts shadows (default: only the sun does).
    pub shadows: bool,
    /// Shadow bias in shadow-map texels: how far the lookup slides off the
    /// surface along its normal (sun: the cascade's texel, local lights: the
    /// tile's texel at that distance). Raise it against acne, lower it
    /// against peter-panning.
    pub shadow_bias: f32,
    /// Sun: angular diameter in degrees, the penumbra size of
    /// `ShadowFilter::Pcss`. Unused by point and spot lights.
    pub size: f32,
}

/// Default of [`Light::shadow_bias`].
pub const DEFAULT_SHADOW_BIAS: f32 = 1.5;
/// Default of [`Light::size`] (a softened sun; the real one is 0.53°).
pub const DEFAULT_LIGHT_SIZE: f32 = 3.0;

impl Light {
    /// Unity-like defaults (warm white, intensity 1, range 10, 30° spot).
    pub fn new(kind: LightKind) -> Self {
        Self {
            kind,
            color: [1.0, 0.96, 0.9],
            intensity: 1.0,
            range: 10.0,
            spot_angle_deg: 30.0,
            shadows: kind == LightKind::Directional,
            shadow_bias: DEFAULT_SHADOW_BIAS,
            size: DEFAULT_LIGHT_SIZE,
        }
    }

    pub fn directional(color: [f32; 3], intensity: f32) -> Self {
        Self {
            color,
            intensity,
            ..Self::new(LightKind::Directional)
        }
    }

    pub fn point(color: [f32; 3], intensity: f32, range: f32) -> Self {
        Self {
            color,
            intensity,
            range,
            ..Self::new(LightKind::Point)
        }
    }

    pub fn spot(color: [f32; 3], intensity: f32, range: f32, spot_angle_deg: f32) -> Self {
        Self {
            color,
            intensity,
            range,
            spot_angle_deg,
            ..Self::new(LightKind::Spot)
        }
    }

    pub(crate) fn radiance(&self) -> Vec3 {
        Vec3::from(self.color) * self.intensity
    }

    /// Packs a non-sun light at `position` shining along `dir`.
    pub(crate) fn pack(&self, position: Vec3, dir: Vec3) -> PackedLight {
        let (range, scale, offset) = match self.kind {
            // range 0 marks a directional light in the shader.
            LightKind::Directional => (0.0, 0.0, 1.0),
            LightKind::Point => (self.range.max(1e-3), 0.0, 1.0),
            LightKind::Spot => {
                // Smooth cone: full at 75% of the half-angle, zero at the edge.
                let half = (self.spot_angle_deg.clamp(1.0, 179.0) * 0.5).to_radians();
                let (cos_outer, cos_inner) = (half.cos(), (half * 0.75).cos());
                let inv = 1.0 / (cos_inner - cos_outer).max(1e-3);
                (self.range.max(1e-3), inv, -cos_outer * inv)
            }
        };
        PackedLight {
            position_range: position.extend(range).into(),
            color_spot_scale: self.radiance().extend(scale).into(),
            direction_spot_offset: dir.extend(offset).into(),
            shadow: [0.0; 4],
        }
    }
}

/// One light in the uniform buffer (std140-safe: three vec4).
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub(crate) struct PackedLight {
    /// xyz: world position; w: range (0 = directional).
    pub position_range: [f32; 4],
    /// rgb: colour × intensity; w: spot cone scale.
    pub color_spot_scale: [f32; 4],
    /// xyz: direction the light travels; w: spot cone offset.
    pub direction_spot_offset: [f32; 4],
    /// x: first atlas tile + 1 (0 = no shadow); y: bias in texels; z: texel
    /// size per metre of distance; w = 1: a point light (six tiles).
    pub shadow: [f32; 4],
}

/// Keeps the [`MAX_LIGHTS`] most relevant lights without allocating.
pub(crate) struct LightList {
    pub lights: [PackedLight; MAX_LIGHTS],
    scores: [f32; MAX_LIGHTS],
    pub len: usize,
}

impl LightList {
    pub fn new() -> Self {
        Self {
            lights: [PackedLight::default(); MAX_LIGHTS],
            scores: [0.0; MAX_LIGHTS],
            len: 0,
        }
    }

    /// Lower score = more important. When full, replaces the worst light.
    /// Returns the light that did not make it (the new one, or the one it
    /// replaced) with its score, for the clustered path.
    pub fn push(&mut self, light: PackedLight, score: f32) -> Option<(PackedLight, f32)> {
        if self.len < MAX_LIGHTS {
            self.lights[self.len] = light;
            self.scores[self.len] = score;
            self.len += 1;
            return None;
        }
        let (worst, &worst_score) = self
            .scores
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .expect("MAX_LIGHTS > 0");
        if score < worst_score {
            let evicted = (self.lights[worst], worst_score);
            self.lights[worst] = light;
            self.scores[worst] = score;
            Some(evicted)
        } else {
            Some((light, score))
        }
    }
}

/// Tiles per side of the point / spot shadow atlas.
pub(crate) const LOCAL_GRID: usize = 4;
/// Tiles in the atlas: a spot light takes one, a point light six.
pub const MAX_LOCAL_TILES: usize = LOCAL_GRID * LOCAL_GRID;
/// Point faces see a little more than 90° so the filter never reads across
/// a face's edge: tan of the half angle.
const POINT_FACE_TAN: f32 = 1.1;

/// A local light's shadow camera for one tile.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LocalFit {
    /// World to the tile's own clip space (what the depth pass draws with).
    pub view_proj: Mat4,
    /// World size of a texel one metre away.
    pub texel_per_metre: f32,
}

fn near_plane(range: f32) -> f32 {
    (range * 0.01).clamp(0.05, 0.5)
}

fn up_for(dir: Vec3) -> Vec3 {
    if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y }
}

/// A perspective camera at the spot light looking along its cone, a little
/// wider than the cone so the filter has room at the rim.
pub(crate) fn fit_spot(
    pos: Vec3,
    dir: Vec3,
    angle_deg: f32,
    range: f32,
    tile: u32,
) -> Option<LocalFit> {
    let dir = dir.try_normalize()?;
    let range = range.max(0.1);
    let fov = (angle_deg.clamp(1.0, 179.0) + 8.0)
        .clamp(10.0, 150.0)
        .to_radians();
    let view = view::look_to_mat4(pos, dir, up_for(dir));
    let proj = directx::perspective(fov, 1.0, near_plane(range), range);
    Some(LocalFit {
        view_proj: proj * view,
        texel_per_metre: 2.0 * (fov * 0.5).tan() / tile as f32,
    })
}

/// The six faces of a point light (+X, -X, +Y, -Y, +Z, -Z).
pub(crate) fn fit_point(pos: Vec3, range: f32, tile: u32) -> [LocalFit; 6] {
    let range = range.max(0.1);
    let proj = directx::perspective(2.0 * POINT_FACE_TAN.atan(), 1.0, near_plane(range), range);
    let faces = [
        (Vec3::X, Vec3::Y),
        (Vec3::NEG_X, Vec3::Y),
        (Vec3::Y, Vec3::Z),
        (Vec3::NEG_Y, Vec3::NEG_Z),
        (Vec3::Z, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y),
    ];
    faces.map(|(dir, up)| LocalFit {
        view_proj: proj * view::look_to_mat4(pos, dir, up),
        texel_per_metre: 2.0 * POINT_FACE_TAN / tile as f32,
    })
}

/// Moves a tile-local view-projection into tile `slot` of the atlas.
pub(crate) fn atlas_matrix(view_proj: Mat4, slot: usize) -> Mat4 {
    let step = 2.0 / LOCAL_GRID as f32;
    let (col, row) = ((slot % LOCAL_GRID) as f32, (slot / LOCAL_GRID) as f32);
    let scale = 1.0 / LOCAL_GRID as f32;
    Mat4::from_translation(Vec3::new(
        -1.0 + step * 0.5 + col * step,
        1.0 - step * 0.5 - row * step,
        0.0,
    )) * Mat4::from_scale(Vec3::new(scale, scale, 1.0))
        * view_proj
}

/// A light asking for shadow tiles.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    pub point: bool,
    /// Higher = served first.
    pub importance: f32,
}

/// How much a local light matters to the viewer: bright, big, close.
pub(crate) fn importance(light: &Light, pos: Vec3, eye: Vec3) -> f32 {
    let lum = light.radiance().dot(Vec3::new(0.2126, 0.7152, 0.0722));
    let gap = (pos.distance(eye) - light.range).max(0.0);
    lum * light.range / ((1.0 + gap) * (1.0 + gap))
}

/// Hands out atlas tiles to the most important candidates: at most
/// `max_spot` spot and `max_point` point lights, [`MAX_LOCAL_TILES`] tiles
/// in all. Returns `(candidate index, first tile)` pairs.
pub(crate) fn assign_tiles(
    cands: &[Candidate],
    max_spot: usize,
    max_point: usize,
) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..cands.len()).collect();
    order.sort_by(|&a, &b| cands[b].importance.total_cmp(&cands[a].importance));
    let (mut spots, mut points, mut next) = (0, 0, 0);
    let mut out = Vec::new();
    for i in order {
        let (cap, used, tiles) = if cands[i].point {
            (max_point, &mut points, 6)
        } else {
            (max_spot, &mut spots, 1)
        };
        if *used >= cap || next + tiles > MAX_LOCAL_TILES {
            continue;
        }
        *used += 1;
        out.push((i, next));
        next += tiles;
    }
    out
}

/// What the shader needs to know about the camera, recovered from view-proj
/// (the editor passes bare matrices, so there's no camera entity to read).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CameraInfo {
    /// Perspective: (eye, 1). Orthographic: (direction towards the camera, 0).
    pub camera: Vec4,
    /// View depth of p = dot(xyz, p) + w.
    pub view_plane: Vec4,
    /// A point to rank lights by distance.
    pub eye: Vec3,
    /// View depth of the near plane.
    near_depth: f32,
    near: [Vec3; 4],
    far: [Vec3; 4],
    /// Depth of the far plane from the near plane (may be infinite).
    depth: f32,
}

impl CameraInfo {
    pub fn from_view_proj(view_proj: Mat4) -> Self {
        let inv = view_proj.inverse();
        let corner = |x: f32, y: f32, z: f32| inv.project_point3(Vec3::new(x, y, z));
        let xy = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        let near = xy.map(|(x, y)| corner(x, y, 0.0));
        let far = xy.map(|(x, y)| corner(x, y, 1.0));
        let near_c = corner(0.0, 0.0, 0.0);
        // The eye is the point clip space sends to w = 0 with x = y = 0.
        let eye_h = inv * Vec4::new(0.0, 0.0, 1.0, 0.0);
        // The forward axis: towards the far plane, robust to an infinite far.
        let probe = corner(0.0, 0.0, 0.5);
        let fwd = (probe - near_c).normalize_or(Vec3::NEG_Z);
        let far_c = corner(0.0, 0.0, 1.0);
        let depth = (far_c - near_c).dot(fwd);
        let (camera, eye) = if eye_h.w.abs() > 1e-6 * eye_h.truncate().length().max(1.0) {
            let eye = eye_h.truncate() / eye_h.w;
            (eye.extend(1.0), eye)
        } else {
            ((-fwd).extend(0.0), near_c)
        };
        let view_plane = fwd.extend(-fwd.dot(eye));
        Self {
            camera,
            view_plane,
            eye,
            near_depth: view_plane.dot(near_c.extend(1.0)),
            near,
            far,
            depth: if depth.is_finite() && depth > 0.0 {
                depth
            } else {
                f32::INFINITY
            },
        }
    }
}

/// Most cascades the sun's shadow map is split into (a 2x2 atlas).
pub const MAX_CASCADES: usize = 4;

/// Blend of logarithmic and uniform split distances (1 = all logarithmic).
const SPLIT_LAMBDA: f32 = 0.6;

/// The sun's cascaded shadow cameras.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShadowCascades {
    pub count: usize,
    /// Orthographic view-projection per cascade, onto its own tile.
    pub view_proj: [Mat4; MAX_CASCADES],
    /// View depth where each cascade ends.
    pub splits: [f32; MAX_CASCADES],
    /// World size of one shadow texel.
    pub texel_world: [f32; MAX_CASCADES],
    /// World length of the depth range 0..1 of each cascade.
    pub depth_world: [f32; MAX_CASCADES],
}

/// Splits the first `distance` metres of the camera frustum into `count`
/// slices (logarithmic / uniform mix, uniform for orthographic cameras) and
/// fits an orthographic shadow camera around each. A box is a bounding
/// sphere (so it doesn't change size as the camera turns) snapped to whole
/// texels (no shimmering when the camera moves); `tile` is the texel size of
/// one cascade's square.
pub(crate) fn fit_cascades(
    cam: &CameraInfo,
    light_dir: Vec3,
    distance: f32,
    count: usize,
    tile: u32,
) -> Option<ShadowCascades> {
    let dir = light_dir.try_normalize()?;
    let count = count.clamp(1, MAX_CASCADES);
    let reach = if cam.depth.is_finite() {
        distance.min(cam.depth)
    } else {
        distance
    };
    if !reach.is_finite() || reach <= 0.0 {
        return None;
    }
    // Slice ends as distances from the near plane.
    let zn = cam.near_depth.max(0.05);
    let perspective = cam.camera.w > 0.5;
    let mut ends = [0.0f32; MAX_CASCADES];
    for (i, end) in ends.iter_mut().enumerate().take(count) {
        let k = (i + 1) as f32 / count as f32;
        let uniform = zn + reach * k;
        let log = zn * ((zn + reach) / zn).powf(k);
        let z = if perspective {
            SPLIT_LAMBDA * log + (1.0 - SPLIT_LAMBDA) * uniform
        } else {
            uniform
        };
        *end = (z - cam.near_depth).clamp(0.0, reach);
    }
    ends[count - 1] = reach;

    let at = |i: usize, d: f32| {
        let (n, f) = (cam.near[i], cam.far[i]);
        if f.is_finite() && cam.depth.is_finite() {
            n + (f - n) * (d / cam.depth)
        } else {
            n + (n - cam.eye).normalize_or_zero() * d
        }
    };
    let mut out = ShadowCascades {
        count,
        view_proj: [Mat4::IDENTITY; MAX_CASCADES],
        splits: [f32::MAX; MAX_CASCADES],
        texel_world: [0.0; MAX_CASCADES],
        depth_world: [0.0; MAX_CASCADES],
    };
    let mut start = 0.0;
    for c in 0..count {
        let mut points = [Vec3::ZERO; 8];
        for i in 0..4 {
            points[i] = at(i, start);
            points[i + 4] = at(i, ends[c]);
        }
        let (view_proj, texel, depth) = fit_slice(&points, dir, tile)?;
        out.view_proj[c] = view_proj;
        out.texel_world[c] = texel;
        out.depth_world[c] = depth;
        out.splits[c] = cam.near_depth + ends[c];
        start = ends[c];
    }
    Some(out)
}

/// Orthographic light camera around the bounding sphere of `points`,
/// snapped to the texel grid. Returns it with the world size of a texel and
/// the world length of its depth range.
fn fit_slice(points: &[Vec3; 8], dir: Vec3, size: u32) -> Option<(Mat4, f32, f32)> {
    let center = points.iter().copied().sum::<Vec3>() / 8.0;
    let radius = points
        .iter()
        .map(|p| p.distance(center))
        .fold(0.0f32, f32::max);
    if !radius.is_finite() || !center.is_finite() || radius <= 0.0 {
        return None;
    }
    // Quantized so tiny float noise doesn't change the texel size.
    let radius = (radius * 16.0).ceil() / 16.0;
    let up = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    let light_view = view::look_to_mat4(Vec3::ZERO, dir, up);
    let texel = 2.0 * radius / size as f32;
    let c = light_view.transform_point3(center);
    let (cx, cy) = ((c.x / texel).floor() * texel, (c.y / texel).floor() * texel);
    // Casters between the light and the slice still need to land in the map.
    let pad = radius.max(50.0);
    let proj = directx::orthographic(
        cx - radius,
        cx + radius,
        cy - radius,
        cy + radius,
        -c.z - radius - pad,
        -c.z + radius,
    );
    Some((proj * light_view, texel, 2.0 * radius + pad))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perspective_camera(eye: Vec3, target: Vec3) -> Mat4 {
        directx::perspective(60f32.to_radians(), 1.0, 0.1, 500.0)
            * view::look_at_mat4(eye, target, Vec3::Y)
    }

    #[test]
    fn camera_info_recovers_eye_and_depth() {
        let eye = Vec3::new(3.0, 2.0, 5.0);
        let info = CameraInfo::from_view_proj(perspective_camera(eye, Vec3::ZERO));
        assert!(info.eye.abs_diff_eq(eye, 1e-3), "{:?}", info.eye);
        assert_eq!(info.camera.w, 1.0);
        let depth = info.view_plane.truncate().dot(Vec3::ZERO) + info.view_plane.w;
        assert!((depth - eye.length()).abs() < 1e-3, "{depth}");

        let ortho = directx::orthographic(-5.0, 5.0, -5.0, 5.0, 0.1, 100.0)
            * view::look_at_mat4(eye, Vec3::ZERO, Vec3::Y);
        let info = CameraInfo::from_view_proj(ortho);
        assert_eq!(info.camera.w, 0.0);
        assert!(info.camera.truncate().abs_diff_eq(eye.normalize(), 1e-3));
    }

    fn test_cascades(eye: Vec3, count: usize) -> ShadowCascades {
        let cam = CameraInfo::from_view_proj(perspective_camera(eye, eye + Vec3::NEG_Z * 10.0));
        fit_cascades(&cam, Vec3::new(-0.3, -1.0, -0.2), 60.0, count, 1024).unwrap()
    }

    #[test]
    fn cascades_split_the_range_and_cover_their_slices() {
        let eye = Vec3::new(0.0, 2.0, 0.0);
        let cam = CameraInfo::from_view_proj(perspective_camera(eye, eye + Vec3::NEG_Z * 10.0));
        let fit = fit_cascades(&cam, Vec3::new(-0.3, -1.0, -0.2), 60.0, 4, 1024).unwrap();
        assert_eq!(fit.count, 4);
        assert!(
            fit.splits.windows(2).all(|w| w[0] < w[1]),
            "{:?}",
            fit.splits
        );
        // The last cascade ends where the shadow distance does (from the near plane).
        assert!((fit.splits[3] - (cam.near_depth + 60.0)).abs() < 1e-3);
        // Texels get coarser with distance.
        assert!(fit.texel_world.windows(2).all(|w| w[0] < w[1]));
        // A point on the view axis inside each slice lands in that cascade's map.
        let mut start = cam.near_depth;
        for c in 0..4 {
            let depth = (start + fit.splits[c]) * 0.5;
            let p = eye + Vec3::NEG_Z * depth;
            let ndc = fit.view_proj[c].project_point3(p);
            assert!(
                ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && (0.0..=1.0).contains(&ndc.z),
                "cascade {c}: {p} -> {ndc}"
            );
            start = fit.splits[c];
        }
    }

    #[test]
    fn one_cascade_covers_the_near_slice_and_snaps_to_texels() {
        let eye = Vec3::new(0.0, 2.0, 0.0);
        let fit = test_cascades(eye, 1);
        assert_eq!(fit.count, 1);
        for p in [Vec3::new(0.0, 0.0, -5.0), Vec3::new(10.0, 0.0, -40.0)] {
            let c = fit.view_proj[0].project_point3(p);
            assert!(
                c.x.abs() <= 1.0 && c.y.abs() <= 1.0 && (0.0..=1.0).contains(&c.z),
                "{p} -> {c}"
            );
        }
        // Moving the camera a fraction of a texel keeps the map's texel grid in place.
        for count in [1, 3] {
            let a = test_cascades(eye, count);
            let b = test_cascades(eye + Vec3::X * 0.001, count);
            let texel_ndc = 2.0 / 1024.0;
            let (pa, pb) = (
                a.view_proj[0].project_point3(Vec3::ZERO),
                b.view_proj[0].project_point3(Vec3::ZERO),
            );
            let dx = ((pa.x - pb.x) / texel_ndc).abs();
            assert!(
                (dx - dx.round()).abs() < 1e-2,
                "{count} cascades: offset is whole texels: {dx}"
            );
        }
    }

    #[test]
    fn light_list_keeps_the_best() {
        let mut list = LightList::new();
        for i in 0..40 {
            list.push(
                PackedLight {
                    position_range: [i as f32, 0.0, 0.0, 1.0],
                    ..Default::default()
                },
                i as f32,
            );
        }
        assert_eq!(list.len, MAX_LIGHTS);
        assert!(
            list.lights
                .iter()
                .all(|l| l.position_range[0] < MAX_LIGHTS as f32)
        );
    }
}

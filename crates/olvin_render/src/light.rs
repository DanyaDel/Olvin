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
/// directional shadow map. `shadows` is ignored on point and spot lights.
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
    pub shadows: bool,
}

impl Light {
    /// Unity-like defaults (warm white, intensity 1, range 10, 30° spot).
    pub fn new(kind: LightKind) -> Self {
        Self { kind, color: [1.0, 0.96, 0.9], intensity: 1.0, range: 10.0, spot_angle_deg: 30.0, shadows: kind == LightKind::Directional }
    }

    pub fn directional(color: [f32; 3], intensity: f32) -> Self {
        Self { color, intensity, ..Self::new(LightKind::Directional) }
    }

    pub fn point(color: [f32; 3], intensity: f32, range: f32) -> Self {
        Self { color, intensity, range, ..Self::new(LightKind::Point) }
    }

    pub fn spot(color: [f32; 3], intensity: f32, range: f32, spot_angle_deg: f32) -> Self {
        Self { color, intensity, range, spot_angle_deg, ..Self::new(LightKind::Spot) }
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
}

/// Keeps the [`MAX_LIGHTS`] most relevant lights without allocating.
pub(crate) struct LightList {
    pub lights: [PackedLight; MAX_LIGHTS],
    scores: [f32; MAX_LIGHTS],
    pub len: usize,
}

impl LightList {
    pub fn new() -> Self {
        Self { lights: [PackedLight::default(); MAX_LIGHTS], scores: [0.0; MAX_LIGHTS], len: 0 }
    }

    /// Lower score = more important. When full, replaces the worst light.
    pub fn push(&mut self, light: PackedLight, score: f32) {
        if self.len < MAX_LIGHTS {
            self.lights[self.len] = light;
            self.scores[self.len] = score;
            self.len += 1;
            return;
        }
        let (worst, &worst_score) =
            self.scores.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).expect("MAX_LIGHTS > 0");
        if score < worst_score {
            self.lights[worst] = light;
            self.scores[worst] = score;
        }
    }
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
        Self { camera, view_plane, eye, near, far, depth: if depth.is_finite() && depth > 0.0 { depth } else { f32::INFINITY } }
    }
}

/// The fitted directional shadow camera.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShadowFit {
    pub view_proj: Mat4,
    /// World size of one shadow texel.
    pub texel_world: f32,
}

/// Fits an orthographic shadow camera around the first `distance` metres of
/// the camera frustum. The box is a bounding sphere (so it doesn't change
/// size as the camera turns) snapped to whole texels (no shimmering when the
/// camera moves).
pub(crate) fn fit_shadow(cam: &CameraInfo, light_dir: Vec3, distance: f32, size: u32) -> Option<ShadowFit> {
    let dir = light_dir.try_normalize()?;
    let t = if cam.depth.is_finite() { (distance / cam.depth).min(1.0) } else { 0.0 };
    let mut points = [Vec3::ZERO; 8];
    for i in 0..4 {
        let (n, f) = (cam.near[i], cam.far[i]);
        let f = if f.is_finite() && t > 0.0 { n + (f - n) * t } else { n + (n - cam.eye).normalize_or_zero() * distance };
        points[i] = n;
        points[i + 4] = f;
    }
    let center = points.iter().copied().sum::<Vec3>() / 8.0;
    let radius = points.iter().map(|p| p.distance(center)).fold(0.0f32, f32::max);
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
    let proj = directx::orthographic(cx - radius, cx + radius, cy - radius, cy + radius, -c.z - radius - pad, -c.z + radius);
    Some(ShadowFit { view_proj: proj * light_view, texel_world: texel })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perspective_camera(eye: Vec3, target: Vec3) -> Mat4 {
        directx::perspective(60f32.to_radians(), 1.0, 0.1, 500.0) * view::look_at_mat4(eye, target, Vec3::Y)
    }

    #[test]
    fn camera_info_recovers_eye_and_depth() {
        let eye = Vec3::new(3.0, 2.0, 5.0);
        let info = CameraInfo::from_view_proj(perspective_camera(eye, Vec3::ZERO));
        assert!(info.eye.abs_diff_eq(eye, 1e-3), "{:?}", info.eye);
        assert_eq!(info.camera.w, 1.0);
        let depth = info.view_plane.truncate().dot(Vec3::ZERO) + info.view_plane.w;
        assert!((depth - eye.length()).abs() < 1e-3, "{depth}");

        let ortho = directx::orthographic(-5.0, 5.0, -5.0, 5.0, 0.1, 100.0) * view::look_at_mat4(eye, Vec3::ZERO, Vec3::Y);
        let info = CameraInfo::from_view_proj(ortho);
        assert_eq!(info.camera.w, 0.0);
        assert!(info.camera.truncate().abs_diff_eq(eye.normalize(), 1e-3));
    }

    #[test]
    fn shadow_fit_covers_the_near_slice_and_snaps_to_texels() {
        let cam = CameraInfo::from_view_proj(perspective_camera(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, 2.0, -10.0)));
        let dir = Vec3::new(-0.3, -1.0, -0.2);
        let fit = fit_shadow(&cam, dir, 60.0, 2048).unwrap();
        for p in [Vec3::new(0.0, 0.0, -5.0), Vec3::new(10.0, 0.0, -40.0)] {
            let c = fit.view_proj.project_point3(p);
            assert!(c.x.abs() <= 1.0 && c.y.abs() <= 1.0 && (0.0..=1.0).contains(&c.z), "{p} -> {c}");
        }
        // Moving the camera a fraction of a texel keeps the map's texel grid in place.
        let moved = CameraInfo::from_view_proj(perspective_camera(Vec3::new(0.001, 2.0, 0.0), Vec3::new(0.001, 2.0, -10.0)));
        let fit2 = fit_shadow(&moved, dir, 60.0, 2048).unwrap();
        let texel_ndc = 2.0 / 2048.0;
        let a = fit.view_proj.project_point3(Vec3::ZERO);
        let b = fit2.view_proj.project_point3(Vec3::ZERO);
        let dx = ((a.x - b.x) / texel_ndc).abs();
        assert!((dx - dx.round()).abs() < 1e-2, "offset is whole texels: {dx}");
    }

    #[test]
    fn light_list_keeps_the_best() {
        let mut list = LightList::new();
        for i in 0..40 {
            list.push(PackedLight { position_range: [i as f32, 0.0, 0.0, 1.0], ..Default::default() }, i as f32);
        }
        assert_eq!(list.len, MAX_LIGHTS);
        assert!(list.lights.iter().all(|l| l.position_range[0] < MAX_LIGHTS as f32));
    }
}

use glam::{Mat4, Vec3, Vec4};

/// Six inward-facing planes extracted from a view-projection matrix (depth 0..1).
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    pub fn from_view_proj(m: Mat4) -> Self {
        let (r0, r1, r2, r3) = (m.row(0), m.row(1), m.row(2), m.row(3));
        let planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2]
            .map(|p| p / p.truncate().length());
        Self { planes }
    }

    #[inline]
    pub fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.planes.iter().all(|p| p.truncate().dot(center) + p.w >= -radius)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::camera::rh::{proj::directx, view};

    #[test]
    fn culls_behind_and_beside_camera() {
        let proj = directx::perspective(60f32.to_radians(), 1.0, 0.1, 100.0);
        let view = view::look_at_mat4(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let f = Frustum::from_view_proj(proj * view);
        assert!(f.intersects_sphere(Vec3::new(0.0, 0.0, -10.0), 1.0));
        assert!(!f.intersects_sphere(Vec3::new(0.0, 0.0, 10.0), 1.0));
        assert!(!f.intersects_sphere(Vec3::new(50.0, 0.0, -10.0), 1.0));
        assert!(!f.intersects_sphere(Vec3::new(0.0, 0.0, -200.0), 1.0));
        // Sphere straddling the left plane is kept.
        assert!(f.intersects_sphere(Vec3::new(-6.2, 0.0, -10.0), 1.0));
    }
}

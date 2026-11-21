//! Engine-agnostic scene components. Plain data: no GPU, no window, no ECS dependency.

#[cfg(feature = "serde")]
pub mod anim;
#[cfg(feature = "serde")]
pub mod file;
#[cfg(feature = "serde")]
pub mod material;
pub mod primitives;

use glam::camera::rh::{proj::directx, view};
use glam::{Mat4, Quat, Vec3};

/// Object name from the scene.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Name(pub String);

/// Unity-style tag ("Player", "Respawn", ...).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tag(pub String);

/// Triangle geometry in the entity's local space, for mesh colliders.
#[derive(Clone, Debug, Default)]
pub struct MeshGeometry {
    pub positions: Vec<Vec3>,
    pub indices: Vec<[u32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }

    pub fn from_xyz(x: f32, y: f32, z: f32) -> Self {
        Self::from_translation(Vec3::new(x, y, z))
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// Rotates so that `forward()` points at `target`.
    pub fn looking_at(mut self, target: Vec3, up: Vec3) -> Self {
        self.look_to(target - self.translation, up);
        self
    }

    pub fn look_to(&mut self, direction: Vec3, up: Vec3) {
        // look_to_quat yields the view (world -> camera) rotation; objects need its inverse.
        self.rotation = view::look_to_quat(direction.normalize(), up).inverse();
    }

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// -Z, the direction cameras look.
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }
}

/// Perspective camera; the view comes from the entity's [`Transform`].
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            fov_y: 60f32.to_radians(),
            near: 0.1,
            far: 1000.0,
        }
    }
}

impl Camera {
    /// Right-handed, depth 0..1 (wgpu / Vulkan / Metal / D3D convention).
    pub fn projection(&self, aspect: f32) -> Mat4 {
        directx::perspective(self.fov_y, aspect, self.near, self.far)
    }

    pub fn view(transform: &Transform) -> Mat4 {
        Mat4::from_rotation_translation(transform.rotation, transform.translation).inverse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looking_at_points_forward_at_target() {
        let t = Transform::from_xyz(0.0, 5.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y);
        let expected = (Vec3::ZERO - t.translation).normalize();
        assert!(t.forward().abs_diff_eq(expected, 1e-5));
    }
}

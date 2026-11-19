//! Plain geometry and colour types the UI works in. Screen space: pixels, +Y down.

use glam::Vec2;

/// Axis-aligned rectangle (`min` top-left, `max` bottom-right).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn new(min: Vec2, size: Vec2) -> Self {
        Self {
            min,
            max: min + size,
        }
    }

    pub fn size(&self) -> Vec2 {
        (self.max - self.min).max(Vec2::ZERO)
    }

    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.x < self.max.x && p.y >= self.min.y && p.y < self.max.y
    }

    /// Shrinks every side by `amount` (never inverts).
    pub fn shrink(&self, amount: f32) -> Self {
        let min = self.min + Vec2::splat(amount);
        let max = self.max - Vec2::splat(amount);
        Self {
            min,
            max: max.max(min),
        }
    }
}

/// RGBA, 0..1. The UI never converts it: the backend decides whether it is
/// sRGB or linear (pick one convention in the theme and keep to it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub [f32; 4]);

impl Color {
    pub const TRANSPARENT: Color = Color([0.0; 4]);

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self([r, g, b, a])
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self([r, g, b, 1.0])
    }
}

/// A navigation direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    /// Quantises a stick or d-pad vector (+Y up, like `ActionState::axis2d`) to the
    /// dominant direction, or `None` below `threshold`.
    pub fn from_axis(v: Vec2, threshold: f32) -> Option<Dir> {
        if v.length() < threshold {
            return None;
        }
        Some(if v.x.abs() > v.y.abs() {
            if v.x > 0.0 { Dir::Right } else { Dir::Left }
        } else if v.y > 0.0 {
            Dir::Up
        } else {
            Dir::Down
        })
    }
}

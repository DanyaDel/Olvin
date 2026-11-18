//! Built-in meshes (Unity's 3D objects), shared by the engine and the editor
//! so both draw exactly the same geometry. Unit-sized like Unity's: cube 1,
//! sphere diameter 1, capsule 1 × 2, cylinder 1 × 2, plane 10 × 10, quad 1.

use std::f32::consts::{PI, TAU};

use glam::{Vec3, vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Shape {
    #[default]
    Cube,
    Sphere,
    Capsule,
    Cylinder,
    Plane,
    Quad,
}

impl Shape {
    pub const ALL: [Shape; 6] = [
        Shape::Cube,
        Shape::Sphere,
        Shape::Capsule,
        Shape::Cylinder,
        Shape::Plane,
        Shape::Quad,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Cube => "Cube",
            Shape::Sphere => "Sphere",
            Shape::Capsule => "Capsule",
            Shape::Cylinder => "Cylinder",
            Shape::Plane => "Plane",
            Shape::Quad => "Quad",
        }
    }

    /// Flat shapes are visible from both sides in the editor.
    pub fn is_flat(self) -> bool {
        matches!(self, Shape::Plane | Shape::Quad)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Primitive {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    /// Counter-clockwise from outside.
    pub indices: Vec<u32>,
    /// Outline for selection highlights: hard edges, or silhouettes of round shapes.
    pub outline: Vec<[Vec3; 2]>,
}

const SEGMENTS: usize = 24;

impl Primitive {
    pub fn new(shape: Shape) -> Self {
        let mut p = Self::default();
        match shape {
            Shape::Cube => p.cube(),
            Shape::Sphere => {
                p.lathe(&arc(0.5, 0.0, -FRAC_PI_2_F, FRAC_PI_2_F, 16));
                p.circles(0.5, &[0.0], true);
            }
            Shape::Capsule => {
                let mut profile = arc(0.5, -0.5, -FRAC_PI_2_F, 0.0, 8);
                profile.extend(arc(0.5, 0.5, 0.0, FRAC_PI_2_F, 8));
                p.lathe(&profile);
                p.circles(0.5, &[-0.5, 0.5], false);
                p.capsule_sides();
            }
            Shape::Cylinder => p.cylinder(),
            Shape::Plane => p.quad(10.0, false),
            Shape::Quad => p.quad(1.0, true),
        }
        p
    }

    fn push(&mut self, position: Vec3, normal: Vec3) -> u32 {
        self.positions.push(position);
        self.normals.push(normal);
        self.positions.len() as u32 - 1
    }

    fn cube(&mut self) {
        let faces = [
            (Vec3::X, Vec3::NEG_Z, Vec3::Y),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::X, Vec3::NEG_Z),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
        ];
        for (n, u, v) in faces {
            let quad = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .map(|(a, b)| (n + u * a + v * b) * 0.5);
            let base = self.positions.len() as u32;
            for q in quad {
                self.push(q, n);
            }
            self.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        let c = |i: usize| {
            vec3(
                if i & 1 != 0 { 0.5 } else { -0.5 },
                if i & 2 != 0 { 0.5 } else { -0.5 },
                if i & 4 != 0 { 0.5 } else { -0.5 },
            )
        };
        for i in 0..8 {
            for bit in [1, 2, 4] {
                if i & bit == 0 {
                    self.outline.push([c(i), c(i | bit)]);
                }
            }
        }
    }

    /// Plane on XZ facing +Y (`size` × `size`); a quad is the XY version facing -Z like Unity's.
    fn quad(&mut self, size: f32, xy: bool) {
        let h = size * 0.5;
        let (n, corners) = if xy {
            (
                Vec3::NEG_Z,
                [
                    vec3(h, -h, 0.0),
                    vec3(-h, -h, 0.0),
                    vec3(-h, h, 0.0),
                    vec3(h, h, 0.0),
                ],
            )
        } else {
            (
                Vec3::Y,
                [
                    vec3(-h, 0.0, h),
                    vec3(h, 0.0, h),
                    vec3(h, 0.0, -h),
                    vec3(-h, 0.0, -h),
                ],
            )
        };
        for c in corners {
            self.push(c, n);
        }
        self.indices.extend([0, 1, 2, 0, 2, 3]);
        for i in 0..4 {
            self.outline.push([corners[i], corners[(i + 1) % 4]]);
        }
    }

    /// Revolves a (radius, y) profile, bottom to top, around Y.
    fn lathe(&mut self, profile: &[(f32, f32, Vec3)]) {
        let base = self.positions.len() as u32;
        for &(r, y, n) in profile {
            for s in 0..=SEGMENTS {
                let a = s as f32 / SEGMENTS as f32 * TAU;
                let (sin, cos) = a.sin_cos();
                self.push(
                    vec3(r * cos, y, r * sin),
                    vec3(n.x * cos, n.y, n.x * sin).normalize_or_zero(),
                );
            }
        }
        let row = SEGMENTS as u32 + 1;
        for i in 0..profile.len() as u32 - 1 {
            for s in 0..SEGMENTS as u32 {
                let (a, b) = (base + i * row + s, base + (i + 1) * row + s);
                self.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
    }

    fn cylinder(&mut self) {
        self.lathe(&[(0.5, -1.0, Vec3::X), (0.5, 1.0, Vec3::X)]);
        for (y, n) in [(-1.0, Vec3::NEG_Y), (1.0, Vec3::Y)] {
            let center = self.push(vec3(0.0, y, 0.0), n);
            for s in 0..SEGMENTS as u32 {
                let a = |s: u32| s as f32 / SEGMENTS as f32 * TAU;
                let i0 = self.push(vec3(0.5 * a(s).cos(), y, 0.5 * a(s).sin()), n);
                let i1 = self.push(vec3(0.5 * a(s + 1).cos(), y, 0.5 * a(s + 1).sin()), n);
                if n.y > 0.0 {
                    self.indices.extend([center, i1, i0]);
                } else {
                    self.indices.extend([center, i0, i1]);
                }
            }
        }
        self.circles(0.5, &[-1.0, 1.0], false);
        for (x, z) in [(0.5, 0.0), (-0.5, 0.0), (0.0, 0.5), (0.0, -0.5)] {
            self.outline.push([vec3(x, -1.0, z), vec3(x, 1.0, z)]);
        }
    }

    fn capsule_sides(&mut self) {
        for (x, z) in [(0.5, 0.0), (-0.5, 0.0), (0.0, 0.5), (0.0, -0.5)] {
            self.outline.push([vec3(x, -0.5, z), vec3(x, 0.5, z)]);
        }
        // End caps' silhouettes.
        for (cy, from, to) in [(0.5, 0.0, PI), (-0.5, PI, TAU)] {
            for plane_xz in [true, false] {
                let pts: Vec<Vec3> = (0..=12)
                    .map(|i| {
                        let a = from + (to - from) * i as f32 / 12.0;
                        let (s, c) = a.sin_cos();
                        if plane_xz {
                            vec3(c * 0.5, cy + s * 0.5, 0.0)
                        } else {
                            vec3(0.0, cy + s * 0.5, c * 0.5)
                        }
                    })
                    .collect();
                self.outline.extend(pts.windows(2).map(|w| [w[0], w[1]]));
            }
        }
    }

    /// Horizontal outline circles at these heights; `vertical` adds two upright ones.
    fn circles(&mut self, r: f32, heights: &[f32], vertical: bool) {
        let ring = |f: &dyn Fn(f32) -> Vec3| -> Vec<[Vec3; 2]> {
            (0..48)
                .map(|i| [f(i as f32 / 48.0 * TAU), f((i + 1) as f32 / 48.0 * TAU)])
                .collect()
        };
        for &y in heights {
            self.outline
                .extend(ring(&|a| vec3(r * a.cos(), y, r * a.sin())));
        }
        if vertical {
            self.outline
                .extend(ring(&|a| vec3(r * a.cos(), r * a.sin(), 0.0)));
            self.outline
                .extend(ring(&|a| vec3(0.0, r * a.sin(), r * a.cos())));
        }
    }
}

const FRAC_PI_2_F: f32 = PI / 2.0;

/// Profile points (radius, y, normal) of a circular arc centred at (0, cy).
fn arc(r: f32, cy: f32, from: f32, to: f32, steps: usize) -> Vec<(f32, f32, Vec3)> {
    (0..=steps)
        .map(|i| {
            let a = from + (to - from) * i as f32 / steps as f32;
            let n = vec3(a.cos(), a.sin(), 0.0);
            (r * a.cos(), cy + r * a.sin(), n)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_are_outward_and_sized_like_unity() {
        for shape in Shape::ALL {
            let p = Primitive::new(shape);
            assert!(
                !p.indices.is_empty() && p.indices.len().is_multiple_of(3),
                "{shape:?}"
            );
            for t in p.indices.chunks(3) {
                let [a, b, c] = [0, 1, 2].map(|i| p.positions[t[i] as usize]);
                let n =
                    p.normals[t[0] as usize] + p.normals[t[1] as usize] + p.normals[t[2] as usize];
                let face = (b - a).cross(c - a);
                if face.length() > 1e-7 {
                    assert!(
                        face.dot(n) > 0.0,
                        "{shape:?}: winding disagrees with normals"
                    );
                }
            }
            let max = p.positions.iter().fold(Vec3::ZERO, |m, v| m.max(v.abs()));
            let expected = match shape {
                Shape::Cube | Shape::Sphere => vec3(0.5, 0.5, 0.5),
                Shape::Capsule | Shape::Cylinder => vec3(0.5, 1.0, 0.5),
                Shape::Plane => vec3(5.0, 0.0, 5.0),
                Shape::Quad => vec3(0.5, 0.5, 0.0),
            };
            assert!(max.abs_diff_eq(expected, 1e-4), "{shape:?}: {max}");
        }
    }
}

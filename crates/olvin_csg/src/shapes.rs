//! Brush shapes in local space: the unit cube `[-0.5, 0.5]³`, so a brush's
//! size is simply its scale. Each shape is one or more convex pieces; each
//! piece is a list of faces (vertex loops, counter-clockwise from outside).

use std::f32::consts::TAU;

use glam::{Vec3, vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BrushShape {
    Box,
    /// Ramp: full bottom and back (-Z) faces, slope down towards +Z.
    Wedge,
    Cylinder { sides: u32 },
    Pyramid { sides: u32 },
    /// Steps rise towards -Z.
    Stairs { steps: u32 },
}

impl BrushShape {
    pub const MIN_SIDES: u32 = 3;
    pub const MAX_SIDES: u32 = 64;
    pub const MAX_STEPS: u32 = 64;

    pub fn label(self) -> &'static str {
        match self {
            Self::Box => "Box",
            Self::Wedge => "Wedge",
            Self::Cylinder { .. } => "Cylinder",
            Self::Pyramid { .. } => "Pyramid",
            Self::Stairs { .. } => "Stairs",
        }
    }
}

pub type Face = Vec<Vec3>;
pub type Piece = Vec<Face>;

pub fn pieces(shape: BrushShape) -> Vec<Piece> {
    match shape {
        BrushShape::Box => vec![cuboid(Vec3::splat(-0.5), Vec3::splat(0.5))],
        BrushShape::Wedge => vec![wedge()],
        BrushShape::Cylinder { sides } => vec![prism(sides.clamp(BrushShape::MIN_SIDES, BrushShape::MAX_SIDES))],
        BrushShape::Pyramid { sides } => vec![pyramid(sides.clamp(BrushShape::MIN_SIDES, BrushShape::MAX_SIDES))],
        BrushShape::Stairs { steps } => {
            let k = steps.clamp(1, BrushShape::MAX_STEPS) as f32;
            (0..steps.clamp(1, BrushShape::MAX_STEPS))
                .map(|s| {
                    let s = s as f32;
                    cuboid(vec3(-0.5, -0.5 + s / k, -0.5), vec3(0.5, -0.5 + (s + 1.0) / k, 0.5 - s / k))
                })
                .collect()
        }
    }
}

fn cuboid(min: Vec3, max: Vec3) -> Piece {
    let c = |x: bool, y: bool, z: bool| {
        vec3(if x { max.x } else { min.x }, if y { max.y } else { min.y }, if z { max.z } else { min.z })
    };
    let (f, t) = (false, true);
    vec![
        vec![c(f, f, f), c(f, f, t), c(f, t, t), c(f, t, f)], // -X
        vec![c(t, f, f), c(t, t, f), c(t, t, t), c(t, f, t)], // +X
        vec![c(f, f, f), c(t, f, f), c(t, f, t), c(f, f, t)], // -Y
        vec![c(f, t, f), c(f, t, t), c(t, t, t), c(t, t, f)], // +Y
        vec![c(f, f, f), c(f, t, f), c(t, t, f), c(t, f, f)], // -Z
        vec![c(f, f, t), c(t, f, t), c(t, t, t), c(f, t, t)], // +Z
    ]
}

fn wedge() -> Piece {
    let (b0, b1, b2, b3) = (vec3(-0.5, -0.5, -0.5), vec3(0.5, -0.5, -0.5), vec3(0.5, -0.5, 0.5), vec3(-0.5, -0.5, 0.5));
    let (t0, t1) = (vec3(-0.5, 0.5, -0.5), vec3(0.5, 0.5, -0.5));
    vec![
        vec![b0, b1, b2, b3], // bottom
        vec![b0, t0, t1, b1], // back
        vec![t0, b3, b2, t1], // slope
        vec![b0, b3, t0],     // left
        vec![b1, t1, b2],     // right
    ]
}

fn ring(sides: u32, y: f32) -> Vec<Vec3> {
    (0..sides)
        .map(|i| {
            // Rotated half a step so a 4-sided "cylinder" is axis-aligned.
            let a = (i as f32 + 0.5) / sides as f32 * TAU;
            vec3(a.cos() * 0.5, y, a.sin() * 0.5)
        })
        .collect()
}

fn prism(sides: u32) -> Piece {
    let bottom = ring(sides, -0.5);
    let top = ring(sides, 0.5);
    let mut faces = vec![bottom.clone(), top.iter().rev().copied().collect()];
    for i in 0..sides as usize {
        let j = (i + 1) % sides as usize;
        faces.push(vec![bottom[i], top[i], top[j], bottom[j]]);
    }
    faces
}

fn pyramid(sides: u32) -> Piece {
    let base = ring(sides, -0.5);
    let apex = vec3(0.0, 0.5, 0.0);
    let mut faces = vec![base.clone()];
    for i in 0..sides as usize {
        let j = (i + 1) % sides as usize;
        faces.push(vec![base[i], apex, base[j]]);
    }
    faces
}

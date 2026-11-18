use std::collections::HashMap;

use glam::{IVec3, Vec3};

use crate::PolyMesh;
use crate::merge::clean_loop;

impl PolyMesh {
    /// Axis-aligned cube centred at the origin with edge length `size`.
    pub fn cube(size: f32) -> Self {
        let h = size * 0.5;
        // Vertex i has x, y, z = bits 0, 1, 2 of i.
        let positions = (0..8)
            .map(|i| {
                let s = |bit: u32| if i & (1 << bit) != 0 { h } else { -h };
                Vec3::new(s(0), s(1), s(2))
            })
            .collect();
        let faces = vec![
            vec![1, 3, 7, 5], // +X
            vec![0, 4, 6, 2], // -X
            vec![2, 6, 7, 3], // +Y
            vec![0, 1, 5, 4], // -Y
            vec![4, 5, 7, 6], // +Z
            vec![0, 2, 3, 1], // -Z
        ];
        Self {
            positions,
            faces,
            ..Default::default()
        }
    }

    /// Square in the XZ plane facing +Y, centred at the origin.
    pub fn plane(size: f32) -> Self {
        let h = size * 0.5;
        Self {
            positions: vec![
                Vec3::new(-h, 0.0, -h),
                Vec3::new(-h, 0.0, h),
                Vec3::new(h, 0.0, h),
                Vec3::new(h, 0.0, -h),
            ],
            faces: vec![vec![0, 1, 2, 3]],
            ..Default::default()
        }
    }

    /// Builds a mesh from loose polygons (CCW seen from outside), welding
    /// vertices closer than `weld_eps`. Polygons that collapse below three
    /// distinct vertices are dropped.
    pub fn from_polygons(polys: &[Vec<Vec3>], weld_eps: f32) -> Self {
        let mut welder = Welder::new(weld_eps);
        let mut mesh = Self::default();
        for poly in polys {
            let mut face: Vec<u32> = poly
                .iter()
                .map(|&p| welder.index(&mut mesh.positions, p))
                .collect();
            clean_loop(&mut face);
            if face.len() >= 3 {
                mesh.faces.push(face);
            }
        }
        mesh
    }
}

/// Spatial hash that snaps points to an existing vertex within `eps`.
pub(crate) struct Welder {
    eps: f32,
    cells: HashMap<IVec3, Vec<u32>>,
}

impl Welder {
    pub(crate) fn new(eps: f32) -> Self {
        Self {
            eps: eps.max(0.0),
            cells: HashMap::new(),
        }
    }

    fn cell(&self, p: Vec3) -> IVec3 {
        if self.eps > 0.0 {
            (p / self.eps).floor().as_ivec3()
        } else {
            IVec3::new(
                p.x.to_bits() as i32,
                p.y.to_bits() as i32,
                p.z.to_bits() as i32,
            )
        }
    }

    /// Existing vertex within `eps` of `p`, if any.
    pub(crate) fn find(&self, positions: &[Vec3], p: Vec3) -> Option<u32> {
        let c = self.cell(p);
        let range = if self.eps > 0.0 { -1..=1 } else { 0..=0 };
        for dx in range.clone() {
            for dy in range.clone() {
                for dz in range.clone() {
                    let Some(list) = self.cells.get(&c.wrapping_add(IVec3::new(dx, dy, dz))) else {
                        continue;
                    };
                    if let Some(&i) = list
                        .iter()
                        .find(|&&i| positions[i as usize].distance(p) <= self.eps)
                    {
                        return Some(i);
                    }
                }
            }
        }
        None
    }

    pub(crate) fn insert(&mut self, p: Vec3, index: u32) {
        let c = self.cell(p);
        self.cells.entry(c).or_default().push(index);
    }

    fn index(&mut self, positions: &mut Vec<Vec3>, p: Vec3) -> u32 {
        if let Some(i) = self.find(positions, p) {
            return i;
        }
        let i = positions.len() as u32;
        positions.push(p);
        self.insert(p, i);
        i
    }
}

//! Levels of detail: simpler copies of a mesh for objects far from the camera.
//!
//! [`MeshData::simplified`] makes a copy with fewer triangles (meshoptimizer),
//! [`Meshes::add_with_lods`] registers a mesh together with its copies, and the
//! renderer picks one per object from how big the object looks on screen: the
//! fraction of the screen's height that its bounding sphere's diameter covers (see
//! [`MeshLod::below`]). No authoring: the same level looks right at any
//! resolution or field of view.

use bytemuck::cast_slice;
use meshopt::{SimplifyOptions, VertexDataAdapter};

use crate::{MeshData, MeshHandle, Meshes, Vertex};

/// One simpler copy of a mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshLod {
    pub mesh: MeshHandle,
    /// The copy is drawn while the object's bounding sphere covers less than
    /// this fraction of the screen height (1.0 = the whole screen). Levels are
    /// tried from the largest `below` down; the smallest that still exceeds
    /// the object's size wins.
    pub below: f32,
}

/// The recipe of one level: how much geometry to keep and from what size on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodSpec {
    /// Share of the triangles kept, 0..1.
    pub keep: f32,
    pub below: f32,
}

/// Three levels that suit most props: half the triangles from a quarter of
/// the screen's height down, a quarter of them below a tenth.
pub const DEFAULT_LODS: [LodSpec; 2] = [
    LodSpec {
        keep: 0.5,
        below: 0.25,
    },
    LodSpec {
        keep: 0.2,
        below: 0.08,
    },
];

/// Meshes with fewer triangles than this are not worth simplifying.
pub const MIN_TRIANGLES_FOR_LOD: usize = 120;

impl MeshData {
    /// A copy with about `keep` (0..1) of the triangles, never straying from
    /// the surface by more than `max_error` (a fraction of the mesh's size).
    /// Never empties the mesh; an already small or unsimplifiable mesh comes
    /// back unchanged. Vertices keep their normals, UVs and tangents.
    pub fn simplified(&self, keep: f32, max_error: f32) -> MeshData {
        let triangles = self.indices.len() / 3;
        if triangles < 2 || self.vertices.is_empty() || !keep.is_finite() {
            return self.clone();
        }
        let target = (((triangles as f32 * keep.clamp(0.0, 1.0)) as usize).max(1)) * 3;
        let Ok(adapter) = VertexDataAdapter::new(
            cast_slice::<Vertex, u8>(&self.vertices),
            std::mem::size_of::<Vertex>(),
            0,
        ) else {
            return self.clone();
        };
        let indices = meshopt::simplify(
            &self.indices,
            &adapter,
            target,
            max_error.clamp(0.0, 1.0),
            SimplifyOptions::None,
            None,
        );
        if indices.len() < 3 || indices.len() >= self.indices.len() {
            return self.clone();
        }
        // Keep only the vertices the new triangles use.
        let mut remap = vec![u32::MAX; self.vertices.len()];
        let mut vertices = Vec::new();
        let indices = indices
            .iter()
            .map(|&i| {
                let slot = &mut remap[i as usize];
                if *slot == u32::MAX {
                    *slot = vertices.len() as u32;
                    vertices.push(self.vertices[i as usize]);
                }
                *slot
            })
            .collect();
        MeshData { vertices, indices }
    }
}

impl Meshes {
    /// Registers `mesh` with simpler copies made from `specs`
    /// ([`DEFAULT_LODS`]): the copies are separate meshes, and the returned
    /// handle (the full-detail one) is what a `MeshRenderer` points at.
    /// Meshes under [`MIN_TRIANGLES_FOR_LOD`] triangles get no levels, and
    /// neither does a level that came out no smaller than the one before.
    pub fn add_with_lods(&mut self, mesh: MeshData, specs: &[LodSpec]) -> MeshHandle {
        let mut levels = Vec::new();
        if mesh.indices.len() / 3 >= MIN_TRIANGLES_FOR_LOD {
            let mut previous = mesh.indices.len();
            for spec in specs {
                let copy = mesh.simplified(spec.keep, 0.05);
                if copy.indices.len() >= previous {
                    continue;
                }
                previous = copy.indices.len();
                levels.push((copy, spec.below));
            }
        }
        let base = self.add(mesh);
        let lods = levels
            .into_iter()
            .map(|(copy, below)| MeshLod {
                mesh: self.add(copy),
                below,
            })
            .collect();
        self.set_lods(base, lods);
        base
    }

    /// Sets the levels of `base` (see [`MeshLod`]); an empty list removes them.
    /// Replacing the geometry with [`Meshes::set`] also removes them.
    pub fn set_lods(&mut self, base: MeshHandle, lods: Vec<MeshLod>) {
        self.lod_updates.retain(|(h, _)| *h != base);
        self.lod_updates.push((base, lods));
    }
}

/// The mesh slot to draw for an object covering `size` of the screen height.
/// `lods` are sorted by `below`, largest first. `bias` scales the size
/// (above 1 keeps detail longer; infinity always picks the full mesh).
pub(crate) fn pick(base: u32, lods: &[(u32, f32)], size: f32, bias: f32) -> u32 {
    let size = size * bias;
    let mut chosen = base;
    for &(mesh, below) in lods {
        if size < below {
            chosen = mesh;
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    /// A latitude-longitude sphere: lots of triangles that can be dropped.
    fn sphere(rings: u32, segments: u32) -> MeshData {
        let mut mesh = MeshData::default();
        for r in 0..=rings {
            let theta = std::f32::consts::PI * r as f32 / rings as f32;
            for s in 0..=segments {
                let phi = std::f32::consts::TAU * s as f32 / segments as f32;
                let n = Vec3::new(
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                );
                mesh.vertices.push(Vertex {
                    position: n.into(),
                    normal: n.into(),
                    uv: [s as f32 / segments as f32, r as f32 / rings as f32],
                    tangent: [1.0, 0.0, 0.0, 1.0],
                });
            }
        }
        for r in 0..rings {
            for s in 0..segments {
                let a = r * (segments + 1) + s;
                let b = a + segments + 1;
                mesh.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        mesh
    }

    #[test]
    fn simplified_has_fewer_triangles_on_the_same_shape() {
        let full = sphere(32, 48);
        let half = full.simplified(0.25, 0.05);
        assert!(
            half.indices.len() < full.indices.len() / 2,
            "{}",
            half.indices.len()
        );
        assert!(half.indices.len() >= 3);
        assert!(
            half.indices
                .iter()
                .all(|&i| (i as usize) < half.vertices.len())
        );
        // The copy stays on the unit sphere's surface, within the error asked.
        let (_, radius) = half.bounding_sphere();
        assert!((radius - 1.0).abs() < 0.1, "{radius}");
        // Compacted: no unused vertices.
        let mut used = vec![false; half.vertices.len()];
        half.indices.iter().for_each(|&i| used[i as usize] = true);
        assert!(used.iter().all(|u| *u));
    }

    #[test]
    fn tiny_or_degenerate_meshes_come_back_unchanged() {
        let quad = MeshData::plane(1.0);
        assert_eq!(quad.simplified(0.1, 0.05).indices, quad.indices);
        let empty = MeshData::default();
        assert!(empty.simplified(0.5, 0.05).indices.is_empty());
        let full = sphere(16, 24);
        assert_eq!(full.simplified(f32::NAN, 0.05).indices, full.indices);
    }

    #[test]
    fn add_with_lods_registers_smaller_copies_only_for_big_meshes() {
        let mut meshes = Meshes::default();
        let big = meshes.add_with_lods(sphere(32, 48), &DEFAULT_LODS);
        let (_, lods) = meshes.lod_updates.iter().find(|(h, _)| *h == big).unwrap();
        assert_eq!(lods.len(), 2);
        assert!(lods[0].below > lods[1].below);
        let small = meshes.add_with_lods(MeshData::cube(1.0), &DEFAULT_LODS);
        let (_, none) = meshes
            .lod_updates
            .iter()
            .find(|(h, _)| *h == small)
            .unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn pick_follows_the_screen_size() {
        let lods = [(11, 0.25), (12, 0.08)];
        assert_eq!(pick(10, &lods, 0.5, 1.0), 10, "big on screen: full detail");
        assert_eq!(pick(10, &lods, 0.2, 1.0), 11);
        assert_eq!(pick(10, &lods, 0.05, 1.0), 12);
        assert_eq!(pick(10, &lods, 0.05, f32::INFINITY), 10, "bias off");
        assert_eq!(pick(10, &lods, 0.2, 0.0), 12, "bias 0: lowest level");
        assert_eq!(pick(10, &[], 0.001, 1.0), 10, "no levels");
    }
}

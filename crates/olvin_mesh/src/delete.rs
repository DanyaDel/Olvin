use std::collections::HashSet;

use glam::Vec2;

use crate::merge::clean_loop_with;
use crate::query::find_directed;
use crate::{PolyMesh, Remap, edge_key, loop_edges};

impl PolyMesh {
    /// Removes the flagged faces. Their edges that no other face uses become
    /// wire edges unless `keep_edge` rejects them.
    fn remove_faces_to_wire(
        &mut self,
        remove: &[bool],
        keep_edge: impl Fn([u32; 2]) -> bool,
    ) -> Vec<Option<usize>> {
        let freed: Vec<[u32; 2]> = self
            .faces
            .iter()
            .zip(remove)
            .filter(|(_, r)| **r)
            .flat_map(|(f, _)| {
                loop_edges(f)
                    .map(|(a, b)| edge_key(a, b))
                    .collect::<Vec<_>>()
            })
            .filter(|&e| keep_edge(e))
            .collect();
        let map = self.remove_faces_mask(remove);
        self.wire.extend(freed);
        self.normalize_wire();
        map
    }

    /// Deletes vertices together with every face and edge using them. Other
    /// edges of the removed faces stay as wire edges (Blender behaviour).
    pub fn delete_vertices(&mut self, verts: &[u32]) -> Remap {
        let mut remove_v = vec![false; self.positions.len()];
        for &v in verts {
            if let Some(r) = remove_v.get_mut(v as usize) {
                *r = true;
            }
        }
        let gone = |v: u32| remove_v[v as usize];
        let remove_f: Vec<bool> = self
            .faces
            .iter()
            .map(|f| f.iter().any(|&v| gone(v)))
            .collect();
        let faces = self.remove_faces_to_wire(&remove_f, |[a, b]| !gone(a) && !gone(b));
        self.wire.retain(|&[a, b]| !gone(a) && !gone(b));
        let vmap = self.remove_vertices_mask(&remove_v);
        Remap {
            vertices: vmap.vertices,
            faces,
        }
    }

    /// Deletes edges and the faces using them. The faces' other edges stay
    /// as wire edges; endpoints left without any edge are removed.
    pub fn delete_edges(&mut self, edges: &[[u32; 2]]) -> Remap {
        let set: HashSet<[u32; 2]> = edges.iter().map(|&[a, b]| edge_key(a, b)).collect();
        let remove_f: Vec<bool> = self
            .faces
            .iter()
            .map(|f| loop_edges(f).any(|(a, b)| set.contains(&edge_key(a, b))))
            .collect();
        let faces = self.remove_faces_to_wire(&remove_f, |e| !set.contains(&e));
        self.wire.retain(|e| !set.contains(e));
        let vmap = self.remove_if_unreferenced(set.iter().flatten().copied());
        Remap {
            vertices: vmap.vertices,
            faces,
        }
    }

    /// Deletes faces. With `keep_edges` only the polygons go and their
    /// border becomes wire edges (Blender "Only Faces"); otherwise edges and
    /// vertices no longer used by anything are removed too.
    pub fn delete_faces(&mut self, faces: &[usize], keep_edges: bool) -> Remap {
        let mut remove_f = vec![false; self.faces.len()];
        for &f in faces {
            if let Some(r) = remove_f.get_mut(f) {
                *r = true;
            }
        }
        let touched: Vec<u32> = self
            .faces
            .iter()
            .zip(&remove_f)
            .filter(|(_, r)| **r)
            .flat_map(|(f, _)| f.iter().copied())
            .collect();
        let face_map = self.remove_faces_to_wire(&remove_f, |_| keep_edges);
        let vertices = if keep_edges {
            Remap::identity(self.positions.len(), 0).vertices
        } else {
            self.remove_if_unreferenced(touched).vertices
        };
        Remap {
            vertices,
            faces: face_map,
        }
    }

    /// Dissolves edges: the two faces sharing each edge merge into one,
    /// regardless of whether they are coplanar. Wire edges are deleted;
    /// border and non-manifold edges are left alone. Vertices are never
    /// removed, so the remap only renumbers faces.
    pub fn dissolve_edges(&mut self, edges: &[[u32; 2]]) -> Remap {
        let mut dead = vec![false; self.faces.len()];
        for &[a, b] in edges {
            let key = edge_key(a, b);
            if let Some(i) = self.wire.iter().position(|&e| e == key) {
                self.wire.swap_remove(i);
                continue;
            }
            let users: Vec<usize> = self
                .faces_with_edge(a, b)
                .into_iter()
                .filter(|&f| !dead[f])
                .collect();
            let &[f1, f2] = users.as_slice() else {
                continue;
            };
            let (a, b) = if find_directed(&self.faces[f1], a, b).is_some() {
                (a, b)
            } else {
                (b, a)
            };
            let has_uvs = self.has_uvs();
            let mut other = self.faces[f2].clone();
            let mut other_uv = if has_uvs {
                self.uvs[f2].clone()
            } else {
                Vec::new()
            };
            if find_directed(&other, b, a).is_none() {
                other.reverse();
                other_uv.reverse();
            }
            // f1 from b round to a, then f2's vertices strictly between a and b.
            let start = |f: &[u32], v: u32| f.iter().position(|&w| w == v).unwrap_or(0);
            let rotate = |n: usize, i: usize| (i..n).chain(0..i).collect::<Vec<usize>>();
            let order1 = rotate(self.faces[f1].len(), start(&self.faces[f1], b));
            let order2 = rotate(other.len(), start(&other, a));
            let inner2 = &order2[1..order2.len() - 1];
            let mut merged: Vec<u32> = order1.iter().map(|&i| self.faces[f1][i]).collect();
            merged.extend(inner2.iter().map(|&i| other[i]));
            let mut merged_uv: Vec<Vec2> = Vec::new();
            if has_uvs {
                merged_uv.extend(order1.iter().map(|&i| self.uvs[f1][i]));
                merged_uv.extend(inner2.iter().map(|&i| other_uv[i]));
            }
            clean_loop_with(&mut merged, |i| {
                if has_uvs {
                    merged_uv.remove(i);
                }
            });
            if merged.len() >= 3 {
                self.faces[f1] = merged;
                if has_uvs {
                    self.uvs[f1] = merged_uv;
                }
            } else {
                dead[f1] = true;
            }
            dead[f2] = true;
        }
        let faces = self.remove_faces_mask(&dead);
        self.normalize_wire();
        Remap {
            vertices: Remap::identity(self.positions.len(), 0).vertices,
            faces,
        }
    }

    /// Removes vertices that no face or wire edge uses. Returns the old ->
    /// new vertex table.
    pub fn compact(&mut self) -> Vec<Option<u32>> {
        let unused = self.unreferenced();
        self.remove_vertices_mask(&unused).vertices
    }
}

use std::collections::HashSet;

use glam::Vec3;

use crate::build::Welder;
use crate::{PolyMesh, Remap, edge_key, loop_edges};

/// Removes consecutive duplicates (cyclically) and spikes `x, y, x`, which
/// merging and dissolving can produce. The result may have fewer than 3
/// vertices; callers drop it then.
pub(crate) fn clean_loop(face: &mut Vec<u32>) {
    clean_loop_with(face, |_| {});
}

/// [`clean_loop`] that reports every removed position (in removal order,
/// each relative to the loop at that moment) so parallel data can follow.
pub(crate) fn clean_loop_with(face: &mut Vec<u32>, mut removed: impl FnMut(usize)) {
    loop {
        let n = face.len();
        if n < 2 {
            return;
        }
        if let Some(i) = (0..n).find(|&i| face[i] == face[(i + 1) % n]) {
            face.remove(i);
            removed(i);
            continue;
        }
        if n >= 3
            && let Some(i) = (0..n).find(|&i| face[i] == face[(i + 2) % n])
        {
            // Drop the spike tip and one copy of its base.
            let tip = (i + 1) % n;
            let (hi, lo) = if tip > i { (tip, i) } else { (i, tip) };
            face.remove(hi);
            removed(hi);
            face.remove(lo);
            removed(lo);
            continue;
        }
        return;
    }
}

impl PolyMesh {
    /// Redirects every use of vertex `v` to `map[v]`, cleans the faces,
    /// turns collapsed faces into wire edges and removes the vertices that
    /// were merged away.
    fn apply_merge(&mut self, map: &[u32]) -> Remap {
        let mut collapsed = Vec::new();
        let mut remove_f = vec![false; self.faces.len()];
        let has_uvs = self.has_uvs();
        for (fi, (face, removed)) in self.faces.iter_mut().zip(&mut remove_f).enumerate() {
            for v in face.iter_mut() {
                *v = map[*v as usize];
            }
            let edges: Vec<[u32; 2]> = loop_edges(face).map(|(a, b)| edge_key(a, b)).collect();
            let uv = if has_uvs {
                Some(&mut self.uvs[fi])
            } else {
                None
            };
            let mut uv = uv;
            clean_loop_with(face, |i| {
                if let Some(uv) = uv.as_mut() {
                    uv.remove(i);
                }
            });
            if face.iter().collect::<HashSet<_>>().len() < 3 {
                // Keep the folded face's edges, as Blender does.
                *removed = true;
                collapsed.extend(edges);
            }
        }
        let faces = self.remove_faces_mask(&remove_f);
        for e in self.wire.iter_mut().chain(&mut self.seams) {
            *e = edge_key(map[e[0] as usize], map[e[1] as usize]);
        }
        self.wire.extend(collapsed);
        self.normalize_wire();
        let merged_away: Vec<bool> = map
            .iter()
            .enumerate()
            .map(|(i, &t)| t as usize != i)
            .collect();
        let vmap = self.remove_vertices_mask(&merged_away);
        Remap {
            vertices: vmap.vertices,
            faces,
        }
    }

    /// Collapses `verts` into one vertex placed at `at` (Blender `M`).
    /// Degenerate faces and duplicate edges are cleaned up. Returns the new
    /// index of the merged vertex, or `None` if `verts` holds no valid vertex.
    pub fn merge_vertices(&mut self, verts: &[u32], at: Vec3) -> Option<(u32, Remap)> {
        let n = self.positions.len();
        let verts: Vec<u32> = verts
            .iter()
            .copied()
            .filter(|&v| (v as usize) < n)
            .collect();
        let target = *verts.iter().min()?;
        let mut map: Vec<u32> = (0..n as u32).collect();
        for &v in &verts {
            map[v as usize] = target;
        }
        self.positions[target as usize] = at;
        let remap = self.apply_merge(&map);
        let new = remap.vertex(target)?;
        Some((new, remap))
    }

    /// Merges vertices closer than `eps` into the lowest-indexed one of each
    /// cluster, keeping its position. Returns how many vertices were removed.
    pub fn merge_by_distance(&mut self, eps: f32) -> (usize, Remap) {
        let mut welder = Welder::new(eps);
        let mut map = Vec::with_capacity(self.positions.len());
        for (i, &p) in self.positions.iter().enumerate() {
            match welder.find(&self.positions, p) {
                Some(t) => map.push(t),
                None => {
                    welder.insert(p, i as u32);
                    map.push(i as u32);
                }
            }
        }
        let removed = map
            .iter()
            .enumerate()
            .filter(|&(i, &t)| t as usize != i)
            .count();
        if removed == 0 {
            return (0, Remap::identity(self.positions.len(), self.faces.len()));
        }
        (removed, self.apply_merge(&map))
    }

    /// Moves the given vertices (duplicates ignored) by `delta`.
    pub fn translate_vertices(&mut self, verts: &[u32], delta: Vec3) {
        let mut seen = HashSet::new();
        for &v in verts {
            if seen.insert(v)
                && let Some(p) = self.positions.get_mut(v as usize)
            {
                *p += delta;
            }
        }
    }
}

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use glam::Vec3;

use crate::query::find_directed;
use crate::{PolyMesh, edge_key, loop_edges};

/// Unique valid face indices, in input order.
pub(crate) fn unique_faces(mesh: &PolyMesh, faces: &[usize]) -> Vec<usize> {
    let mut seen = HashSet::new();
    faces
        .iter()
        .copied()
        .filter(|&f| f < mesh.faces.len() && seen.insert(f))
        .collect()
}

impl PolyMesh {
    /// Region extrude (Blender `E` on faces). Connected selected faces move
    /// together by `offset`; side quads are built only along the boundary of
    /// the selection, so no walls appear between selected neighbours. The
    /// original faces are moved, not copied: like Blender, extruding a lone
    /// plane leaves its bottom open. Returns the moved faces (same indices).
    pub fn extrude_faces(&mut self, faces: &[usize], offset: Vec3) -> Vec<usize> {
        let selected = unique_faces(self, faces);
        let sel_set: HashSet<usize> = selected.iter().copied().collect();

        let mut edge_uses: HashMap<[u32; 2], u32> = HashMap::new();
        for &f in &selected {
            for (a, b) in loop_edges(&self.faces[f]) {
                *edge_uses.entry(edge_key(a, b)).or_default() += 1;
            }
        }
        let is_boundary = |a: u32, b: u32| edge_uses.get(&edge_key(a, b)) == Some(&1);

        // A vertex can move in place only if nothing outside the region uses it.
        let mut detached: HashSet<u32> = self.wire.iter().flatten().copied().collect();
        for (i, face) in self.faces.iter().enumerate() {
            if !sel_set.contains(&i) {
                detached.extend(face.iter().copied());
            }
        }
        for &f in &selected {
            for (a, b) in loop_edges(&self.faces[f]) {
                if is_boundary(a, b) {
                    detached.insert(a);
                    detached.insert(b);
                }
            }
        }

        let mut top: HashMap<u32, u32> = HashMap::new();
        let mut moved: HashSet<u32> = HashSet::new();
        for &f in &selected {
            for i in 0..self.faces[f].len() {
                let v = self.faces[f][i];
                if detached.contains(&v) {
                    if let Entry::Vacant(slot) = top.entry(v) {
                        slot.insert(self.add_vertex(self.positions[v as usize] + offset));
                    }
                } else if moved.insert(v) {
                    self.positions[v as usize] += offset;
                }
            }
        }

        let mut walls = Vec::new();
        for &f in &selected {
            for (a, b) in loop_edges(&self.faces[f]) {
                if is_boundary(a, b) {
                    walls.push(vec![a, b, top[&b], top[&a]]);
                }
            }
            for v in &mut self.faces[f] {
                if let Some(&nv) = top.get(v) {
                    *v = nv;
                }
            }
        }
        self.faces.extend(walls);
        self.normalize_wire();
        selected
    }

    /// Extrudes edges into quads (Blender `E` on edges). Quads continue the
    /// winding of the face an edge borders; wire edges become faces.
    /// Edges that do not exist are ignored. Returns the new vertices.
    pub fn extrude_edges(&mut self, edges: &[[u32; 2]], offset: Vec3) -> Vec<u32> {
        let mut seen = HashSet::new();
        let edges: Vec<[u32; 2]> = edges
            .iter()
            .map(|&[a, b]| edge_key(a, b))
            .filter(|&[a, b]| a != b && self.has_edge(a, b) && seen.insert([a, b]))
            .collect();
        let mut new_verts = Vec::new();
        let mut top: HashMap<u32, u32> = HashMap::new();
        for &[a, b] in &edges {
            for v in [a, b] {
                top.entry(v).or_insert_with(|| {
                    let nv = self.positions.len() as u32;
                    self.positions.push(self.positions[v as usize] + offset);
                    new_verts.push(nv);
                    nv
                });
            }
        }
        for &[a, b] in &edges {
            let forward = self.faces.iter().any(|f| find_directed(f, a, b).is_some());
            let quad = if forward {
                vec![b, a, top[&a], top[&b]]
            } else {
                vec![a, b, top[&b], top[&a]]
            };
            self.faces.push(quad);
        }
        self.normalize_wire();
        new_verts
    }

    /// Duplicates vertices by `offset`, joining each copy to its source with
    /// a wire edge. Returns the new vertices.
    pub fn extrude_vertices(&mut self, verts: &[u32], offset: Vec3) -> Vec<u32> {
        let mut seen = HashSet::new();
        let verts: Vec<u32> = verts
            .iter()
            .copied()
            .filter(|&v| (v as usize) < self.positions.len() && seen.insert(v))
            .collect();
        verts
            .into_iter()
            .map(|v| {
                let nv = self.add_vertex(self.positions[v as usize] + offset);
                self.wire.push(edge_key(v, nv));
                nv
            })
            .collect()
    }

    /// Per-face inset (Blender `I` with "individual"): every edge moves
    /// `amount` inwards within the face plane and a ring of quads joins the
    /// inner face to the old border. Returns the inner faces (same indices).
    pub fn inset_faces(&mut self, faces: &[usize], amount: f32) -> Vec<usize> {
        let selected = unique_faces(self, faces);
        for &f in &selected {
            let normal = self.face_normal(f);
            let loop_ = self.faces[f].clone();
            let n = loop_.len();
            let pts: Vec<Vec3> = self.face_positions(f).collect();
            let center = self.face_center(f);
            let inner: Vec<u32> = (0..n)
                .map(|i| {
                    let (prev, cur, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
                    let in1 = normal.cross(cur - prev).normalize_or_zero();
                    let in2 = normal.cross(next - cur).normalize_or_zero();
                    let bis = (in1 + in2).normalize_or_zero();
                    let cos = bis.dot(in1);
                    let p = if bis == Vec3::ZERO || cos < 0.05 {
                        cur + (center - cur).normalize_or_zero() * amount
                    } else {
                        cur + bis * (amount / cos)
                    };
                    self.add_vertex(p)
                })
                .collect();
            for i in 0..n {
                let j = (i + 1) % n;
                self.faces
                    .push(vec![loop_[i], loop_[j], inner[j], inner[i]]);
            }
            self.faces[f] = inner;
        }
        selected
    }
}

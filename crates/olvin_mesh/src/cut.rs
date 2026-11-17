use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use glam::Vec3;

use crate::extrude::unique_faces;
use crate::query::find_directed;
use crate::{MeshError, PolyMesh, Result, edge_key, loop_edges};

impl PolyMesh {
    /// Inserts `chain` (ordered from `a` to `b`) into every face and wire
    /// edge using the edge `a`-`b`.
    pub(crate) fn insert_on_edge(&mut self, a: u32, b: u32, chain: &[u32]) {
        for face in &mut self.faces {
            if !loop_edges(face).any(|(u, v)| (u, v) == (a, b) || (u, v) == (b, a)) {
                continue;
            }
            let n = face.len();
            let mut out = Vec::with_capacity(n + chain.len());
            for i in 0..n {
                let (u, v) = (face[i], face[(i + 1) % n]);
                out.push(u);
                if (u, v) == (a, b) {
                    out.extend_from_slice(chain);
                } else if (u, v) == (b, a) {
                    out.extend(chain.iter().rev());
                }
            }
            *face = out;
        }
        let key = edge_key(a, b);
        if let Some(i) = self.wire.iter().position(|&e| e == key) {
            self.wire.swap_remove(i);
            let mut prev = a;
            for &v in chain.iter().chain([&b]) {
                self.wire.push(edge_key(prev, v));
                prev = v;
            }
        }
    }

    /// Adds `cuts` evenly spaced vertices along `a`-`b` and inserts them into
    /// all users of the edge. Returns them ordered from `a` to `b`.
    fn cut_edge_evenly(&mut self, a: u32, b: u32, cuts: u32) -> Vec<u32> {
        let (pa, pb) = (self.positions[a as usize], self.positions[b as usize]);
        let chain: Vec<u32> = (1..=cuts)
            .map(|k| self.add_vertex(pa.lerp(pb, k as f32 / (cuts + 1) as f32)))
            .collect();
        self.insert_on_edge(a, b, &chain);
        chain
    }

    /// Inserts a vertex on edge `a`-`b` at `t` (0 = `a`, 1 = `b`, clamped)
    /// into every face and wire edge using that edge.
    pub fn split_edge(&mut self, a: u32, b: u32, t: f32) -> Result<u32> {
        self.check_vertex(a)?;
        self.check_vertex(b)?;
        if a == b || !self.has_edge(a, b) {
            return Err(MeshError::NoSuchEdge(a, b));
        }
        let p = self.positions[a as usize].lerp(self.positions[b as usize], t.clamp(0.0, 1.0));
        let m = self.add_vertex(p);
        self.insert_on_edge(a, b, &[m]);
        Ok(m)
    }

    /// Splits face `f` along the diagonal `a`-`b` (both in the face and not
    /// adjacent). The part starting at `a` keeps index `f`; the other one is
    /// appended. Returns the new face index.
    fn split_face(&mut self, f: usize, a: u32, b: u32) -> usize {
        let face = &self.faces[f];
        let n = face.len();
        let ia = face.iter().position(|&v| v == a).unwrap_or(0);
        let ib = face.iter().position(|&v| v == b).unwrap_or(0);
        let walk = |from: usize, to: usize| {
            let mut out = vec![face[from]];
            let mut i = from;
            while i != to {
                i = (i + 1) % n;
                out.push(face[i]);
            }
            out
        };
        let (first, second) = (walk(ia, ib), walk(ib, ia));
        self.faces[f] = first;
        self.faces.push(second);
        self.faces.len() - 1
    }

    /// The knife primitive: splits the face containing both `a` and `b`
    /// (not adjacent in it) into two faces joined by the new edge `a`-`b`.
    /// The new edge replaces a matching wire edge.
    pub fn connect_vertices(&mut self, a: u32, b: u32) -> Result<()> {
        self.check_vertex(a)?;
        self.check_vertex(b)?;
        if a == b {
            return Err(MeshError::NotEnoughVertices);
        }
        let candidates: Vec<usize> = self
            .faces
            .iter()
            .enumerate()
            .filter(|(_, f)| f.contains(&a) && f.contains(&b))
            .map(|(i, _)| i)
            .collect();
        if candidates.is_empty() {
            return Err(MeshError::NoSharedFace);
        }
        let f = candidates
            .into_iter()
            .find(|&f| {
                find_directed(&self.faces[f], a, b).is_none()
                    && find_directed(&self.faces[f], b, a).is_none()
            })
            .ok_or(MeshError::AlreadyConnected)?;
        self.split_face(f, a, b);
        self.normalize_wire();
        Ok(())
    }

    /// Cuts a face between a point on one of its edges and a point on
    /// another: each argument is `(a, b, t)` for the point `lerp(a, b, t)`.
    /// Both edges are split everywhere they are used, then the face that
    /// held both is divided. Returns the two new vertices.
    pub fn knife(&mut self, from: (u32, u32, f32), to: (u32, u32, f32)) -> Result<[u32; 2]> {
        let (a1, b1, t1) = from;
        let (a2, b2, t2) = to;
        for v in [a1, b1, a2, b2] {
            self.check_vertex(v)?;
        }
        for (a, b) in [(a1, b1), (a2, b2)] {
            if a == b || !self.has_edge(a, b) {
                return Err(MeshError::NoSuchEdge(a, b));
            }
        }
        if edge_key(a1, b1) == edge_key(a2, b2) {
            return Err(MeshError::NoSharedFace);
        }
        let has = |f: &Vec<u32>, a: u32, b: u32| {
            find_directed(f, a, b).is_some() || find_directed(f, b, a).is_some()
        };
        let f = self
            .faces
            .iter()
            .position(|f| has(f, a1, b1) && has(f, a2, b2))
            .ok_or(MeshError::NoSharedFace)?;
        let m1 = self.split_edge(a1, b1, t1)?;
        let m2 = self.split_edge(a2, b2, t2)?;
        self.split_face(f, m1, m2);
        Ok([m1, m2])
    }

    /// Blender `Ctrl+R`: walks the edge ring through quads from edge `a`-`b`
    /// in both directions (stopping at non-quads and borders, or when the
    /// ring closes) and inserts `cuts` evenly spaced edge loops across it.
    /// Faces at the open ends of the ring receive the new vertices on their
    /// shared edge. Returns the new vertices.
    pub fn loop_cut(&mut self, a: u32, b: u32, cuts: u32) -> Result<Vec<u32>> {
        self.check_vertex(a)?;
        self.check_vertex(b)?;
        if a == b || !self.has_edge(a, b) {
            return Err(MeshError::NoSuchEdge(a, b));
        }
        if cuts == 0 {
            return Ok(Vec::new());
        }
        // Each step: (quad, entry edge, aligned exit edge, quad holds entry
        // forwards). Edges are directed so entry.0 and exit.0 lie on the same
        // side of the ring.
        type Step = (usize, (u32, u32), (u32, u32), bool);
        let start_faces = self.faces_with_edge(a, b);
        let mut steps: Vec<Step> = Vec::new();
        let mut visited = HashSet::new();
        for (dir, &f0) in start_faces.iter().take(2).enumerate() {
            let mut walk: Vec<Step> = Vec::new();
            let mut closed = false;
            let (mut face, mut edge) = (f0, (a, b));
            loop {
                let quad = &self.faces[face];
                if quad.len() != 4 || !visited.insert(face) {
                    break;
                }
                let (exit, forward) = if let Some(i) = find_directed(quad, edge.0, edge.1) {
                    ((quad[(i + 3) % 4], quad[(i + 2) % 4]), true)
                } else {
                    let i = find_directed(quad, edge.1, edge.0).unwrap_or(0);
                    ((quad[(i + 2) % 4], quad[(i + 3) % 4]), false)
                };
                walk.push((face, edge, exit, forward));
                if edge_key(exit.0, exit.1) == edge_key(a, b) {
                    closed = true;
                    break;
                }
                let next: Vec<usize> = self
                    .faces_with_edge(exit.0, exit.1)
                    .into_iter()
                    .filter(|&g| g != face)
                    .collect();
                if next.len() != 1 {
                    break;
                }
                (face, edge) = (next[0], exit);
            }
            if dir == 0 {
                steps = walk;
            } else {
                // Prepend the other direction, reversed so the ring reads in order.
                let mut walk: Vec<Step> = walk
                    .into_iter()
                    .rev()
                    .map(|(f, e, x, fwd)| (f, x, e, !fwd))
                    .collect();
                walk.append(&mut steps);
                steps = walk;
            }
            if closed {
                break;
            }
        }

        let mut ring_edges: Vec<(u32, u32)> = vec![(a, b)];
        for &(_, entry, exit, _) in &steps {
            ring_edges.extend([entry, exit]);
        }
        let mut chains: HashMap<[u32; 2], Vec<u32>> = HashMap::new();
        let mut new_verts = Vec::new();
        for (u, v) in ring_edges {
            let key = edge_key(u, v);
            if let Entry::Vacant(slot) = chains.entry(key) {
                let chain = self.cut_edge_evenly(key[0], key[1], cuts);
                new_verts.extend_from_slice(&chain);
                slot.insert(chain);
            }
        }
        // Full chain from u to v, endpoints included.
        let chain = |u: u32, v: u32| {
            let inner = &chains[&edge_key(u, v)];
            let mut c = vec![u];
            if u < v {
                c.extend_from_slice(inner);
            } else {
                c.extend(inner.iter().rev());
            }
            c.push(v);
            c
        };
        for &(f, entry, exit, forward) in &steps {
            let (ca, cb) = (chain(entry.0, entry.1), chain(exit.0, exit.1));
            let strips: Vec<Vec<u32>> = (0..=cuts as usize)
                .map(|k| {
                    let s = vec![ca[k], ca[k + 1], cb[k + 1], cb[k]];
                    if forward {
                        s
                    } else {
                        s.into_iter().rev().collect()
                    }
                })
                .collect();
            let mut strips = strips.into_iter();
            if let Some(first) = strips.next() {
                self.faces[f] = first;
            }
            self.faces.extend(strips);
        }
        self.normalize_wire();
        Ok(new_verts)
    }

    /// Subdivides faces with `cuts` cuts per edge: quads become a
    /// `(cuts + 1)^2` grid, other faces a fan of triangles around their
    /// centre. Neighbouring faces receive the new edge vertices so the mesh
    /// stays watertight. Returns all faces replacing the selection.
    pub fn subdivide_faces(&mut self, faces: &[usize], cuts: u32) -> Vec<usize> {
        let selected = unique_faces(self, faces);
        if cuts == 0 {
            return selected;
        }
        let originals: Vec<Vec<u32>> = selected.iter().map(|&f| self.faces[f].clone()).collect();
        let mut chains: HashMap<[u32; 2], Vec<u32>> = HashMap::new();
        for face in &originals {
            for (u, v) in loop_edges(face) {
                let key = edge_key(u, v);
                if let Entry::Vacant(slot) = chains.entry(key) {
                    slot.insert(self.cut_edge_evenly(key[0], key[1], cuts));
                }
            }
        }
        let chain = |u: u32, v: u32| {
            let inner = &chains[&edge_key(u, v)];
            let mut c = vec![u];
            if u < v {
                c.extend_from_slice(inner);
            } else {
                c.extend(inner.iter().rev());
            }
            c.push(v);
            c
        };
        let n = cuts as usize + 1;
        let mut result = Vec::new();
        for (&f, face) in selected.iter().zip(&originals) {
            let mut new_faces: Vec<Vec<u32>> = Vec::new();
            if face.len() == 4 {
                let (q0, q1, q2, q3) = (face[0], face[1], face[2], face[3]);
                let (bottom, right, top, left) =
                    (chain(q0, q1), chain(q1, q2), chain(q3, q2), chain(q0, q3));
                // grid[i][j]: i along q0 -> q1, j along q0 -> q3.
                let mut grid = vec![left];
                for i in 1..n {
                    let (pb, pt) = (
                        self.positions[bottom[i] as usize],
                        self.positions[top[i] as usize],
                    );
                    let mut column = vec![bottom[i]];
                    column
                        .extend((1..n).map(|j| self.add_vertex(pb.lerp(pt, j as f32 / n as f32))));
                    column.push(top[i]);
                    grid.push(column);
                }
                grid.push(right);
                for i in 0..n {
                    for j in 0..n {
                        new_faces.push(vec![
                            grid[i][j],
                            grid[i + 1][j],
                            grid[i + 1][j + 1],
                            grid[i][j + 1],
                        ]);
                    }
                }
            } else {
                let center = face
                    .iter()
                    .map(|&v| self.positions[v as usize])
                    .sum::<Vec3>()
                    / face.len() as f32;
                let c = self.add_vertex(center);
                for (u, v) in loop_edges(face) {
                    let seg = chain(u, v);
                    for w in seg.windows(2) {
                        new_faces.push(vec![w[0], w[1], c]);
                    }
                }
            }
            let mut it = new_faces.into_iter();
            if let Some(first) = it.next() {
                self.faces[f] = first;
                result.push(f);
            }
            for nf in it {
                self.faces.push(nf);
                result.push(self.faces.len() - 1);
            }
        }
        self.normalize_wire();
        result
    }
}

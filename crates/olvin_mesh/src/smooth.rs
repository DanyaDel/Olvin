use std::collections::HashMap;

use glam::Vec3;

use crate::{PolyMesh, edge_key, loop_edges};

impl PolyMesh {
    /// Catmull-Clark subdivision surface applied `levels` times. Borders use
    /// the cubic B-spline boundary rule; wire edges and loose vertices are
    /// dropped.
    pub fn catmull_clark(&self, levels: u32) -> PolyMesh {
        let mut mesh = self.clone();
        mesh.wire.clear();
        for _ in 0..levels {
            mesh = mesh.catmull_clark_once();
        }
        mesh
    }

    fn catmull_clark_once(&self) -> PolyMesh {
        let face_points: Vec<Vec3> = (0..self.faces.len()).map(|f| self.face_center(f)).collect();
        let mut edge_faces: HashMap<[u32; 2], Vec<usize>> = HashMap::new();
        for (i, f) in self.faces.iter().enumerate() {
            for (a, b) in loop_edges(f) {
                edge_faces.entry(edge_key(a, b)).or_default().push(i);
            }
        }
        let p = |v: u32| self.positions[v as usize];

        let mut out = PolyMesh {
            positions: Vec::new(),
            faces: Vec::new(),
            wire: Vec::new(),
        };
        let mut edge_index: HashMap<[u32; 2], u32> = HashMap::new();
        let mut edges: Vec<_> = edge_faces.iter().collect();
        edges.sort_unstable_by_key(|(k, _)| **k);
        let base = self.positions.len() as u32;
        out.positions.extend_from_slice(&self.positions);
        for (&[a, b], fs) in edges {
            let mid = (p(a) + p(b)) * 0.5;
            let point = if fs.len() == 2 {
                (p(a) + p(b) + face_points[fs[0]] + face_points[fs[1]]) * 0.25
            } else {
                mid
            };
            edge_index.insert([a, b], out.positions.len() as u32);
            out.positions.push(point);
        }

        // Vertex points: interior rule, border rule, or unchanged.
        let mut vert_faces: Vec<Vec<usize>> = vec![Vec::new(); self.positions.len()];
        let mut vert_edges: Vec<Vec<[u32; 2]>> = vec![Vec::new(); self.positions.len()];
        for (i, f) in self.faces.iter().enumerate() {
            for &v in f {
                vert_faces[v as usize].push(i);
            }
        }
        for &e in edge_faces.keys() {
            vert_edges[e[0] as usize].push(e);
            vert_edges[e[1] as usize].push(e);
        }
        for v in 0..self.positions.len() {
            let es = &vert_edges[v];
            let fs = &vert_faces[v];
            if fs.is_empty() {
                continue;
            }
            let s = self.positions[v];
            let border: Vec<[u32; 2]> = es
                .iter()
                .copied()
                .filter(|e| edge_faces[e].len() == 1)
                .collect();
            out.positions[v] = if border.is_empty() && es.iter().all(|e| edge_faces[e].len() == 2) {
                let n = es.len() as f32;
                let q = fs.iter().map(|&f| face_points[f]).sum::<Vec3>() / fs.len() as f32;
                let r = es.iter().map(|&[a, b]| (p(a) + p(b)) * 0.5).sum::<Vec3>() / n;
                (q + 2.0 * r + (n - 3.0) * s) / n
            } else if border.len() == 2 {
                let other = |[a, b]: [u32; 2]| if a as usize == v { p(b) } else { p(a) };
                (6.0 * s + other(border[0]) + other(border[1])) / 8.0
            } else {
                s
            };
        }

        let face_base = base + edge_index.len() as u32;
        out.positions.extend_from_slice(&face_points);
        for (fi, f) in self.faces.iter().enumerate() {
            let n = f.len();
            let fp = face_base + fi as u32;
            for i in 0..n {
                let (prev, cur, next) = (f[(i + n - 1) % n], f[i], f[(i + 1) % n]);
                out.faces.push(vec![
                    cur,
                    edge_index[&edge_key(cur, next)],
                    fp,
                    edge_index[&edge_key(prev, cur)],
                ]);
            }
        }
        out.compact();
        out
    }
}

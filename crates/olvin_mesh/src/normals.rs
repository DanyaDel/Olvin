use std::collections::{HashMap, VecDeque};

use glam::Vec3;

use crate::query::find_directed;
use crate::{PolyMesh, edge_key, loop_edges};

impl PolyMesh {
    /// Reverses the winding of the given faces.
    pub fn flip_faces(&mut self, faces: &[usize]) {
        let mut flipped = vec![false; self.faces.len()];
        for &f in faces {
            if f < self.faces.len() && !flipped[f] {
                flipped[f] = true;
                self.faces[f].reverse();
            }
        }
    }

    /// Makes winding consistent within each edge-connected component, then
    /// flips components whose signed volume is negative. Open components
    /// with no volume are judged by whether their faces point away from the
    /// component centroid on average.
    pub fn recalculate_normals_outside(&mut self) {
        let mut edge_faces: HashMap<[u32; 2], Vec<usize>> = HashMap::new();
        for (i, f) in self.faces.iter().enumerate() {
            for (a, b) in loop_edges(f) {
                edge_faces.entry(edge_key(a, b)).or_default().push(i);
            }
        }
        let mut done = vec![false; self.faces.len()];
        for seed in 0..self.faces.len() {
            if done[seed] {
                continue;
            }
            done[seed] = true;
            let mut component = vec![seed];
            let mut queue = VecDeque::from([seed]);
            while let Some(f) = queue.pop_front() {
                let edges: Vec<(u32, u32)> = loop_edges(&self.faces[f]).collect();
                for (a, b) in edges {
                    for &g in &edge_faces[&edge_key(a, b)] {
                        if done[g] {
                            continue;
                        }
                        // A consistent neighbour walks the shared edge backwards.
                        if find_directed(&self.faces[g], a, b).is_some() {
                            self.faces[g].reverse();
                        }
                        done[g] = true;
                        component.push(g);
                        queue.push_back(g);
                    }
                }
            }

            let verts = component.iter().flat_map(|&f| self.faces[f].iter());
            let (sum, count) = verts.fold((Vec3::ZERO, 0), |(s, c), &v| {
                (s + self.positions[v as usize], c + 1)
            });
            let origin = sum / count.max(1) as f32;
            let mut volume = 0.0;
            let mut facing = 0.0;
            let mut area = 0.0;
            for &f in &component {
                area += self.face_area_vector(f).length();
                for [a, b, c] in self.face_triangles(f) {
                    let p = |v: u32| self.positions[v as usize] - origin;
                    volume += p(a).dot(p(b).cross(p(c)));
                }
                facing += self.face_area_vector(f).dot(self.face_center(f) - origin);
            }
            let outward = if volume.abs() > 1e-6 * area.powf(1.5) {
                volume
            } else {
                facing
            };
            if outward < 0.0 {
                for &f in &component {
                    self.faces[f].reverse();
                }
            }
        }
    }
}

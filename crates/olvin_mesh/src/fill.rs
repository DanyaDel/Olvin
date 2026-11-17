use std::collections::{HashMap, HashSet};

use glam::Vec3;

use crate::query::{find_directed, newell};
use crate::{MeshError, PolyMesh, Result, edge_key, loop_edges};

impl PolyMesh {
    /// Blender `F` on vertices. Two vertices make a wire edge (`Ok(None)`,
    /// also when the edge already exists). Three or more make a face whose
    /// loop is sorted by angle around the centroid in the best-fit plane,
    /// so selection order does not matter. The face is wound against any
    /// neighbour it shares an edge with, otherwise its normal points away
    /// from the mesh centroid.
    pub fn make_face(&mut self, verts: &[u32]) -> Result<Option<usize>> {
        let mut seen = HashSet::new();
        let verts: Vec<u32> = verts.iter().copied().filter(|v| seen.insert(*v)).collect();
        for &v in &verts {
            self.check_vertex(v)?;
        }
        match verts.len() {
            0 | 1 => Err(MeshError::NotEnoughVertices),
            2 => {
                if !self.has_edge(verts[0], verts[1]) {
                    self.wire.push(edge_key(verts[0], verts[1]));
                }
                Ok(None)
            }
            _ => {
                let pts: Vec<Vec3> = verts.iter().map(|&v| self.positions[v as usize]).collect();
                let center = pts.iter().sum::<Vec3>() / pts.len() as f32;
                // Order-independent plane: the largest cross product of
                // centred position pairs.
                let mut normal = Vec3::ZERO;
                for i in 0..pts.len() {
                    for j in i + 1..pts.len() {
                        let c = (pts[i] - center).cross(pts[j] - center);
                        let c = if c.dot(normal) < 0.0 { -c } else { c };
                        if c.length_squared() > normal.length_squared() {
                            normal = c;
                        }
                    }
                }
                let normal = normal.normalize_or_zero();
                if normal == Vec3::ZERO {
                    return Err(MeshError::NotEnoughVertices);
                }
                let u = normal.any_orthonormal_vector();
                let w = normal.cross(u);
                let mut order: Vec<(f32, u32)> = verts
                    .iter()
                    .zip(&pts)
                    .map(|(&v, &p)| {
                        let d = p - center;
                        (d.dot(w).atan2(d.dot(u)), v)
                    })
                    .collect();
                order.sort_by(|x, y| x.0.total_cmp(&y.0));
                let face = order.into_iter().map(|(_, v)| v).collect();
                self.add_oriented_face(face).map(Some)
            }
        }
    }

    /// Blender `F` on edges: a closed loop of edges becomes a face, an open
    /// chain is closed into a face, and two disjoint edges are bridged into
    /// a quad (pairing ends to keep the new edges short). Returns the face.
    pub fn fill_edges(&mut self, edges: &[[u32; 2]]) -> Result<usize> {
        let mut seen = HashSet::new();
        let edges: Vec<[u32; 2]> = edges
            .iter()
            .map(|&[a, b]| edge_key(a, b))
            .filter(|&[a, b]| a != b && seen.insert([a, b]))
            .collect();
        for &v in edges.iter().flatten() {
            self.check_vertex(v)?;
        }
        let mut adj: HashMap<u32, Vec<u32>> = HashMap::new();
        for &[a, b] in &edges {
            adj.entry(a).or_default().push(b);
            adj.entry(b).or_default().push(a);
        }
        if adj.values().any(|n| n.len() > 2) || edges.is_empty() {
            return Err(MeshError::NotAChain);
        }

        if edges.len() == 2 && adj.len() == 4 {
            let ([a0, a1], [b0, b1]) = (edges[0], edges[1]);
            let p = |v: u32| self.positions[v as usize];
            let straight = p(a1).distance(p(b1)) + p(b0).distance(p(a0));
            let crossed = p(a1).distance(p(b0)) + p(b1).distance(p(a0));
            let face = if straight <= crossed {
                vec![a0, a1, b1, b0]
            } else {
                vec![a0, a1, b0, b1]
            };
            return self.add_oriented_face(face);
        }

        // Single chain: start at an end if open, anywhere if closed.
        let start = adj
            .iter()
            .filter(|(_, n)| n.len() == 1)
            .map(|(&v, _)| v)
            .min()
            .unwrap_or(edges[0][0]);
        let mut chain = vec![start];
        let mut prev = u32::MAX;
        let mut cur = start;
        while let Some(&next) = adj[&cur]
            .iter()
            .find(|&&n| n != prev && !chain.contains(&n))
        {
            chain.push(next);
            prev = cur;
            cur = next;
        }
        let ends = adj.values().filter(|n| n.len() == 1).count();
        let connected = chain.len() == adj.len();
        let shape_ok = (ends == 0 && edges.len() == chain.len())
            || (ends == 2 && edges.len() + 1 == chain.len());
        if !connected || !shape_ok || chain.len() < 3 {
            return Err(MeshError::NotAChain);
        }
        self.add_oriented_face(chain)
    }

    /// Appends a face loop, flipping it to agree with neighbours (or to face
    /// away from the mesh centroid), and absorbs wire edges it covers.
    pub(crate) fn add_oriented_face(&mut self, mut face: Vec<u32>) -> Result<usize> {
        if face.iter().collect::<HashSet<_>>().len() < 3 {
            return Err(MeshError::NotEnoughVertices);
        }
        let set: HashSet<u32> = face.iter().copied().collect();
        if self
            .faces
            .iter()
            .any(|f| f.len() == set.len() && f.iter().all(|v| set.contains(v)))
        {
            return Err(MeshError::FaceExists);
        }
        // Some(true): a neighbour runs a shared edge the same way, so flip.
        let neighbour = loop_edges(&face).find_map(|(u, v)| {
            self.faces.iter().find_map(|f| {
                if find_directed(f, u, v).is_some() {
                    Some(true)
                } else if find_directed(f, v, u).is_some() {
                    Some(false)
                } else {
                    None
                }
            })
        });
        if let Some(flip) = neighbour {
            if flip {
                face.reverse();
            }
        } else {
            let pts: Vec<Vec3> = face.iter().map(|&v| self.positions[v as usize]).collect();
            let normal = newell(pts.iter().copied());
            let center = pts.iter().sum::<Vec3>() / pts.len() as f32;
            if normal.dot(center - self.centroid()) < 0.0 {
                face.reverse();
            }
        }
        self.faces.push(face);
        self.normalize_wire();
        Ok(self.faces.len() - 1)
    }
}

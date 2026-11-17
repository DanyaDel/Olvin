use glam::Vec3;

use crate::{PolyMesh, edge_key, loop_edges};

/// Newell normal of a polygon, scaled by twice its area.
pub(crate) fn newell(points: impl IntoIterator<Item = Vec3> + Clone) -> Vec3 {
    let first = points.clone().into_iter().next().unwrap_or(Vec3::ZERO);
    let mut n = Vec3::ZERO;
    let mut it = points.into_iter().peekable();
    while let Some(p) = it.next() {
        let q = it.peek().copied().unwrap_or(first);
        n += Vec3::new(
            (p.y - q.y) * (p.z + q.z),
            (p.z - q.z) * (p.x + q.x),
            (p.x - q.x) * (p.y + q.y),
        );
    }
    n
}

impl PolyMesh {
    /// All unique edges (face edges and wire edges), smaller index first, sorted.
    pub fn edges(&self) -> Vec<[u32; 2]> {
        let mut edges: Vec<[u32; 2]> = self
            .face_edge_set()
            .into_iter()
            .chain(self.wire.iter().copied())
            .collect();
        edges.sort_unstable();
        edges.dedup();
        edges
    }

    /// Whether `a` and `b` are joined by a face edge or wire edge.
    pub fn has_edge(&self, a: u32, b: u32) -> bool {
        let key = edge_key(a, b);
        self.wire.contains(&key) || !self.faces_with_edge(a, b).is_empty()
    }

    pub fn face_positions(&self, f: usize) -> impl Iterator<Item = Vec3> + Clone + '_ {
        self.faces[f].iter().map(|&v| self.positions[v as usize])
    }

    /// Unit normal (Newell), or zero for a degenerate face. Panics if `f` is out of range.
    pub fn face_normal(&self, f: usize) -> Vec3 {
        newell(self.face_positions(f)).normalize_or_zero()
    }

    /// Area-weighted normal; its length is the face area.
    pub fn face_area_vector(&self, f: usize) -> Vec3 {
        newell(self.face_positions(f)) * 0.5
    }

    /// Average of the face's vertices. Panics if `f` is out of range.
    pub fn face_center(&self, f: usize) -> Vec3 {
        let face = &self.faces[f];
        self.face_positions(f).sum::<Vec3>() / face.len().max(1) as f32
    }

    /// Faces having `a`-`b` as an edge, in either direction.
    pub fn faces_with_edge(&self, a: u32, b: u32) -> Vec<usize> {
        self.faces
            .iter()
            .enumerate()
            .filter(|(_, f)| loop_edges(f).any(|(u, v)| (u, v) == (a, b) || (u, v) == (b, a)))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn faces_with_vertex(&self, v: u32) -> Vec<usize> {
        self.faces
            .iter()
            .enumerate()
            .filter(|(_, f)| f.contains(&v))
            .map(|(i, _)| i)
            .collect()
    }

    /// Average of all vertex positions.
    pub fn centroid(&self) -> Vec3 {
        if self.positions.is_empty() {
            Vec3::ZERO
        } else {
            self.positions.iter().sum::<Vec3>() / self.positions.len() as f32
        }
    }
}

/// Position of the directed edge `a -> b` in `face` (index of `a`).
pub(crate) fn find_directed(face: &[u32], a: u32, b: u32) -> Option<usize> {
    loop_edges(face).position(|e| e == (a, b))
}

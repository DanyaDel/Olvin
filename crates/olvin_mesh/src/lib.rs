//! Editable polygon meshes in the spirit of Blender's BMesh.
//!
//! [`PolyMesh`] is deliberately plain: positions, faces as index loops and
//! loose "wire" edges. Adjacency is recomputed on demand, which keeps every
//! operation easy to reason about and the data trivially serializable. Edit
//! meshes are small (thousands of faces at most), so the linear scans are
//! not a concern.
//!
//! Operations that only add geometry keep existing indices valid. Operations
//! that remove geometry return a [`Remap`] so an editor can carry its
//! selection over.

mod build;
mod cut;
mod delete;
mod extrude;
mod fill;
mod merge;
mod normals;
mod query;
mod smooth;
mod triangulate;
mod walk;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::fmt;

use glam::Vec3;

pub use triangulate::ear_clip;
pub use walk::Adjacency;

/// An editable polygon mesh.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PolyMesh {
    pub positions: Vec<Vec3>,
    /// Faces as vertex-index loops, counter-clockwise seen from outside (normal by right-hand rule).
    pub faces: Vec<Vec<u32>>,
    /// Loose edges not belonging to any face ("wire" edges, Blender-style).
    pub wire: Vec<[u32; 2]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshError {
    InvalidVertex(u32),
    InvalidFace(usize),
    /// The two vertices are not joined by a face or wire edge.
    NoSuchEdge(u32, u32),
    /// No single face contains all the given vertices or edges.
    NoSharedFace,
    /// The vertices are already adjacent in every face that contains both.
    AlreadyConnected,
    NotEnoughVertices,
    /// A face with exactly this vertex set already exists.
    FaceExists,
    /// The edges form neither a closed loop, an open chain nor two bridgeable edges.
    NotAChain,
    /// A structural invariant is broken (see [`PolyMesh::validate`]).
    Corrupt(&'static str),
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVertex(v) => write!(f, "vertex {v} does not exist"),
            Self::InvalidFace(i) => write!(f, "face {i} does not exist"),
            Self::NoSuchEdge(a, b) => write!(f, "no edge between vertices {a} and {b}"),
            Self::NoSharedFace => f.write_str("no single face contains the given elements"),
            Self::AlreadyConnected => f.write_str("vertices are already connected by an edge"),
            Self::NotEnoughVertices => f.write_str("not enough distinct vertices"),
            Self::FaceExists => f.write_str("face already exists"),
            Self::NotAChain => f.write_str("edges do not form a fillable chain"),
            Self::Corrupt(what) => write!(f, "corrupt mesh: {what}"),
        }
    }
}

impl std::error::Error for MeshError {}

pub type Result<T, E = MeshError> = std::result::Result<T, E>;

/// Old index -> new index tables produced by operations that remove elements.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remap {
    pub vertices: Vec<Option<u32>>,
    pub faces: Vec<Option<usize>>,
}

impl Remap {
    fn identity(vertices: usize, faces: usize) -> Self {
        Self {
            vertices: (0..vertices as u32).map(Some).collect(),
            faces: (0..faces).map(Some).collect(),
        }
    }

    pub fn vertex(&self, old: u32) -> Option<u32> {
        self.vertices.get(old as usize).copied().flatten()
    }

    pub fn face(&self, old: usize) -> Option<usize> {
        self.faces.get(old).copied().flatten()
    }
}

/// Edge key with the smaller index first.
#[inline]
pub fn edge_key(a: u32, b: u32) -> [u32; 2] {
    if a <= b { [a, b] } else { [b, a] }
}

/// Iterates the directed edges of a face loop, including the closing one.
#[inline]
pub(crate) fn loop_edges(face: &[u32]) -> impl Iterator<Item = (u32, u32)> + '_ {
    let n = face.len();
    (0..n).map(move |i| (face[i], face[(i + 1) % n]))
}

impl PolyMesh {
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Checks the invariants every operation preserves: indices in range,
    /// faces with at least 3 distinct vertices and no consecutive repeats,
    /// wire edges normalized, unique and not duplicating a face edge.
    pub fn validate(&self) -> Result<()> {
        let n = self.positions.len() as u32;
        for face in &self.faces {
            if face.iter().any(|&v| v >= n) {
                return Err(MeshError::Corrupt("face index out of range"));
            }
            if loop_edges(face).any(|(a, b)| a == b) {
                return Err(MeshError::Corrupt("consecutive duplicate in face"));
            }
            if face.iter().collect::<HashSet<_>>().len() < 3 {
                return Err(MeshError::Corrupt("face with fewer than 3 vertices"));
            }
        }
        let face_edges = self.face_edge_set();
        let mut seen = HashSet::new();
        for &[a, b] in &self.wire {
            if a >= n || b >= n {
                return Err(MeshError::Corrupt("wire index out of range"));
            }
            if a >= b {
                return Err(MeshError::Corrupt("wire edge not normalized"));
            }
            if !seen.insert([a, b]) {
                return Err(MeshError::Corrupt("duplicate wire edge"));
            }
            if face_edges.contains(&[a, b]) {
                return Err(MeshError::Corrupt("wire edge duplicates a face edge"));
            }
        }
        Ok(())
    }

    pub(crate) fn check_vertex(&self, v: u32) -> Result<()> {
        if (v as usize) < self.positions.len() {
            Ok(())
        } else {
            Err(MeshError::InvalidVertex(v))
        }
    }

    pub(crate) fn add_vertex(&mut self, p: Vec3) -> u32 {
        self.positions.push(p);
        (self.positions.len() - 1) as u32
    }

    pub(crate) fn face_edge_set(&self) -> HashSet<[u32; 2]> {
        self.faces
            .iter()
            .flat_map(|f| loop_edges(f).map(|(a, b)| edge_key(a, b)))
            .collect()
    }

    /// Normalizes, deduplicates and drops wire edges that are degenerate or
    /// already covered by a face.
    pub(crate) fn normalize_wire(&mut self) {
        let face_edges = self.face_edge_set();
        let n = self.positions.len() as u32;
        let mut seen = HashSet::new();
        let wire = std::mem::take(&mut self.wire);
        self.wire = wire
            .into_iter()
            .filter(|&[a, b]| a != b && a < n && b < n)
            .map(|[a, b]| edge_key(a, b))
            .filter(|e| !face_edges.contains(e) && seen.insert(*e))
            .collect();
    }

    /// Removes the flagged vertices; faces and wire edges using them are
    /// dropped as well (callers normally remove those first).
    pub(crate) fn remove_vertices_mask(&mut self, remove: &[bool]) -> Remap {
        let mut vertices = Vec::with_capacity(self.positions.len());
        let mut next = 0u32;
        for &r in remove {
            if r {
                vertices.push(None);
            } else {
                vertices.push(Some(next));
                next += 1;
            }
        }
        let mut i = 0;
        self.positions.retain(|_| {
            i += 1;
            !remove[i - 1]
        });
        let map = |v: u32| vertices[v as usize];
        let mut faces = Vec::with_capacity(self.faces.len());
        let mut kept = 0;
        let old_faces = std::mem::take(&mut self.faces);
        for face in old_faces {
            let mapped: Option<Vec<u32>> = face.iter().map(|&v| map(v)).collect();
            match mapped {
                Some(f) => {
                    self.faces.push(f);
                    faces.push(Some(kept));
                    kept += 1;
                }
                None => faces.push(None),
            }
        }
        self.wire = self
            .wire
            .iter()
            .filter_map(|&[a, b]| Some(edge_key(map(a)?, map(b)?)))
            .collect();
        Remap { vertices, faces }
    }

    /// Removes faces flagged in `remove`, returning the face remap.
    pub(crate) fn remove_faces_mask(&mut self, remove: &[bool]) -> Vec<Option<usize>> {
        let mut map = Vec::with_capacity(self.faces.len());
        let mut next = 0;
        for &r in remove {
            if r {
                map.push(None);
            } else {
                map.push(Some(next));
                next += 1;
            }
        }
        let mut i = 0;
        self.faces.retain(|_| {
            i += 1;
            !remove[i - 1]
        });
        map
    }

    /// Flags for vertices not referenced by any face or wire edge.
    pub(crate) fn unreferenced(&self) -> Vec<bool> {
        let mut unused = vec![true; self.positions.len()];
        for &v in self
            .faces
            .iter()
            .flatten()
            .chain(self.wire.iter().flatten())
        {
            unused[v as usize] = false;
        }
        unused
    }

    /// Removes the given vertices if nothing references them anymore.
    pub(crate) fn remove_if_unreferenced(
        &mut self,
        candidates: impl IntoIterator<Item = u32>,
    ) -> Remap {
        let unused = self.unreferenced();
        let mut remove = vec![false; self.positions.len()];
        for v in candidates {
            if let Some(u) = unused.get(v as usize) {
                remove[v as usize] = *u;
            }
        }
        self.remove_vertices_mask(&remove)
    }
}

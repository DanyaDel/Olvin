use bytemuck::{Pod, Zeroable};
use glam::Vec3;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

/// CPU-side mesh. Upload happens lazily on the render thread via [`Meshes`].
#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn cube(size: f32) -> Self {
        let h = size * 0.5;
        // (normal, tangent u, tangent v) per face; 4 vertices each so normals stay flat.
        let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
            ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
            ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ];
        let mut mesh = MeshData::default();
        for (n, u, v) in faces {
            let (n, u, v) = (Vec3::from(n), Vec3::from(u), Vec3::from(v));
            let base = mesh.vertices.len() as u32;
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let p = (n + u * su + v * sv) * h;
                mesh.vertices.push(Vertex { position: p.into(), normal: n.into() });
            }
            mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        mesh
    }

    /// Flat square on the XZ plane facing +Y.
    pub fn plane(size: f32) -> Self {
        let h = size * 0.5;
        let n = [0.0, 1.0, 0.0];
        let vertices = [[-h, 0.0, h], [h, 0.0, h], [h, 0.0, -h], [-h, 0.0, -h]]
            .map(|position| Vertex { position, normal: n })
            .to_vec();
        Self { vertices, indices: vec![0, 1, 2, 0, 2, 3] }
    }

    /// Local-space bounding sphere (center, radius), used for frustum culling.
    pub fn bounding_sphere(&self) -> (Vec3, f32) {
        if self.vertices.is_empty() {
            return (Vec3::ZERO, 0.0);
        }
        let (min, max) = self.vertices.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(min, max), v| (min.min(v.position.into()), max.max(v.position.into())),
        );
        let center = (min + max) * 0.5;
        let radius = self
            .vertices
            .iter()
            .map(|v| Vec3::from(v.position).distance(center))
            .fold(0.0, f32::max);
        (center, radius)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshHandle(pub(crate) u32);

/// Mesh registry. Usable before the GPU exists: meshes queue up and are
/// uploaded by the renderer on the next frame.
#[derive(Default)]
pub struct Meshes {
    next: u32,
    pub(crate) pending: Vec<(MeshHandle, MeshData)>,
}

impl Meshes {
    pub fn add(&mut self, mesh: MeshData) -> MeshHandle {
        let handle = MeshHandle(self.next);
        self.next += 1;
        self.pending.push((handle, mesh));
        handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_geometry() {
        let cube = MeshData::cube(2.0);
        assert_eq!(cube.vertices.len(), 24);
        assert_eq!(cube.indices.len(), 36);
        let (c, r) = cube.bounding_sphere();
        assert!(c.length() < 1e-6);
        assert!((r - 3f32.sqrt()).abs() < 1e-5);
        // Every face winds counter-clockwise when seen from outside.
        for tri in cube.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|i| Vec3::from(cube.vertices[tri[i] as usize].position));
            let n = Vec3::from(cube.vertices[tri[0] as usize].normal);
            assert!((b - a).cross(c - a).dot(n) > 0.0);
        }
    }
}

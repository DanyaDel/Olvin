use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// One mesh vertex: 48 bytes, 4 attributes (locations 0..=3).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Texture coordinates; (0, 0) is the image's top-left corner.
    pub uv: [f32; 2],
    /// xyz: direction of increasing `u`; w: handedness (±1), the bitangent
    /// (image up, decreasing `v`) is `cross(normal, tangent.xyz) * w`.
    /// OpenGL-style (green up) normal maps, like glTF and Blender.
    pub tangent: [f32; 4],
}

impl Vertex {
    /// Position and normal; UV zero, tangent filled by [`MeshData::compute_tangents`].
    pub fn new(position: [f32; 3], normal: [f32; 3]) -> Self {
        Self {
            position,
            normal,
            ..Default::default()
        }
    }
}

/// (normal, u axis, image-up axis) of the six box-projection faces. `cross(n, u) = v`.
const BOX_FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
    ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
    ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
];

/// Box projection: the face picked by the normal's dominant axis, `size`
/// metres per texture repeat, the face centre at UV (0.5, 0.5).
fn box_uv(p: Vec3, n: Vec3, size: f32) -> [f32; 2] {
    let a = n.abs();
    let face = if a.x >= a.y && a.x >= a.z {
        if n.x >= 0.0 { 0 } else { 1 }
    } else if a.y >= a.z {
        if n.y >= 0.0 { 2 } else { 3 }
    } else if n.z >= 0.0 {
        4
    } else {
        5
    };
    let (_, u, v) = BOX_FACES[face];
    [
        p.dot(Vec3::from(u)) / size + 0.5,
        0.5 - p.dot(Vec3::from(v)) / size,
    ]
}

/// CPU-side mesh. Upload happens lazily on the render thread via [`Meshes`].
#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Cube centred at the origin; each face maps the whole texture.
    pub fn cube(size: f32) -> Self {
        let h = size * 0.5;
        let mut mesh = MeshData::default();
        for (n, u, v) in BOX_FACES {
            let (n, u, v) = (Vec3::from(n), Vec3::from(u), Vec3::from(v));
            let base = mesh.vertices.len() as u32;
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let p = (n + u * su + v * sv) * h;
                mesh.vertices.push(Vertex {
                    position: p.into(),
                    normal: n.into(),
                    uv: [(su + 1.0) * 0.5, (1.0 - sv) * 0.5],
                    tangent: u.extend(1.0).into(),
                });
            }
            mesh.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        mesh
    }

    /// Flat square on the XZ plane facing +Y; the texture covers it once
    /// (top of the image towards -Z).
    pub fn plane(size: f32) -> Self {
        let h = size * 0.5;
        let vertices = [[-h, 0.0, h], [h, 0.0, h], [h, 0.0, -h], [-h, 0.0, -h]]
            .map(|position| Vertex {
                position,
                normal: [0.0, 1.0, 0.0],
                uv: box_uv(Vec3::from(position), Vec3::Y, size),
                tangent: [1.0, 0.0, 0.0, 1.0],
            })
            .to_vec();
        Self {
            vertices,
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    /// Triangles of positions, normals and indices. UVs are box-projected
    /// from the normal at one repeat per unit (good for CSG and modelled
    /// meshes); tangents are computed.
    pub fn from_triangles(
        positions: &[glam::Vec3],
        normals: &[glam::Vec3],
        indices: Vec<u32>,
    ) -> Self {
        let uvs: Vec<[f32; 2]> = positions
            .iter()
            .zip(normals)
            .map(|(&p, &n)| box_uv(p, n, 1.0))
            .collect();
        Self::from_triangles_uv(positions, normals, &uvs, indices)
    }

    /// Triangles with explicit UVs; tangents are computed.
    pub fn from_triangles_uv(
        positions: &[glam::Vec3],
        normals: &[glam::Vec3],
        uvs: &[[f32; 2]],
        indices: Vec<u32>,
    ) -> Self {
        let vertices = positions
            .iter()
            .zip(normals)
            .zip(uvs.iter().copied().chain(std::iter::repeat([0.0; 2])))
            .map(|((p, n), uv)| Vertex {
                position: p.to_array(),
                normal: n.to_array(),
                uv,
                tangent: [0.0; 4],
            })
            .collect();
        let mut mesh = Self { vertices, indices };
        mesh.compute_tangents();
        mesh
    }

    /// Unity-style built-in mesh (shared with the editor). UVs: box faces for
    /// the cube, planar for plane / quad, cylindrical wrap (seam at +X,
    /// `v` top to bottom) for sphere / capsule / cylinder, planar caps.
    pub fn primitive(shape: olvin_scene::primitives::Shape) -> Self {
        use olvin_scene::primitives::{Primitive, Shape};
        let p = Primitive::new(shape);
        let mut uvs: Vec<[f32; 2]> = match shape {
            Shape::Cube | Shape::Quad => p
                .positions
                .iter()
                .zip(&p.normals)
                .map(|(&p, &n)| box_uv(p, n, 1.0))
                .collect(),
            Shape::Plane => p
                .positions
                .iter()
                .zip(&p.normals)
                .map(|(&p, &n)| box_uv(p, n, 10.0))
                .collect(),
            Shape::Sphere | Shape::Capsule => lathe_uvs(&p.positions, &p.normals, None),
            // Two rings of side wall, then the caps.
            Shape::Cylinder => lathe_uvs(&p.positions, &p.normals, Some(2)),
        };
        uvs.resize(p.positions.len(), [0.0; 2]);
        Self::from_triangles_uv(&p.positions, &p.normals, &uvs, p.indices)
    }

    /// Per-vertex tangents from UV derivatives: per-triangle accumulation,
    /// Gram-Schmidt against the normal, handedness from the bitangent (like
    /// MikkTSpace, without its vertex splitting). Degenerate UVs get any
    /// tangent perpendicular to the normal.
    pub fn compute_tangents(&mut self) {
        let n = self.vertices.len();
        let mut tan = vec![Vec3::ZERO; n];
        let mut up = vec![Vec3::ZERO; n];
        for tri in self.indices.as_chunks::<3>().0 {
            let [a, b, c] = tri.map(|i| i as usize);
            if a >= n || b >= n || c >= n {
                continue;
            }
            let [va, vb, vc] = [self.vertices[a], self.vertices[b], self.vertices[c]];
            let e1 = Vec3::from(vb.position) - Vec3::from(va.position);
            let e2 = Vec3::from(vc.position) - Vec3::from(va.position);
            let (du1, dv1) = (vb.uv[0] - va.uv[0], vb.uv[1] - va.uv[1]);
            let (du2, dv2) = (vc.uv[0] - va.uv[0], vc.uv[1] - va.uv[1]);
            let r = du1 * dv2 - du2 * dv1;
            if r.abs() < 1e-12 {
                continue;
            }
            let t = (e1 * dv2 - e2 * dv1) / r;
            // Direction of decreasing v: image up.
            let bt = -(e2 * du1 - e1 * du2) / r;
            if !t.is_finite() || !bt.is_finite() {
                continue;
            }
            for i in [a, b, c] {
                tan[i] += t;
                up[i] += bt;
            }
        }
        for (i, v) in self.vertices.iter_mut().enumerate() {
            let normal = Vec3::from(v.normal).normalize_or(Vec3::Y);
            let t = tan[i] - normal * normal.dot(tan[i]);
            let t = t
                .try_normalize()
                .unwrap_or_else(|| normal.any_orthonormal_vector());
            let w = if normal.cross(t).dot(up[i]) < 0.0 {
                -1.0
            } else {
                1.0
            };
            v.tangent = t.extend(w).into();
        }
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

/// Cylindrical UVs for lathed primitives: the first `rings` rings of equal
/// height, `u` by position in the ring (so the seam vertex gets u = 1), `v`
/// by height (top = 0). Remaining (cap) vertices are planar.
fn lathe_uvs(positions: &[Vec3], normals: &[Vec3], rings: Option<usize>) -> Vec<[f32; 2]> {
    let Some(first) = positions.first() else {
        return Vec::new();
    };
    let row = positions
        .iter()
        .position(|p| (p.y - first.y).abs() > 1e-6)
        .unwrap_or(positions.len())
        .max(2);
    let lathe = rings.map_or(positions.len(), |r| (r * row).min(positions.len()));
    let (min, max) = positions[..lathe.max(1)]
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            (lo.min(p.y), hi.max(p.y))
        });
    let height = (max - min).max(1e-6);
    positions
        .iter()
        .zip(normals)
        .enumerate()
        .map(|(i, (p, n))| {
            if i < lathe {
                let s = (i % row) as f32 / (row - 1) as f32;
                [1.0 - s, (max - p.y) / height]
            } else {
                box_uv(*p, *n, 1.0)
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshHandle(pub(crate) u32);

/// Mesh registry. Usable before the GPU exists: meshes queue up and are
/// uploaded by the renderer on the next frame.
#[derive(Default)]
pub struct Meshes {
    next: u32,
    pub(crate) pending: Vec<(MeshHandle, MeshData)>,
    /// Poses of skinned meshes waiting for the renderer (see [`Meshes::skin`]).
    pub(crate) skin_updates: Vec<crate::skinning::SkinUpdate>,
}

impl Meshes {
    pub fn add(&mut self, mesh: MeshData) -> MeshHandle {
        let handle = MeshHandle(self.next);
        self.next += 1;
        self.pending.push((handle, mesh));
        handle
    }

    /// Replaces a mesh's geometry (re-uploaded into the same GPU slot).
    pub fn set(&mut self, handle: MeshHandle, mesh: MeshData) {
        self.pending.retain(|(h, _)| *h != handle);
        self.pending.push((handle, mesh));
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

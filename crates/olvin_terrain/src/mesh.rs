//! Terrain geometry: chunks of the grid with simpler copies for distance,
//! collision triangles, and a ray cast for brushes.
//!
//! The terrain is centred on its object: x and z run from `-size / 2` to
//! `size / 2`, y from 0 to `size.y`. UVs run 0..1 across the whole terrain.

use glam::{Vec2, Vec3};
use olvin_render::{MeshData, MeshHandle, MeshLod, Meshes, Vertex};
use olvin_scene::MeshGeometry;

use crate::TerrainData;

/// Quads per chunk side.
pub const CHUNK: u32 = 32;
/// Grid steps of the levels of detail and the share of screen height below
/// which each is drawn (see `olvin_render::MeshLod`).
const LODS: [(u32, f32); 3] = [(2, 0.35), (4, 0.15), (8, 0.06)];

/// Vertex `(x, z)` in the terrain's local space.
pub fn position(data: &TerrainData, size: Vec3, x: u32, z: u32) -> Vec3 {
    let last = (data.resolution() - 1) as f32;
    Vec3::new(
        (x as f32 / last - 0.5) * size.x,
        data.height(x, z) * size.y,
        (z as f32 / last - 0.5) * size.z,
    )
}

/// Normal at vertex `(x, z)`, from its neighbours.
pub fn normal(data: &TerrainData, size: Vec3, x: u32, z: u32) -> Vec3 {
    let last = data.resolution() - 1;
    let (x0, x1) = (x.saturating_sub(1), (x + 1).min(last));
    let (z0, z1) = (z.saturating_sub(1), (z + 1).min(last));
    let dx = position(data, size, x1, z) - position(data, size, x0, z);
    let dz = position(data, size, x, z1) - position(data, size, x, z0);
    dz.cross(dx).normalize_or(Vec3::Y)
}

/// The chunks the grid is cut into: `(x, z)` of each chunk's first quad.
pub fn chunks(data: &TerrainData) -> Vec<(u32, u32)> {
    let quads = data.resolution() - 1;
    let mut out = Vec::new();
    for cz in (0..quads).step_by(CHUNK as usize) {
        for cx in (0..quads).step_by(CHUNK as usize) {
            out.push((cx, cz));
        }
    }
    out
}

/// The mesh of the chunk starting at quad `(cx, cz)`, every `step` vertices,
/// with a skirt hanging from its edges: neighbours drawn at another level
/// leave no visible gap.
pub fn chunk_mesh(data: &TerrainData, size: Vec3, (cx, cz): (u32, u32), step: u32) -> MeshData {
    let last = data.resolution() - 1;
    let x1 = (cx + CHUNK).min(last);
    let z1 = (cz + CHUNK).min(last);
    // The chunk's corners are always on the grid, whatever the step.
    let along = |from: u32, to: u32| -> Vec<u32> {
        let mut v: Vec<u32> = (from..to).step_by(step.max(1) as usize).collect();
        v.push(to);
        v
    };
    let (xs, zs) = (along(cx, x1), along(cz, z1));
    let uv = |x: u32, z: u32| [x as f32 / last as f32, z as f32 / last as f32];
    let vertex = |p: Vec3, n: Vec3, uv: [f32; 2]| Vertex {
        position: p.to_array(),
        normal: n.to_array(),
        uv,
        tangent: [1.0, 0.0, 0.0, 1.0],
    };
    let mut mesh = MeshData {
        vertices: Vec::with_capacity(xs.len() * zs.len()),
        indices: Vec::new(),
    };
    for &z in &zs {
        for &x in &xs {
            mesh.vertices.push(vertex(
                position(data, size, x, z),
                normal(data, size, x, z),
                uv(x, z),
            ));
        }
    }
    let w = xs.len() as u32;
    for j in 0..zs.len() as u32 - 1 {
        for i in 0..w - 1 {
            let a = j * w + i;
            let (b, c, d) = (a + 1, a + w, a + w + 1);
            mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    // Skirt: each edge again, pushed down, joined to the edge.
    let depth = (size.y * 0.1).max(1.0);
    let edges: [Vec<u32>; 4] = [
        (0..w).collect(), // z = cz
        (0..w)
            .rev()
            .map(|i| (zs.len() as u32 - 1) * w + i)
            .collect(), // z = z1
        (0..zs.len() as u32).rev().map(|j| j * w).collect(), // x = cx
        (0..zs.len() as u32).map(|j| j * w + w - 1).collect(), // x = x1
    ];
    for edge in edges {
        let base = mesh.vertices.len() as u32;
        for &i in &edge {
            let mut v = mesh.vertices[i as usize];
            v.position[1] -= depth;
            mesh.vertices.push(v);
        }
        for k in 0..edge.len() as u32 - 1 {
            let (top0, top1) = (edge[k as usize], edge[k as usize + 1]);
            let (low0, low1) = (base + k, base + k + 1);
            // Both windings: which side faces out depends on the edge.
            mesh.indices
                .extend_from_slice(&[top0, low0, top1, top1, low0, low1]);
            mesh.indices
                .extend_from_slice(&[top0, top1, low0, top1, low1, low0]);
        }
    }
    mesh
}

/// Mesh handles of one chunk: the full grid and its simpler copies.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkMeshes {
    pub origin: (u32, u32),
    pub mesh: MeshHandle,
    pub lods: Vec<MeshHandle>,
}

/// Adds every chunk with its levels of detail to `meshes`.
pub fn add_chunks(meshes: &mut Meshes, data: &TerrainData, size: Vec3) -> Vec<ChunkMeshes> {
    chunks(data)
        .into_iter()
        .map(|origin| {
            let mesh = meshes.add(chunk_mesh(data, size, origin, 1));
            let lods: Vec<MeshHandle> = LODS
                .iter()
                .map(|(step, _)| meshes.add(chunk_mesh(data, size, origin, *step)))
                .collect();
            set_lods(meshes, mesh, &lods);
            ChunkMeshes { origin, mesh, lods }
        })
        .collect()
}

fn set_lods(meshes: &mut Meshes, mesh: MeshHandle, lods: &[MeshHandle]) {
    meshes.set_lods(
        mesh,
        lods.iter()
            .zip(LODS)
            .map(|(m, (_, below))| MeshLod { mesh: *m, below })
            .collect(),
    );
}

/// Rebuilds the chunks that touch the vertex rectangle `min..=max` (after a
/// brush stroke): their meshes keep their handles.
pub fn update_chunks(
    meshes: &mut Meshes,
    chunks: &[ChunkMeshes],
    data: &TerrainData,
    size: Vec3,
    (min, max): ((u32, u32), (u32, u32)),
) {
    for c in chunks {
        let (cx, cz) = c.origin;
        // A chunk's normals read one vertex past its edge.
        let touches = min.0 <= cx + CHUNK + 1
            && max.0 + 1 >= cx
            && min.1 <= cz + CHUNK + 1
            && max.1 + 1 >= cz;
        if !touches {
            continue;
        }
        meshes.set(c.mesh, chunk_mesh(data, size, c.origin, 1));
        for (m, (step, _)) in c.lods.iter().zip(LODS) {
            meshes.set(*m, chunk_mesh(data, size, c.origin, step));
        }
        set_lods(meshes, c.mesh, &c.lods);
    }
}

/// The whole grid as triangles, for a static mesh collider.
pub fn collision(data: &TerrainData, size: Vec3) -> MeshGeometry {
    let n = data.resolution();
    let mut positions = Vec::with_capacity((n * n) as usize);
    for z in 0..n {
        for x in 0..n {
            positions.push(position(data, size, x, z));
        }
    }
    let mut indices = Vec::with_capacity(((n - 1) * (n - 1) * 2) as usize);
    for z in 0..n - 1 {
        for x in 0..n - 1 {
            let a = z * n + x;
            let (b, c, d) = (a + 1, a + n, a + n + 1);
            indices.push([a, c, b]);
            indices.push([b, c, d]);
        }
    }
    MeshGeometry { positions, indices }
}

/// Height of the surface at local `(x, z)`, or `None` outside the terrain.
pub fn height_at(data: &TerrainData, size: Vec3, at: Vec2) -> Option<f32> {
    let u = at.x / size.x + 0.5;
    let v = at.y / size.z + 0.5;
    ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then(|| data.sample(u, v) * size.y)
}

/// Where a ray (local space) first meets the surface.
pub fn raycast(data: &TerrainData, size: Vec3, origin: Vec3, dir: Vec3) -> Option<Vec3> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    // Clip the ray to the terrain's box, then march half a cell at a time.
    let half = Vec3::new(size.x * 0.5, size.y.max(0.0), size.z * 0.5);
    let (lo, hi) = (
        Vec3::new(-half.x, -1.0, -half.z),
        Vec3::new(half.x, half.y + 1.0, half.z),
    );
    let inv = dir.recip();
    let (t0, t1) = ((lo - origin) * inv, (hi - origin) * inv);
    let enter = t0.min(t1).max_element().max(0.0);
    let exit = t0.max(t1).min_element();
    // NaN (a ray parallel to a face) counts as a miss too.
    if enter
        .partial_cmp(&exit)
        .is_none_or(|o| o == std::cmp::Ordering::Greater)
    {
        return None;
    }
    let cell = (size.x.min(size.z) / (data.resolution() - 1) as f32).max(1e-3);
    let step = cell * 0.5;
    let above = |t: f32| {
        let p = origin + dir * t;
        height_at(data, size, Vec2::new(p.x, p.z)).map(|h| p.y - h)
    };
    let mut prev = enter;
    let mut t = enter;
    while t <= exit {
        if above(t).is_some_and(|d| d <= 0.0) {
            // Refine between the last point above and this one.
            let (mut a, mut b) = (prev, t);
            for _ in 0..20 {
                let m = (a + b) * 0.5;
                if above(m).is_some_and(|d| d <= 0.0) {
                    b = m;
                } else {
                    a = m;
                }
            }
            return Some(origin + dir * b);
        }
        prev = t;
        t += step;
    }
    None
}

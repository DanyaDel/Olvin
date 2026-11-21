//! Random operation sequences: nothing may panic and `validate` must hold.

use glam::Vec3;

use crate::PolyMesh;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn f(&mut self) -> f32 {
        (self.next() % 2000) as f32 / 1000.0 - 1.0
    }
    fn vec(&mut self) -> Vec3 {
        Vec3::new(self.f(), self.f(), self.f())
    }
    fn verts(&mut self, m: &PolyMesh, max: usize) -> Vec<u32> {
        let n = m.positions.len();
        (0..self.below(max) + 1)
            .map(|_| self.below(n) as u32)
            .collect()
    }
    fn faces(&mut self, m: &PolyMesh, max: usize) -> Vec<usize> {
        let n = m.faces.len();
        if n == 0 {
            return Vec::new();
        }
        (0..self.below(max) + 1).map(|_| self.below(n)).collect()
    }
    fn edge(&mut self, m: &PolyMesh) -> [u32; 2] {
        let e = m.edges();
        if e.is_empty() {
            [0, 1]
        } else {
            e[self.below(e.len())]
        }
    }
}

fn step(m: &mut PolyMesh, r: &mut Rng) -> &'static str {
    if m.positions.is_empty() {
        *m = PolyMesh::cube(1.0);
    }
    match r.below(26) {
        0 => {
            let f = r.faces(m, 3);
            m.extrude_faces(&f, r.vec());
            "extrude_faces"
        }
        1 => {
            let f = r.faces(m, 3);
            m.inset_faces(&f, r.f());
            "inset_faces"
        }
        2 => {
            let e = [r.edge(m)];
            let _ = m.split_edge(e[0][0], e[0][1], r.f().abs());
            "split_edge"
        }
        3 => {
            let (a, b) = (
                r.below(m.positions.len()) as u32,
                r.below(m.positions.len()) as u32,
            );
            let _ = m.connect_vertices(a, b);
            "connect_vertices"
        }
        4 => {
            let e = r.edge(m);
            let _ = m.loop_cut(e[0], e[1], r.below(4) as u32);
            "loop_cut"
        }
        5 => {
            let f = r.faces(m, 3);
            m.subdivide_faces(&f, r.below(3) as u32);
            "subdivide_faces"
        }
        6 => {
            let v = r.verts(m, 3);
            m.delete_vertices(&v);
            "delete_vertices"
        }
        7 => {
            let e = vec![r.edge(m)];
            m.delete_edges(&e);
            "delete_edges"
        }
        8 => {
            let f = r.faces(m, 3);
            m.delete_faces(&f, r.below(2) == 0);
            "delete_faces"
        }
        9 => {
            let e = vec![r.edge(m), r.edge(m)];
            m.dissolve_edges(&e);
            "dissolve_edges"
        }
        10 => {
            let v = r.verts(m, 4);
            let at = r.vec();
            m.merge_vertices(&v, at);
            "merge_vertices"
        }
        11 => {
            m.merge_by_distance(r.f().abs() * 0.5);
            "merge_by_distance"
        }
        12 => {
            let v = r.verts(m, 4);
            let _ = m.make_face(&v);
            "make_face"
        }
        13 => {
            let e = vec![r.edge(m), r.edge(m)];
            let _ = m.fill_edges(&e);
            "fill_edges"
        }
        14 => {
            let e = vec![r.edge(m)];
            m.extrude_edges(&e, r.vec());
            "extrude_edges"
        }
        15 => {
            let v = r.verts(m, 2);
            m.extrude_vertices(&v, r.vec());
            "extrude_vertices"
        }
        16 => {
            let f = r.faces(m, 4);
            m.flip_faces(&f);
            "flip_faces"
        }
        17 => {
            m.recalculate_normals_outside();
            "recalc_normals"
        }
        18 => {
            let f = r.faces(m, 4);
            m.unwrap_faces(&f, 0.01);
            "unwrap_faces"
        }
        19 => {
            let f = r.faces(m, 4);
            m.project_box(&f, 1.0);
            m.project_planar(&f, None, 1.0);
            m.project_cylinder(&f, Vec3::Y);
            m.project_sphere(&f, Vec3::Y);
            "project"
        }
        20 => {
            let v = r.verts(m, 4);
            m.duplicate(&v);
            "duplicate"
        }
        21 => {
            let e = vec![r.edge(m)];
            m.mark_seam(&e, true);
            m.pack_islands(0.02);
            "seam/pack"
        }
        22 => {
            let f = r.faces(m, 4);
            if !f.is_empty() {
                m.ensure_uvs();
                m.follow_active_quads(&f, f[0]);
            }
            "follow_active_quads"
        }
        23 => {
            let _ = m.triangulate();
            let _ = m.triangulate_uv();
            "triangulate"
        }
        24 => {
            let v = r.verts(m, 4);
            m.translate_vertices(&v, r.vec());
            m.compact();
            "translate/compact"
        }
        _ => {
            let k = *m.positions.last().unwrap();
            m.positions.push(k);
            "dup point"
        }
    }
}

#[test]
fn random_operations_keep_the_mesh_valid() {
    for seed in 1..=300u64 {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut m = match seed % 4 {
            0 => PolyMesh::cube(1.0),
            1 => PolyMesh::plane(2.0),
            // Collapsed geometry: zero-area faces everywhere.
            2 => PolyMesh::cube(0.0),
            _ => PolyMesh::plane(0.0),
        };
        let mut log = Vec::new();
        for _ in 0..25 {
            let before = m.clone();
            let name = step(&mut m, &mut r);
            log.push(name);
            if m.faces.len() > 400 {
                m = PolyMesh::cube(1.0);
            }
            if let Err(e) = m.validate() {
                panic!("seed {seed}: {log:?} broke the mesh: {e}\nbefore: {before:?}");
            }
        }
    }
}

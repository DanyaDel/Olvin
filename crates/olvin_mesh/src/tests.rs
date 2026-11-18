use std::collections::HashMap;

use glam::{Vec2, Vec3};

use crate::{MeshError, PolyMesh, ear_clip, edge_key, loop_edges};

const EPS: f32 = 1e-5;

/// Every edge is used by exactly two faces, in opposite directions.
fn assert_closed(m: &PolyMesh) {
    m.validate().unwrap();
    let mut directed: HashMap<(u32, u32), u32> = HashMap::new();
    for f in &m.faces {
        for e in loop_edges(f) {
            *directed.entry(e).or_default() += 1;
        }
    }
    for (&(a, b), &n) in &directed {
        assert_eq!(n, 1, "directed edge {a}->{b} used {n} times");
        assert!(directed.contains_key(&(b, a)), "edge {a}-{b} is a border");
    }
}

/// For convex meshes: every face points away from the centroid.
fn assert_outward(m: &PolyMesh) {
    let c = m.centroid();
    for f in 0..m.faces.len() {
        assert!(
            m.face_normal(f).dot(m.face_center(f) - c) > 0.0,
            "face {f} points inwards"
        );
    }
}

fn total_area(m: &PolyMesh) -> f32 {
    (0..m.faces.len())
        .map(|f| m.face_area_vector(f).length())
        .sum()
}

fn top_face(m: &PolyMesh) -> usize {
    (0..m.faces.len())
        .find(|&f| m.face_normal(f).dot(Vec3::Y) > 0.99)
        .unwrap()
}

#[test]
fn cube_is_closed_and_outward() {
    let c = PolyMesh::cube(2.0);
    assert_eq!(
        (c.positions.len(), c.faces.len(), c.edges().len()),
        (8, 6, 12)
    );
    assert_closed(&c);
    assert_outward(&c);
    assert!((total_area(&c) - 24.0).abs() < EPS);
}

#[test]
fn plane_faces_up() {
    let p = PolyMesh::plane(2.0);
    assert!(p.face_normal(0).abs_diff_eq(Vec3::Y, EPS));
    assert!((total_area(&p) - 4.0).abs() < EPS);
}

#[test]
fn from_polygons_welds() {
    let c = PolyMesh::cube(1.0);
    let polys: Vec<Vec<Vec3>> = (0..6)
        .map(|f| c.face_positions(f).map(|p| p + Vec3::splat(1e-6)).collect())
        .collect();
    let m = PolyMesh::from_polygons(&polys, 1e-4);
    assert_eq!(m.positions.len(), 8);
    assert_closed(&m);
    let raw = PolyMesh::from_polygons(&polys, 0.0);
    assert_eq!(
        raw.positions.len(),
        8,
        "exact duplicates weld even with eps 0"
    );
}

#[test]
fn extrude_top_face() {
    let mut c = PolyMesh::cube(1.0);
    let top = top_face(&c);
    let moved = c.extrude_faces(&[top], Vec3::Y);
    assert_eq!(moved, vec![top]);
    assert_eq!(c.faces.len(), 10);
    assert_eq!(c.positions.len(), 12);
    assert_closed(&c);
    assert_outward(&c);
    assert!((c.face_center(top).y - 1.5).abs() < EPS);
    assert!(c.face_normal(top).abs_diff_eq(Vec3::Y, EPS));
}

#[test]
fn region_extrude_has_no_internal_walls() {
    let mut c = PolyMesh::cube(1.0);
    let top = top_face(&c);
    let side = (0..6)
        .find(|&f| c.face_normal(f).dot(Vec3::X) > 0.99)
        .unwrap();
    c.extrude_faces(&[top, side], Vec3::new(0.5, 0.5, 0.0));
    // 6 border edges of the two-face region -> 6 walls, none on the shared edge.
    assert_eq!(c.faces.len(), 12);
    assert_eq!(c.positions.len(), 14);
    assert_closed(&c);
}

#[test]
fn region_extrude_moves_interior_vertices_in_place() {
    let mut p = PolyMesh::plane(2.0);
    let faces = p.subdivide_faces(&[0], 1);
    assert_eq!(faces.len(), 4);
    let before = p.positions.len();
    p.extrude_faces(&faces, Vec3::Y);
    // The centre vertex is used only by selected faces, so it just moves.
    assert_eq!(p.positions.len(), before + 8);
    assert_eq!(p.faces.len(), 4 + 8);
    p.validate().unwrap();
    for f in faces {
        assert!(p.face_center(f).y > 0.99);
    }
}

#[test]
fn extrude_lone_plane_leaves_bottom_open() {
    let mut p = PolyMesh::plane(1.0);
    p.extrude_faces(&[0], Vec3::Y);
    assert_eq!(p.faces.len(), 5);
    p.recalculate_normals_outside();
    p.validate().unwrap();
}

#[test]
fn extrude_edges_and_vertices() {
    let mut p = PolyMesh::plane(1.0);
    let new = p.extrude_edges(&[[2, 3]], Vec3::X);
    assert_eq!(new.len(), 2);
    assert_eq!(p.faces.len(), 2);
    // The new quad continues the plane's winding: shared edge runs opposite.
    assert!(p.face_normal(1).abs_diff_eq(Vec3::Y, EPS));
    p.validate().unwrap();

    let tip = p.extrude_vertices(&[0, 0], Vec3::NEG_Y);
    assert_eq!(tip.len(), 1);
    assert_eq!(p.wire, vec![edge_key(0, tip[0])]);
    let more = p.extrude_edges(&[[0, tip[0]]], Vec3::NEG_X);
    assert_eq!(more.len(), 2);
    assert!(p.wire.is_empty(), "the wire edge became a face edge");
    p.validate().unwrap();
}

#[test]
fn inset_quad() {
    let mut c = PolyMesh::cube(1.0);
    let top = top_face(&c);
    let inner = c.inset_faces(&[top], 0.1);
    assert_eq!(inner, vec![top]);
    assert_eq!(c.faces.len(), 6 + 4);
    assert_closed(&c);
    assert!((c.face_area_vector(top).length() - 0.64).abs() < 1e-4);
    assert!(c.face_normal(top).abs_diff_eq(Vec3::Y, EPS));
    assert!((total_area(&c) - 6.0).abs() < 1e-4);
}

#[test]
fn split_edge_updates_all_faces() {
    let mut c = PolyMesh::cube(1.0);
    let users = c.faces_with_edge(2, 6);
    assert_eq!(users.len(), 2);
    let m = c.split_edge(2, 6, 0.25).unwrap();
    assert!(c.positions[m as usize].abs_diff_eq(c.positions[2].lerp(c.positions[6], 0.25), EPS));
    for f in users {
        assert_eq!(c.faces[f].len(), 5);
    }
    assert_closed(&c);
    assert_eq!(c.split_edge(0, 7, 0.5), Err(MeshError::NoSuchEdge(0, 7)));
}

#[test]
fn split_wire_edge() {
    let mut m = PolyMesh {
        positions: vec![Vec3::ZERO, Vec3::X],
        faces: vec![],
        wire: vec![[0, 1]],
    };
    let v = m.split_edge(0, 1, 0.5).unwrap();
    assert_eq!(m.edges(), vec![[0, v], [1, v]]);
}

#[test]
fn connect_vertices_splits_face() {
    let mut p = PolyMesh::plane(1.0);
    assert_eq!(p.connect_vertices(0, 1), Err(MeshError::AlreadyConnected));
    p.connect_vertices(0, 2).unwrap();
    assert_eq!(p.faces.len(), 2);
    for f in 0..2 {
        assert_eq!(p.faces[f].len(), 3);
        assert!(p.face_normal(f).abs_diff_eq(Vec3::Y, EPS));
    }
    assert!((total_area(&p) - 1.0).abs() < EPS);

    let mut c = PolyMesh::cube(1.0);
    assert_eq!(c.connect_vertices(0, 7), Err(MeshError::NoSharedFace));
}

#[test]
fn knife_across_quad() {
    let mut c = PolyMesh::cube(1.0);
    let top = top_face(&c);
    let f = c.faces[top].clone();
    let [m1, m2] = c.knife((f[0], f[1], 0.5), (f[2], f[3], 0.5)).unwrap();
    assert_eq!(c.faces.len(), 7);
    let halves: Vec<usize> = (0..c.faces.len())
        .filter(|&i| c.faces[i].contains(&m1) && c.faces[i].contains(&m2))
        .collect();
    assert_eq!(halves.len(), 2);
    for h in halves {
        assert!(c.face_normal(h).abs_diff_eq(Vec3::Y, EPS));
        assert!((c.face_area_vector(h).length() - 0.5).abs() < EPS);
    }
    assert_closed(&c);
    assert_eq!(
        c.knife((0, 1, 0.5), (6, 7, 0.5)),
        Err(MeshError::NoSharedFace)
    );
}

#[test]
fn loop_cut_around_cube() {
    let mut c = PolyMesh::cube(1.0);
    let new = c.loop_cut(2, 6, 1).unwrap();
    assert_eq!(new.len(), 4);
    assert_eq!(c.faces.len(), 10);
    assert_closed(&c);
    assert_outward(&c);
    // The loop lies in the plane z = 0.
    for v in &new {
        assert!(c.positions[*v as usize].z.abs() < EPS);
    }

    let mut c = PolyMesh::cube(1.0);
    let new = c.loop_cut(2, 6, 3).unwrap();
    assert_eq!(new.len(), 12);
    assert_eq!(c.faces.len(), 6 + 4 * 3);
    assert_closed(&c);
    assert!((total_area(&c) - 6.0).abs() < 1e-4);
}

#[test]
fn loop_cut_open_strip() {
    // A 2x1 strip of quads: the ring crosses both and stops at the borders.
    let mut p = PolyMesh::plane(2.0);
    p.knife((0, 1, 0.5), (2, 3, 0.5)).unwrap();
    assert_eq!(p.faces.len(), 2);
    let new = p.loop_cut(0, 3, 1).unwrap();
    assert_eq!(new.len(), 3);
    assert_eq!(p.faces.len(), 4);
    p.validate().unwrap();
    for f in 0..p.faces.len() {
        assert!(p.face_normal(f).abs_diff_eq(Vec3::Y, EPS));
    }
    assert!((total_area(&p) - 4.0).abs() < 1e-4);
}

#[test]
fn delete_faces_keep_edges_then_make_face() {
    let mut c = PolyMesh::cube(1.0);
    let top = top_face(&c);
    let ring = c.faces[top].clone();
    let remap = c.delete_faces(&[top], true);
    assert_eq!(remap.face(top), None);
    assert_eq!(c.faces.len(), 5);
    // The border edges survive, still owned by the side faces.
    assert!(c.wire.is_empty());
    assert_eq!(c.edges().len(), 12);
    assert_eq!(c.positions.len(), 8);
    c.validate().unwrap();

    // Scrambled selection order must not matter.
    let f = c
        .make_face(&[ring[0], ring[2], ring[1], ring[3]])
        .unwrap()
        .unwrap();
    assert!(c.wire.is_empty());
    assert!(c.face_normal(f).abs_diff_eq(Vec3::Y, EPS));
    assert_closed(&c);
    assert_eq!(c.make_face(&ring), Err(MeshError::FaceExists));
}

#[test]
fn delete_faces_wire_or_loose() {
    let mut p = PolyMesh::plane(1.0);
    p.delete_faces(&[0], true);
    assert!(p.faces.is_empty());
    assert_eq!(p.wire, vec![[0, 1], [1, 2], [2, 3], [0, 3]]);
    p.validate().unwrap();
    p.make_face(&[3, 1, 0, 2]).unwrap();
    assert!(p.wire.is_empty());
    assert!(
        p.face_normal(0).abs_diff_eq(Vec3::Y, EPS)
            || p.face_normal(0).abs_diff_eq(Vec3::NEG_Y, EPS)
    );

    let mut p = PolyMesh::plane(1.0);
    p.delete_faces(&[0], false);
    assert_eq!(p, PolyMesh::default());
}

#[test]
fn make_face_orientation_without_neighbours() {
    let mut m = PolyMesh {
        positions: vec![
            Vec3::ZERO,
            Vec3::X,
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::Z,
            Vec3::new(0.5, -1.0, 0.5),
        ],
        ..Default::default()
    };
    // Mesh centroid is below the square, so the face must point up.
    let f = m.make_face(&[0, 1, 2, 3]).unwrap().unwrap();
    assert!(m.face_normal(f).abs_diff_eq(Vec3::Y, EPS));
    assert_eq!(m.make_face(&[0, 4]), Ok(None));
    assert_eq!(m.wire, vec![[0, 4]]);
    assert_eq!(m.make_face(&[0]), Err(MeshError::NotEnoughVertices));
}

#[test]
fn fill_edges_loop_chain_and_bridge() {
    let mut c = PolyMesh::cube(1.0);
    let top = top_face(&c);
    let ring = c.faces[top].clone();
    c.delete_faces(&[top], true);
    let edges: Vec<[u32; 2]> = loop_edges(&ring).map(|(a, b)| [a, b]).collect();
    c.fill_edges(&edges).unwrap();
    assert_closed(&c);

    // Open chain of three edges is closed by the fourth.
    let mut c = PolyMesh::cube(1.0);
    c.delete_faces(&[top], true);
    c.fill_edges(&edges[..3]).unwrap();
    assert_closed(&c);

    // Two opposite border edges of a cube without top and bottom.
    let mut c = PolyMesh::cube(1.0);
    let bottom = (0..6)
        .find(|&f| c.face_normal(f).dot(Vec3::NEG_Y) > 0.99)
        .unwrap();
    let r0 = c.faces[top].clone();
    let remap = c.delete_faces(&[top, bottom], false);
    let r0: Vec<u32> = r0.iter().map(|&v| remap.vertex(v).unwrap()).collect();
    let f = c.fill_edges(&[[r0[0], r0[1]], [r0[2], r0[3]]]).unwrap();
    assert!(c.face_normal(f).abs_diff_eq(Vec3::Y, EPS));
    c.validate().unwrap();

    assert_eq!(c.fill_edges(&[[0, 1]]), Err(MeshError::NotAChain));
}

#[test]
fn delete_vertices_and_edges() {
    let mut c = PolyMesh::cube(1.0);
    let remap = c.delete_vertices(&[7]);
    assert_eq!(c.positions.len(), 7);
    assert_eq!(c.faces.len(), 3);
    assert!(c.wire.is_empty());
    assert_eq!(remap.vertex(7), None);
    assert_eq!(remap.vertex(6), Some(6));
    c.validate().unwrap();

    let mut p = PolyMesh::plane(1.0);
    let remap = p.delete_edges(&[[1, 0]]);
    assert!(p.faces.is_empty());
    assert_eq!(p.wire.len(), 3);
    assert_eq!(p.positions.len(), 4);
    assert_eq!(remap.face(0), None);
    p.validate().unwrap();
}

#[test]
fn merge_vertices_collapses_and_cleans() {
    let mut c = PolyMesh::cube(1.0);
    let top = top_face(&c);
    let ring = c.faces[top].clone();
    let (v, remap) = c.merge_vertices(&ring, Vec3::new(0.0, 1.0, 0.0)).unwrap();
    // Top face vanishes, sides become triangles: a pyramid.
    assert_eq!(c.positions.len(), 5);
    assert_eq!(c.faces.len(), 5);
    assert_eq!(remap.face(top), None);
    assert!(c.positions[v as usize].abs_diff_eq(Vec3::Y, EPS));
    assert_eq!(c.faces.iter().filter(|f| f.len() == 3).count(), 4);
    assert_closed(&c);
    assert_outward(&c);

    let mut p = PolyMesh::plane(1.0);
    p.merge_vertices(&[0, 2], Vec3::ZERO).unwrap();
    // The quad folds onto itself and leaves its edges as wire.
    assert!(p.faces.is_empty());
    assert_eq!(p.positions.len(), 3);
    assert_eq!(p.wire.len(), 2);
    p.validate().unwrap();
}

#[test]
fn merge_by_distance_rebuilds_cube() {
    let c = PolyMesh::cube(1.0);
    let polys: Vec<Vec<Vec3>> = (0..6).map(|f| c.face_positions(f).collect()).collect();
    let mut m = PolyMesh::default();
    for poly in &polys {
        let base = m.positions.len() as u32;
        m.positions.extend(poly);
        m.faces.push((base..base + poly.len() as u32).collect());
    }
    assert_eq!(m.positions.len(), 24);
    let (removed, remap) = m.merge_by_distance(1e-4);
    assert_eq!(removed, 16);
    assert_eq!(remap.vertices.iter().flatten().count(), 8);
    assert_closed(&m);
}

#[test]
fn triangulate_concave_l() {
    let pts = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(2.0, 0.0, 0.0),
        Vec3::new(2.0, 1.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(1.0, 2.0, 0.0),
        Vec3::new(0.0, 2.0, 0.0),
    ];
    // Rotate so the projection is not trivially the XY plane.
    let rot = glam::Quat::from_euler(glam::EulerRot::XYZ, 0.3, 0.7, -0.2);
    let poly: Vec<Vec3> = pts.iter().map(|&p| rot * p).collect();
    let m = PolyMesh::from_polygons(&[poly], 0.0);
    let normal = rot * Vec3::Z;
    let (pos, nrm, idx) = m.triangulate();
    assert_eq!(pos.len(), 6);
    assert_eq!(idx.len(), 12);
    assert!(nrm.iter().all(|n| n.abs_diff_eq(normal, 1e-5)));
    let mut area = 0.0;
    for t in idx.chunks(3) {
        let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
        let signed = (b - a).cross(c - a).dot(normal) * 0.5;
        assert!(signed > 1e-6, "triangle flipped or degenerate");
        area += signed;
        // Every triangle lies inside the L.
        let centroid = rot.inverse() * ((a + b + c) / 3.0);
        assert!(centroid.x < 1.0 + EPS || centroid.y < 1.0 + EPS);
    }
    assert!((area - 3.0).abs() < 1e-4);
}

#[test]
fn ear_clip_handles_collinear_and_clockwise() {
    let square_with_mid = [
        Vec2::ZERO,
        Vec2::new(1.0, 0.0),
        Vec2::new(2.0, 0.0),
        Vec2::new(2.0, 2.0),
        Vec2::new(0.0, 2.0),
    ];
    let tris = ear_clip(&square_with_mid);
    let area = |t: &[u32; 3], pts: &[Vec2]| {
        let (a, b, c) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
        (b - a).perp_dot(c - a) * 0.5
    };
    let total: f32 = tris.iter().map(|t| area(t, &square_with_mid)).sum();
    assert!((total - 4.0).abs() < EPS);

    let cw: Vec<Vec2> = square_with_mid.iter().rev().copied().collect();
    let total: f32 = ear_clip(&cw).iter().map(|t| area(t, &cw)).sum();
    assert!((total + 4.0).abs() < EPS);
}

#[test]
fn flip_and_recalculate_normals() {
    let mut c = PolyMesh::cube(1.0);
    c.flip_faces(&[0, 3, 3]);
    assert!(c.face_normal(0).dot(Vec3::X) < 0.0);
    c.recalculate_normals_outside();
    assert_closed(&c);
    assert_outward(&c);

    c.flip_faces(&[0, 1, 2, 3, 4, 5]);
    c.recalculate_normals_outside();
    assert_outward(&c);

    // Extruded geometry keeps the right orientation too.
    let top = top_face(&c);
    c.extrude_faces(&[top], Vec3::Y);
    let all: Vec<usize> = (0..c.faces.len()).collect();
    c.flip_faces(&all);
    c.recalculate_normals_outside();
    assert_closed(&c);
    assert_outward(&c);
}

#[test]
fn compact_remaps() {
    let mut c = PolyMesh::cube(1.0);
    c.positions.insert(0, Vec3::splat(9.0));
    for v in c.faces.iter_mut().flatten() {
        *v += 1;
    }
    let remap = c.compact();
    assert_eq!(remap[0], None);
    assert_eq!(remap[1], Some(0));
    assert_eq!(c, PolyMesh::cube(1.0));
}

#[test]
fn dissolve_restores_quad() {
    let mut p = PolyMesh::plane(1.0);
    p.connect_vertices(0, 2).unwrap();
    let remap = p.dissolve_edges(&[[2, 0]]);
    assert_eq!(p.faces.len(), 1);
    assert_eq!(remap.face(1), None);
    assert_eq!(p.faces[0].len(), 4);
    assert!(p.face_normal(0).abs_diff_eq(Vec3::Y, EPS));
    assert!((total_area(&p) - 1.0).abs() < EPS);

    let mut c = PolyMesh::cube(1.0);
    c.loop_cut(2, 6, 1).unwrap();
    let ring: Vec<[u32; 2]> = c
        .edges()
        .into_iter()
        .filter(|e| e.iter().all(|&v| v >= 8))
        .collect();
    assert_eq!(ring.len(), 4);
    c.dissolve_edges(&ring);
    assert_eq!(c.faces.len(), 6);
    assert_closed(&c);
}

#[test]
fn subdivide_and_catmull_clark() {
    let mut c = PolyMesh::cube(1.0);
    let top = top_face(&c);
    let new = c.subdivide_faces(&[top], 1);
    assert_eq!(new.len(), 4);
    assert_eq!(c.faces.len(), 9);
    assert_closed(&c);
    assert!((total_area(&c) - 6.0).abs() < 1e-4);

    let mut t = PolyMesh::from_polygons(&[vec![Vec3::ZERO, Vec3::X, Vec3::Y]], 0.0);
    assert_eq!(t.subdivide_faces(&[0], 2).len(), 9);
    t.validate().unwrap();

    let s = PolyMesh::cube(1.0).catmull_clark(2);
    assert_eq!(s.faces.len(), 96);
    assert_eq!(s.positions.len(), 98);
    assert_closed(&s);
    assert_outward(&s);
    // Smoothing shrinks the cube towards a sphere.
    assert!(s.positions.iter().all(|p| p.length() < 0.87));
}

#[test]
fn ops_on_invalid_input_do_not_panic() {
    let mut c = PolyMesh::cube(1.0);
    assert_eq!(c.extrude_faces(&[99], Vec3::Y), Vec::<usize>::new());
    assert_eq!(c.split_edge(0, 99, 0.5), Err(MeshError::InvalidVertex(99)));
    assert!(c.merge_vertices(&[99], Vec3::ZERO).is_none());
    c.delete_faces(&[99], false);
    c.delete_vertices(&[99]);
    assert_eq!(c.loop_cut(0, 7, 1), Err(MeshError::NoSuchEdge(0, 7)));
    assert_closed(&c);
    assert_eq!(c.edges().len(), 12);
}

/// Cube with every face cut into 2×2 quads: all mid-edge and face-centre
/// vertices are 4-valent, corners are poles.
fn grid_cube() -> PolyMesh {
    let mut m = PolyMesh::cube(2.0);
    m.subdivide_faces(&[0, 1, 2, 3, 4, 5], 1);
    m
}

/// An edge from the +Y face centre to a mid-edge vertex.
fn belt_edge(m: &PolyMesh) -> (u32, u32) {
    let at = |p: Vec3| {
        m.positions
            .iter()
            .position(|q| q.distance(p) < EPS)
            .unwrap() as u32
    };
    (at(Vec3::Y), at(Vec3::new(1.0, 1.0, 0.0)))
}

#[test]
fn edge_loop_goes_around_the_cube() {
    let m = grid_cube();
    let (a, b) = belt_edge(&m);
    let edges = m.edge_loop(a, b);
    assert_eq!(edges.len(), 8, "a closed belt through four face centres");
    // Closed: every vertex of the loop is used by exactly two of its edges.
    let mut uses: HashMap<u32, u32> = HashMap::new();
    for v in edges.iter().flatten() {
        *uses.entry(*v).or_default() += 1;
    }
    assert!(uses.values().all(|&n| n == 2));
    assert!(
        edges
            .iter()
            .flatten()
            .all(|v| m.positions[*v as usize].z.abs() < EPS)
    );
}

#[test]
fn edge_loop_stops_at_poles_and_follows_borders() {
    let mut m = PolyMesh::plane(3.0);
    m.subdivide_faces(&[0], 2);
    // An interior edge crosses the 3×3 grid: 3 edges, ending at the border.
    let at = |p: Vec3| {
        m.positions
            .iter()
            .position(|q| q.distance(p) < EPS)
            .unwrap() as u32
    };
    let (a, b) = (
        at(Vec3::new(-0.5, 0.0, -0.5)),
        at(Vec3::new(0.5, 0.0, -0.5)),
    );
    assert_eq!(m.edge_loop(a, b).len(), 3);
    // A border edge walks the whole border.
    let (c, d) = (
        at(Vec3::new(-1.5, 0.0, -1.5)),
        at(Vec3::new(-0.5, 0.0, -1.5)),
    );
    assert_eq!(m.edge_loop(c, d).len(), 12);
    assert!(m.edge_loop(a, a).is_empty());
}

#[test]
fn ring_selects_face_loop() {
    let m = grid_cube();
    let (a, b) = belt_edge(&m);
    let (faces, edges) = m.ring(a, b);
    assert_eq!(faces.len(), 8, "the belt of quads crossing the edge");
    assert_eq!(edges.len(), 8);
    for f in faces {
        let c = m.face_center(f);
        assert!(c.x.abs() > 0.4 || c.y.abs() > 0.4, "belt faces around Z");
    }
}

#[test]
fn linked_grow_shrink() {
    let mut m = PolyMesh::cube(1.0);
    let copies = m.duplicate(&(0..8).collect::<Vec<_>>());
    m.translate_vertices(&copies, Vec3::X * 3.0);
    m.validate().unwrap();
    assert_eq!((m.positions.len(), m.faces.len()), (16, 12));
    assert_eq!(m.linked(&[0]), (0..8).collect::<Vec<_>>());
    assert_eq!(m.linked(&[copies[3]]), (8..16).collect::<Vec<_>>());

    let mut g = PolyMesh::plane(3.0);
    g.subdivide_faces(&[0], 2);
    let centre: Vec<u32> =
        g.faces[(0..9).find(|&f| g.face_center(f).length() < EPS).unwrap()].clone();
    let grown = g.grow_selection(&centre);
    assert_eq!(grown.len(), 16, "the whole 3×3 grid");
    assert_eq!(
        g.shrink_selection(&grown).len(),
        16,
        "border of the mesh is kept"
    );
    assert!(g.shrink_selection(&centre).is_empty());
}

#[test]
fn duplicate_edges_and_vertices() {
    let mut m = PolyMesh::cube(1.0);
    let copies = m.duplicate(&[0, 1, 2]);
    assert_eq!(copies.len(), 3);
    assert_eq!(m.faces.len(), 6, "no face lies entirely in the selection");
    assert_eq!(m.wire.len(), 2, "edges 0-1 and 0-2 are copied as wire");
    m.validate().unwrap();
}

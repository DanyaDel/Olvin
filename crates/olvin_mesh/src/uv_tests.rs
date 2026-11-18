use std::f32::consts::TAU;

use glam::{Affine2, Mat4, Vec2, Vec3};

use crate::uv::signed_area;
use crate::{PolyMesh, box_uv, edge_key};

const EPS: f32 = 1e-4;

fn top_face(m: &PolyMesh) -> usize {
    (0..m.faces.len())
        .find(|&f| m.face_normal(f).dot(Vec3::Y) > 0.99)
        .unwrap()
}

fn boxed(mut m: PolyMesh) -> PolyMesh {
    assert!(m.ensure_uvs());
    m
}

/// UVs present, parallel, finite.
fn assert_uvs(m: &PolyMesh) {
    m.validate().unwrap();
    assert_eq!(m.uvs.len(), m.faces.len(), "uvs parallel to faces");
}

/// Every corner carries exactly its box projection (true for any planar
/// subdivision of box-projected axis-aligned faces).
fn assert_box_mapped(m: &PolyMesh) {
    assert_uvs(m);
    for f in 0..m.faces.len() {
        let n = m.face_normal(f);
        for (i, p) in m.face_positions(f).enumerate() {
            let want = box_uv(p, n, 1.0);
            assert!(
                m.uvs[f][i].abs_diff_eq(want, EPS),
                "face {f} corner {i}: {} != {want}",
                m.uvs[f][i]
            );
        }
    }
}

fn point_in_polygon(p: Vec2, poly: &[Vec2]) -> bool {
    let n = poly.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

/// No face's UV triangles (sampled at their centroids) lie inside another face.
fn assert_no_overlap(m: &PolyMesh) {
    for f in 0..m.faces.len() {
        for t in m.face_triangles_local(f) {
            let c = t.iter().map(|&i| m.uvs[f][i as usize]).sum::<Vec2>() / 3.0;
            for g in (0..m.faces.len()).filter(|&g| g != f) {
                assert!(
                    !point_in_polygon(c, &m.uvs[g]),
                    "face {f} overlaps face {g} in UV"
                );
            }
        }
    }
}

fn assert_in_unit_square(m: &PolyMesh) {
    for t in m.uvs.iter().flatten() {
        assert!(t.is_finite(), "NaN uv");
        assert!(
            t.x >= -EPS && t.y >= -EPS && t.x <= 1.0 + EPS && t.y <= 1.0 + EPS,
            "{t} outside 0..1"
        );
    }
}

/// UV area / surface area of each face.
fn stretch(m: &PolyMesh, f: usize) -> f32 {
    signed_area(&m.uvs[f]) / m.face_area_vector(f).length()
}

fn cylinder(segments: usize, radius: f32, half: f32) -> PolyMesh {
    let ring = |y: f32| -> Vec<Vec3> {
        (0..segments)
            .map(|i| {
                let a = TAU * i as f32 / segments as f32;
                Vec3::new(a.cos() * radius, y, a.sin() * radius)
            })
            .collect()
    };
    let (bottom, top) = (ring(-half), ring(half));
    let mut polys = vec![bottom.clone(), top.clone()];
    for i in 0..segments {
        let j = (i + 1) % segments;
        polys.push(vec![bottom[i], top[i], top[j], bottom[j]]);
    }
    let mut m = PolyMesh::from_polygons(&polys, 1e-5);
    m.recalculate_normals_outside();
    m
}

// --- invariants through modelling operations ---------------------------------

#[test]
fn ensure_uvs_box_projects_once() {
    let mut c = PolyMesh::cube(2.0);
    assert!(!c.has_uvs());
    assert!(c.ensure_uvs());
    assert!(!c.ensure_uvs());
    assert_box_mapped(&c);
    // Box UVs of a cube: every face is its own island.
    assert_eq!(c.islands().len(), 6);
    let mut empty = PolyMesh::default();
    assert!(!empty.ensure_uvs());
    empty.validate().unwrap();
}

#[test]
fn validate_rejects_broken_uvs_and_seams() {
    let mut c = boxed(PolyMesh::cube(1.0));
    c.uvs.pop();
    assert!(c.validate().is_err());
    let mut c = boxed(PolyMesh::cube(1.0));
    c.uvs[2].push(Vec2::ZERO);
    assert!(c.validate().is_err());
    let mut c = boxed(PolyMesh::cube(1.0));
    c.uvs[0][0] = Vec2::NAN;
    assert!(c.validate().is_err());
    let mut c = PolyMesh::cube(1.0);
    c.seams.push([3, 1]);
    assert!(c.validate().is_err());
}

#[test]
fn extrude_faces_continue_neighbour_uvs() {
    let mut c = boxed(PolyMesh::cube(1.0));
    let top = top_face(&c);
    c.extrude_faces(&[top], Vec3::Y);
    // Walls continue the side faces' box UVs upwards: still box-mapped.
    assert_box_mapped(&c);
    // A two-face region extruded diagonally keeps valid UVs.
    let mut c = boxed(PolyMesh::cube(1.0));
    let top = top_face(&c);
    let side = (0..6).find(|&f| c.face_normal(f).x > 0.99).unwrap();
    c.extrude_faces(&[top, side], Vec3::new(0.5, 0.5, 0.0));
    assert_uvs(&c);
}

#[test]
fn extrude_lone_plane_walls_go_outward() {
    let mut p = boxed(PolyMesh::plane(2.0));
    p.extrude_faces(&[0], Vec3::Y * 0.5);
    assert_uvs(&p);
    assert_eq!(p.faces.len(), 5);
    for uv in &p.uvs[1..5] {
        // Height 0.5 on a 2 m edge: area 1, never mirrored, not on the plane.
        assert!((signed_area(uv) - 1.0).abs() < EPS, "{uv:?}");
    }
    assert_no_overlap(&p);
}

#[test]
fn extrude_edges_and_wire() {
    let mut p = boxed(PolyMesh::plane(2.0));
    let new = p.extrude_edges(&[[0, 1]], Vec3::Y);
    assert_eq!(new.len(), 2);
    assert_uvs(&p);
    assert!((signed_area(&p.uvs[1]) - 2.0).abs() < EPS);
    let w = p.extrude_vertices(&[new[0]], Vec3::X);
    p.extrude_edges(&[edge_key(new[0], w[0])], Vec3::Y);
    assert_uvs(&p);
    assert!((signed_area(&p.uvs[2]).abs() - 1.0).abs() < EPS);
}

#[test]
fn inset_split_knife_connect_interpolate() {
    let mut c = boxed(PolyMesh::cube(2.0));
    let top = top_face(&c);
    c.inset_faces(&[top], 0.25);
    assert_box_mapped(&c);
    let mut c = boxed(PolyMesh::cube(2.0));
    c.split_edge(0, 1, 0.25).unwrap();
    assert_box_mapped(&c);
    c.connect_vertices(0, 3).unwrap();
    assert_box_mapped(&c);
    let mut c = boxed(PolyMesh::cube(2.0));
    let f = top_face(&c);
    let face = c.faces[f].clone();
    c.knife((face[0], face[1], 0.3), (face[2], face[3], 0.6))
        .unwrap();
    assert_box_mapped(&c);
}

#[test]
fn loop_cut_subdivide_interpolate() {
    let mut c = boxed(PolyMesh::cube(2.0));
    c.loop_cut(0, 1, 2).unwrap();
    assert_box_mapped(&c);
    let mut c = boxed(PolyMesh::cube(2.0));
    let all: Vec<usize> = (0..6).collect();
    c.subdivide_faces(&all, 2);
    assert_eq!(c.faces.len(), 54);
    assert_box_mapped(&c);
    // Non-quads fan around the centre.
    let mut t =
        PolyMesh::from_polygons(&[vec![Vec3::ZERO, Vec3::X, Vec3::new(0.0, 0.0, -1.0)]], 0.0);
    t.ensure_uvs();
    t.subdivide_faces(&[0], 1);
    assert_box_mapped(&t);
}

#[test]
fn catmull_clark_interpolates_uvs_and_splits_seams() {
    let mut c = boxed(PolyMesh::cube(2.0));
    c.mark_seam(&[[0, 1]], true);
    let s = c.catmull_clark(2);
    assert_uvs(&s);
    assert_eq!(s.faces.len(), 96);
    assert_eq!(s.seams.len(), 4, "one seam edge, split twice");
    assert!(s.uvs.iter().flatten().all(|t| t.is_finite()));
}

#[test]
fn delete_dissolve_merge_carry_uvs() {
    let mut c = boxed(PolyMesh::cube(2.0));
    let top = top_face(&c);
    c.delete_faces(&[top], false);
    assert_box_mapped(&c);
    let mut c = boxed(PolyMesh::cube(2.0));
    c.delete_vertices(&[0]);
    assert_box_mapped(&c);
    let mut c = boxed(PolyMesh::cube(2.0));
    c.delete_edges(&[[0, 1]]);
    assert_box_mapped(&c);

    let mut p = boxed(PolyMesh::plane(2.0));
    let faces = p.subdivide_faces(&[0], 1);
    assert_eq!(faces.len(), 4);
    let inner: Vec<[u32; 2]> = p
        .edges()
        .into_iter()
        .filter(|&[a, b]| p.faces_with_edge(a, b).len() == 2)
        .collect();
    p.dissolve_edges(&inner);
    assert_eq!(p.faces.len(), 1);
    assert_box_mapped(&p);

    let mut c = boxed(PolyMesh::cube(2.0));
    c.merge_vertices(&[0, 1], Vec3::new(0.0, -1.0, -1.0))
        .unwrap();
    assert_uvs(&c);
    // Merging corners keeps every surviving corner's UV.
    let mut c = boxed(PolyMesh::cube(2.0));
    let before = c.clone();
    c.positions.push(c.positions[0]);
    let (n, _) = c.merge_by_distance(1e-4);
    assert_eq!(n, 1);
    assert_eq!(c, before);
}

#[test]
fn duplicate_flip_recalc_compact_fill() {
    let mut c = boxed(PolyMesh::cube(2.0));
    c.mark_seam(&[[0, 1]], true);
    let all: Vec<u32> = (0..8).collect();
    c.duplicate(&all);
    assert_eq!(c.faces.len(), 12);
    assert_eq!(c.seams.len(), 2);
    assert_box_mapped(&c);

    let mut c = boxed(PolyMesh::cube(2.0));
    let before = c.uvs[0].clone();
    c.flip_faces(&[0]);
    let mut rev = before.clone();
    rev.reverse();
    assert_eq!(c.uvs[0], rev);
    c.recalculate_normals_outside();
    assert_eq!(c.uvs[0], before);
    assert_box_mapped(&c);

    let mut c = boxed(PolyMesh::cube(2.0));
    c.positions.push(Vec3::splat(9.0));
    c.compact();
    assert_box_mapped(&c);

    let mut c = boxed(PolyMesh::cube(2.0));
    let top = top_face(&c);
    let verts = c.faces[top].clone();
    c.delete_faces(&[top], true);
    c.make_face(&verts).unwrap();
    assert_box_mapped(&c);
    let edges: Vec<[u32; 2]> = (0..4)
        .map(|i| edge_key(verts[i], verts[(i + 1) % 4]))
        .collect();
    let top = top_face(&c);
    c.delete_faces(&[top], true);
    c.fill_edges(&edges).unwrap();
    assert_box_mapped(&c);
}

#[test]
fn ops_without_uvs_stay_without() {
    let mut c = PolyMesh::cube(1.0);
    c.extrude_faces(&[0], Vec3::X);
    c.inset_faces(&[1], 0.1);
    c.loop_cut(0, 1, 1).unwrap();
    let all: Vec<usize> = (0..c.faces.len()).collect();
    c.subdivide_faces(&all, 1);
    let s = c.catmull_clark(1);
    assert!(c.uvs.is_empty() && s.uvs.is_empty());
    c.validate().unwrap();
}

// --- seams -----------------------------------------------------------------

#[test]
fn seams_mark_clear_and_follow_cuts() {
    let mut c = PolyMesh::cube(2.0);
    c.mark_seam(&[[1, 0], [0, 7], [0, 1]], true);
    assert_eq!(c.seams, vec![[0, 1]], "non-edges ignored, deduplicated");
    assert!(c.is_seam(1, 0));
    let m = c.split_edge(0, 1, 0.5).unwrap();
    assert!(c.is_seam(0, m) && c.is_seam(m, 1) && !c.is_seam(0, 1));
    c.validate().unwrap();
    c.mark_seam(&[[0, m]], false);
    assert_eq!(c.seams, vec![edge_key(m, 1)]);
    // Deleting the vertex drops its seams; remapping keeps the rest valid.
    c.delete_vertices(&[1]);
    assert!(c.seams.is_empty());
    c.validate().unwrap();
    // Merging the seam's ends drops it.
    let mut c = PolyMesh::cube(2.0);
    c.mark_seam(&[[0, 1], [2, 3]], true);
    c.merge_vertices(&[0, 1], Vec3::ZERO).unwrap();
    assert_eq!(c.seams.len(), 1);
    c.validate().unwrap();
}

// --- unwrap ----------------------------------------------------------------

#[test]
fn unwrap_cube_without_seams_gives_six_islands() {
    let mut c = PolyMesh::cube(2.0);
    let all: Vec<usize> = (0..6).collect();
    assert_eq!(c.unwrap_faces(&all, 0.02), 6);
    assert_uvs(&c);
    assert_in_unit_square(&c);
    assert_no_overlap(&c);
    assert_eq!(c.islands().len(), 6);
    let s: Vec<f32> = (0..6).map(|f| stretch(&c, f)).collect();
    let (lo, hi) = s
        .iter()
        .fold((f32::MAX, 0.0f32), |(l, h), &x| (l.min(x), h.max(x)));
    assert!(lo > 0.0 && hi / lo < 1.01, "equal texel density: {s:?}");
    // Squares stay squares.
    for uv in &c.uvs {
        let (a, b) = ((uv[1] - uv[0]).length(), (uv[2] - uv[1]).length());
        assert!((a - b).abs() < 1e-3, "{uv:?}");
    }
}

#[test]
fn unwrap_cube_cross_is_one_isometric_island() {
    let mut c = PolyMesh::cube(2.0);
    // Spanning tree of seams: top path 2-6-7-3 plus the four verticals.
    c.mark_seam(
        &[[2, 6], [6, 7], [3, 7], [0, 2], [1, 3], [4, 6], [5, 7]],
        true,
    );
    let all: Vec<usize> = (0..6).collect();
    assert_eq!(c.unwrap_faces(&all, 0.01), 1);
    assert_uvs(&c);
    assert_in_unit_square(&c);
    assert_no_overlap(&c);
    assert_eq!(c.islands().len(), 1);
    for f in 0..6 {
        assert!(signed_area(&c.uvs[f]) > 0.0, "face {f} mirrored");
        let ratio = stretch(&c, f) / stretch(&c, 0);
        assert!((ratio - 1.0).abs() < 0.01, "face {f}: {ratio}");
        let uv = &c.uvs[f];
        let d = (uv[1] - uv[0]).dot(uv[2] - uv[1]);
        assert!(
            d.abs() < 1e-3 * (uv[1] - uv[0]).length_squared(),
            "right angles"
        );
    }
}

#[test]
fn unwrap_cylinder_with_seam_unrolls_side() {
    let segments = 16;
    let mut m = cylinder(segments, 1.0, 1.0);
    // Cap rims and one vertical seam.
    let mut seams = Vec::new();
    for face in &m.faces {
        if face.len() == segments {
            seams.extend((0..segments).map(|i| edge_key(face[i], face[(i + 1) % segments])));
        }
    }
    let vertical = m
        .edges()
        .into_iter()
        .find(|&[a, b]| (m.positions[a as usize] - m.positions[b as usize]).y.abs() > 1.9)
        .unwrap();
    seams.push(vertical);
    m.mark_seam(&seams, true);
    let all: Vec<usize> = (0..m.faces.len()).collect();
    assert_eq!(m.unwrap_faces(&all, 0.01), 3);
    assert_uvs(&m);
    assert_in_unit_square(&m);
    assert_no_overlap(&m);
    // The side is a rectangle: perimeter by height.
    let side: Vec<usize> = (0..m.faces.len())
        .filter(|&f| m.faces[f].len() == 4)
        .collect();
    let (lo, hi) = side
        .iter()
        .flat_map(|&f| m.uvs[f].iter())
        .fold((Vec2::MAX, Vec2::MIN), |(l, h), &t| (l.min(t), h.max(t)));
    let size = hi - lo;
    let perimeter = segments as f32 * 2.0 * (std::f32::consts::PI / segments as f32).sin();
    let aspect = size.x.max(size.y) / size.x.min(size.y);
    assert!(
        (aspect - perimeter / 2.0).abs() < 0.02,
        "{aspect} vs {}",
        perimeter / 2.0
    );
    let s0 = stretch(&m, side[0]);
    for &f in &side {
        assert!((stretch(&m, f) / s0 - 1.0).abs() < 0.01);
    }

    // Without seams the closed cylinder is cut by normal direction instead.
    let mut m = cylinder(segments, 1.0, 1.0);
    let n = m.unwrap_faces(&all, 0.01);
    assert!(n >= 3, "{n}");
    assert_in_unit_square(&m);
    assert_no_overlap(&m);
}

#[test]
fn unwrap_subset_and_sphere_has_no_nans() {
    let mut c = PolyMesh::cube(2.0);
    let all: Vec<usize> = (0..6).collect();
    c.subdivide_faces(&all, 3);
    let s = c.catmull_clark(1);
    let mut s = s;
    let all: Vec<usize> = (0..s.faces.len()).collect();
    let n = s.unwrap_faces(&all, 0.01);
    assert!(n >= 6);
    assert_uvs(&s);
    assert_in_unit_square(&s);
    // Only the given faces move.
    let mut c = boxed(PolyMesh::cube(2.0));
    let before = c.uvs.clone();
    c.unwrap_faces(&[0, 1], 0.0);
    assert_eq!(c.uvs[2..], before[2..]);
}

// --- projections and transforms -------------------------------------------

#[test]
fn cylinder_projection_handles_the_wrap_seam() {
    let mut m = cylinder(12, 1.0, 1.0);
    let side: Vec<usize> = (0..m.faces.len())
        .filter(|&f| m.faces[f].len() == 4)
        .collect();
    m.project_cylinder(&side, Vec3::Y);
    assert_uvs(&m);
    for &f in &side {
        let us: Vec<f32> = m.uvs[f].iter().map(|t| t.x).collect();
        let span = us.iter().cloned().fold(f32::MIN, f32::max)
            - us.iter().cloned().fold(f32::MAX, f32::min);
        assert!(span < 0.2, "face {f} spans the seam: {us:?}");
        assert!(signed_area(&m.uvs[f]) > 0.0, "not mirrored");
        for t in &m.uvs[f] {
            assert!(t.y >= -EPS && t.y <= 1.0 + EPS);
        }
    }
    let caps: Vec<usize> = (0..m.faces.len())
        .filter(|&f| m.faces[f].len() != 4)
        .collect();
    m.project_sphere(&caps, Vec3::Y);
    m.validate().unwrap();
}

#[test]
fn sphere_planar_view_box_reset_projections() {
    let mut c = PolyMesh::cube(2.0);
    let all: Vec<usize> = (0..6).collect();
    c.subdivide_faces(&all, 2);
    let mut s = c.catmull_clark(1);
    let all: Vec<usize> = (0..s.faces.len()).collect();
    s.project_sphere(&all, Vec3::Y);
    assert_uvs(&s);
    for f in &all {
        let ok = s.uvs[*f]
            .iter()
            .all(|t| t.is_finite() && t.y >= -EPS && t.y <= 1.0 + EPS);
        assert!(ok);
    }
    let mut p = PolyMesh::plane(2.0);
    p.project_planar(&[0], None, 0.5);
    assert_uvs(&p);
    let (lo, hi) = p.uvs[0]
        .iter()
        .fold((Vec2::MAX, Vec2::MIN), |(l, h), &t| (l.min(t), h.max(t)));
    assert!(lo.abs_diff_eq(Vec2::ZERO, EPS) && hi.abs_diff_eq(Vec2::ONE, EPS));
    assert!(signed_area(&p.uvs[0]) > 0.0);
    // Top view of a 2 m plane through an orthographic 2x2 view: fills 0..1.
    // Orthographic top view: screen x = X, screen y = -Z.
    let view_proj = Mat4::from_cols(
        glam::Vec4::X,
        glam::Vec4::new(0.0, 0.0, 0.1, 0.0),
        glam::Vec4::new(0.0, -1.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, 0.5, 1.0),
    );
    p.project_from_view(&[0], view_proj);
    let (lo, hi) = p.uvs[0]
        .iter()
        .fold((Vec2::MAX, Vec2::MIN), |(l, h), &t| (l.min(t), h.max(t)));
    assert!(lo.abs_diff_eq(Vec2::ZERO, EPS) && hi.abs_diff_eq(Vec2::ONE, EPS));
    p.project_box(&[0], 1.0);
    assert_box_mapped(&p);
    let mut c = PolyMesh::cube(1.0);
    c.reset_uvs(&[0, 1]);
    assert_eq!(c.uvs[0], vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]);
    let mut t = PolyMesh::from_polygons(&[vec![Vec3::ZERO, Vec3::X, Vec3::NEG_Z]], 0.0);
    t.reset_uvs(&[0]);
    assert!(signed_area(&t.uvs[0]) > 0.0);
    assert_in_unit_square(&t);
}

#[test]
fn transform_pack_islands_and_density() {
    let mut c = boxed(PolyMesh::cube(1.0));
    assert!((c.texel_density(&[0, 1, 2], 512.0).unwrap() - 512.0).abs() < 0.01);
    let corners = c.face_corners(&[0]);
    let before = c.uvs[0].clone();
    c.transform_uvs(&corners, Affine2::from_translation(Vec2::new(3.0, -1.0)));
    for (a, b) in c.uvs[0].iter().zip(&before) {
        assert!(a.abs_diff_eq(*b + Vec2::new(3.0, -1.0), EPS));
    }
    c.transform_uvs(
        &[(0, 0), (99, 0), (0, 99)],
        Affine2::from_scale(Vec2::splat(2.0)),
    );
    c.pack_islands(0.05);
    assert_in_unit_square(&c);
    assert_no_overlap(&c);
    assert_eq!(c.islands().len(), 6);
    c.validate().unwrap();
}

#[test]
fn follow_active_quads_lays_out_a_grid() {
    let mut p = PolyMesh::plane(2.0);
    let faces = p.subdivide_faces(&[0], 3);
    assert_eq!(faces.len(), 16);
    p.reset_uvs(&faces);
    assert_eq!(p.islands().len(), 16);
    assert_eq!(p.follow_active_quads(&faces, faces[5]), 16);
    assert_eq!(p.islands().len(), 1);
    for &f in &faces {
        assert!((stretch(&p, f) - stretch(&p, faces[5])).abs() < 1e-3);
    }
    assert_no_overlap(&p);
}

#[test]
fn triangulate_uv_matches_corners() {
    let c = PolyMesh::cube(2.0);
    let (p, n, uv, i) = c.triangulate_uv();
    assert_eq!((p.len(), n.len(), uv.len(), i.len()), (24, 24, 24, 36));
    let boxed = boxed(c.clone());
    let (_, _, uv2, _) = boxed.triangulate_uv();
    assert_eq!(uv, uv2, "on-the-fly box projection equals ensure_uvs");
    assert_eq!(uv2[..4], boxed.uvs[0][..]);
}

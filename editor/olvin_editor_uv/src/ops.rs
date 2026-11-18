//! UV operations behind the toolbar and hotkeys. They work on a
//! [`Target`] (faces, edges and sticky corners from the UV selection) and
//! never change topology, so corner selections stay valid.

use olvin_editor::glam::{Affine2, Vec2, Vec3};
use olvin_mesh::{PolyMesh, UvCorner, edge_key};

use crate::transform::bounds_center;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UvOp {
    Unwrap,
    ProjectBox,
    ProjectPlanar,
    ProjectCylinder,
    ProjectSphere,
    FollowActiveQuads,
    Reset,
    MarkSeam,
    ClearSeam,
    SeamsFromIslands,
    Pack,
    FlipU,
    FlipV,
    RotateCcw,
    RotateCw,
    AlignU,
    AlignV,
}

impl UvOp {
    pub fn label(self) -> &'static str {
        match self {
            UvOp::Unwrap => "Unwrap",
            UvOp::ProjectBox => "Box Projection",
            UvOp::ProjectPlanar => "Planar Projection",
            UvOp::ProjectCylinder => "Cylinder Projection",
            UvOp::ProjectSphere => "Sphere Projection",
            UvOp::FollowActiveQuads => "Follow Active Quads",
            UvOp::Reset => "Reset",
            UvOp::MarkSeam => "Mark Seam",
            UvOp::ClearSeam => "Clear Seam",
            UvOp::SeamsFromIslands => "Seams from Islands",
            UvOp::Pack => "Pack Islands",
            UvOp::FlipU => "Flip U",
            UvOp::FlipV => "Flip V",
            UvOp::RotateCcw => "Rotate 90° ⟲",
            UvOp::RotateCw => "Rotate 90° ⟳",
            UvOp::AlignU => "Align U",
            UvOp::AlignV => "Align V",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            UvOp::Unwrap => "LSCM unwrap split at seams, packed into 0..1 (U)",
            UvOp::ProjectBox => "Project each face along its dominant axis, 1 UV per metre",
            UvOp::ProjectPlanar => "Project along the faces' average normal",
            UvOp::ProjectCylinder => "Wrap around the object's Y axis",
            UvOp::ProjectSphere => "Longitude / latitude around the object's Y axis",
            UvOp::FollowActiveQuads => "Lay quads out as a grid from the last clicked face",
            UvOp::Reset => "Every face fills the whole 0..1 square",
            UvOp::MarkSeam => "Mark the selected UV edges as seams (Ctrl+E)",
            UvOp::ClearSeam => "Clear seams on the selected UV edges",
            UvOp::SeamsFromIslands => "Mark the borders of the current islands as seams",
            UvOp::Pack => "Pack islands into 0..1 keeping their relative size (Ctrl+P)",
            UvOp::FlipU => "Mirror the selection horizontally",
            UvOp::FlipV => "Mirror the selection vertically",
            UvOp::RotateCcw => "Rotate the selection 90° counter-clockwise",
            UvOp::RotateCw => "Rotate the selection 90° clockwise",
            UvOp::AlignU => "Line the selection up on its average U",
            UvOp::AlignV => "Line the selection up on its average V",
        }
    }

    pub const PROJECT: [UvOp; 6] = [
        UvOp::ProjectBox,
        UvOp::ProjectPlanar,
        UvOp::ProjectCylinder,
        UvOp::ProjectSphere,
        UvOp::FollowActiveQuads,
        UvOp::Reset,
    ];
    pub const SEAMS: [UvOp; 3] = [UvOp::MarkSeam, UvOp::ClearSeam, UvOp::SeamsFromIslands];
    pub const ARRANGE: [UvOp; 6] = [
        UvOp::FlipU,
        UvOp::FlipV,
        UvOp::RotateCcw,
        UvOp::RotateCw,
        UvOp::AlignU,
        UvOp::AlignV,
    ];
}

/// What an operation acts on.
#[derive(Clone, Debug, Default)]
pub struct Target {
    /// Faces to (re)map: the selected ones, or all shown faces if none are.
    pub faces: Vec<usize>,
    /// Whether `faces` came from an explicit selection.
    pub selected: bool,
    /// Mesh edges of the selected UV edges.
    pub edges: Vec<[u32; 2]>,
    /// Selected corners grown to whole UV vertices.
    pub corners: Vec<UvCorner>,
    /// Last clicked face (Follow Active Quads).
    pub active: Option<usize>,
}

/// Runs `op`; `Ok` carries a message for the log (may be empty).
pub fn apply(mesh: &mut PolyMesh, op: UvOp, t: &Target, margin: f32) -> Result<String, String> {
    mesh.ensure_uvs();
    let need_faces = || {
        if t.faces.is_empty() {
            Err("no faces to map".to_owned())
        } else {
            Ok(())
        }
    };
    match op {
        UvOp::Unwrap => {
            need_faces()?;
            let n = mesh.unwrap_faces(&t.faces, margin);
            Ok(format!(
                "unwrapped {} faces into {n} islands",
                t.faces.len()
            ))
        }
        UvOp::ProjectBox => need_faces().map(|_| {
            mesh.project_box(&t.faces, 1.0);
            String::new()
        }),
        UvOp::ProjectPlanar => need_faces().map(|_| {
            mesh.project_planar(&t.faces, None, 1.0);
            String::new()
        }),
        UvOp::ProjectCylinder => need_faces().map(|_| {
            mesh.project_cylinder(&t.faces, Vec3::Y);
            String::new()
        }),
        UvOp::ProjectSphere => need_faces().map(|_| {
            mesh.project_sphere(&t.faces, Vec3::Y);
            String::new()
        }),
        UvOp::FollowActiveQuads => {
            let active = t
                .active
                .filter(|f| t.faces.contains(f))
                .or_else(|| t.faces.iter().copied().find(|&f| mesh.faces[f].len() == 4))
                .ok_or("Follow Active Quads needs a quad")?;
            let n = mesh.follow_active_quads(&t.faces, active);
            if n == 0 {
                return Err("the active face is not a quad".to_owned());
            }
            Ok(format!("laid out {n} quads"))
        }
        UvOp::Reset => need_faces().map(|_| {
            mesh.reset_uvs(&t.faces);
            String::new()
        }),
        UvOp::MarkSeam | UvOp::ClearSeam => {
            if t.edges.is_empty() {
                return Err("select UV edges first".to_owned());
            }
            mesh.mark_seam(&t.edges, op == UvOp::MarkSeam);
            Ok(String::new())
        }
        UvOp::SeamsFromIslands => {
            let n = seams_from_islands(mesh, &t.faces);
            Ok(format!("{n} seams marked"))
        }
        UvOp::Pack => {
            if t.selected {
                mesh.pack_face_islands(&t.faces, margin);
            } else {
                mesh.pack_islands(margin);
            }
            Ok(String::new())
        }
        UvOp::FlipU
        | UvOp::FlipV
        | UvOp::RotateCcw
        | UvOp::RotateCw
        | UvOp::AlignU
        | UvOp::AlignV => {
            if t.corners.is_empty() {
                return Err("select something in the UV view first".to_owned());
            }
            let uvs = || t.corners.iter().map(|&(f, i)| mesh.uvs[f][i]);
            let pivot = match op {
                UvOp::AlignU | UvOp::AlignV => uvs().sum::<Vec2>() / t.corners.len() as f32,
                _ => bounds_center(uvs()).unwrap_or_default(),
            };
            let m = match op {
                UvOp::FlipU | UvOp::AlignU => {
                    Affine2::from_scale(Vec2::new(if op == UvOp::FlipU { -1.0 } else { 0.0 }, 1.0))
                }
                UvOp::FlipV | UvOp::AlignV => {
                    Affine2::from_scale(Vec2::new(1.0, if op == UvOp::FlipV { -1.0 } else { 0.0 }))
                }
                UvOp::RotateCcw => Affine2::from_angle(std::f32::consts::FRAC_PI_2),
                _ => Affine2::from_angle(-std::f32::consts::FRAC_PI_2),
            };
            let m = Affine2::from_translation(pivot) * m * Affine2::from_translation(-pivot);
            mesh.transform_uvs(&t.corners, m);
            Ok(String::new())
        }
    }
}

/// Marks every edge between two of `faces` whose UVs disagree across it
/// (an island border inside the mesh). Returns how many seams were added.
pub fn seams_from_islands(mesh: &mut PolyMesh, faces: &[usize]) -> usize {
    if !mesh.has_uvs() {
        return 0;
    }
    let adj = mesh.adjacency();
    let mut edges: Vec<[u32; 2]> = Vec::new();
    for (&[a, b], fs) in &adj.edge_faces {
        let fs: Vec<usize> = fs.iter().copied().filter(|f| faces.contains(f)).collect();
        if fs.len() < 2 {
            continue;
        }
        let uv = |f: usize, v: u32| {
            let i = mesh.faces[f].iter().position(|&w| w == v).unwrap_or(0);
            mesh.uvs[f][i]
        };
        let split = fs[1..].iter().any(|&g| {
            uv(fs[0], a).distance(uv(g, a)) > 1e-5 || uv(fs[0], b).distance(uv(g, b)) > 1e-5
        });
        if split && !mesh.is_seam(a, b) {
            edges.push(edge_key(a, b));
        }
    }
    mesh.mark_seam(&edges, true);
    edges.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(mesh: &PolyMesh, faces: Vec<usize>) -> Target {
        Target {
            corners: mesh.face_corners(&faces),
            faces,
            selected: true,
            ..Default::default()
        }
    }

    #[test]
    fn flip_rotate_align_about_the_selection() {
        let mut m = PolyMesh::plane(2.0);
        m.reset_uvs(&[0]);
        let t = target(&m, vec![0]);
        let before = m.uvs[0].clone();
        apply(&mut m, UvOp::FlipU, &t, 0.0).unwrap();
        for (a, b) in m.uvs[0].iter().zip(&before) {
            assert_eq!(*a, Vec2::new(1.0 - b.x, b.y));
        }
        apply(&mut m, UvOp::FlipU, &t, 0.0).unwrap();
        apply(&mut m, UvOp::RotateCcw, &t, 0.0).unwrap();
        // (1, 0) turns to (1, 1) around the centre.
        assert!(m.uvs[0][1].abs_diff_eq(Vec2::new(1.0, 1.0), 1e-5));
        apply(&mut m, UvOp::RotateCw, &t, 0.0).unwrap();
        for (a, b) in m.uvs[0].iter().zip(&before) {
            assert!(a.abs_diff_eq(*b, 1e-5));
        }
        apply(&mut m, UvOp::AlignV, &t, 0.0).unwrap();
        assert!(m.uvs[0].iter().all(|t| (t.y - 0.5).abs() < 1e-5));
        assert!(apply(&mut m, UvOp::FlipV, &Target::default(), 0.0).is_err());
    }

    #[test]
    fn unwrap_seams_pack_follow() {
        let mut c = PolyMesh::cube(1.0);
        let all: Vec<usize> = (0..6).collect();
        let t = Target {
            faces: all.clone(),
            ..Default::default()
        };
        let msg = apply(&mut c, UvOp::Unwrap, &t, 0.02).unwrap();
        assert!(msg.contains("6 islands"), "{msg}");
        // Every cube edge separates two islands now.
        assert_eq!(seams_from_islands(&mut c, &all), 12);
        assert_eq!(c.seams.len(), 12);
        assert!(
            apply(&mut c, UvOp::MarkSeam, &t, 0.0).is_err(),
            "no edges selected"
        );
        let t2 = Target {
            edges: vec![[0, 1]],
            ..t.clone()
        };
        apply(&mut c, UvOp::ClearSeam, &t2, 0.0).unwrap();
        assert_eq!(c.seams.len(), 11);
        apply(&mut c, UvOp::Pack, &t, 0.05).unwrap();
        assert!(
            c.uvs
                .iter()
                .flatten()
                .all(|t| t.x >= -1e-4 && t.y >= -1e-4 && t.x <= 1.0001 && t.y <= 1.0001)
        );
        for op in UvOp::PROJECT {
            apply(&mut c, op, &t, 0.0).unwrap();
            c.validate().unwrap();
        }
        let tri = PolyMesh::from_polygons(&[vec![Vec3::ZERO, Vec3::X, Vec3::Z]], 0.0);
        let mut tri = tri;
        let t = target(&tri, vec![0]);
        assert!(apply(&mut tri, UvOp::FollowActiveQuads, &t, 0.0).is_err());
    }
}

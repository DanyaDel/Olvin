//! Mesh operations on the Edit Mode selection, shared by hotkeys and the panel.

use olvin_editor::EditorState;
use olvin_editor::glam::{Mat4, Vec3};
use olvin_editor::olvin_scene::file::{MeshDesc, PolyMesh, Shape};
use olvin_editor::olvin_scene::primitives::Primitive;

use crate::{Mode, face_verts, selected_edges, selected_faces, shared};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    SelectAll,
    Fill,
    Delete,
    DeleteOnlyFaces,
    DissolveEdges,
    Merge,
    MergeByDistance,
    Subdivide,
    Smooth,
    Flip,
    RecalculateNormals,
    LoopCutSelected,
}

impl Op {
    pub fn label(self) -> &'static str {
        match self {
            Op::SelectAll => "Select All  A",
            Op::Fill => "Make Face / Fill  F",
            Op::Delete => "Delete  X",
            Op::DeleteOnlyFaces => "Faces → Edges",
            Op::DissolveEdges => "Dissolve Edges",
            Op::Merge => "Merge at Center  M",
            Op::MergeByDistance => "Merge by Distance",
            Op::Subdivide => "Subdivide",
            Op::Smooth => "Smooth (Catmull-Clark)",
            Op::Flip => "Flip Normals",
            Op::RecalculateNormals => "Recalculate Outside  Shift+N",
            Op::LoopCutSelected => "Loop Cut Selected Edge",
        }
    }
}

/// Runs an op on the edited mesh. Returns a message for the log on failure.
pub fn apply(state: &mut EditorState, op: Op) -> Result<(), String> {
    let Some(id) = shared(|s| s.entity) else {
        return Err("not in Edit Mode".into());
    };
    let Some(mesh) = state.scene.get_mut(id).and_then(|e| e.poly.as_mut()) else {
        return Err("no editable mesh".into());
    };
    let (mode, verts, cuts) = shared(|s| (s.mode, s.verts.clone(), s.cuts.max(1)));
    let faces = selected_faces(mesh, &verts);
    let edges = selected_edges(mesh, &verts);
    let mut select: Option<Vec<u32>> = None;
    match op {
        Op::SelectAll => {
            select = Some(if verts.len() == mesh.positions.len() {
                Vec::new()
            } else {
                (0..mesh.positions.len() as u32).collect()
            });
        }
        Op::Fill => match mode {
            Mode::Edge if !edges.is_empty() => {
                let f = mesh.fill_edges(&edges).map_err(|e| e.to_string())?;
                select = Some(face_verts(mesh, &[f]));
            }
            _ => {
                mesh.make_face(&verts).map_err(|e| e.to_string())?;
            }
        },
        Op::Delete => {
            match mode {
                Mode::Vertex => drop(mesh.delete_vertices(&verts)),
                Mode::Edge => drop(mesh.delete_edges(&edges)),
                Mode::Face => drop(mesh.delete_faces(&faces, false)),
            }
            select = Some(Vec::new());
        }
        Op::DeleteOnlyFaces => {
            mesh.delete_faces(&faces, true);
            select = Some(Vec::new());
        }
        Op::DissolveEdges => {
            mesh.dissolve_edges(&edges);
            select = Some(Vec::new());
        }
        Op::Merge => {
            if verts.len() < 2 {
                return Err("select at least two vertices".into());
            }
            let center = verts
                .iter()
                .map(|&v| mesh.positions[v as usize])
                .sum::<Vec3>()
                / verts.len() as f32;
            let merged = mesh.merge_vertices(&verts, center).map(|(v, _)| v);
            select = Some(merged.into_iter().collect());
        }
        Op::MergeByDistance => {
            let (removed, _) = mesh.merge_by_distance(1e-3);
            state.info(format!("merged {removed} vertices"));
            shared(|s| s.verts.clear());
            return Ok(());
        }
        Op::Subdivide => {
            let targets: Vec<usize> = if faces.is_empty() {
                (0..mesh.faces.len()).collect()
            } else {
                faces
            };
            let new = mesh.subdivide_faces(&targets, cuts);
            select = Some(face_verts(mesh, &new));
        }
        Op::Smooth => {
            *mesh = mesh.catmull_clark(1);
            select = Some(Vec::new());
        }
        Op::Flip => mesh.flip_faces(&faces),
        Op::RecalculateNormals => mesh.recalculate_normals_outside(),
        Op::LoopCutSelected => {
            let Some([a, b]) = edges.first().copied() else {
                return Err("select an edge".into());
            };
            select = Some(mesh.loop_cut(a, b, cuts).map_err(|e| e.to_string())?);
        }
    }
    if let Some(v) = select {
        shared(|s| s.verts = v);
    }
    Ok(())
}

/// Polygon mesh of a primitive (cube and planes as quads, round ones welded).
pub fn from_shape(shape: Shape) -> PolyMesh {
    match shape {
        Shape::Cube => PolyMesh::cube(1.0),
        Shape::Plane => PolyMesh::plane(10.0),
        _ => {
            let p = Primitive::new(shape);
            let tris: Vec<Vec<Vec3>> = p
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| t.iter().map(|&i| p.positions[i as usize]).collect())
                .collect();
            let mut mesh = PolyMesh::from_polygons(&tris, 1e-4);
            // Triangles of one flat cap or quad strip merge back into bigger faces.
            let flat: Vec<[u32; 2]> = mesh
                .edges()
                .into_iter()
                .filter(|[a, b]| {
                    let f = mesh.faces_with_edge(*a, *b);
                    f.len() == 2 && mesh.face_normal(f[0]).dot(mesh.face_normal(f[1])) > 0.9999
                })
                .collect();
            mesh.dissolve_edges(&flat);
            mesh
        }
    }
}

/// Turns a primitive or a single CSG brush into an editable mesh in place.
pub fn make_editable(state: &mut EditorState, id: u64) -> Result<(), String> {
    let Some(e) = state.scene.get(id) else {
        return Err("nothing selected".into());
    };
    if e.poly.is_some() {
        return Ok(());
    }
    let (poly, color) = if let Some(brush) = &e.brush {
        let input = olvin_csg::BrushInput {
            id,
            shape: brush.shape,
            op: olvin_csg::CsgOp::Add,
            transform: Mat4::IDENTITY,
            color: brush.color,
            vertices: brush.vertices.clone(),
        };
        let polys: Vec<Vec<Vec3>> = olvin_csg::build(&[input])
            .into_iter()
            .map(|p| p.vertices)
            .collect();
        (PolyMesh::from_polygons(&polys, 1e-4), brush.color)
    } else if let Some(mesh) = &e.mesh {
        (from_shape(mesh.shape), mesh.color)
    } else {
        return Err("only meshes and CSG brushes can be edited".into());
    };
    let name = e.name.clone();
    let e = state.scene.get_mut(id).unwrap();
    e.brush = None;
    e.mesh = Some(MeshDesc {
        shape: Shape::Cube,
        color,
    });
    e.poly = Some(poly);
    state.info(format!("{name}: converted to an editable mesh"));
    Ok(())
}

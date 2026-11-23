//! Mesh operations on the Edit Mode selection, shared by hotkeys and the panel.

use olvin_editor::EditorState;
use olvin_editor::glam::{Mat4, Vec3};
use olvin_editor::olvin_scene::file::{MeshDesc, PolyMesh, Shape};
use olvin_editor::olvin_scene::primitives::Primitive;

use crate::{Mode, face_verts, mask, selected_edges, selected_faces, shared};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    SelectAll,
    SelectNone,
    SelectInvert,
    SelectLinked,
    SelectGrow,
    SelectShrink,
    Hide,
    HideUnselected,
    Reveal,
    Duplicate,
    Connect,
    /// Moves the selection by a world-space offset.
    MoveBy(Vec3),
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
            Op::SelectAll => "All  A",
            Op::SelectNone => "None  Alt+A",
            Op::SelectInvert => "Invert  Ctrl+I",
            Op::SelectLinked => "Linked  Ctrl+L",
            Op::SelectGrow => "Grow  Ctrl+=",
            Op::SelectShrink => "Shrink  Ctrl+-",
            Op::Hide => "Hide  H",
            Op::HideUnselected => "Hide Others  Shift+H",
            Op::Reveal => "Reveal  Alt+H",
            Op::Duplicate => "Duplicate  Shift+D",
            Op::Connect => "Connect Vertices  J",
            Op::MoveBy(_) => "Move",
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
    let world = state
        .scene
        .world_matrices()
        .get(&id)
        .copied()
        .unwrap_or(Mat4::IDENTITY);
    let Some(mesh) = state.scene.get_mut(id).and_then(|e| e.poly.as_mut()) else {
        return Err("no editable mesh".into());
    };
    let (mode, verts, cuts, hidden) =
        shared(|s| (s.mode, s.verts.clone(), s.cuts.max(1), s.hidden.clone()));
    let n = mesh.positions.len();
    let shown: Vec<u32> = {
        let h = mask(n, &hidden);
        (0..n as u32).filter(|v| !h[*v as usize]).collect()
    };
    let faces = selected_faces(mesh, &verts);
    let edges = selected_edges(mesh, &verts);
    let mut select: Option<Vec<u32>> = None;
    match op {
        Op::SelectAll => {
            select = Some(if verts.len() >= shown.len() {
                Vec::new()
            } else {
                shown
            });
        }
        Op::SelectNone => select = Some(Vec::new()),
        Op::SelectInvert => {
            let on = mask(n, &verts);
            select = Some(shown.into_iter().filter(|v| !on[*v as usize]).collect());
        }
        Op::SelectLinked => select = Some(mesh.linked(&verts)),
        Op::SelectGrow => select = Some(mesh.grow_selection(&verts)),
        Op::SelectShrink => select = Some(mesh.shrink_selection(&verts)),
        Op::Hide | Op::HideUnselected => {
            let on = mask(n, &verts);
            let hide = shown
                .into_iter()
                .filter(|v| on[*v as usize] == (op == Op::Hide));
            shared(|s| s.hidden.extend(hide));
            select = Some(if op == Op::Hide {
                Vec::new()
            } else {
                verts.clone()
            });
        }
        Op::Reveal => {
            shared(|s| s.hidden.clear());
            select = Some(hidden.clone());
        }
        Op::Duplicate => {
            if verts.is_empty() {
                return Err("nothing selected to duplicate".into());
            }
            select = Some(mesh.duplicate(&verts));
        }
        Op::Connect => {
            let [a, b] = verts[..] else {
                return Err("select exactly two vertices to connect".into());
            };
            mesh.connect_vertices(a, b).map_err(|e| e.to_string())?;
        }
        Op::MoveBy(d) => mesh.translate_vertices(&verts, world.inverse().transform_vector3(d)),
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
            shared(|s| {
                s.verts.clear();
                s.hidden.clear();
            });
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
    // Removing vertices renumbers them: the hidden set no longer applies.
    if mesh.positions.len() < n || op == Op::Smooth {
        shared(|s| s.hidden.clear());
    }
    if let Some(mut v) = select {
        let h = shared(|s| mask(mesh.positions.len(), &s.hidden));
        v.retain(|i| !h.get(*i as usize).copied().unwrap_or(true));
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

/// Makes the object editable in Edit Mode. A primitive becomes a mesh; a CSG
/// brush stays a brush (operation, order and colour kept) and its shape becomes
/// the edited cage, so the level is carved by exactly what you model.
pub fn make_editable(state: &mut EditorState, id: u64) -> Result<(), String> {
    let Some(e) = state.scene.get(id) else {
        return Err("nothing selected".into());
    };
    if e.poly.is_some() {
        return Ok(());
    }
    let name = e.name.clone();
    if e.brush.is_some() {
        let poly = brush_cage(e).ok_or("empty brush")?;
        state.scene.get_mut(id).ok_or("nothing selected")?.poly = Some(poly);
        return Ok(());
    }
    let Some(mesh) = &e.mesh else {
        return Err("only meshes and CSG brushes can be edited".into());
    };
    let (poly, color) = (from_shape(mesh.shape), mesh.color);
    let e = state.scene.get_mut(id).ok_or("nothing selected")?;
    e.mesh = Some(MeshDesc {
        shape: Shape::Cube,
        color,
    });
    e.poly = Some(poly);
    state.info(format!("{name}: converted to an editable mesh"));
    Ok(())
}

/// A brush's current surface (shape or edited hull) as a polygon mesh.
fn brush_cage(e: &olvin_editor::olvin_scene::file::EntityDesc) -> Option<PolyMesh> {
    let mut input = e.brush_input(Mat4::IDENTITY)?;
    input.op = olvin_csg::CsgOp::Add;
    input.faces = None;
    let polys: Vec<Vec<Vec3>> = olvin_csg::build(&[input])
        .into_iter()
        .map(|p| p.vertices)
        .collect();
    let mut poly = PolyMesh::from_polygons(&polys, 1e-4);
    // CSG fragments of one flat side merge back into a single face.
    let flat: Vec<[u32; 2]> = poly
        .edges()
        .into_iter()
        .filter(|[a, b]| {
            let f = poly.faces_with_edge(*a, *b);
            f.len() == 2 && poly.face_normal(f[0]).dot(poly.face_normal(f[1])) > 0.9999
        })
        .collect();
    poly.dissolve_edges(&flat);
    (!poly.faces.is_empty()).then_some(poly)
}

/// Takes a modelled brush out of the CSG level: it becomes a plain mesh.
pub fn detach_brush(state: &mut EditorState, id: u64) {
    let Some(e) = state.scene.get(id) else { return };
    let Some(color) = e.brush.as_ref().map(|b| b.color) else {
        return;
    };
    let Some(poly) = e.poly.clone().or_else(|| brush_cage(e)) else {
        return;
    };
    let Some(e) = state.scene.get_mut(id) else {
        return;
    };
    e.brush = None;
    e.poly = Some(poly);
    e.mesh = Some(MeshDesc {
        shape: Shape::Cube,
        color,
    });
}

/// Drops the modelled shape of a brush: back to its parametric shape.
pub fn reset_brush(state: &mut EditorState, id: u64) {
    if let Some(e) = state.scene.get_mut(id)
        && let Some(b) = &mut e.brush
    {
        b.vertices = None;
        e.poly = None;
    }
    shared(|s| s.verts.clear());
}

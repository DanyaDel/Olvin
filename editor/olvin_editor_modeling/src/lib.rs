//! Editor element: Blender-style Edit Mode for polygon meshes (`olvin_mesh`).
//! Tab toggles Edit Mode; 1/2/3 pick vertices/edges/faces; G R S with axis
//! constraints and typed values, E I F K J X M, Ctrl+R, box select, loops,
//! X-ray and a move gizmo work like in Blender.

mod cage;
mod edit;
mod gizmo;
mod ops;
mod panel;
mod select;
mod transform;

use std::cell::RefCell;

use olvin_editor::egui_dock::{DockState, NodeIndex};
use olvin_editor::ext::CreateItem;
use olvin_editor::glam::Vec3;
use olvin_editor::olvin_scene::file::{MeshDesc, PolyMesh, Shape};
use olvin_editor::{
    EditorPlugin, EditorState, NewEntity, PanelKind, Registry, Tab, TabAlloc, WorkspaceTemplate,
    panels,
};

pub use ops::{Op, make_editable};

pub const EDIT_TOOL: &str = "model.edit";
const PANEL: &str = "model.panel";

pub struct ModelingPlugin;

impl EditorPlugin for ModelingPlugin {
    fn name(&self) -> &'static str {
        "Modeling"
    }

    fn build(&self, registry: &mut Registry) {
        registry
            .add_viewport_extension(|| Box::<edit::EditMode>::default())
            .add_panel(PanelKind {
                id: PANEL,
                title: "◆ Modeling",
                factory: || Box::new(panel::ModelingPanel),
            })
            .add_workspace_template(WorkspaceTemplate {
                name: "Modeling",
                build: layout,
            });
        for (path, create) in CREATE {
            registry.add_create_item(CreateItem { path, create });
        }
    }
}

type Create = fn(&mut EditorState, Option<u64>) -> u64;

const CREATE: [(&str, Create); 4] = [
    ("Mesh/Cube", |s, p| spawn(s, p, "Cube", PolyMesh::cube(1.0))),
    ("Mesh/Plane", |s, p| {
        spawn(s, p, "Plane", PolyMesh::plane(2.0))
    }),
    ("Mesh/Cylinder", |s, p| {
        spawn(s, p, "Cylinder", ops::from_shape(Shape::Cylinder))
    }),
    ("Mesh/Sphere", |s, p| {
        spawn(s, p, "Sphere", ops::from_shape(Shape::Sphere))
    }),
];

fn spawn(state: &mut EditorState, parent: Option<u64>, name: &str, poly: PolyMesh) -> u64 {
    let id = state.create(NewEntity::Empty, parent);
    let e = state.scene.get_mut(id).unwrap();
    e.name = name.to_owned();
    e.position = Vec3::new(0.0, 1.0, 0.0);
    e.mesh = Some(MeshDesc {
        shape: Shape::Cube,
        color: [0.78, 0.78, 0.8, 1.0],
    });
    e.poly = Some(poly);
    id
}

fn layout(a: &mut TabAlloc) -> DockState<Tab> {
    let mut dock = DockState::new(vec![a.tab(panels::VIEWPORT)]);
    let tree = dock.main_surface_mut();
    let [center, right] = tree.split_right(NodeIndex::root(), 0.78, vec![a.tab(PANEL)]);
    tree.split_below(right, 0.55, vec![a.tab(panels::INSPECTOR)]);
    tree.split_left(center, 0.18, vec![a.tab(panels::HIERARCHY)]);
    dock
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Vertex,
    Edge,
    Face,
}

/// Edit Mode selection, shared by the viewport tool and the panel.
#[derive(Default)]
pub(crate) struct Shared {
    pub mode: Mode,
    pub entity: Option<u64>,
    /// Selected vertices; edges and faces are selected through their vertices.
    pub verts: Vec<u32>,
    pub cuts: u32,
    /// Hidden vertices (H / Alt+H): elements using them are not drawn or picked.
    pub hidden: Vec<u32>,
    /// Alt+Z: everything is pickable, nothing is occluded.
    pub xray: bool,
    /// Show the move gizmo at the selection.
    pub gizmo: bool,
    /// The panel's "Move selection by" offset (world space).
    pub move_by: Vec3,
    /// What the viewport is doing (modal status), for the panel.
    pub status: String,
}

thread_local! {
    static SHARED: RefCell<Shared> = RefCell::new(Shared { cuts: 1, gizmo: true, ..Default::default() });
}

pub(crate) fn shared<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    SHARED.with(|s| f(&mut s.borrow_mut()))
}

/// Edges whose two vertices are selected.
pub(crate) fn selected_edges(mesh: &PolyMesh, verts: &[u32]) -> Vec<[u32; 2]> {
    mesh.edges()
        .into_iter()
        .filter(|[a, b]| verts.contains(a) && verts.contains(b))
        .collect()
}

/// Faces whose vertices are all selected.
pub(crate) fn selected_faces(mesh: &PolyMesh, verts: &[u32]) -> Vec<usize> {
    (0..mesh.faces.len())
        .filter(|&f| mesh.faces[f].iter().all(|v| verts.contains(v)))
        .collect()
}

/// Per-vertex flags for a vertex list (out-of-range indices are ignored).
pub(crate) fn mask(n: usize, verts: &[u32]) -> Vec<bool> {
    let mut on = vec![false; n];
    for &v in verts {
        if let Some(x) = on.get_mut(v as usize) {
            *x = true;
        }
    }
    on
}

pub(crate) fn face_verts(mesh: &PolyMesh, faces: &[usize]) -> Vec<u32> {
    let mut out: Vec<u32> = faces
        .iter()
        .flat_map(|&f| mesh.faces[f].iter().copied())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_editor::NewEntity;

    fn editing(state: &mut EditorState, poly: PolyMesh) -> u64 {
        let id = state.create(NewEntity::Empty, None);
        state.scene.get_mut(id).unwrap().poly = Some(poly);
        shared(|s| {
            s.entity = Some(id);
            s.verts.clear();
        });
        id
    }

    fn mesh(state: &EditorState, id: u64) -> &PolyMesh {
        state.scene.get(id).unwrap().poly.as_ref().unwrap()
    }

    #[test]
    fn faces_to_edges_and_back() {
        let mut state = EditorState::new(None);
        let id = editing(&mut state, PolyMesh::cube(1.0));
        let top = (0..6)
            .find(|&f| mesh(&state, id).face_normal(f).y > 0.9)
            .unwrap();
        let verts = mesh(&state, id).faces[top].clone();
        shared(|s| {
            s.mode = Mode::Face;
            s.verts = verts.clone();
        });
        ops::apply(&mut state, Op::DeleteOnlyFaces).unwrap();
        assert_eq!(mesh(&state, id).faces.len(), 5);
        shared(|s| {
            s.mode = Mode::Edge;
            s.verts = verts;
        });
        ops::apply(&mut state, Op::Fill).unwrap();
        assert_eq!(mesh(&state, id).faces.len(), 6);
        mesh(&state, id).validate().unwrap();
    }

    #[test]
    fn primitives_and_brushes_become_editable() {
        let mut state = EditorState::new(None);
        for kind in [
            NewEntity::Primitive(Shape::Cube),
            NewEntity::Primitive(Shape::Cylinder),
            NewEntity::Primitive(Shape::Sphere),
        ] {
            let id = state.create(kind, None);
            make_editable(&mut state, id).unwrap();
            let poly = mesh(&state, id);
            poly.validate().unwrap();
            assert!(!poly.faces.is_empty());
        }
        let cube = state.create(NewEntity::Primitive(Shape::Cube), None);
        make_editable(&mut state, cube).unwrap();
        assert_eq!(mesh(&state, cube).faces.len(), 6, "cube stays quads");
    }

    #[test]
    fn brushes_are_modelled_in_place_and_carve_by_their_shape() {
        use olvin_editor::olvin_scene::file::{BrushDesc, BrushShape, CsgOp};
        let mut state = EditorState::new(None);
        let id = state.create(NewEntity::Empty, None);
        state.scene.get_mut(id).unwrap().brush = Some(BrushDesc {
            shape: BrushShape::Box,
            op: CsgOp::Add,
            color: [1.0; 4],
            vertices: None,
        });
        make_editable(&mut state, id).unwrap();
        let e = state.scene.get(id).unwrap();
        assert!(e.brush.is_some(), "still a brush");
        assert_eq!(mesh(&state, id).faces.len(), 6);
        // Extrude the top face: the brush becomes a taller box, and CSG follows.
        let poly = state.scene.get_mut(id).unwrap().poly.as_mut().unwrap();
        let top = (0..6).find(|&f| poly.face_normal(f).y > 0.9).unwrap();
        poly.extrude_faces(&[top], Vec3::Y);
        let input = state.scene.csg_brushes().remove(0);
        assert!(input.faces.is_some());
        let height = olvin_csg::build(&[input])
            .iter()
            .flat_map(|p| p.vertices.iter().map(|v| v.y))
            .fold(f32::MIN, f32::max);
        assert!((height - 1.5).abs() < 1e-4, "{height}");
        ops::detach_brush(&mut state, id);
        let e = state.scene.get(id).unwrap();
        assert!(e.brush.is_none() && e.poly.is_some() && e.mesh.is_some());
    }

    #[test]
    fn subdivide_and_merge() {
        let mut state = EditorState::new(None);
        let id = editing(&mut state, PolyMesh::cube(1.0));
        ops::apply(&mut state, Op::Subdivide).unwrap();
        assert_eq!(mesh(&state, id).faces.len(), 24);
        // Subdivide selects the result, so A toggles everything off, then on.
        ops::apply(&mut state, Op::SelectAll).unwrap();
        assert!(shared(|s| s.verts.is_empty()));
        ops::apply(&mut state, Op::SelectAll).unwrap();
        let n = shared(|s| s.verts.len());
        assert_eq!(n, mesh(&state, id).positions.len());
        mesh(&state, id).validate().unwrap();
    }

    #[test]
    fn select_hide_duplicate_connect() {
        let mut state = EditorState::new(None);
        let id = editing(&mut state, PolyMesh::cube(1.0));
        shared(|s| s.verts = vec![0]);
        ops::apply(&mut state, Op::SelectGrow).unwrap();
        assert_eq!(
            shared(|s| s.verts.len()),
            7,
            "every vertex sharing a face with 0"
        );
        ops::apply(&mut state, Op::SelectInvert).unwrap();
        assert_eq!(shared(|s| s.verts.clone()), [7]);
        ops::apply(&mut state, Op::Hide).unwrap();
        ops::apply(&mut state, Op::SelectAll).unwrap();
        assert_eq!(
            shared(|s| s.verts.len()),
            7,
            "hidden vertices stay unselected"
        );
        ops::apply(&mut state, Op::Reveal).unwrap();
        assert_eq!(shared(|s| (s.verts.clone(), s.hidden.len())), (vec![7], 0));

        ops::apply(&mut state, Op::SelectAll).unwrap();
        ops::apply(&mut state, Op::Duplicate).unwrap();
        assert_eq!(mesh(&state, id).faces.len(), 12);
        ops::apply(&mut state, Op::MoveBy(Vec3::X * 2.0)).unwrap();
        assert!(shared(|s| s.verts.iter().all(|&v| v >= 8)));
        assert_eq!(
            mesh(&state, id).positions[8].x,
            mesh(&state, id).positions[0].x + 2.0
        );

        // Connect two opposite corners of the -Z face (0, 3).
        shared(|s| s.verts = vec![0, 3]);
        ops::apply(&mut state, Op::Connect).unwrap();
        assert_eq!(mesh(&state, id).faces.len(), 13);
        shared(|s| s.verts = vec![0]);
        assert!(ops::apply(&mut state, Op::Connect).is_err());
        mesh(&state, id).validate().unwrap();
    }
}

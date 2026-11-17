//! Editor element: Blender-style Edit Mode for polygon meshes (`olvin_mesh`).
//! Tab toggles Edit Mode; 1/2/3 pick vertices/edges/faces; G E I F K X M and
//! Ctrl+R work like in Blender.

mod edit;
mod ops;
mod panel;

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
}

thread_local! {
    static SHARED: RefCell<Shared> = RefCell::new(Shared { cuts: 1, ..Default::default() });
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
}

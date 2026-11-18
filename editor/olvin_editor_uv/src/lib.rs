//! Editor element: a Blender-style UV Editor for `olvin_mesh` polygon
//! meshes. The "UV Editor" panel shows the selected object's per-corner UVs
//! over a checker; 1/2/3/4 pick vertex/edge/face/island selection, click,
//! Shift+click, box drag, L, A; G/R/S with U/V axis constraints (X/Y), typed
//! values and Ctrl snapping; U unwraps (LSCM), Ctrl+E marks seams, Ctrl+P
//! packs. The "UV Editing" workspace puts it next to the scene view.
//!
//! Two hooks are left for wiring from other crates: [`set_face_filter`]
//! (show only faces selected in the 3D Edit Mode) and [`set_background`]
//! (draw the material's texture instead of the procedural checker).

mod ops;
mod panel;
mod select;
mod transform;
mod view;

mod defaults;

use std::sync::RwLock;

use olvin_editor::egui;
use olvin_editor::egui_dock::{DockState, NodeIndex};
use olvin_editor::{
    EditorPlugin, EditorState, PanelKind, Registry, Tab, TabAlloc, WorkspaceTemplate, panels,
};

pub use ops::{Target, UvOp, apply as apply_op};
pub use panel::UvEditor;
pub use select::{Combine, Hit, SelectMode, Selection, Topo};
pub use transform::{Kind, NumInput, Params, matrix};
pub use view::View;

pub const PANEL: &str = "uv.editor";

pub struct UvPlugin;

impl EditorPlugin for UvPlugin {
    fn name(&self) -> &'static str {
        "UV Editor"
    }

    fn build(&self, registry: &mut Registry) {
        set_face_filter(defaults::edit_mode_faces);
        set_background(defaults::material_texture);
        registry
            .add_panel(PanelKind {
                id: PANEL,
                title: "▦ UV Editor",
                factory: || Box::<UvEditor>::default(),
            })
            .add_workspace_template(WorkspaceTemplate {
                name: "UV Editing",
                build: layout,
            });
    }
}

/// UV editor on the left, the scene view on the right, hierarchy and
/// inspector in a side column.
fn layout(a: &mut TabAlloc) -> DockState<Tab> {
    let mut dock = DockState::new(vec![a.tab(PANEL)]);
    let tree = dock.main_surface_mut();
    let [_, view] = tree.split_right(NodeIndex::root(), 0.42, vec![a.tab(panels::VIEWPORT)]);
    let [_, side] = tree.split_right(view, 0.72, vec![a.tab(panels::HIERARCHY)]);
    tree.split_below(side, 0.35, vec![a.tab(panels::INSPECTOR)]);
    dock
}

/// Faces of entity `id` to show when "Only selected in 3D" is on (Blender's
/// UV editor with sync off); `None` shows every face.
pub type FaceFilter = fn(&EditorState, u64) -> Option<Vec<usize>>;

/// Draws the background of the 0..1 square, `rect` being its screen
/// rectangle (V up: `rect.top()` is V = 1). Returns `false` to fall back to
/// the procedural checker (e.g. while the texture is still loading).
pub type Background = fn(&egui::Painter, egui::Rect, &EditorState, u64) -> bool;

#[derive(Clone, Copy, Default)]
struct Hooks {
    face_filter: Option<FaceFilter>,
    background: Option<Background>,
}

static HOOKS: RwLock<Hooks> = RwLock::new(Hooks {
    face_filter: None,
    background: None,
});

fn hooks() -> Hooks {
    HOOKS.read().map(|h| *h).unwrap_or_default()
}

/// Enables the "Only selected in 3D" toggle with this face source.
pub fn set_face_filter(filter: FaceFilter) {
    if let Ok(mut h) = HOOKS.write() {
        h.face_filter = Some(filter);
    }
}

/// Replaces the checker under the 0..1 square (e.g. the material's base texture).
pub fn set_background(background: Background) {
    if let Ok(mut h) = HOOKS.write() {
        h.background = Some(background);
    }
}

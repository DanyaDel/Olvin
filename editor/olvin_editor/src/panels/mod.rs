//! Built-in panels: the base every editor installation has.

mod assets;
mod hierarchy;
mod inspector;
mod viewport;

use egui_dock::{DockState, NodeIndex};

use crate::{PanelKind, Registry, Tab, TabAlloc, WorkspaceTemplate};

pub const VIEWPORT: &str = "olvin.viewport";
pub const HIERARCHY: &str = "olvin.hierarchy";
pub const INSPECTOR: &str = "olvin.inspector";
pub const ASSETS: &str = "olvin.assets";

pub(crate) fn register(registry: &mut Registry) {
    registry
        .add_panel(PanelKind { id: VIEWPORT, title: "🎥 Сцена", factory: || Box::<viewport::Viewport>::default() })
        .add_panel(PanelKind { id: HIERARCHY, title: "☰ Иерархия", factory: || Box::<hierarchy::Hierarchy>::default() })
        .add_panel(PanelKind { id: INSPECTOR, title: "ℹ Инспектор", factory: || Box::<inspector::Inspector>::default() })
        .add_panel(PanelKind { id: ASSETS, title: "📁 Ассеты", factory: || Box::<assets::Assets>::default() })
        .add_workspace_template(WorkspaceTemplate { name: "Сцена", build: scene_layout });
}

/// Unity-style: hierarchy left, scene center, inspector right, assets bottom.
pub fn scene_layout(a: &mut TabAlloc) -> DockState<Tab> {
    let mut dock = DockState::new(vec![a.tab(VIEWPORT)]);
    let tree = dock.main_surface_mut();
    let [center, _] = tree.split_right(NodeIndex::root(), 0.78, vec![a.tab(INSPECTOR)]);
    let [center, _] = tree.split_left(center, 0.22, vec![a.tab(HIERARCHY)]);
    tree.split_below(center, 0.7, vec![a.tab(ASSETS)]);
    dock
}

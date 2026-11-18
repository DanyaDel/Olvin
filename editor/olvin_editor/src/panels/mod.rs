//! Built-in panels: the base every editor installation has.

mod assets;
mod game;
pub(crate) mod hierarchy;
pub(crate) mod inspector;
pub(crate) mod lighting;
mod viewport;

use egui_dock::{DockState, NodeIndex};

use crate::{PanelKind, Registry, Tab, TabAlloc, WorkspaceTemplate};

pub const VIEWPORT: &str = "olvin.viewport";
pub const HIERARCHY: &str = "olvin.hierarchy";
pub const INSPECTOR: &str = "olvin.inspector";
pub const ASSETS: &str = "olvin.assets";
pub const GAME: &str = "olvin.game";
pub const LIGHTING: &str = "olvin.lighting";

pub(crate) fn register(registry: &mut Registry) {
    registry
        .add_panel(PanelKind {
            id: VIEWPORT,
            title: "🎥 Scene",
            factory: || Box::<viewport::Viewport>::default(),
        })
        .add_panel(PanelKind {
            id: HIERARCHY,
            title: "☰ Hierarchy",
            factory: || Box::<hierarchy::Hierarchy>::default(),
        })
        .add_panel(PanelKind {
            id: INSPECTOR,
            title: "ℹ Inspector",
            factory: || Box::<inspector::Inspector>::default(),
        })
        .add_panel(PanelKind {
            id: ASSETS,
            title: "📁 Assets",
            factory: || Box::<assets::Assets>::default(),
        })
        .add_panel(PanelKind {
            id: GAME,
            title: "🎮 Game",
            factory: || Box::<game::GameView>::default(),
        })
        .add_panel(PanelKind {
            id: LIGHTING,
            title: "☀ Lighting",
            factory: || Box::<lighting::Lighting>::default(),
        })
        .add_workspace_template(WorkspaceTemplate {
            name: "Lighting",
            build: lighting_layout,
        })
        .add_workspace_template(WorkspaceTemplate {
            name: "Layout",
            build: scene_layout,
        });
}

/// Unity-style: hierarchy left, scene center, inspector right, assets bottom.
pub fn scene_layout(a: &mut TabAlloc) -> DockState<Tab> {
    let mut dock = DockState::new(vec![a.tab(VIEWPORT), a.tab(GAME)]);
    let tree = dock.main_surface_mut();
    let [center, _] = tree.split_right(NodeIndex::root(), 0.78, vec![a.tab(INSPECTOR)]);
    let [center, _] = tree.split_left(center, 0.22, vec![a.tab(HIERARCHY)]);
    tree.split_below(center, 0.7, vec![a.tab(ASSETS)]);
    dock
}

/// Scene view with the Lighting window and inspector stacked on the right.
pub fn lighting_layout(a: &mut TabAlloc) -> DockState<Tab> {
    let mut dock = DockState::new(vec![a.tab(VIEWPORT), a.tab(GAME)]);
    let tree = dock.main_surface_mut();
    let [center, right] = tree.split_right(NodeIndex::root(), 0.72, vec![a.tab(LIGHTING)]);
    tree.split_below(right, 0.55, vec![a.tab(INSPECTOR)]);
    tree.split_left(center, 0.2, vec![a.tab(HIERARCHY)]);
    dock
}

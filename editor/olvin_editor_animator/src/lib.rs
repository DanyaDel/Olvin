//! Editor element: object animation the way Blender does it.
//!
//! The animation lives in the scene (`SceneFile::animation`, an
//! [`olvin_scene::action::Action`] of F-curves), so it saves, loads, undoes
//! and plays in the game with the scene. This crate is the UI:
//!
//! - `clock`: playback, the frame keys that work everywhere (Space, arrows,
//!   Up / Down, Esc), and the evaluation of the animation into the objects.
//! - `timeline`, `dopesheet`, `graph`: the three editors.
//! - `modal`: G / S operations; `ops`: selection; `view`: ruler and axes.
//!
//! Keying (`I` in the viewport, the Inspector's field menu) lives in the
//! editor core and uses the same `Action` API.

mod clock;
mod dopesheet;
mod graph;
mod modal;
mod ops;
mod timeline;
mod view;

use olvin_editor::egui_dock::{DockState, NodeIndex};
use olvin_editor::{
    EditorPlugin, Panel, PanelKind, Registry, Tab, TabAlloc, WorkspaceTemplate, panels,
};

const TIMELINE: &str = "animator.timeline";
const DOPESHEET: &str = "animator.dopesheet";
const GRAPH: &str = "animator.graph";

pub struct AnimatorPlugin;

impl EditorPlugin for AnimatorPlugin {
    fn name(&self) -> &'static str {
        "Animator"
    }

    fn build(&self, registry: &mut Registry) {
        registry
            .add_panel(PanelKind {
                id: TIMELINE,
                title: "⏱ Timeline",
                factory: || Box::<timeline::Timeline>::default() as Box<dyn Panel>,
            })
            .add_panel(PanelKind {
                id: DOPESHEET,
                title: "◆ Dope Sheet",
                factory: || Box::<dopesheet::DopeSheet>::default() as Box<dyn Panel>,
            })
            .add_panel(PanelKind {
                id: GRAPH,
                title: "∿ Graph Editor",
                factory: || Box::<graph::GraphEditor>::default() as Box<dyn Panel>,
            })
            .add_frame_hook(clock::frame_hook)
            .add_workspace_template(WorkspaceTemplate {
                name: "Animation",
                build: animation_layout,
            });
    }
}

/// Blender's Animation workspace: the 3D view with the Dope Sheet (and the
/// Graph Editor next to it) below, the Timeline under that.
fn animation_layout(a: &mut TabAlloc) -> DockState<Tab> {
    let mut dock = DockState::new(vec![a.tab(panels::VIEWPORT)]);
    let tree = dock.main_surface_mut();
    let [center, _] = tree.split_right(NodeIndex::root(), 0.8, vec![a.tab(panels::INSPECTOR)]);
    let [center, _] = tree.split_left(center, 0.2, vec![a.tab(panels::HIERARCHY)]);
    let [_, sheet] = tree.split_below(center, 0.48, vec![a.tab(DOPESHEET), a.tab(GRAPH)]);
    tree.split_below(sheet, 0.8, vec![a.tab(TIMELINE)]);
    dock
}

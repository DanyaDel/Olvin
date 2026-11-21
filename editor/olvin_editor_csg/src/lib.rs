//! Editor element: CSG level design in the spirit of Realtime CSG.
//!
//! Brushes are ordinary scene entities with a `brush` component; their order
//! in the hierarchy is the order boolean operations apply in. The level mesh
//! is rebuilt live whenever a brush changes, and games rebuild it on load.

mod panel;
mod viewport;

use std::cell::RefCell;

use olvin_editor::egui::{self, DragValue, RichText};
use olvin_editor::egui_dock::{DockState, NodeIndex};
use olvin_editor::ext::{ComponentEditor, CreateItem};
use olvin_editor::glam::Vec3;
use olvin_editor::olvin_scene::file::{BrushDesc, BrushShape, CsgOp, EntityDesc};
use olvin_editor::{
    EditorPlugin, EditorState, PanelKind, Registry, Tab, TabAlloc, WorkspaceTemplate, panels,
};

pub const DRAW_TOOL: &str = "csg.draw";
pub const EDIT_TOOL: &str = "csg.edit";
/// Edit Mode of the Modeling element: brushes are modelled there too.
pub const MODEL_EDIT_TOOL: &str = "model.edit";
const PANEL: &str = "csg.panel";

pub struct CsgPlugin;

impl EditorPlugin for CsgPlugin {
    fn name(&self) -> &'static str {
        "CSG"
    }

    fn build(&self, registry: &mut Registry) {
        registry
            .add_viewport_extension(|| Box::<viewport::CsgViewport>::default())
            .add_panel(PanelKind {
                id: PANEL,
                title: "◩ Mapping",
                factory: || Box::new(panel::CsgPanel),
            })
            .add_component(ComponentEditor {
                title: "CSG Brush",
                has: |e| e.brush.is_some(),
                add: |e| e.brush = Some(default_brush(BrushShape::Box)),
                remove: |e| e.brush = None,
                ui: brush_inspector,
            })
            .add_workspace_template(WorkspaceTemplate {
                name: "Mapping",
                build: level_layout,
            });
        for (path, create) in CREATE {
            registry.add_create_item(CreateItem { path, create });
        }
    }
}

type Create = fn(&mut EditorState, Option<u64>) -> u64;

const CREATE: [(&str, Create); 6] = [
    ("CSG/Box", |s, p| spawn(s, p, BrushShape::Box)),
    ("CSG/Wedge", |s, p| spawn(s, p, BrushShape::Wedge)),
    ("CSG/Cylinder", |s, p| {
        spawn(s, p, BrushShape::Cylinder { sides: 16 })
    }),
    ("CSG/Pyramid", |s, p| {
        spawn(s, p, BrushShape::Pyramid { sides: 4 })
    }),
    ("CSG/Stairs", |s, p| {
        spawn(s, p, BrushShape::Stairs { steps: 8 })
    }),
    ("CSG/Room", spawn_room),
];

/// Settings shared by the panel, the tools and the inspector. The editor UI is
/// single-threaded, so a thread-local is enough.
pub(crate) struct Shared {
    pub op: CsgOp,
    pub shape: BrushShape,
    pub color: [f32; 4],
    pub outlines: bool,
    pub grid_texture: bool,
    pub wall: f32,
    pub stats: Stats,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Stats {
    pub brushes: usize,
    pub polygons: usize,
    pub triangles: usize,
    pub build_ms: f32,
}

thread_local! {
    pub(crate) static SHARED: RefCell<Shared> = RefCell::new(Shared {
        op: CsgOp::Add,
        shape: BrushShape::Box,
        color: [0.72, 0.72, 0.74, 1.0],
        outlines: true,
        grid_texture: true,
        wall: 0.25,
        stats: Stats::default(),
    });
}

pub(crate) fn shared<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    SHARED.with(|s| f(&mut s.borrow_mut()))
}

pub(crate) fn default_brush(shape: BrushShape) -> BrushDesc {
    shared(|s| BrushDesc {
        shape,
        op: s.op,
        color: s.color,
        ..Default::default()
    })
}

fn spawn(state: &mut EditorState, parent: Option<u64>, shape: BrushShape) -> u64 {
    let id = state.create(olvin_editor::NewEntity::Empty, parent);
    let e = state.scene.get_mut(id).unwrap();
    e.name = shape.label().to_owned();
    e.scale = Vec3::splat(2.0);
    e.position = Vec3::new(0.0, 1.0, 0.0);
    e.brush = Some(default_brush(shape));
    id
}

/// A box with a subtractive child inside: walls, floor and ceiling at once.
fn spawn_room(state: &mut EditorState, parent: Option<u64>) -> u64 {
    let id = spawn(state, parent, BrushShape::Box);
    let e = state.scene.get_mut(id).unwrap();
    e.name = "Room".into();
    e.scale = Vec3::new(8.0, 4.0, 8.0);
    e.position = Vec3::new(0.0, 2.0, 0.0);
    hollow(state, id);
    state.selection = Some(id);
    id
}

/// Adds a subtractive child that leaves walls of the configured thickness.
pub(crate) fn hollow(state: &mut EditorState, id: u64) -> Option<u64> {
    let wall = shared(|s| s.wall);
    // A flat or NaN brush has no inside: dividing by its scale gave infinities.
    let scale = state.scene.get(id)?.scale.abs().max(Vec3::splat(0.01));
    let color = state.scene.get(id)?.brush.as_ref()?.color;
    let inner = (scale - Vec3::splat(2.0 * wall)).max(Vec3::splat(0.01)) / scale;
    let child = state.create(olvin_editor::NewEntity::Empty, Some(id));
    let e = state.scene.get_mut(child).unwrap();
    e.name = "Hollow".into();
    e.scale = inner;
    e.brush = Some(BrushDesc {
        shape: BrushShape::Box,
        op: CsgOp::Subtract,
        color,
        ..Default::default()
    });
    Some(child)
}

fn level_layout(a: &mut TabAlloc) -> DockState<Tab> {
    let mut dock = DockState::new(vec![a.tab(panels::VIEWPORT)]);
    let tree = dock.main_surface_mut();
    let [center, right] = tree.split_right(NodeIndex::root(), 0.78, vec![a.tab(panels::INSPECTOR)]);
    tree.split_below(right, 0.55, vec![a.tab(PANEL)]);
    tree.split_left(center, 0.2, vec![a.tab(panels::HIERARCHY)]);
    dock
}

pub(crate) fn op_color(op: CsgOp) -> egui::Color32 {
    match op {
        CsgOp::Add => egui::Color32::from_rgb(110, 170, 255),
        CsgOp::Subtract => egui::Color32::from_rgb(255, 140, 70),
        CsgOp::Intersect => egui::Color32::from_rgb(190, 120, 255),
    }
}

pub(crate) fn op_buttons(ui: &mut egui::Ui, op: &mut CsgOp) {
    ui.horizontal(|ui| {
        for candidate in CsgOp::ALL {
            let text = RichText::new(candidate.label()).color(op_color(candidate));
            ui.selectable_value(op, candidate, text);
        }
    });
}

pub(crate) fn shape_editor(ui: &mut egui::Ui, shape: &mut BrushShape) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("csg-shape")
            .selected_text(shape.label())
            .show_ui(ui, |ui| {
                for candidate in [
                    BrushShape::Box,
                    BrushShape::Wedge,
                    BrushShape::Cylinder { sides: 16 },
                    BrushShape::Pyramid { sides: 4 },
                    BrushShape::Stairs { steps: 8 },
                ] {
                    let same = std::mem::discriminant(shape) == std::mem::discriminant(&candidate);
                    if ui.selectable_label(same, candidate.label()).clicked() && !same {
                        *shape = candidate;
                    }
                }
            });
        match shape {
            BrushShape::Cylinder { sides } | BrushShape::Pyramid { sides } => {
                ui.add(
                    DragValue::new(sides)
                        .range(BrushShape::MIN_SIDES..=BrushShape::MAX_SIDES)
                        .prefix("sides "),
                );
            }
            BrushShape::Stairs { steps } => {
                ui.add(
                    DragValue::new(steps)
                        .range(1..=BrushShape::MAX_STEPS)
                        .prefix("steps "),
                );
            }
            _ => {}
        }
    });
}

fn brush_inspector(ui: &mut egui::Ui, e: &mut EntityDesc) {
    let Some(brush) = &mut e.brush else { return };
    egui::Grid::new("csg-brush")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Operation");
            op_buttons(ui, &mut brush.op);
            ui.end_row();
            ui.label("Shape");
            shape_editor(ui, &mut brush.shape);
            ui.end_row();
            ui.label("Color");
            olvin_editor::widgets::color_rgba(ui, &mut brush.color);
            ui.end_row();
        });
    ui.weak("Size = Scale. Operations apply in hierarchy order.");
}

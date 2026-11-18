//! The Mapping window: tool modes (Edit Shape is the Modeling Edit Mode), boolean
//! operations on the selected brush, defaults for new brushes, build stats.

use olvin_editor::egui::{self, DragValue, RichText};
use olvin_editor::glam::{EulerRot, Quat, Vec3};
use olvin_editor::olvin_scene::file::{CsgOp, EntityDesc, euler_to_quat};
use olvin_editor::{EditorState, Panel, tools};

use crate::{DRAW_TOOL, EDIT_TOOL, MODEL_EDIT_TOOL, hollow, op_buttons, shape_editor, shared};

pub struct CsgPanel;

impl Panel for CsgPanel {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                // Tool modes, like Realtime CSG's Object / Generate / Edit.
                ui.horizontal_wrapped(|ui| {
                    for (tool, label, key) in [
                        (tools::MOVE, "⊹ Object", "W"),
                        (DRAW_TOOL, "✏ Draw", "B"),
                        (EDIT_TOOL, "⬚ Resize", "T"),
                        (MODEL_EDIT_TOOL, "◆ Edit Shape", "Tab"),
                    ] {
                        let text = RichText::new(label).size(14.0);
                        if ui
                            .selectable_label(state.tool == tool, text)
                            .on_hover_text(key)
                            .clicked()
                        {
                            state.tool = tool.to_owned();
                        }
                    }
                });
                ui.add_space(4.0);

                let brush = state
                    .selection
                    .filter(|id| state.scene.get(*id).is_some_and(|e| e.brush.is_some()));
                booleans(ui, state, brush);
                new_brushes(ui);

                section(ui, "Display", |ui| {
                    shared(|s| {
                        ui.checkbox(&mut s.outlines, "Brush outlines");
                        ui.checkbox(&mut s.grid_texture, "Grid texture on surfaces");
                    });
                });
                let stats = shared(|s| s.stats);
                section(ui, "Build", |ui| {
                    ui.label(format!(
                        "{} brushes → {} polygons, {} triangles",
                        stats.brushes, stats.polygons, stats.triangles
                    ));
                    ui.weak(format!("last rebuild {:.2} ms", stats.build_ms));
                });
            });
    }
}

fn booleans(ui: &mut egui::Ui, state: &mut EditorState, brush: Option<u64>) {
    section(ui, "Boolean", |ui| {
        let Some(id) = brush else {
            ui.weak("Select a brush to change how it combines with the brushes before it");
            return;
        };
        if let Some(b) = state.scene.get_mut(id).and_then(|e| e.brush.as_mut()) {
            op_buttons(ui, &mut b.op);
        }
        ui.weak("Brushes apply top to bottom in the hierarchy.");
        ui.horizontal(|ui| {
            if ui.button("▲ Earlier").clicked() {
                state.move_sibling(id, -1);
            }
            if ui.button("▼ Later").clicked() {
                state.move_sibling(id, 1);
            }
            if ui
                .button("Duplicate as Carve")
                .on_hover_text("Same volume, subtractive")
                .clicked()
                && let Some(copy) = state.duplicate(id)
                && let Some(b) = state.scene.get_mut(copy).and_then(|e| e.brush.as_mut())
            {
                b.op = CsgOp::Subtract;
            }
        });
        ui.horizontal(|ui| {
            let wall = shared(|s| s.wall);
            let mut w = wall;
            if ui
                .button("Hollow")
                .on_hover_text("Add a subtractive inner brush: a room")
                .clicked()
            {
                hollow(state, id);
                state.selection = Some(id);
            }
            ui.add(
                DragValue::new(&mut w)
                    .range(0.01..=10.0)
                    .speed(0.01)
                    .prefix("wall "),
            );
            if w != wall {
                shared(|s| s.wall = w);
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Rotate 90°").clicked() {
                edit(state, id, |e| {
                    let q = Quat::from_rotation_y(90f32.to_radians()) * euler_to_quat(e.rotation);
                    let (y, x, z) = q.to_euler(EulerRot::YXZ);
                    e.rotation = Vec3::new(x, y, z).map(f32::to_degrees);
                });
            }
            if ui.button("Flip X").clicked() {
                edit(state, id, |e| e.scale.x = -e.scale.x);
            }
            if ui.button("Flip Z").clicked() {
                edit(state, id, |e| e.scale.z = -e.scale.z);
            }
        });
    });
}

fn new_brushes(ui: &mut egui::Ui) {
    section(ui, "New brushes", |ui| {
        shared(|s| {
            egui::Grid::new("csg-new")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Operation");
                    op_buttons(ui, &mut s.op);
                    ui.end_row();
                    ui.label("Shape");
                    shape_editor(ui, &mut s.shape);
                    ui.end_row();
                    ui.label("Color");
                    ui.color_edit_button_rgba_unmultiplied(&mut s.color);
                    ui.end_row();
                });
        });
        ui.weak("Draw: drag on a surface, then move to set the height. Down carves, Shift forces Subtract, Esc/RMB cancels.");
    });
}

fn edit(state: &mut EditorState, id: u64, f: impl FnOnce(&mut EntityDesc)) {
    if let Some(e) = state.scene.get_mut(id) {
        f(e);
    }
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            ui.add_space(2.0);
            body(ui);
        });
    ui.add_space(4.0);
}

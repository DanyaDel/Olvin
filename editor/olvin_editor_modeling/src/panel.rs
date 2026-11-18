//! The Modeling panel: Edit Mode controls and every operation as a button.

use olvin_editor::egui::{self, DragValue, RichText};
use olvin_editor::glam::{Mat4, Vec3};
use olvin_editor::{EditorState, Panel, tools};

use crate::ops::{self, Op, make_editable};
use crate::{EDIT_TOOL, Mode, shared};

pub struct ModelingPanel;

impl Panel for ModelingPanel {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let editing = state.tool == EDIT_TOOL;
            let has_mesh = state.selection.and_then(|id| state.scene.get(id)).is_some_and(|e| e.poly.is_some());
            ui.horizontal(|ui| {
                let label = if editing { "◀ Object Mode" } else { "◆ Edit Mode" };
                if ui.add(egui::Button::new(RichText::new(label).strong()).min_size(egui::vec2(130.0, 26.0))).on_hover_text("Tab").clicked() {
                    if editing {
                        state.tool = tools::MOVE.to_owned();
                    } else if let Some(id) = state.selection {
                        match make_editable(state, id) {
                            Ok(()) => state.tool = EDIT_TOOL.to_owned(),
                            Err(e) => state.warn(e),
                        }
                    }
                }
                if !has_mesh && state.selection.is_some() && ui.button("Make Editable").clicked() {
                    let id = state.selection.unwrap();
                    if let Err(e) = make_editable(state, id) {
                        state.warn(e);
                    }
                }
            });
            ui.add_space(6.0);
            brush_section(ui, state);

            let mut run = None;
            section(ui, "Select", |ui| {
                ui.horizontal(|ui| {
                    shared(|s| {
                        for (mode, label) in [(Mode::Vertex, "Vertex 1"), (Mode::Edge, "Edge 2"), (Mode::Face, "Face 3")] {
                            ui.selectable_value(&mut s.mode, mode, label);
                        }
                    });
                });
                grid(ui, &mut run, &[Op::SelectAll, Op::SelectNone, Op::SelectInvert, Op::SelectLinked, Op::SelectGrow, Op::SelectShrink]);
                grid(ui, &mut run, &[Op::Hide, Op::HideUnselected, Op::Reveal]);
                ui.horizontal(|ui| {
                    shared(|s| {
                        ui.toggle_value(&mut s.xray, "X-ray").on_hover_text("Select through the mesh (Alt+Z)");
                        ui.toggle_value(&mut s.gizmo, "Gizmo").on_hover_text("Move gizmo at the selection");
                    });
                });
                let (n, hidden) = shared(|s| (s.verts.len(), s.hidden.len()));
                let hidden = if hidden > 0 { format!(" · {hidden} hidden") } else { String::new() };
                ui.weak(format!("{n} vertices selected{hidden}"));
                ui.weak("Click · Shift+click adds · drag: box (Shift adds, Ctrl subtracts) · Alt+click: loop · Ctrl+Alt+click: ring · L: linked under cursor");
            });

            section(ui, "Transform", |ui| {
                match pivot(state) {
                    Some(p) => ui.label(format!("Pivot (median)  {:.3}  {:.3}  {:.3}", p.x, p.y, p.z)),
                    None => ui.weak("Nothing selected"),
                };
                ui.horizontal(|ui| {
                    ui.label("Move by");
                    shared(|s| {
                        for (i, c) in ["X", "Y", "Z"].into_iter().enumerate() {
                            ui.add(DragValue::new(&mut s.move_by[i]).speed(0.01).prefix(format!("{c} ")));
                        }
                    });
                    if ui.button("Apply").clicked() {
                        run = Some(Op::MoveBy(shared(|s| s.move_by)));
                    }
                });
                ui.weak("Viewport: G move · R rotate · S scale, then X/Y/Z (twice: local / normal), Shift+X/Y/Z plane, type a value, Ctrl snap, Shift precise");
            });

            section(ui, "Create", |ui| {
                grid(ui, &mut run, &[Op::Fill, Op::Duplicate, Op::Connect]);
                ui.weak("E extrude · I inset · K knife · Ctrl+R loop cut (in the viewport)");
            });
            section(ui, "Cut", |ui| {
                ui.horizontal(|ui| {
                    shared(|s| {
                        ui.label("Cuts");
                        ui.add(DragValue::new(&mut s.cuts).range(1..=16));
                    });
                });
                grid(ui, &mut run, &[Op::LoopCutSelected, Op::Subdivide]);
            });
            section(ui, "Delete & Convert", |ui| {
                grid(ui, &mut run, &[Op::Delete, Op::DeleteOnlyFaces, Op::DissolveEdges]);
                ui.weak("Faces → Edges keeps the outline as edges; select edges and F to turn them back into a face.");
            });
            section(ui, "Clean Up", |ui| {
                grid(ui, &mut run, &[Op::Merge, Op::MergeByDistance, Op::Flip, Op::RecalculateNormals, Op::Smooth]);
            });
            if let Some(op) = run {
                if !editing {
                    state.warn("enter Edit Mode first (Tab)");
                } else if let Err(e) = ops::apply(state, op) {
                    state.warn(e);
                }
            }

            if let Some(poly) = state.selection.and_then(|id| state.scene.get(id)).and_then(|e| e.poly.as_ref()) {
                section(ui, "Mesh", |ui| {
                    ui.label(format!("{} vertices · {} edges · {} faces", poly.positions.len(), poly.edges().len(), poly.faces.len()));
                    if !poly.wire.is_empty() {
                        ui.weak(format!("{} loose edges", poly.wire.len()));
                    }
                });
            }
        });
    }
}

/// Median of the selected vertices in world space.
fn pivot(state: &EditorState) -> Option<Vec3> {
    let id = shared(|s| s.entity)?;
    let mesh = state.scene.get(id)?.poly.as_ref()?;
    let world = state
        .scene
        .world_matrices()
        .get(&id)
        .copied()
        .unwrap_or(Mat4::IDENTITY);
    let verts: Vec<Vec3> = shared(|s| s.verts.clone())
        .into_iter()
        .filter_map(|v| mesh.positions.get(v as usize))
        .map(|p| world.transform_point3(*p))
        .collect();
    (!verts.is_empty()).then(|| verts.iter().sum::<Vec3>() / verts.len() as f32)
}

fn grid(ui: &mut egui::Ui, run: &mut Option<Op>, ops: &[Op]) {
    ui.horizontal_wrapped(|ui| {
        for op in ops {
            if ui.button(op.label()).clicked() {
                *run = Some(*op);
            }
        }
    });
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            ui.add_space(2.0);
            body(ui);
        });
    ui.add_space(4.0);
}

/// CSG part of a modelled brush: the same object is edited and carves the level.
fn brush_section(ui: &mut egui::Ui, state: &mut EditorState) {
    let Some(id) = state.selection else { return };
    let Some(e) = state.scene.get_mut(id) else {
        return;
    };
    let modelled = e.poly.is_some();
    let Some(brush) = &mut e.brush else { return };
    let (mut detach, mut reset) = (false, false);
    section(ui, "CSG Brush", |ui| {
        ui.horizontal(|ui| {
            for op in olvin_csg::CsgOp::ALL {
                ui.selectable_value(&mut brush.op, op, op.label());
            }
        });
        ui.horizontal(|ui| {
            ui.label("Color");
            ui.color_edit_button_rgba_unmultiplied(&mut brush.color);
        });
        ui.horizontal(|ui| {
            if modelled
                && ui
                    .button("Reset to Shape")
                    .on_hover_text("Drop the modelled shape")
                    .clicked()
            {
                reset = true;
            }
            if ui
                .button("Detach from CSG")
                .on_hover_text("Make it a plain mesh object")
                .clicked()
            {
                detach = true;
            }
        });
        ui.weak("Edit Mode works on brushes too: the level is carved by exactly what you model, concave shapes included.");
    });
    if reset {
        ops::reset_brush(state, id);
    }
    if detach {
        ops::detach_brush(state, id);
    }
}

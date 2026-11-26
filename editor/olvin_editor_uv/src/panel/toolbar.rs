//! The toolbar (modes, operator menus, settings) and the requests it issues.

use olvin_editor::EditorState;
use olvin_editor::egui;
use olvin_editor::keymap::Command;
use olvin_mesh::{PolyMesh, UvCorner};

use super::{Request, UvEditor, display, poly_mut};
use crate::hooks;
use crate::ops::{self, Target, UvOp};
use crate::select::{Combine, SelectMode, Topo};

impl UvEditor {
    pub(super) fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        state: &EditorState,
        id: Option<u64>,
        request: &mut Option<Request>,
    ) {
        let busy = self.modal.is_some();
        let key = |command| state.prefs.keymap.shortcut_text(command);
        let label = |name, command| format!("{name}  {}", key(command));
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (mode, name, command) in [
                    (SelectMode::Vertex, "Vertex", Command::UvVertex),
                    (SelectMode::Edge, "Edge", Command::UvEdge),
                    (SelectMode::Face, "Face", Command::UvFace),
                    (SelectMode::Island, "Island", Command::UvIsland),
                ] {
                    ui.selectable_value(&mut self.mode, mode, format!("{name} {}", key(command)));
                }
                ui.separator();
                let mut op = |ui: &mut egui::Ui, op: UvOp, label: &str| {
                    if ui.button(label).on_hover_text(op.tooltip()).clicked() {
                        *request = Some(Request::Op(op));
                        ui.close();
                    }
                };
                op(ui, UvOp::Unwrap, &label("Unwrap", Command::UvUnwrap));
                ui.menu_button("Project", |ui| {
                    for o in UvOp::PROJECT {
                        op(ui, o, o.label());
                    }
                });
                ui.menu_button("Seams", |ui| {
                    for o in UvOp::SEAMS {
                        if o == UvOp::MarkSeam && !state.prefs.keymap.is_default(Command::UvSeam) {
                            op(ui, o, &label(o.label(), Command::UvSeam));
                        } else {
                            op(ui, o, o.label());
                        }
                    }
                });
                op(ui, UvOp::Pack, &label("Pack", Command::UvPack));
                ui.menu_button("Arrange", |ui| {
                    for o in UvOp::ARRANGE {
                        op(ui, o, o.label());
                    }
                });
                ui.menu_button("Select", |ui| {
                    for (r, label) in [
                        (Request::SelectAll, label("All", Command::UvSelectAll)),
                        (Request::SelectNone, label("None", Command::UvSelectNone)),
                        (Request::SelectInvert, label("Invert", Command::UvInvert)),
                        (Request::SelectLinked, label("Linked", Command::UvLinked)),
                    ] {
                        if ui.button(label).clicked() {
                            *request = Some(r);
                            ui.close();
                        }
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                let wired = hooks().face_filter.is_some();
                ui.add_enabled(wired, egui::Checkbox::new(&mut self.only_selected_3d, "Only selected in 3D"))
                    .on_hover_text("Show only the faces selected in Edit Mode (Blender's UV sync off)")
                    .on_disabled_hover_text(
                        "Not connected yet: the Modeling element has to provide its Edit Mode selection (olvin_editor_uv::set_face_filter)",
                    );
                ui.toggle_value(&mut self.gizmo, "Gizmo");
                ui.label("Texture");
                ui.add(egui::DragValue::new(&mut self.texture_px).range(16..=16384).suffix(" px"));
                ui.label("Margin");
                ui.add(egui::DragValue::new(&mut self.margin).range(0.0..=0.2).speed(0.001));
                if let Some(mesh) = id.and_then(|id| display(state, id)) {
                    let topo = Topo::new(&mesh, self.visible(state, id.unwrap_or(0), &mesh));
                    let mut faces = self.sel.faces(&topo);
                    let scope = if faces.is_empty() {
                        faces = topo.visible.clone();
                        "all"
                    } else {
                        "selected"
                    };
                    match mesh.texel_density(&faces, self.texture_px as f32) {
                        Some(d) => ui.label(format!("Texel density ({scope}): {d:.1} px/m")),
                        None => ui.weak("Texel density: —"),
                    };
                }
            });
        });
    }

    pub(super) fn run_request(
        &mut self,
        r: Request,
        state: &mut EditorState,
        id: u64,
        mesh: &PolyMesh,
        visible: &[usize],
    ) {
        let topo = Topo::new(mesh, visible.to_vec());
        match r {
            Request::SelectAll => self.sel.combine(&topo.corners(), Combine::Replace),
            Request::SelectNone => self.sel.clear(),
            Request::SelectInvert => {
                let rest: Vec<UvCorner> = topo
                    .corners()
                    .into_iter()
                    .filter(|c| !self.sel.contains(*c))
                    .collect();
                self.sel.combine(&rest, Combine::Replace);
            }
            Request::SelectLinked => {
                let faces: Vec<usize> = self.sel.corners.iter().map(|c| c.0).collect();
                let picked = topo.face_corners(&topo.islands_of(&faces));
                self.sel.combine(&picked, Combine::Add);
            }
            Request::Op(op) => {
                let selected = self.sel.faces(&topo);
                let target = Target {
                    selected: !selected.is_empty(),
                    faces: if selected.is_empty() {
                        topo.visible.clone()
                    } else {
                        selected
                    },
                    edges: self.sel.edges(&topo),
                    corners: topo.sticky(self.sel.corners.iter().copied()),
                    active: self.active_face,
                };
                let margin = self.margin;
                let Some(poly) = poly_mut(state, id) else {
                    return;
                };
                match ops::apply(poly, op, &target, margin) {
                    Ok(msg) if !msg.is_empty() => state.info(format!("UV: {msg}")),
                    Ok(_) => {}
                    Err(e) => state.warn(format!("UV {}: {e}", op.label())),
                }
            }
        }
    }
}

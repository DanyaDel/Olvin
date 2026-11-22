//! The UV Editor panel: toolbar, 2D canvas, selection, modal G/R/S, gizmo.

use olvin_editor::egui::{self, Align2, FontId, Key, Modifiers, Pos2, Sense};
use olvin_editor::glam::Vec2;
use olvin_editor::{EditorState, Panel, theme};
use olvin_mesh::{PolyMesh, UvCorner};

use crate::hooks;
use crate::ops::UvOp;
use crate::select::{SelectMode, Selection};
use crate::transform::{Kind, NumInput};
use crate::view::View;

mod draw;
mod input;
mod modal;
#[cfg(test)]
mod tests;
mod toolbar;

fn p2(v: Vec2) -> Pos2 {
    Pos2::new(v.x, v.y)
}

fn v2(p: Pos2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

/// A G / R / S in progress: one undo step (`state.busy` while it runs).
struct Modal {
    kind: Kind,
    /// The mesh as it was (restored on cancel).
    original: PolyMesh,
    /// `original` with UVs ensured; every frame starts from it.
    before: PolyMesh,
    corners: Vec<UvCorner>,
    pivot: Vec2,
    start: Vec2,
    axis: Option<usize>,
    num: NumInput,
    /// Started by dragging the gizmo: confirms on release.
    drag: bool,
    summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    Op(UvOp),
    SelectAll,
    SelectNone,
    SelectInvert,
    SelectLinked,
}

pub struct UvEditor {
    pub view: View,
    framed: bool,
    pub mode: SelectMode,
    pub sel: Selection,
    entity: Option<u64>,
    active_face: Option<usize>,
    modal: Option<Modal>,
    boxing: Option<Pos2>,
    /// The press that confirmed a modal must not also select on release.
    swallow_click: bool,
    pub texture_px: u32,
    pub margin: f32,
    pub only_selected_3d: bool,
    pub gizmo: bool,
    /// Canvas centre of the last frame.
    last_origin: Vec2,
}

impl Default for UvEditor {
    fn default() -> Self {
        Self {
            view: View::default(),
            framed: false,
            mode: SelectMode::Vertex,
            sel: Selection::default(),
            entity: None,
            active_face: None,
            modal: None,
            boxing: None,
            swallow_click: false,
            texture_px: 1024,
            margin: 0.02,
            only_selected_3d: false,
            gizmo: true,
            last_origin: Vec2::ZERO,
        }
    }
}

/// The entity's mesh with UVs (box-projected on the fly if it has none yet).
fn display(state: &EditorState, id: u64) -> Option<PolyMesh> {
    let mut m = state.scene.get(id)?.poly.clone()?;
    m.ensure_uvs();
    Some(m)
}

fn poly_mut(state: &mut EditorState, id: u64) -> Option<&mut PolyMesh> {
    state.scene.get_mut(id)?.poly.as_mut()
}

fn chord(ui: &egui::Ui, mods: Modifiers, key: Key) -> bool {
    ui.input(|i| i.key_pressed(key) && i.modifiers.matches_exact(mods))
}

impl Panel for UvEditor {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let id = state
            .selection
            .filter(|id| state.scene.get(*id).is_some_and(|e| e.poly.is_some()));
        if id != self.entity {
            self.entity = id;
            self.sel.clear();
            self.active_face = None;
            self.modal = None;
            self.boxing = None;
        }
        let mut request = None;
        self.toolbar(ui, state, id, &mut request);

        let size = ui.available_size().max(egui::vec2(32.0, 32.0));
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 4.0, theme::palette(ui.ctx()).canvas());
        let origin = v2(rect.center());
        self.last_origin = origin;
        if !self.framed && rect.width() > 1.0 {
            self.view
                .frame(v2(rect.size().to_pos2()), Vec2::ZERO, Vec2::ONE);
            self.framed = true;
        }
        if self.modal.is_none() {
            self.navigate(ui, &response, origin);
        }

        let Some((id, mut mesh)) = id.and_then(|id| Some((id, display(state, id)?))) else {
            self.draw_background(&painter, origin, state, None);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Select a mesh object (Modeling: Make Editable) to edit its UVs",
                FontId::proportional(13.0),
                theme::palette(ui.ctx()).muted,
            );
            return;
        };
        let mut visible = self.visible(state, id, &mesh);
        self.sel.sanitize(&mesh);
        if response.hovered() {
            state.tool_owns_keys = true;
        }
        let keys =
            (response.hovered() || self.modal.is_some()) && !ui.ctx().egui_wants_keyboard_input();

        if self.modal.is_some() {
            state.busy = true;
            self.run_modal(ui, state, id, origin);
        } else {
            if keys {
                self.hotkeys(
                    ui,
                    state,
                    id,
                    &mesh,
                    &visible,
                    origin,
                    rect,
                    &response,
                    &mut request,
                );
            }
            if self.modal.is_none() {
                self.pointer(ui, &response, state, id, &mesh, &visible, origin);
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) && !response.clicked() {
            self.swallow_click = false;
        }
        if let Some(r) = request {
            self.run_request(r, state, id, &mesh, &visible);
        }
        if let Some(m) = display(state, id) {
            mesh = m;
            visible = self.visible(state, id, &mesh);
            self.sel.sanitize(&mesh);
        }
        let real_uvs = state
            .scene
            .get(id)
            .and_then(|e| e.poly.as_ref())
            .is_some_and(|p| p.has_uvs() || p.faces.is_empty());
        self.draw(
            ui, &painter, rect, &response, state, id, &mesh, visible, real_uvs,
        );
    }
}

impl UvEditor {
    fn visible(&self, state: &EditorState, id: u64, mesh: &PolyMesh) -> Vec<usize> {
        let all = || (0..mesh.faces.len()).collect();
        match hooks().face_filter {
            Some(filter) if self.only_selected_3d => filter(state, id).unwrap_or_else(all),
            _ => all(),
        }
    }
}

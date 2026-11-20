//! Canvas input: navigation, framing, hotkeys, click / box selection.

use olvin_editor::EditorState;
use olvin_editor::egui::{self, Key, Modifiers, PointerButton, Rect};
use olvin_editor::glam::Vec2;
use olvin_mesh::{PolyMesh, UvCorner};

use super::{Request, UvEditor, chord, v2};
use crate::ops::UvOp;
use crate::select::{Combine, Hit, SelectMode, Topo};
use crate::transform::{self, Kind};

impl UvEditor {
    pub(super) fn navigate(&mut self, ui: &egui::Ui, response: &egui::Response, origin: Vec2) {
        let (scroll, zoom, alt) =
            ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta(), i.modifiers.alt));
        if response.hovered()
            && let Some(mouse) = response.hover_pos()
        {
            let factor = (scroll * 0.0015).exp() * zoom;
            if (factor - 1.0).abs() > 1e-6 {
                self.view.zoom_at(origin, v2(mouse), factor);
            }
        }
        if response.dragged_by(PointerButton::Middle)
            || (alt && response.dragged_by(PointerButton::Primary))
        {
            self.view.pan(v2(response.drag_delta().to_pos2()));
        }
    }

    fn frame(&mut self, rect: Rect, mesh: &PolyMesh, visible: &[usize], selected_only: bool) {
        let topo = Topo::new(mesh, visible.to_vec());
        let corners: Vec<UvCorner> = if selected_only && !self.sel.is_empty() {
            self.sel
                .corners
                .iter()
                .copied()
                .filter(|c| topo.is_shown(c.0))
                .collect()
        } else {
            topo.corners()
        };
        let mut pts: Vec<Vec2> = corners.iter().map(|&c| topo.uv(c)).collect();
        if !selected_only || pts.is_empty() {
            pts.extend([Vec2::ZERO, Vec2::ONE]);
        }
        let (lo, hi) = pts
            .iter()
            .fold((Vec2::MAX, Vec2::MIN), |(l, h), &p| (l.min(p), h.max(p)));
        self.view.frame(v2(rect.size().to_pos2()), lo, hi);
    }

    // --- hotkeys --------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub(super) fn hotkeys(
        &mut self,
        ui: &egui::Ui,
        state: &mut EditorState,
        id: u64,
        mesh: &PolyMesh,
        visible: &[usize],
        origin: Vec2,
        rect: Rect,
        response: &egui::Response,
        request: &mut Option<Request>,
    ) {
        let none = Modifiers::NONE;
        for (key, mode) in [
            (Key::Num1, SelectMode::Vertex),
            (Key::Num2, SelectMode::Edge),
            (Key::Num3, SelectMode::Face),
            (Key::Num4, SelectMode::Island),
        ] {
            if chord(ui, none, key) {
                self.mode = mode;
            }
        }
        let r = if chord(ui, none, Key::A) {
            Some(Request::SelectAll)
        } else if chord(ui, Modifiers::ALT, Key::A) {
            Some(Request::SelectNone)
        } else if chord(ui, Modifiers::COMMAND, Key::I) {
            Some(Request::SelectInvert)
        } else if chord(ui, Modifiers::COMMAND, Key::L) {
            Some(Request::SelectLinked)
        } else if chord(ui, none, Key::U) {
            Some(Request::Op(UvOp::Unwrap))
        } else if chord(ui, Modifiers::COMMAND, Key::E) {
            Some(Request::Op(UvOp::MarkSeam))
        } else if chord(ui, Modifiers::COMMAND, Key::P) {
            Some(Request::Op(UvOp::Pack))
        } else {
            None
        };
        if r.is_some() {
            *request = r;
        }
        if chord(ui, none, Key::F) {
            self.frame(rect, mesh, visible, true);
        } else if chord(ui, none, Key::Home) {
            self.frame(rect, mesh, visible, false);
        }
        let mouse = response.hover_pos().map(v2);
        // L / Shift+L: (de)select the island under the cursor.
        for (mods, how) in [(none, Combine::Add), (Modifiers::SHIFT, Combine::Subtract)] {
            if chord(ui, mods, Key::L)
                && let Some(m) = mouse
            {
                let topo = Topo::new(mesh, visible.to_vec());
                let view = self.view;
                if let Some(hit) = topo.pick(SelectMode::Face, |t| view.to_screen(origin, t), m) {
                    let picked = topo.expand(hit, SelectMode::Island);
                    self.sel.combine(&picked, how);
                } else if let Some(hit) =
                    topo.pick(SelectMode::Vertex, |t| view.to_screen(origin, t), m)
                {
                    let (Hit::Vertex(c) | Hit::Edge(c)) = hit else {
                        continue;
                    };
                    let picked = topo.expand(Hit::Face(c.0), SelectMode::Island);
                    self.sel.combine(&picked, how);
                }
            }
        }
        let start = mouse.unwrap_or(origin);
        for (key, kind) in [
            (Key::G, Kind::Move),
            (Key::R, Kind::Rotate),
            (Key::S, Kind::Scale),
        ] {
            if chord(ui, none, key) {
                self.start_transform(
                    state,
                    id,
                    mesh,
                    visible,
                    kind,
                    self.view.to_uv(origin, start),
                    false,
                    None,
                );
            }
        }
    }

    // --- pointer ----------------------------------------------------------------

    /// Pivot of the selection (sticky corners' bounding-box centre).
    pub(super) fn pivot(&self, topo: &Topo) -> Option<(Vec<UvCorner>, Vec2)> {
        let corners = topo.sticky(self.sel.corners.iter().copied());
        let pivot = transform::bounds_center(corners.iter().map(|&c| topo.uv(c)))?;
        Some((corners, pivot))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        state: &mut EditorState,
        id: u64,
        mesh: &PolyMesh,
        visible: &[usize],
        origin: Vec2,
    ) {
        let (alt, shift, ctrl) =
            ui.input(|i| (i.modifiers.alt, i.modifiers.shift, i.modifiers.command));
        let topo = Topo::new(mesh, visible.to_vec());
        let view = self.view;
        let to_screen = |t: Vec2| view.to_screen(origin, t);
        let gizmo_at = self
            .pivot(&topo)
            .filter(|_| self.gizmo)
            .map(|(_, p)| to_screen(p));

        if !alt && response.drag_started_by(PointerButton::Primary) {
            let press = ui
                .input(|i| i.pointer.press_origin())
                .or(response.hover_pos());
            let handle = gizmo_at
                .zip(press)
                .and_then(|(g, p)| transform::gizmo_hit(g, v2(p)));
            if let (Some(axis), Some(p)) = (handle, press) {
                let start = view.to_uv(origin, v2(p));
                self.start_transform(state, id, mesh, visible, Kind::Move, start, true, axis);
                return;
            }
            self.boxing = press;
        }
        if let Some(start) = self.boxing {
            if !ui.input(|i| i.pointer.primary_down()) {
                let end = response
                    .hover_pos()
                    .or(ui.input(|i| i.pointer.latest_pos()))
                    .unwrap_or(start);
                let (a, b) = (view.to_uv(origin, v2(start)), view.to_uv(origin, v2(end)));
                let (lo, hi) = (a.min(b), a.max(b));
                let picked = topo.boxed(self.mode, |t| t.cmpge(lo).all() && t.cmple(hi).all());
                let how = if ctrl {
                    Combine::Subtract
                } else if shift {
                    Combine::Add
                } else {
                    Combine::Replace
                };
                self.sel.combine(&picked, how);
                self.boxing = None;
            }
            return;
        }
        if response.clicked_by(PointerButton::Primary) && !alt && !self.swallow_click {
            let Some(mouse) = response.hover_pos().map(v2) else {
                return;
            };
            if gizmo_at.is_some_and(|g| transform::gizmo_hit(g, mouse).is_some()) {
                return;
            }
            match topo.pick(self.mode, to_screen, mouse) {
                Some(hit) => {
                    let picked = topo.expand(hit, self.mode);
                    self.active_face = Some(match hit {
                        Hit::Vertex(c) | Hit::Edge(c) => c.0,
                        Hit::Face(f) => f,
                    });
                    let how = if shift {
                        Combine::Toggle
                    } else {
                        Combine::Replace
                    };
                    self.sel.combine(&picked, how);
                }
                None if !shift => self.sel.clear(),
                None => {}
            }
        }
    }
}

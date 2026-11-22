//! Canvas drawing: background, grids, faces / edges / points, gizmo, status.

use olvin_editor::egui::{self, Align2, Color32, FontId, Rect, Stroke};
use olvin_editor::glam::Vec2;
use olvin_editor::{EditorState, theme};
use olvin_mesh::PolyMesh;

use super::{UvEditor, p2, v2};
use crate::select::{Hit, SelectMode, Topo};
use crate::transform::{self, GIZMO_ARM, GIZMO_BOX, Kind};
use crate::{Background, hooks};

const CHECKER_TILES: usize = 8;

impl UvEditor {
    /// The 0..1 square's background: the [`crate::set_background`] hook, or
    /// a procedural checker until a texture can be drawn.
    pub(super) fn draw_background(
        &self,
        painter: &egui::Painter,
        origin: Vec2,
        state: &EditorState,
        id: Option<u64>,
    ) {
        let view = self.view;
        let pal = theme::palette(painter.ctx());
        let a = view.to_screen(origin, Vec2::new(0.0, 1.0));
        let b = view.to_screen(origin, Vec2::new(1.0, 0.0));
        let square = Rect::from_min_max(p2(a), p2(b));
        let hook: Option<Background> = hooks().background;
        let drawn = match (hook, id) {
            (Some(draw), Some(id)) => draw(painter, square, state, id),
            _ => false,
        };
        if !drawn {
            background(painter, square, pal.dark);
        }
        painter.rect_stroke(
            square,
            0.0,
            Stroke::new(1.0, pal.muted),
            egui::StrokeKind::Outside,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        response: &egui::Response,
        state: &EditorState,
        id: u64,
        mesh: &PolyMesh,
        visible: Vec<usize>,
        real_uvs: bool,
    ) {
        let origin = v2(rect.center());
        let view = self.view;
        let pal = theme::palette(ui.ctx());
        let to = |t: Vec2| p2(view.to_screen(origin, t));
        self.draw_background(painter, origin, state, Some(id));
        self.draw_grids(painter, rect, origin);

        let topo = Topo::new(mesh, visible);
        let hover = response
            .hover_pos()
            .filter(|_| self.modal.is_none() && self.boxing.is_none())
            .and_then(|m| topo.pick(self.mode, |t| view.to_screen(origin, t), v2(m)));

        // Faces.
        let selected_faces = self.sel.faces(&topo);
        let mut fill = egui::Mesh::default();
        for &f in &topo.visible {
            let color = if selected_faces.contains(&f) {
                pal.selected().gamma_multiply(0.28)
            } else if hover == Some(Hit::Face(f)) {
                pal.ink(30)
            } else {
                pal.ink(10)
            };
            let base = fill.vertices.len() as u32;
            for &t in &mesh.uvs[f] {
                fill.colored_vertex(to(t), color);
            }
            for [a, b, c] in topo.triangles(f) {
                fill.add_triangle(base + a, base + b, base + c);
            }
        }
        painter.add(egui::Shape::mesh(fill));

        // Edges: seams red, selected orange, hovered white.
        for c in topo.corners() {
            let n = topo.next(c);
            let face = &mesh.faces[c.0];
            let (a, b) = (face[c.1], face[n.1]);
            let stroke = if hover == Some(Hit::Edge(c)) {
                Stroke::new(2.5, pal.text)
            } else if self.sel.has_edge(&topo, c) {
                Stroke::new(1.8, pal.selected())
            } else if mesh.is_seam(a, b) {
                Stroke::new(1.8, pal.axes()[0])
            } else {
                Stroke::new(1.0, pal.text_2)
            };
            painter.line_segment([to(topo.uv(c)), to(topo.uv(n))], stroke);
        }

        match self.mode {
            SelectMode::Vertex | SelectMode::Edge => {
                for c in topo.corners() {
                    let (r, color) = if self.sel.contains(c) {
                        (3.5, pal.selected())
                    } else if hover == Some(Hit::Vertex(c)) {
                        (4.0, pal.text)
                    } else {
                        (2.0, pal.bg)
                    };
                    painter.circle_filled(to(topo.uv(c)), r, color);
                }
            }
            SelectMode::Face | SelectMode::Island => {
                for &f in &topo.visible {
                    let color = if selected_faces.contains(&f) {
                        pal.selected()
                    } else {
                        pal.bg
                    };
                    painter.circle_filled(to(topo.center(f)), 2.5, color);
                }
            }
        }

        if let Some(start) = self.boxing {
            let end = response.hover_pos().unwrap_or(start);
            let r = Rect::from_two_pos(start, end);
            painter.rect_filled(r, 0.0, pal.ink(12));
            painter.rect_stroke(r, 0.0, (1.0, pal.ink(200)), egui::StrokeKind::Middle);
        }

        // Gizmo or modal guides.
        if let Some(modal) = &self.modal {
            let p = view.to_screen(origin, modal.pivot);
            if let Some(axis) = modal.axis {
                let dir = if axis == 0 { Vec2::X } else { Vec2::NEG_Y };
                let far = rect.width().max(rect.height()) * 2.0;
                painter.line_segment(
                    [p2(p - dir * far), p2(p + dir * far)],
                    Stroke::new(1.5, pal.axes()[axis]),
                );
            }
            painter.circle_stroke(p2(p), 4.0, Stroke::new(1.5, pal.text));
            if modal.kind != Kind::Move
                && let Some(m) = ui.input(|i| i.pointer.latest_pos())
            {
                painter.add(egui::Shape::dashed_line(
                    &[p2(p), m],
                    Stroke::new(1.0, pal.ink(160)),
                    5.0,
                    4.0,
                ));
            }
        } else if self.gizmo
            && self.boxing.is_none()
            && let Some((_, pivot)) = self.pivot(&topo)
        {
            let c = view.to_screen(origin, pivot);
            let hot = response
                .hover_pos()
                .and_then(|m| transform::gizmo_hit(c, v2(m)));
            for (axis, dir) in [(0usize, Vec2::X), (1, Vec2::NEG_Y)] {
                let color = if hot == Some(Some(axis)) {
                    pal.text
                } else {
                    pal.axes()[axis]
                };
                let end = c + dir * GIZMO_ARM;
                painter.line_segment([p2(c), p2(end)], Stroke::new(2.5, color));
                let side = dir.perp() * 5.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![p2(end + dir * 10.0), p2(end + side), p2(end - side)],
                    color,
                    Stroke::NONE,
                ));
            }
            let color = if hot == Some(None) {
                pal.text
            } else {
                pal.accent
            };
            painter.rect_stroke(
                Rect::from_center_size(p2(c), egui::vec2(GIZMO_BOX * 2.0, GIZMO_BOX * 2.0)),
                2.0,
                Stroke::new(1.5, color),
                egui::StrokeKind::Middle,
            );
        }

        // Status lines.
        let status = match &self.modal {
            Some(m) => format!(
                "{}   X/Y axis · type a value · Ctrl snap · LMB/Enter confirm · RMB/Esc cancel",
                m.summary
            ),
            None => format!(
                "{} corners selected · G/R/S · U unwrap · L island · A all · F/Home frame · MMB/Alt+LMB pan · wheel zoom",
                self.sel.corners.len()
            ),
        };
        painter.text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            status,
            FontId::proportional(12.0),
            pal.text,
        );
        if !real_uvs {
            painter.text(
                rect.left_top() + egui::vec2(8.0, 8.0),
                Align2::LEFT_TOP,
                "No UVs yet: showing a box projection. Unwrap (U) or project to create them.",
                FontId::proportional(12.0),
                ui.visuals().warn_fg_color,
            );
        }
    }

    /// Faint lines on whole UV tiles, and the 1/64 pixel grid when zoomed in.
    fn draw_grids(&self, painter: &egui::Painter, rect: Rect, origin: Vec2) {
        let view = self.view;
        let pal = theme::palette(painter.ctx());
        let lo = view.to_uv(origin, v2(rect.left_bottom()));
        let hi = view.to_uv(origin, v2(rect.right_top()));
        let lines = |step: f32, color: Color32, limit: f32| {
            if (hi.x - lo.x) / step > limit || (hi.y - lo.y) / step > limit {
                return;
            }
            let mut u = (lo.x / step).floor() * step;
            while u <= hi.x {
                let (a, b) = (
                    view.to_screen(origin, Vec2::new(u, lo.y)),
                    view.to_screen(origin, Vec2::new(u, hi.y)),
                );
                painter.line_segment([p2(a), p2(b)], Stroke::new(1.0, color));
                u += step;
            }
            let mut v = (lo.y / step).floor() * step;
            while v <= hi.y {
                let (a, b) = (
                    view.to_screen(origin, Vec2::new(lo.x, v)),
                    view.to_screen(origin, Vec2::new(hi.x, v)),
                );
                painter.line_segment([p2(a), p2(b)], Stroke::new(1.0, color));
                v += step;
            }
        };
        lines(1.0, pal.ink(22), 200.0);
        if view.zoom * transform::PIXEL >= 10.0 {
            lines(transform::PIXEL, pal.ink(40), 400.0);
        }
    }
}

/// Procedural checker filling `square` (the 0..1 UV square on screen).
/// This is the fallback of the [`crate::Background`] hook: once materials can
/// hand their base texture to the editor, a hook draws that instead.
pub fn background(painter: &egui::Painter, square: Rect, dark: bool) {
    let n = CHECKER_TILES;
    let checker = if dark {
        [
            Color32::from_rgb(78, 82, 92),
            Color32::from_rgb(100, 105, 116),
        ]
    } else {
        [
            Color32::from_rgb(190, 194, 204),
            Color32::from_rgb(214, 217, 224),
        ]
    };
    let step = square.size() / n as f32;
    painter.rect_filled(square, 0.0, checker[0]);
    for i in 0..n {
        for j in 0..n {
            if (i + j) % 2 == 1 {
                let min = square.min + egui::vec2(step.x * i as f32, step.y * j as f32);
                painter.rect_filled(Rect::from_min_size(min, step), 0.0, checker[1]);
            }
        }
    }
}

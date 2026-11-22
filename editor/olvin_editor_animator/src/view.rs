//! Shared drawing and view maths of the Timeline, Dope Sheet and Graph Editor:
//! a pannable / zoomable axis, the frame ruler with the playhead, key diamonds.

use olvin_editor::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, Shape, pos2, vec2};
use olvin_editor::theme;

/// Blender's current-frame blue.
pub const PLAYHEAD: Color32 = Color32::from_rgb(71, 114, 179);
pub const RULER_H: f32 = 22.0;

/// A 1D view: `origin` is the value at pixel `base`, `scale` pixels per unit
/// (negative for an upward axis).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub origin: f32,
    pub scale: f32,
}

impl Axis {
    pub fn px(&self, v: f32, base: f32) -> f32 {
        base + (v - self.origin) * self.scale
    }

    pub fn value(&self, px: f32, base: f32) -> f32 {
        self.origin + (px - base) / self.scale
    }

    /// Zooms by `factor`, keeping the value under `around` fixed.
    pub fn zoom(&mut self, around: f32, base: f32, factor: f32, limits: (f32, f32)) {
        let v = self.value(around, base);
        let sign = self.scale.signum();
        self.scale = sign * (self.scale.abs() * factor).clamp(limits.0, limits.1);
        self.origin = v - (around - base) / self.scale;
    }

    pub fn pan(&mut self, dpx: f32) {
        self.origin -= dpx / self.scale;
    }

    /// Fits `lo..hi` into `len` pixels with a margin.
    pub fn fit(lo: f32, hi: f32, len: f32, margin: f32, upward: bool) -> Self {
        let span = (hi - lo).max(1e-3);
        let scale = ((len - 2.0 * margin) / span).max(1e-3);
        if upward {
            // Base is the bottom edge; `lo` sits `margin` above it.
            Self {
                origin: lo - margin / scale,
                scale: -scale,
            }
        } else {
            Self {
                origin: lo - margin / scale,
                scale,
            }
        }
    }
}

/// The smallest "nice" step (1, 2, 5 x 10^n) that spans at least `min_px`.
pub fn nice_step(scale: f32, min_px: f32) -> f32 {
    let want = min_px / scale.abs().max(1e-6);
    let mut mag = 10f32.powf(want.log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if want <= m * mag {
            return (m * mag).max(1e-4);
        }
    }
    mag *= 10.0;
    mag
}

/// Wheel / drag navigation for a view: returns `true` if it moved.
/// `zoom_wheel`: the plain wheel zooms (otherwise it is left to the caller).
pub fn navigate_x(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    axis: &mut Axis,
    zoom_wheel: bool,
) {
    let ctrl = ui.input(|i| i.modifiers.ctrl);
    if response.dragged_by(egui::PointerButton::Middle) {
        let d = response.drag_delta();
        if ctrl {
            let at = response
                .interact_pointer_pos()
                .map_or(rect.center().x, |p| p.x);
            axis.zoom(at, rect.left(), (1.0 + d.x * 0.01).max(0.1), (0.2, 400.0));
        } else {
            axis.pan(d.x);
        }
    }
    if response.hovered() {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 {
            if ctrl {
                axis.pan(wheel);
            } else if zoom_wheel {
                let at = response.hover_pos().map_or(rect.center().x, |p| p.x);
                axis.zoom(
                    at,
                    rect.left(),
                    (1.0 + wheel * 0.002_f32).clamp(0.5, 2.0),
                    (0.2, 400.0),
                );
            }
        }
    }
}

/// The frame ruler: ticks and numbers, the playback range, and the playhead
/// marker. `rect` is the strip itself; `body` the area the playhead line
/// extends over.
pub fn ruler(
    ui: &egui::Ui,
    painter: &Painter,
    rect: Rect,
    body: Rect,
    axis: &Axis,
    range: (i32, i32),
    current: i32,
) {
    let visuals = ui.visuals();
    let pal = theme::palette(ui.ctx());
    painter.rect_filled(rect, 0.0, visuals.faint_bg_color);
    let (x_start, x_end) = (
        axis.px(range.0 as f32, rect.left()),
        axis.px(range.1 as f32 + 1.0, rect.left()),
    );
    // Dim everything outside the playback range.
    let dim = pal.dim();
    let clip = painter.clip_rect();
    if x_start > clip.left() {
        painter.rect_filled(
            Rect::from_min_max(
                pos2(clip.left(), body.top()),
                pos2(x_start.min(clip.right()), body.bottom()),
            ),
            0.0,
            dim,
        );
    }
    if x_end < clip.right() {
        painter.rect_filled(
            Rect::from_min_max(
                pos2(x_end.max(clip.left()), body.top()),
                pos2(clip.right(), body.bottom()),
            ),
            0.0,
            dim,
        );
    }
    let step = nice_step(axis.scale, 56.0);
    let first = (axis.value(rect.left(), rect.left()) / step).floor() * step;
    let mut f = first;
    let grid = pal.ink(14);
    let text = visuals.weak_text_color();
    while axis.px(f, rect.left()) < rect.right() {
        let x = axis.px(f, rect.left());
        painter.line_segment(
            [pos2(x, rect.bottom() - 7.0), pos2(x, rect.bottom())],
            (1.0, text),
        );
        painter.line_segment(
            [pos2(x, rect.bottom()), pos2(x, body.bottom())],
            (1.0, grid),
        );
        painter.text(
            pos2(x + 3.0, rect.top() + 3.0),
            Align2::LEFT_TOP,
            format!("{}", f.round() as i32),
            FontId::monospace(10.0),
            text,
        );
        f += step;
    }
    // Playhead.
    let x = axis.px(current as f32, rect.left());
    if x >= rect.left() - 20.0 && x <= rect.right() + 20.0 {
        let label = current.to_string();
        let w = 8.0 + 6.5 * label.len() as f32;
        let tag = Rect::from_center_size(pos2(x, rect.center().y), vec2(w, RULER_H - 4.0));
        painter.line_segment(
            [pos2(x, rect.bottom()), pos2(x, body.bottom())],
            (1.5, PLAYHEAD),
        );
        painter.rect_filled(tag, 3.0, PLAYHEAD);
        painter.text(
            tag.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::monospace(11.0),
            Color32::WHITE,
        );
    }
}

/// A key diamond centred on `c`.
pub fn diamond(painter: &Painter, c: Pos2, r: f32, fill: Color32) {
    painter.add(Shape::convex_polygon(
        vec![
            c + vec2(0.0, -r),
            c + vec2(r, 0.0),
            c + vec2(0.0, r),
            c + vec2(-r, 0.0),
        ],
        fill,
        (1.0, Color32::from_black_alpha(200)),
    ));
}

/// Frame under pixel `x`, rounded to a whole frame (Blender's scrubbing).
pub fn scrub_frame(axis: &Axis, rect: Rect, x: f32) -> i32 {
    axis.value(x, rect.left()).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_zoom_keeps_the_anchor_and_pan_moves_the_view() {
        let mut a = Axis {
            origin: 0.0,
            scale: 10.0,
        };
        let anchor = a.value(300.0, 100.0);
        a.zoom(300.0, 100.0, 2.0, (0.2, 400.0));
        assert_eq!(a.scale, 20.0);
        assert!((a.value(300.0, 100.0) - anchor).abs() < 1e-4);
        let before = a.value(200.0, 100.0);
        a.pan(50.0);
        assert!((a.value(250.0, 100.0) - before).abs() < 1e-4);
        // Upward axes keep their sign.
        let mut up = Axis {
            origin: 0.0,
            scale: -10.0,
        };
        up.zoom(50.0, 100.0, 2.0, (0.2, 400.0));
        assert_eq!(up.scale, -20.0);
    }

    #[test]
    fn ruler_steps_are_nice() {
        assert_eq!(nice_step(10.0, 50.0), 5.0);
        assert_eq!(nice_step(4.0, 56.0), 20.0);
        assert_eq!(nice_step(100.0, 50.0), 0.5);
        assert_eq!(nice_step(1.0, 56.0), 100.0);
    }

    #[test]
    fn fit_covers_the_range() {
        let a = Axis::fit(1.0, 101.0, 600.0, 50.0, false);
        assert!((a.px(1.0, 0.0) - 50.0).abs() < 1e-3);
        assert!((a.px(101.0, 0.0) - 550.0).abs() < 1e-3);
        let up = Axis::fit(0.0, 10.0, 300.0, 20.0, true);
        assert!((up.px(0.0, 300.0) - 280.0).abs() < 1e-3);
        assert!((up.px(10.0, 300.0) - 20.0).abs() < 1e-3);
    }
}

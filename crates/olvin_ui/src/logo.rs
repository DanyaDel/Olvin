//! The Olvin logo painted from geometry, the same shapes as
//! `assets/logo/olvin-wordmark.svg` and `olvin-mark.svg` (STYLE.md §9).
//!
//! Units are the SVG's: cap height 68, stroke `W` = 12, gap = `W`. The ring
//! O, the green cursor (the "l" of the word), a geometric v, an i with a
//! square dot and an n arch; everything is `W` thick. Colours come from the
//! theme: `logo_ring` for the letters, `accent` for the cursor.

use std::f32::consts::PI;
use std::time::Duration;

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Vec2, vec2};

use crate::{Palette, palette, reduce_motion};

/// Cap height in SVG units: the O diameter, the cursor height.
pub const UNITS: f32 = 68.0;
/// Stroke width `W` in SVG units (the gap is the same).
pub const W: f32 = 12.0;
/// Word width in SVG units (O at 0, n ends at 240).
pub const WORD_UNITS: f32 = 240.0;
/// Mark width in SVG units: O, gap, cursor.
pub const MARK_UNITS: f32 = 92.0;
/// The intro needs room for the cursor after the n (240 + gap + W).
pub const INTRO_UNITS: f32 = WORD_UNITS + 2.0 * W;

/// The cursor's home: the "l" slot, x = 80..92.
const CURSOR_X: f32 = 80.0;

/// Letters other than the cursor, in typing order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Letter {
    O,
    V,
    I,
    N,
}

const LETTERS: [Letter; 4] = [Letter::O, Letter::V, Letter::I, Letter::N];

/// Size of [`wordmark`] at `height` px.
pub fn wordmark_size(height: f32) -> Vec2 {
    vec2(WORD_UNITS, UNITS) * (height / UNITS)
}

/// Size of [`mark`] at `size` px high.
pub fn mark_size(size: f32) -> Vec2 {
    vec2(MARK_UNITS, UNITS) * (size / UNITS)
}

/// Maps SVG units to screen points.
#[derive(Clone, Copy)]
struct Frame {
    min: Pos2,
    scale: f32,
}

impl Frame {
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.min + vec2(x, y) * self.scale
    }

    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(self.p(x0, y0), self.p(x1, y1))
    }
}

fn paint_letter(painter: &Painter, f: Frame, letter: Letter, color: Color32) {
    let w = W * f.scale;
    match letter {
        // Ring: outer radius 34, inner 22, so the stroke centre is at 28.
        Letter::O => {
            painter.circle_stroke(f.p(34.0, 34.0), 28.0 * f.scale, Stroke::new(w, color));
        }
        // Two arms, each a convex quad; their union is the SVG's V and the
        // overlap (the bottom triangle) is invisible with an opaque colour.
        Letter::V => {
            let arms = [
                [
                    (105.53, 26.0),
                    (118.47, 26.0),
                    (135.47, 68.0),
                    (122.53, 68.0),
                ],
                [
                    (139.53, 26.0),
                    (152.47, 26.0),
                    (135.47, 68.0),
                    (122.53, 68.0),
                ],
            ];
            for arm in arms {
                let pts = arm.iter().map(|&(x, y)| f.p(x, y)).collect();
                painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
            }
        }
        Letter::I => {
            painter.rect_filled(f.rect(166.0, 26.0, 178.0, 68.0), 0.0, color);
            painter.rect_filled(f.rect(166.0, 4.0, 178.0, 16.0), 0.0, color);
        }
        // One open polyline (legs + arch, centre radius 19), so no seams.
        Letter::N => {
            let (cx, cy, r) = (215.0, 51.0, 19.0);
            let segments = 24;
            let mut pts = vec![f.p(cx - r, 68.0)];
            pts.extend((0..=segments).map(|i| {
                let a = PI + PI * i as f32 / segments as f32;
                f.p(cx + r * a.cos(), cy + r * a.sin())
            }));
            pts.push(f.p(cx + r, 68.0));
            painter.add(Shape::line(pts, Stroke::new(w, color)));
        }
    }
}

/// Brightness of the cursor in the "off" half of the blink. The cursor is
/// also the "l", so it dims instead of vanishing: the word never reads "O vin".
const CURSOR_OFF: f32 = 0.3;

/// The cursor in its "off" phase: dimmed, no glow.
fn paint_cursor_off(painter: &Painter, f: Frame, x: f32, p: &Palette) {
    let rect = f.rect(x, 0.0, x + W, UNITS);
    painter.rect_filled(rect, 0.0, p.accent.gamma_multiply(CURSOR_OFF));
}

/// The cursor block at `x` (SVG units), with the STYLE.md glow when big
/// enough for a glow to read.
fn paint_cursor(painter: &Painter, f: Frame, x: f32, p: &Palette) {
    let rect = f.rect(x, 0.0, x + W, UNITS);
    if rect.height() >= 32.0 {
        let glow = egui::Shadow {
            offset: [0, 0],
            blur: 8,
            spread: 0,
            color: p.accent_glow,
        };
        painter.add(glow.as_shape(rect, 0));
    }
    painter.rect_filled(rect, 0.0, p.accent);
}

/// Paints the word at `min` (top-left of the cap box), `height` px high.
/// `letters` = how many of O, v, i, n are visible (4 = all); the cursor is
/// drawn at `cursor_x` SVG units when `Some`. With all letters and `None` the
/// "l" stays as a dimmed cursor (blink off phase). The building block of the
/// widgets below, for custom layouts (splash screens, buttons).
pub fn paint_wordmark(
    painter: &Painter,
    min: Pos2,
    height: f32,
    palette: &Palette,
    letters: usize,
    cursor_x: Option<f32>,
) {
    let f = Frame {
        min,
        scale: height / UNITS,
    };
    for letter in LETTERS.iter().take(letters) {
        paint_letter(painter, f, *letter, palette.logo_ring);
    }
    match cursor_x {
        Some(x) => paint_cursor(painter, f, x, palette),
        None if letters >= LETTERS.len() => paint_cursor_off(painter, f, CURSOR_X, palette),
        None => {}
    }
}

/// Paints the mark (O + cursor) at `min`, `size` px high; `cursor: false`
/// is the blink off phase (dimmed cursor).
pub fn paint_mark(painter: &Painter, min: Pos2, size: f32, palette: &Palette, cursor: bool) {
    let f = Frame {
        min,
        scale: size / UNITS,
    };
    paint_letter(painter, f, Letter::O, palette.logo_ring);
    if cursor {
        paint_cursor(painter, f, CURSOR_X, palette);
    } else {
        paint_cursor_off(painter, f, CURSOR_X, palette);
    }
}

/// Cursor blink: on for the first half of every second (CSS `step-end`
/// over 1 s). Schedules exactly one repaint at the next flip instead of
/// repainting continuously. Always on under reduce motion.
pub fn cursor_visible(ctx: &egui::Context) -> bool {
    if reduce_motion(ctx) {
        return true;
    }
    let phase = ctx.input(|i| i.time).rem_euclid(1.0);
    let next = if phase < 0.5 {
        0.5 - phase
    } else {
        1.0 - phase
    };
    ctx.request_repaint_after(Duration::from_secs_f64(next.max(0.001)));
    phase < 0.5
}

/// The Olvin wordmark, `height` px high (cap height; ~16–18 for a toolbar),
/// with a blinking cursor.
pub fn wordmark(ui: &mut egui::Ui, height: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(wordmark_size(height), Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = palette(ui.ctx());
        let cursor = cursor_visible(ui.ctx()).then_some(CURSOR_X);
        paint_wordmark(ui.painter(), rect.min, height, &p, LETTERS.len(), cursor);
    }
    response
}

/// The mark: the O and the blinking cursor, `size` px high.
pub fn mark(ui: &mut egui::Ui, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(mark_size(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = palette(ui.ctx());
        let cursor = cursor_visible(ui.ctx());
        paint_mark(ui.painter(), rect.min, size, &p, cursor);
    }
    response
}

/// Start delay before typing (STYLE.md §6).
const INTRO_DELAY: f32 = 0.5;
/// Per-letter typing times, within STYLE.md's 50–150 ms, uneven like a hand.
const TYPE_TIMES: [f32; 4] = [0.11, 0.07, 0.13, 0.09];
/// Pause with the cursor after the n before it glides home.
const INTRO_HOLD: f32 = 0.15;
/// The glide back into the "l" slot.
const INTRO_GLIDE: f32 = 0.3;

/// Where the cursor waits before typing letter `i` (and after the last one):
/// the next letter's cell, like a terminal.
const CURSOR_STOPS: [f32; 5] = [0.0, CURSOR_X, 166.0, 190.0, WORD_UNITS + W];

/// Seconds from `t0` until [`wordmark_intro`] settles into the plain blinking
/// wordmark.
pub fn intro_duration() -> f32 {
    INTRO_DELAY + TYPE_TIMES.iter().sum::<f32>() + INTRO_HOLD + INTRO_GLIDE
}

/// Intro state at `t` seconds after start: letters shown and cursor x.
fn intro_frame(t: f32) -> (usize, f32, bool) {
    if t < INTRO_DELAY {
        return (0, CURSOR_STOPS[0], false);
    }
    // Letter k appears when typing reaches it; the first lands right at the
    // end of the delay.
    let mut at = INTRO_DELAY;
    let mut shown = 0;
    for (k, dt) in TYPE_TIMES.iter().enumerate() {
        if t >= at {
            shown = k + 1;
        }
        at += dt;
    }
    if shown < LETTERS.len() || t < at + INTRO_HOLD {
        return (shown, CURSOR_STOPS[shown], false);
    }
    let glide = ((t - at - INTRO_HOLD) / INTRO_GLIDE).clamp(0.0, 1.0);
    let x = CURSOR_STOPS[4] + (CURSOR_X - CURSOR_STOPS[4]) * crate::anim::ease_out(glide);
    (LETTERS.len(), x, glide >= 1.0)
}

/// Launcher/splash intro: the cursor blinks for 500 ms, types O, v, i, n one
/// by one (50–150 ms each) moving along like a terminal cursor, then glides
/// back into its "l" slot and keeps blinking. `t0` is the start time from
/// `ctx.input(|i| i.time)`. Allocates [`INTRO_UNITS`] wide so the cursor fits
/// after the n. Under reduce motion: the finished wordmark, no blink.
pub fn wordmark_intro(ui: &mut egui::Ui, height: f32, t0: f64) -> Response {
    let size = vec2(INTRO_UNITS, UNITS) * (height / UNITS);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let ctx = ui.ctx();
    let p = palette(ctx);
    if reduce_motion(ctx) {
        paint_wordmark(ui.painter(), rect.min, height, &p, 4, Some(CURSOR_X));
        return response;
    }
    let t = (ctx.input(|i| i.time) - t0).max(0.0) as f32;
    let (letters, x, done) = intro_frame(t);
    let typing = letters > 0 && !done;
    if !done {
        ctx.request_repaint();
    }
    // The cursor stays solid while it moves, blinks while it waits.
    let visible = typing || cursor_visible(ctx);
    paint_wordmark(
        ui.painter(),
        rect.min,
        height,
        &p,
        letters,
        visible.then_some(x),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_frame as run;

    #[test]
    fn wordmark_and_mark_paint_headless() {
        for kind in crate::ThemeKind::ALL {
            let ctx = egui::Context::default();
            crate::apply(&ctx, kind);
            for t in [0.0, 0.3, 0.7, 1.0, 1.4, 3.0] {
                let out = run(&ctx, t, |ui| {
                    wordmark_intro(ui, 48.0, 0.0);
                });
                assert!(t < f64::from(INTRO_DELAY) || !out.shapes.is_empty());
            }
            // Settled frames: the blink schedules a repaint instead of spinning.
            for t in [3.1, 3.2] {
                let out = run(&ctx, t, |ui| {
                    let r = wordmark(ui, 18.0);
                    assert!((r.rect.size() - wordmark_size(18.0)).length() < 0.01);
                    mark(ui, 32.0);
                });
                assert!(!out.shapes.is_empty());
                let repaint = out.viewport_output.values().next().unwrap().repaint_delay;
                assert!(repaint > Duration::from_millis(100), "{repaint:?}");
            }
        }
    }

    #[test]
    fn intro_types_then_settles_home() {
        assert_eq!(intro_frame(0.2), (0, 0.0, false));
        assert_eq!(intro_frame(INTRO_DELAY).0, 1);
        let mid = intro_frame(INTRO_DELAY + 0.2);
        assert!(mid.0 >= 2 && mid.0 < 4, "{mid:?}");
        let (letters, x, done) = intro_frame(intro_duration() + 0.01);
        assert_eq!((letters, done), (4, true));
        assert!((x - CURSOR_X).abs() < 1e-3);
    }

    #[test]
    fn blink_follows_step_end() {
        let ctx = egui::Context::default();
        let mut seen = Vec::new();
        for t in [0.1, 0.6, 1.2, 1.7] {
            run(&ctx, t, |ui| seen.push(cursor_visible(ui.ctx())));
        }
        assert_eq!(seen, [true, false, true, false]);
        crate::set_reduce_motion(&ctx, true);
        run(&ctx, 0.6, |ui| assert!(cursor_visible(ui.ctx())));
    }
}

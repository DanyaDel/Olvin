//! Chips that never wrap and the status dot.

use eframe::egui::{
    self, Color32, Galley, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
    pos2, vec2,
};
use std::sync::Arc;

use super::*;
use crate::ui::{FS_SM, Palette, RADIUS_PILL};

const CHIP_H: f32 = 20.0;
const CHIP_PAD: f32 = 8.0;
const CHIP_GAP: f32 = 6.0;

fn chip_galley(painter: &Painter, text: &str, color: Color32) -> Arc<Galley> {
    painter.layout_no_wrap(text.to_owned(), sans(FS_SM - 1.0), color)
}

/// A pill tag at `min` (top-left); returns its width.
pub fn paint_chip(painter: &Painter, min: Pos2, text: &str, color: Color32, fill: bool) -> f32 {
    let g = chip_galley(painter, text, color);
    let rect = Rect::from_min_size(min, vec2(g.size().x + 2.0 * CHIP_PAD, CHIP_H));
    let bg = if fill {
        color.gamma_multiply(0.14)
    } else {
        Color32::TRANSPARENT
    };
    painter.rect(
        rect,
        RADIUS_PILL,
        bg,
        Stroke::new(1.0, color.gamma_multiply(0.45)),
        StrokeKind::Inside,
    );
    painter.galley(
        pos2(rect.min.x + CHIP_PAD, rect.center().y - g.size().y / 2.0),
        g,
        color,
    );
    rect.width()
}

pub fn chip_width(painter: &Painter, text: &str) -> f32 {
    chip_galley(painter, text, Color32::PLACEHOLDER).size().x + 2.0 * CHIP_PAD
}

/// Chips on one line inside `rect` (vertically centred). What doesn't fit
/// collapses into a `+N` chip; nothing ever wraps. Returns the used width.
pub fn chip_line(painter: &Painter, rect: Rect, items: &[String], p: &Palette) -> f32 {
    let y = rect.center().y - CHIP_H / 2.0;
    let widths: Vec<f32> = items.iter().map(|t| chip_width(painter, t)).collect();
    let mut shown = 0;
    let mut used = 0.0;
    for (i, w) in widths.iter().enumerate() {
        let rest = items.len() - i - 1;
        let more = if rest > 0 {
            CHIP_GAP + chip_width(painter, &format!("+{rest}"))
        } else {
            0.0
        };
        let next = used + if shown > 0 { CHIP_GAP } else { 0.0 } + w;
        if next + more > rect.width() {
            break;
        }
        used = next;
        shown += 1;
    }
    let mut x = rect.min.x;
    for item in &items[..shown] {
        x += paint_chip(painter, pos2(x, y), item, p.text_2, false) + CHIP_GAP;
    }
    let hidden = items.len() - shown;
    if hidden > 0 {
        let label = format!("+{hidden}");
        if x + chip_width(painter, &label) <= rect.max.x + 0.5 {
            x += paint_chip(painter, pos2(x, y), &label, p.muted, true) + CHIP_GAP;
        }
    }
    (x - rect.min.x - CHIP_GAP).max(0.0)
}

/// Status dot (6 px) with an `--fs-xs` caps label.
pub fn status(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let g = caps_galley(ui.painter(), text, color);
    let (rect, response) = ui.allocate_exact_size(
        vec2(14.0 + g.size().x, g.size().y.max(12.0)),
        Sense::hover(),
    );
    status_dot(ui.painter(), pos2(rect.min.x + 3.0, rect.center().y), color);
    ui.painter().galley(
        pos2(rect.min.x + 14.0, rect.center().y - g.size().y / 2.0),
        g,
        color,
    );
    response
}

pub fn status_dot(painter: &Painter, center: Pos2, color: Color32) {
    let glow = egui::Shadow {
        offset: [0, 0],
        blur: 8,
        spread: 0,
        color: color.gamma_multiply(0.45),
    };
    painter.add(glow.as_shape(Rect::from_center_size(center, Vec2::splat(6.0)), 3));
    painter.circle_filled(center, 3.0, color);
}

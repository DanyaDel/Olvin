//! Pill switches, toggles and the card background.

use eframe::egui::{
    self, Color32, CursorIcon, Id, Painter, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
    pos2, vec2,
};

use super::*;
use crate::ui::{self, FS_SM, Palette, RADIUS_PILL, anim};

/// A pill switch (STYLE.md §6: active segment white with dark text). The
/// highlight slides between segments. Returns true when the value changed.
pub fn pill_switch<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: Id,
    value: &mut T,
    options: &[(T, Option<Icon>, &str)],
) -> bool {
    let ctx = ui.ctx().clone();
    let p = ui::palette(&ctx);
    let seg_w: Vec<f32> = options
        .iter()
        .map(|(_, icon, text)| {
            let t = if text.is_empty() {
                0.0
            } else {
                ui.painter()
                    .layout_no_wrap(text.to_string(), sans(FS_SM), Color32::PLACEHOLDER)
                    .size()
                    .x
            };
            let i = if icon.is_some() { 16.0 } else { 0.0 };
            let gap = if icon.is_some() && !text.is_empty() {
                6.0
            } else {
                0.0
            };
            (t + i + gap + 24.0).max(36.0)
        })
        .collect();
    let total: f32 = seg_w.iter().sum::<f32>() + 6.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 34.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect(
        rect,
        RADIUS_PILL,
        p.glass,
        Stroke::new(1.0, p.line),
        StrokeKind::Inside,
    );

    let mut changed = false;
    let mut x = rect.min.x + 3.0;
    let mut segs = Vec::new();
    for (i, w) in seg_w.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.min.y + 3.0), vec2(*w, rect.height() - 6.0));
        let resp = ui
            .interact(r, id.with(i), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let (opt, _, text) = options[i];
        // Tooltips only for icon-only segments.
        let resp = if text.is_empty() {
            resp.on_hover_text(match options[i].1 {
                Some(Icon::Grid) => "Grid",
                Some(Icon::List) => "List",
                _ => "",
            })
        } else {
            resp
        };
        if resp.clicked() && *value != opt {
            *value = opt;
            changed = true;
        }
        segs.push((r, resp.hovered()));
        x += w;
    }
    let sel = options.iter().position(|(o, _, _)| o == value).unwrap_or(0);
    let left = tween(&ctx, id.with("x"), segs[sel].0.min.x, 0.3);
    let width = tween(&ctx, id.with("w"), segs[sel].0.width(), 0.3);
    let hi = Rect::from_min_size(
        pos2(left, segs[sel].0.min.y),
        vec2(width, segs[sel].0.height()),
    );
    painter.rect_filled(hi, RADIUS_PILL, p.text);
    for (i, (r, hovered)) in segs.iter().enumerate() {
        // Text colour follows the moving highlight so it stays readable.
        let cover = (r.intersect(hi).width().max(0.0) / r.width()).clamp(0.0, 1.0);
        let idle = if *hovered { p.text } else { p.muted };
        let color = anim::lerp_color(idle, p.bg, cover);
        let (_, icon, text) = options[i];
        let g = (!text.is_empty())
            .then(|| painter.layout_no_wrap(text.to_string(), sans(FS_SM), color));
        let content = g.as_ref().map_or(0.0, |g| g.size().x)
            + if icon.is_some() { 16.0 } else { 0.0 }
            + if icon.is_some() && g.is_some() {
                6.0
            } else {
                0.0
            };
        let mut cx = r.center().x - content / 2.0;
        if let Some(icon) = icon {
            paint_icon(
                &painter,
                Rect::from_center_size(pos2(cx + 8.0, r.center().y), Vec2::splat(15.0)),
                icon,
                color,
            );
            cx += 22.0;
        }
        if let Some(g) = g {
            painter.galley(pos2(cx, r.center().y - g.size().y / 2.0), g, color);
        }
    }
    changed
}

/// An on/off switch, 40×22, the knob slides (colour fades with reduce motion).
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let ctx = ui.ctx().clone();
    let p = ui::palette(&ctx);
    let (rect, mut response) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ctx.animate_bool_with_time(response.id, *on, 0.2);
    let knob_t = if ui::reduce_motion(&ctx) {
        if *on { 1.0 } else { 0.0 }
    } else {
        anim::ease_out(t)
    };
    let painter = ui.painter();
    let track = anim::lerp_color(p.ghost, p.accent, t);
    painter.rect_filled(rect, RADIUS_PILL, track);
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            RADIUS_PILL,
            Stroke::new(2.0, p.accent),
            StrokeKind::Outside,
        );
    }
    let x = egui::lerp(rect.min.x + 11.0..=rect.max.x - 11.0, knob_t);
    painter.circle_filled(pos2(x, rect.center().y), 8.0, Color32::WHITE);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// Paints a card background with the launcher hover: lighter fill, stronger
/// shadow (STYLE.md §5/§6). `hover` is 0..1.
pub fn card_bg(painter: &Painter, rect: Rect, radius: u8, hover: f32, selected: bool, p: &Palette) {
    let dark = p.dark;
    let shadow = egui::Shadow {
        offset: [0, (6.0 + 19.0 * hover) as i8],
        blur: (24.0 + 36.0 * hover) as u8,
        spread: 0,
        color: Color32::from_black_alpha(if dark {
            (60.0 + 93.0 * hover) as u8
        } else {
            (8.0 + 30.0 * hover) as u8
        }),
    };
    painter.add(shadow.as_shape(rect, radius));
    let base = if dark {
        Color32::from_rgba_unmultiplied(30, 30, 30, 77)
    } else {
        p.surface
    };
    let fill = anim::lerp_color(
        base,
        if dark {
            Color32::from_rgba_unmultiplied(48, 48, 50, 140)
        } else {
            p.surface
        },
        hover,
    );
    // Opaque underlay so the gradient behind doesn't make text muddy.
    painter.rect_filled(rect, radius, p.bg.gamma_multiply(0.6));
    painter.rect_filled(rect, radius, fill);
    let border = if selected {
        p.accent_border
    } else {
        anim::lerp_color(p.line_soft, p.line_top, hover)
    };
    painter.rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    // Light top edge of glass surfaces.
    if dark {
        let top = Rect::from_min_max(
            rect.min + vec2(radius as f32, 0.0),
            pos2(rect.max.x - radius as f32, rect.min.y + 1.0),
        );
        painter.rect_filled(top, 0.0, p.line.gamma_multiply(0.6 + 0.4 * hover));
    }
}

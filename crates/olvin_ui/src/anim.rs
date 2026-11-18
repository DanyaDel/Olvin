//! Motion from STYLE.md §7: three easing curves, a few durations and small
//! helpers over egui's animation manager. Every helper returns its end value
//! at once when [`crate::reduce_motion`] is on.
//!
//! Editor rules: nothing jumps or resizes on hover, and things that change
//! constantly (dragging, inspector input, the timeline) are never animated.

use std::hash::Hash;

use egui::{Color32, Id, RichText, Stroke};

use crate::reduce_motion;

/// Hover and colour changes (STYLE.md: 200–300 ms).
pub const HOVER: f32 = 0.2;
/// Button press: never longer than 120 ms.
pub const PRESS: f32 = 0.12;
/// Panels and menus sliding or fading in (300–400 ms).
pub const PANEL: f32 = 0.32;
/// Launcher cards and the slider: up to 500 ms.
pub const LAUNCHER: f32 = 0.45;
/// Delay between neighbouring items in [`stagger`].
pub const STAGGER_STEP: f32 = 0.04;

/// CSS `cubic-bezier(x1, y1, x2, y2)` evaluated at progress `x` (0..=1).
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    // B(s) = ((a s + b) s + c) s for each axis, P0 = 0 and P3 = 1.
    let coeffs = |p1: f32, p2: f32| {
        let c = 3.0 * p1;
        let b = 3.0 * (p2 - p1) - c;
        (1.0 - c - b, b, c)
    };
    let (ax, bx, cx) = coeffs(x1, x2);
    let (ay, by, cy) = coeffs(y1, y2);
    let curve_x = |s: f32| ((ax * s + bx) * s + cx) * s;
    let slope_x = |s: f32| (3.0 * ax * s + 2.0 * bx) * s + cx;

    // Newton from s = x, then bisection if the slope goes flat.
    let mut s = x;
    for _ in 0..8 {
        let err = curve_x(s) - x;
        if err.abs() < 1e-5 {
            return ((ay * s + by) * s + cy) * s;
        }
        let d = slope_x(s);
        if d.abs() < 1e-6 {
            break;
        }
        s -= err / d;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    s = x;
    for _ in 0..32 {
        let v = curve_x(s);
        if (v - x).abs() < 1e-5 {
            break;
        }
        if v < x {
            lo = s;
        } else {
            hi = s;
        }
        s = (lo + hi) * 0.5;
    }
    ((ay * s + by) * s + cy) * s
}

/// `--ease-out`: panels sliding out, things appearing.
pub fn ease_out(t: f32) -> f32 {
    cubic_bezier(0.16, 1.0, 0.3, 1.0, t)
}

/// `--ease-pop`: popup menus, with a slight overshoot past 1.
pub fn ease_pop(t: f32) -> f32 {
    cubic_bezier(0.175, 0.885, 0.32, 1.275, t)
}

/// `--ease-std`: hover, colour changes.
pub fn ease_std(t: f32) -> f32 {
    cubic_bezier(0.25, 0.8, 0.25, 1.0, t)
}

/// 0 → 1 while `on` and back, over `seconds` with `easing`. The general form
/// of [`fade`] and [`hover_t`].
pub fn toward(ctx: &egui::Context, id: Id, on: bool, seconds: f32, easing: fn(f32) -> f32) -> f32 {
    // A near-zero time still advances egui's state, so turning reduce
    // motion off later doesn't replay a stale animation.
    let seconds = if reduce_motion(ctx) { 1e-6 } else { seconds };
    ctx.animate_bool_with_time_and_easing(id, on, seconds, easing)
}

/// Opacity for something shown while `on`: ease-out over 250 ms.
pub fn fade(ctx: &egui::Context, id: Id, on: bool) -> f32 {
    toward(ctx, id, on, 0.25, ease_out)
}

/// Hover progress for colour fades: ease-std over [`HOVER`].
pub fn hover_t(ctx: &egui::Context, id: Id, hovered: bool) -> f32 {
    toward(ctx, id, hovered, HOVER, ease_std)
}

/// `animate_value_with_time` over [`HOVER`] (linear), instant under reduce motion.
pub fn value(ctx: &egui::Context, id: Id, target: f32) -> f32 {
    let seconds = if reduce_motion(ctx) { 0.0 } else { HOVER };
    ctx.animate_value_with_time(id, target, seconds)
}

/// Gamma-space blend from `a` (t = 0) to `b` (t = 1), alpha included.
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

fn now(ctx: &egui::Context) -> f64 {
    ctx.input(|i| i.time)
}

/// Seconds since `id`'s clock started; the first call starts it.
fn clock(ctx: &egui::Context, id: Id) -> f32 {
    let t = now(ctx);
    let start = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || t));
    (t - start) as f32
}

/// List items appearing one after another: item `index` fades in over
/// [`PANEL`] with ease-out, [`STAGGER_STEP`] after the previous one. All
/// items of one list share `id`; [`restart`] replays it.
pub fn stagger(ctx: &egui::Context, id: Id, index: usize) -> f32 {
    if reduce_motion(ctx) {
        return 1.0;
    }
    let local = clock(ctx, id) - index as f32 * STAGGER_STEP;
    let t = (local / PANEL).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    ease_out(t)
}

/// Restarts the clock of [`stagger`] / [`appear`] for `id`.
pub fn restart(ctx: &egui::Context, id: Id) {
    let t = now(ctx);
    ctx.data_mut(|d| d.insert_temp(id, t));
}

/// 0 → 1 (ease-out, 250 ms) every time `key` changes, e.g. a new status
/// message fading in.
pub fn appear(ctx: &egui::Context, id: Id, key: impl Hash) -> f32 {
    if reduce_motion(ctx) {
        return 1.0;
    }
    let key = {
        use std::hash::Hasher;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        h.finish()
    };
    let t = now(ctx);
    let start = ctx.data_mut(|d| {
        let slot = d.get_temp_mut_or_insert_with(id, || (key, t));
        if slot.0 != key {
            *slot = (key, t);
        }
        slot.1
    });
    let p = (((t - start) as f32) / 0.25).clamp(0.0, 1.0);
    if p < 1.0 {
        ctx.request_repaint();
    }
    ease_out(p)
}

/// A slow breathing 0..1 wave with `period` seconds, for "running" status
/// dots. Constant 1 under reduce motion. Repaints at ~30 fps while used.
pub fn pulse(ctx: &egui::Context, period: f32) -> f32 {
    if reduce_motion(ctx) {
        return 1.0;
    }
    let phase = (now(ctx) as f32 / period.max(0.1)).fract();
    ctx.request_repaint_after(std::time::Duration::from_millis(33));
    0.5 - 0.5 * (phase * std::f32::consts::TAU).cos()
}

/// Colours of a selectable button at its current hover/selection progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonColors {
    pub fill: Color32,
    pub stroke: Stroke,
    pub text: Color32,
    /// Selection progress, 0..1 (for callers that tint more than the button).
    pub selected_t: f32,
}

/// Colours for the widget `ui` adds next: a frameless selectable button whose
/// hover fill, border and text fade in (ease-std, [`HOVER`]), selection fades
/// to the accent, and a press switches instantly. Reads last frame's hover
/// state, like egui's own buttons. Also counts an open popup/menu of that
/// button as hovered.
pub fn button_colors(ui: &egui::Ui, selected: bool) -> ButtonColors {
    let ctx = ui.ctx();
    let id = ui.next_auto_id();
    let response = ctx.read_response(id);
    let enabled = ui.is_enabled() && response.as_ref().is_none_or(|r| r.enabled());
    let open = egui::Popup::is_id_open(ctx, id.with("popup"));
    let hovered = enabled && (response.as_ref().is_some_and(|r| r.hovered()) || open);
    let pressed = enabled
        && response
            .as_ref()
            .is_some_and(|r| r.is_pointer_button_down_on());

    let w = &ui.visuals().widgets;
    let sel = &ui.visuals().selection;
    let h = hover_t(ctx, id.with("olvin-hover"), hovered);
    let s = toward(ctx, id.with("olvin-selected"), selected, HOVER, ease_std);

    let idle_text = w.inactive.fg_stroke.color;
    let mut fill = lerp_color(Color32::TRANSPARENT, w.hovered.weak_bg_fill, h);
    fill = lerp_color(fill, sel.bg_fill, s);
    let mut stroke = Stroke::new(
        w.hovered.bg_stroke.width,
        lerp_color(Color32::TRANSPARENT, w.hovered.bg_stroke.color, h),
    );
    let mut text = lerp_color(idle_text, w.hovered.fg_stroke.color, h);
    text = lerp_color(text, sel.stroke.color, s);
    if pressed && !selected {
        fill = w.active.weak_bg_fill;
        stroke = w.active.bg_stroke;
    }
    ButtonColors {
        fill,
        stroke,
        text,
        selected_t: s,
    }
}

/// A selectable button with animated colours (see [`button_colors`]). Add it
/// right away (`ui.add(...)`, `MenuButton::from_button(...)`) so its id
/// matches.
pub fn button(ui: &egui::Ui, text: impl Into<RichText>, selected: bool) -> egui::Button<'static> {
    let c = button_colors(ui, selected);
    egui::Button::new(text.into().color(c.text))
        .fill(c.fill)
        .stroke(c.stroke)
        .frame_when_inactive(true)
        .selected(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_hit_their_ends_and_shape() {
        for f in [ease_out, ease_pop, ease_std] {
            assert_eq!(f(0.0), 0.0);
            assert_eq!(f(1.0), 1.0);
        }
        // ease-out is fast at the start.
        assert!(ease_out(0.2) > 0.6);
        // ease-pop overshoots.
        assert!((0..50).any(|i| ease_pop(0.5 + i as f32 / 100.0) > 1.0));
        // A linear bezier is the identity.
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            assert!((cubic_bezier(0.0, 0.0, 1.0, 1.0, x) - x).abs() < 1e-3);
        }
    }

    #[test]
    fn reduce_motion_is_instant() {
        let ctx = egui::Context::default();
        crate::set_reduce_motion(&ctx, true);
        let mut out = (0.0, 0.0, 0.0);
        crate::test_frame(&ctx, 0.0, |ui| {
            let id = Id::new("x");
            fade(ui.ctx(), id, false);
            out = (
                fade(ui.ctx(), id, true),
                stagger(ui.ctx(), Id::new("list"), 9),
                appear(ui.ctx(), Id::new("msg"), 1),
            );
        });
        assert_eq!(out, (1.0, 1.0, 1.0));
    }

    #[test]
    fn stagger_orders_items() {
        let ctx = egui::Context::default();
        let mut vals = Vec::new();
        for frame in 0..3 {
            crate::test_frame(&ctx, frame as f64 * 0.1, |ui| {
                vals = (0..4).map(|i| stagger(ui.ctx(), Id::new("l"), i)).collect();
            });
        }
        assert!(vals.windows(2).all(|w| w[0] >= w[1]), "{vals:?}");
        assert!(vals[0] > 0.0);
    }
}

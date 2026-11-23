//! Tweens and staggered entrances (instant with reduce motion).

use eframe::egui::{self, Id};

use crate::ui::{self, anim};

#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    start: f64,
}

/// Moves towards `target` over `secs` with `--ease-out`, restarting from the
/// current value whenever the target changes. Instant with reduce motion.
pub fn tween(ctx: &egui::Context, id: Id, target: f32, secs: f32) -> f32 {
    let now = ctx.input(|i| i.time);
    if ui::reduce_motion(ctx) {
        ctx.data_mut(|d| {
            d.insert_temp(
                id,
                Tween {
                    from: target,
                    to: target,
                    start: now,
                },
            )
        });
        return target;
    }
    let value_at = |t: &Tween, now: f64| {
        let k = ((now - t.start) as f32 / secs).clamp(0.0, 1.0);
        t.from + (t.to - t.from) * anim::ease_out(k)
    };
    let state = ctx.data(|d| d.get_temp::<Tween>(id));
    let state = match state {
        None => Tween {
            from: target,
            to: target,
            start: now,
        },
        Some(t) if (t.to - target).abs() > 0.01 => Tween {
            from: value_at(&t, now),
            to: target,
            start: now,
        },
        Some(t) => t,
    };
    ctx.data_mut(|d| d.insert_temp(id, state));
    if now - state.start < secs as f64 {
        ctx.request_repaint();
    }
    value_at(&state, now)
}

/// Staggered entrance of item `n` on a page opened at `page_at`: every page
/// visit gets its own stagger clock, so items replay their entrance.
pub fn entrance(ctx: &egui::Context, page_at: f64, n: usize) -> f32 {
    anim::stagger(ctx, Id::new(("entrance", page_at.to_bits())), n.min(12))
}

/// Progress 0..1 of something that began at `start`, eased; repaints until done.
pub fn since(ctx: &egui::Context, start: f64, secs: f32, ease: fn(f32) -> f32) -> f32 {
    if ui::reduce_motion(ctx) {
        return 1.0;
    }
    let t = ((ctx.input(|i| i.time) - start) as f32 / secs).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    ease(t)
}

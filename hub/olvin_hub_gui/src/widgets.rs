//! Launcher widgets painted by hand: buttons with the press scale, chips
//! that never wrap, pill switches, toggles and line icons (STYLE.md §6, §8).

use eframe::egui::{
    self, Color32, CursorIcon, FontFamily, FontId, Galley, Id, Painter, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, Ui, Vec2, Widget, pos2, text::LayoutJob, vec2,
};
use std::sync::Arc;

use crate::ui::{self, FS_MD, FS_SM, FS_XS, Palette, RADIUS_PILL, RADIUS_SM, anim};

// --- icons ------------------------------------------------------------------------

/// Line icons, 1.5 px stroke, drawn in a 20×20 grid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Folder,
    FolderPlus,
    Pencil,
    Gear,
    Plus,
    Play,
    More,
    Search,
    Grid,
    List,
    Close,
    Check,
    Refresh,
    Trash,
    Package,
    Stop,
}

pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 20.0;
    let o = rect.center() - vec2(10.0, 10.0) * s;
    let p = |x: f32, y: f32| pos2(o.x + x * s, o.y + y * s);
    let stroke = Stroke::new(1.5 * s.max(0.8), color);
    let line = |pts: &[(f32, f32)]| {
        painter.add(Shape::line(
            pts.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    let closed = |pts: &[(f32, f32)]| {
        painter.add(Shape::closed_line(
            pts.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    match icon {
        Icon::Folder => closed(&[
            (2.5, 5.0),
            (7.5, 5.0),
            (9.5, 7.0),
            (17.5, 7.0),
            (17.5, 16.0),
            (2.5, 16.0),
        ]),
        Icon::FolderPlus => {
            closed(&[
                (2.5, 5.0),
                (7.5, 5.0),
                (9.5, 7.0),
                (17.5, 7.0),
                (17.5, 16.0),
                (2.5, 16.0),
            ]);
            line(&[(10.0, 9.0), (10.0, 14.0)]);
            line(&[(7.5, 11.5), (12.5, 11.5)]);
        }
        Icon::Pencil => {
            closed(&[
                (13.0, 3.5),
                (16.5, 7.0),
                (7.0, 16.5),
                (3.5, 16.5),
                (3.5, 13.0),
            ]);
            line(&[(11.0, 5.5), (14.5, 9.0)]);
        }
        Icon::Gear => {
            painter.circle_stroke(p(10.0, 10.0), 2.6 * s, stroke);
            painter.circle_stroke(p(10.0, 10.0), 5.6 * s, stroke);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let (sin, cos) = a.sin_cos();
                line(&[
                    (10.0 + 5.6 * cos, 10.0 + 5.6 * sin),
                    (10.0 + 8.0 * cos, 10.0 + 8.0 * sin),
                ]);
            }
        }
        Icon::Plus => {
            line(&[(10.0, 4.0), (10.0, 16.0)]);
            line(&[(4.0, 10.0), (16.0, 10.0)]);
        }
        Icon::Play => closed(&[(6.0, 4.0), (16.0, 10.0), (6.0, 16.0)]),
        Icon::Stop => closed(&[(5.0, 5.0), (15.0, 5.0), (15.0, 15.0), (5.0, 15.0)]),
        Icon::More => {
            for x in [4.5, 10.0, 15.5] {
                painter.circle_filled(p(x, 10.0), 1.4 * s, color);
            }
        }
        Icon::Search => {
            painter.circle_stroke(p(8.5, 8.5), 5.0 * s, stroke);
            line(&[(12.2, 12.2), (16.5, 16.5)]);
        }
        Icon::Grid => {
            for (x, y) in [(3.5, 3.5), (11.0, 3.5), (3.5, 11.0), (11.0, 11.0)] {
                closed(&[(x, y), (x + 5.5, y), (x + 5.5, y + 5.5), (x, y + 5.5)]);
            }
        }
        Icon::List => {
            for y in [5.0, 10.0, 15.0] {
                painter.circle_filled(p(3.5, y), 1.1 * s, color);
                line(&[(7.0, y), (17.0, y)]);
            }
        }
        Icon::Close => {
            line(&[(5.0, 5.0), (15.0, 15.0)]);
            line(&[(15.0, 5.0), (5.0, 15.0)]);
        }
        Icon::Check => line(&[(4.0, 10.5), (8.0, 14.5), (16.0, 5.5)]),
        Icon::Refresh => {
            let arc: Vec<(f32, f32)> = (0..=18)
                .map(|i| {
                    let a = -0.6 + i as f32 / 18.0 * 4.9;
                    (10.0 + 6.0 * a.cos(), 10.0 + 6.0 * a.sin())
                })
                .collect();
            line(&arc);
            line(&[(16.8, 3.0), (15.0, 5.2), (12.4, 4.4)]);
        }
        Icon::Trash => {
            line(&[(3.5, 5.5), (16.5, 5.5)]);
            line(&[(8.0, 5.5), (8.0, 3.5), (12.0, 3.5), (12.0, 5.5)]);
            closed(&[(5.0, 5.5), (15.0, 5.5), (14.0, 17.0), (6.0, 17.0)]);
        }
        Icon::Package => {
            closed(&[
                (10.0, 2.5),
                (17.0, 6.0),
                (17.0, 14.0),
                (10.0, 17.5),
                (3.0, 14.0),
                (3.0, 6.0),
            ]);
            line(&[(3.0, 6.0), (10.0, 9.5), (17.0, 6.0)]);
            line(&[(10.0, 9.5), (10.0, 17.5)]);
        }
    }
}

// --- text -------------------------------------------------------------------------

/// One line of text, cut with `…` at `max_width`.
pub fn truncated(
    painter: &Painter,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(8.0));
    painter.layout_job(job)
}

/// `--fs-xs` caps label as a galley (for painted headers).
pub fn caps_galley(painter: &Painter, text: &str, color: Color32) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        egui::TextFormat {
            font_id: FontId::new(FS_XS, ui::bold()),
            color,
            extra_letter_spacing: FS_XS * 0.06,
            ..Default::default()
        },
    );
    painter.layout_job(job)
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn sans(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, ui::bold())
}

// --- buttons ----------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Accent fill, one per screen.
    Primary,
    /// Glass fill with a line border.
    Secondary,
    /// No border until hovered: icon buttons inside rows.
    Ghost,
}

/// A launcher button: 32 px tall, scale 0.98 while pressed (STYLE.md §6).
pub struct Btn<'a> {
    text: &'a str,
    icon: Option<Icon>,
    kind: Kind,
    height: f32,
    min_width: f32,
}

impl<'a> Btn<'a> {
    pub fn new(kind: Kind, text: &'a str) -> Self {
        Self {
            text,
            icon: None,
            kind,
            height: 34.0,
            min_width: 0.0,
        }
    }

    pub fn primary(text: &'a str) -> Self {
        Self::new(Kind::Primary, text)
    }

    pub fn secondary(text: &'a str) -> Self {
        Self::new(Kind::Secondary, text)
    }

    /// A square icon-only button.
    pub fn icon_only(icon: Icon) -> Self {
        Self {
            text: "",
            icon: Some(icon),
            kind: Kind::Ghost,
            height: 32.0,
            min_width: 32.0,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn small(mut self) -> Self {
        self.height = 30.0;
        self
    }

    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
}

impl Widget for Btn<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = ui::palette(&ctx);
        let enabled = ui.is_enabled();
        let primary = self.kind == Kind::Primary;
        let label = if primary {
            self.text.to_uppercase()
        } else {
            self.text.to_owned()
        };
        let font = if primary { bold(FS_MD) } else { sans(FS_MD) };
        let galley = ui
            .painter()
            .layout_no_wrap(label, font, Color32::PLACEHOLDER);
        let icon_w = if self.icon.is_some() { 16.0 } else { 0.0 };
        let gap = if self.icon.is_some() && !self.text.is_empty() {
            7.0
        } else {
            0.0
        };
        let pad = if self.text.is_empty() { 8.0 } else { 14.0 };
        let width = (galley.size().x + icon_w + gap + 2.0 * pad).max(self.min_width);
        let (rect, response) = ui.allocate_exact_size(vec2(width, self.height), Sense::click());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let hover = anim::hover_t(
            &ctx,
            response.id.with("hover"),
            response.hovered() && enabled,
        );
        let down = response.is_pointer_button_down_on() && enabled;
        let press = if ui::reduce_motion(&ctx) {
            0.0
        } else {
            ctx.animate_bool_with_time(response.id.with("press"), down, 0.08)
        };
        let r = rect.scale_from_center(1.0 - 0.02 * press);

        let lerp = anim::lerp_color;
        let (fill, border, text) = match self.kind {
            Kind::Primary => (
                lerp(p.accent, lerp(p.accent, p.text, 0.25), hover * 0.6),
                Stroke::NONE,
                p.on_accent,
            ),
            Kind::Secondary => (
                lerp(p.glass, p.line, hover),
                Stroke::new(1.0, lerp(p.line, p.accent_border, hover)),
                lerp(p.text_2, p.text, hover),
            ),
            Kind::Ghost => (
                lerp(Color32::TRANSPARENT, p.line, hover),
                Stroke::new(1.0, lerp(Color32::TRANSPARENT, p.line_soft, hover)),
                lerp(p.muted, p.text, hover),
            ),
        };
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.4) };
        let painter = ui.painter();
        if primary && enabled && hover > 0.0 {
            let glow = egui::Shadow {
                offset: [0, 0],
                blur: 15,
                spread: 0,
                color: p.accent_glow.gamma_multiply(hover),
            };
            painter.add(glow.as_shape(r, RADIUS_SM));
        }
        painter.rect(
            r,
            RADIUS_SM,
            fade(fill),
            Stroke::new(border.width, fade(border.color)),
            StrokeKind::Inside,
        );
        if response.has_focus() {
            painter.rect_stroke(
                r.expand(2.0),
                RADIUS_SM + 2,
                Stroke::new(2.0, p.accent),
                StrokeKind::Outside,
            );
        }
        let content_w = icon_w + gap + galley.size().x;
        let mut x = r.center().x - content_w / 2.0;
        if let Some(icon) = self.icon {
            let ir = Rect::from_center_size(pos2(x + 8.0, r.center().y), Vec2::splat(16.0));
            paint_icon(painter, ir, icon, fade(text));
            x += icon_w + gap;
        }
        painter.galley(
            pos2(x, r.center().y - galley.size().y / 2.0),
            galley,
            fade(text),
        );
        if enabled {
            response.on_hover_cursor(CursorIcon::PointingHand)
        } else {
            response
        }
    }
}

// --- chips --------------------------------------------------------------------------

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

// --- tweens -------------------------------------------------------------------------

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

// --- switches -----------------------------------------------------------------------

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

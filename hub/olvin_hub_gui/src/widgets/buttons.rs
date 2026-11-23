//! The launcher button: 32 px, scale 0.98 while pressed.

use eframe::egui::{
    self, Color32, CursorIcon, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, pos2,
    vec2,
};

use super::*;
use crate::ui::{self, FS_MD, RADIUS_SM, anim};

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

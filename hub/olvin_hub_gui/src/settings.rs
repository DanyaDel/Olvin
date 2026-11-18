//! The Settings page: theme picker, reduce motion, engine source, about.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2,
    vec2,
};
use olvin_hub::EngineSource;

use crate::Hub;
use crate::chrome::{page_header, section};
use crate::ui::{
    self, FS_MD, FS_SM, Palette, RADIUS_LG, RADIUS_MD, RADIUS_SM, ThemeKind, anim, logo,
};
use crate::widgets::{self, Icon, bold, card_bg, mono, paint_icon, sans, truncated};

impl Hub {
    pub(crate) fn settings_page(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let p = ui::palette(&ctx);
        page_header(ui, "Settings", "Saved in the hub config", |_| {});
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                section(ui, "Theme");
                let w = ui.available_width();
                let gap = 14.0;
                let cols = ((w + gap) / (150.0 + gap)).floor().clamp(1.0, 4.0) as usize;
                let card_w = (w - gap * (cols - 1) as f32) / cols as f32;
                let card_h = 150.0;
                let mut picked = None;
                for chunk in ThemeKind::ALL.chunks(cols) {
                    let (strip, _) = ui.allocate_exact_size(vec2(w, card_h), Sense::hover());
                    for (k, kind) in chunk.iter().enumerate() {
                        let rect = Rect::from_min_size(
                            pos2(strip.min.x + k as f32 * (card_w + gap), strip.min.y),
                            vec2(card_w, card_h),
                        );
                        if theme_card(ui, rect, *kind, self.theme == *kind, &p) {
                            picked = Some(*kind);
                        }
                    }
                    ui.add_space(gap);
                }
                if let Some(kind) = picked
                    && kind != self.theme
                {
                    self.theme = kind;
                    self.save_prefs();
                }

                ui.add_space(6.0);
                section(ui, "Motion");
                let mut reduce = self.reduce_motion;
                setting_row(
                    ui,
                    &p,
                    "Reduce motion",
                    "Turn off slides, lifts, typing and blinking. Colour changes stay.",
                    |ui| {
                        widgets::toggle(ui, &mut reduce).on_hover_text("Reduce motion");
                    },
                );
                if reduce != self.reduce_motion {
                    self.reduce_motion = reduce;
                    self.save_prefs();
                }

                ui.add_space(14.0);
                section(ui, "Engine");
                let (kind, value) = match &self.config.engine {
                    EngineSource::Path(path) => ("Local checkout", path.display().to_string()),
                    EngineSource::Git(url) => ("Git", url.clone()),
                };
                setting_row(
                    ui,
                    &p,
                    kind,
                    "New projects use this engine. Change it with olvin-hub engine --path <dir> or --git <url>.",
                    |_| {},
                );
                // The source itself, mono, under the row.
                let w = ui.available_width();
                let g = truncated(ui.painter(), &value, mono(FS_SM), p.text_2, w - 32.0);
                let (r, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
                ui.painter()
                    .galley(pos2(r.min.x + 16.0, r.center().y - g.size().y / 2.0), g, p.text_2);
                resp.on_hover_text(&value);

                ui.add_space(18.0);
                section(ui, "About");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    logo::wordmark(ui, 30.0);
                });
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Olvin · by NLPA Collab").color(p.text_2));
                ui.label(
                    egui::RichText::new(format!("Hub {}", env!("CARGO_PKG_VERSION")))
                        .family(egui::FontFamily::Monospace)
                        .size(FS_SM)
                        .color(p.muted),
                );
                ui.add_space(24.0);
            });
    }
}

/// A full-width card with a title, a description and a control on the right.
fn setting_row(ui: &mut Ui, p: &Palette, title: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    let text_w = (w - 120.0).max(160.0);
    let mut job = egui::text::LayoutJob::simple(desc.to_owned(), sans(FS_SM), p.text_2, text_w);
    job.wrap.max_rows = 3;
    let desc_g = ui.painter().layout_job(job);
    let h = 34.0 + desc_g.size().y + 14.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    card_bg(ui.painter(), rect, RADIUS_MD, 0.0, false, p);
    ui.painter().text(
        pos2(rect.min.x + 16.0, rect.min.y + 22.0),
        Align2::LEFT_CENTER,
        title,
        bold(FS_MD + 1.0),
        p.text,
    );
    ui.painter()
        .galley(pos2(rect.min.x + 16.0, rect.min.y + 36.0), desc_g, p.text_2);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(rect.max.x - 100.0, rect.min.y),
                pos2(rect.max.x - 16.0, rect.max.y),
            ))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    control(&mut child);
    ui.add_space(8.0);
}

/// A theme card: a miniature hub in that theme's colours. Returns true on click.
fn theme_card(ui: &mut Ui, rect: Rect, kind: ThemeKind, selected: bool, p: &Palette) -> bool {
    let ctx = ui.ctx().clone();
    let response = ui
        .interact(rect, Id::new(("theme", kind.label())), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let hover = anim::hover_t(&ctx, response.id.with("h"), response.hovered());
    let lift = if ui::reduce_motion(&ctx) {
        0.0
    } else {
        -4.0 * hover
    };
    let r = rect.translate(vec2(0.0, lift));
    let painter = ui.painter();
    card_bg(painter, r, RADIUS_LG, hover, selected, p);
    let sel = anim::fade(&ctx, response.id.with("sel"), selected);
    if sel > 0.0 {
        let glow = egui::Shadow {
            offset: [0, 0],
            blur: 15,
            spread: 0,
            color: p.accent_glow.gamma_multiply(sel * 0.7),
        };
        painter.add(glow.as_shape(r, RADIUS_LG));
        painter.rect_stroke(
            r,
            RADIUS_LG,
            Stroke::new(1.5, p.accent.gamma_multiply(sel)),
            StrokeKind::Inside,
        );
    }

    // Miniature: sidebar, a title bar, two rows, the accent dot.
    let t = kind.palette();
    let mini = Rect::from_min_max(
        r.min + vec2(12.0, 12.0),
        pos2(r.max.x - 12.0, r.min.y + 96.0),
    );
    painter.rect(
        mini,
        RADIUS_SM,
        t.bg,
        Stroke::new(1.0, t.line),
        StrokeKind::Inside,
    );
    let side = Rect::from_min_max(mini.min, pos2(mini.min.x + mini.width() * 0.26, mini.max.y));
    painter.rect_filled(
        side,
        egui::CornerRadius {
            nw: RADIUS_SM,
            sw: RADIUS_SM,
            ne: 0,
            se: 0,
        },
        t.surface,
    );
    for i in 0..3 {
        let y = side.min.y + 14.0 + i as f32 * 12.0;
        let c = if i == 0 { t.accent } else { t.muted };
        painter.rect_filled(
            Rect::from_min_size(pos2(side.min.x + 7.0, y), vec2(side.width() - 14.0, 4.0)),
            2.0,
            c.gamma_multiply(if i == 0 { 1.0 } else { 0.6 }),
        );
    }
    let body_x = side.max.x + 10.0;
    painter.rect_filled(
        Rect::from_min_size(
            pos2(body_x, mini.min.y + 12.0),
            vec2(mini.width() * 0.3, 6.0),
        ),
        3.0,
        t.text,
    );
    for i in 0..2 {
        let row = Rect::from_min_max(
            pos2(body_x, mini.min.y + 28.0 + i as f32 * 26.0),
            pos2(mini.max.x - 10.0, mini.min.y + 48.0 + i as f32 * 26.0),
        );
        painter.rect(
            row,
            4.0,
            t.surface_raised,
            Stroke::new(1.0, t.line_soft),
            StrokeKind::Inside,
        );
        painter.rect_filled(
            Rect::from_min_size(row.min + vec2(6.0, 6.0), vec2(8.0, 8.0)),
            2.0,
            if i == 0 { t.accent } else { t.ghost },
        );
        painter.rect_filled(
            Rect::from_min_size(row.min + vec2(20.0, 8.0), vec2(row.width() * 0.4, 4.0)),
            2.0,
            t.text_2,
        );
    }

    let label_y = r.max.y - 26.0;
    painter.text(
        pos2(r.min.x + 16.0, label_y),
        Align2::LEFT_CENTER,
        kind.label(),
        if selected {
            bold(FS_MD + 1.0)
        } else {
            sans(FS_MD + 1.0)
        },
        if selected { p.text } else { p.text_2 },
    );
    if selected {
        let c = pos2(r.max.x - 26.0, label_y);
        painter.circle_filled(c, 10.0, p.accent);
        paint_icon(
            painter,
            Rect::from_center_size(c, vec2(14.0, 14.0)),
            Icon::Check,
            p.on_accent,
        );
    } else {
        painter.circle_stroke(
            pos2(r.max.x - 26.0, label_y),
            9.0,
            Stroke::new(1.0, p.line_top),
        );
    }
    let _ = Color32::TRANSPARENT;
    response.clicked()
}

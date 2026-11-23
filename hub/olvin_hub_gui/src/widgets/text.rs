//! Text helpers: truncated single lines, caps labels and the launcher fonts.

use eframe::egui::{self, Color32, FontFamily, FontId, Galley, Painter, text::LayoutJob};
use std::sync::Arc;

use crate::ui::{self, FS_XS};

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

//! Problems: cargo diagnostics as a list; click to jump.

use olvin_editor::egui::{self, RichText, Sense};
use olvin_editor::{EditorState, Panel, theme};

use crate::diagnostics::Severity;
use crate::ide::{self, Ide};
use crate::panels::code::{build_toolbar, severity_color};

pub struct Problems {
    errors: bool,
    warnings: bool,
    notes: bool,
    filter: String,
}

impl Default for Problems {
    fn default() -> Self {
        Self {
            errors: true,
            warnings: true,
            notes: true,
            filter: String::new(),
        }
    }
}

impl Panel for Problems {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        ide::with(|ide| {
            ide.frame(ui, state);
            self.draw(ui, state, ide);
        });
    }
}

impl Problems {
    fn draw(&mut self, ui: &mut egui::Ui, state: &mut EditorState, ide: &mut Ide) {
        let count = |s| ide.diagnostics.iter().filter(|d| d.severity == s).count();
        let (ne, nw, nn) = (
            count(Severity::Error),
            count(Severity::Warning),
            count(Severity::Note),
        );
        ui.horizontal(|ui| {
            build_toolbar(ui, ide, state);
            ui.separator();
            ui.toggle_value(
                &mut self.errors,
                RichText::new(format!("⛔ {ne}")).color(ui.visuals().error_fg_color),
            );
            ui.toggle_value(
                &mut self.warnings,
                RichText::new(format!("⚠ {nw}")).color(ui.visuals().warn_fg_color),
            );
            ui.toggle_value(&mut self.notes, format!("ℹ {nn}"));
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("🔍 Filter")
                    .desired_width(f32::INFINITY),
            );
        });
        ui.separator();

        let filter = self.filter.to_lowercase();
        let shown: Vec<usize> = ide
            .diagnostics
            .iter()
            .enumerate()
            .filter(|(_, d)| match d.severity {
                Severity::Error => self.errors,
                Severity::Warning => self.warnings,
                Severity::Note => self.notes,
            })
            .filter(|(_, d)| {
                filter.is_empty()
                    || d.summary().to_lowercase().contains(&filter)
                    || d.path.to_string_lossy().to_lowercase().contains(&filter)
            })
            .map(|(i, _)| i)
            .collect();
        if shown.is_empty() {
            let msg = if ide.job_running() {
                "Building…"
            } else if ide.last_job.is_none() {
                "No problems yet: press Check to run cargo check."
            } else {
                "No problems."
            };
            ui.label(RichText::new(msg).color(theme::TEXT_WEAK));
            return;
        }
        let mut jump = None;
        let row_h = 22.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            row_h,
            shown.len(),
            |ui, range| {
                for &i in &shown[range] {
                    let d = &ide.diagnostics[i];
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 4.0, theme::RAISED);
                    }
                    let p = ui.painter_at(rect);
                    let y = rect.center().y;
                    p.text(
                        egui::pos2(rect.left() + 6.0, y),
                        egui::Align2::LEFT_CENTER,
                        d.severity.icon(),
                        egui::TextStyle::Body.resolve(ui.style()),
                        severity_color(ui, d.severity),
                    );
                    let loc = format!("{}:{}:{}", ide.rel(&d.path), d.line, d.column);
                    let loc_rect = p.text(
                        egui::pos2(rect.right() - 6.0, y),
                        egui::Align2::RIGHT_CENTER,
                        &loc,
                        egui::TextStyle::Small.resolve(ui.style()),
                        theme::TEXT_WEAK,
                    );
                    let msg_clip = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(loc_rect.left() - 8.0, rect.bottom()),
                    );
                    ui.painter_at(msg_clip).text(
                        egui::pos2(rect.left() + 26.0, y),
                        egui::Align2::LEFT_CENTER,
                        d.summary(),
                        egui::TextStyle::Body.resolve(ui.style()),
                        theme::TEXT,
                    );
                    let tip = match &d.code {
                        Some(c) => format!("{} [{c}]\n{loc}", d.summary()),
                        None => format!("{}\n{loc}", d.summary()),
                    };
                    let resp = resp.on_hover_text(tip);
                    if resp.clicked() {
                        jump = Some((d.path.clone(), d.line, d.column));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Copy Message").clicked() {
                            ui.ctx().copy_text(d.summary());
                        }
                    });
                }
            },
        );
        if let Some((path, line, col)) = jump {
            ide.open_at(&path, line, col, state);
        }
    }
}

//! Output: what cargo and the game print.

use olvin_editor::egui::{self, RichText};
use olvin_editor::{EditorState, Panel, theme};

use crate::ide::{self, Command, Ide, OutKind};

pub struct Output {
    follow: bool,
}

impl Default for Output {
    fn default() -> Self {
        Self { follow: true }
    }
}

impl Panel for Output {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        ide::with(|ide| {
            ide.frame(ui, state);
            self.draw(ui, state, ide);
        });
    }
}

/// `--> src/main.rs:3:18` in rustc output.
pub fn location(line: &str) -> Option<(&str, usize, usize)> {
    let rest = line.trim_start().strip_prefix("--> ")?;
    let mut parts = rest.rsplitn(3, ':');
    let col = parts.next()?.trim().parse().ok()?;
    let ln = parts.next()?.parse().ok()?;
    Some((parts.next()?, ln, col))
}

impl Output {
    fn draw(&mut self, ui: &mut egui::Ui, state: &mut EditorState, ide: &mut Ide) {
        ui.horizontal(|ui| {
            if ui.button("Clear").clicked() {
                ide.run(Command::ClearOutput, state);
            }
            if ui
                .add_enabled(ide.job_running(), egui::Button::new("⏹ Stop"))
                .clicked()
            {
                ide.run(Command::Stop, state);
            }
            ui.toggle_value(&mut self.follow, "⬇ Follow")
                .on_hover_text("Keep scrolled to the end");
            if ui.button("Copy All").clicked() {
                let all: Vec<&str> = ide.output.iter().map(|l| l.text.as_str()).collect();
                ui.ctx().copy_text(all.join("\n"));
            }
            ui.label(
                RichText::new(format!("{} lines", ide.output.len()))
                    .small()
                    .color(theme::palette(ui.ctx()).muted),
            );
        });
        ui.separator();
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
        let mut jump = None;
        egui::ScrollArea::both()
            .auto_shrink(false)
            .stick_to_bottom(self.follow)
            .show_rows(ui, row_h, ide.output.len(), |ui, range| {
                for line in &ide.output[range] {
                    let color = match line.kind {
                        OutKind::Normal => theme::palette(ui.ctx()).text,
                        OutKind::Error => ui.visuals().error_fg_color,
                        OutKind::Warning => ui.visuals().warn_fg_color,
                        OutKind::Info => theme::INFO,
                        OutKind::Success => theme::ACCENT,
                    };
                    let text = RichText::new(&line.text).monospace().color(color);
                    if let Some(loc) = location(&line.text) {
                        if ui
                            .add(egui::Label::new(text.underline()).sense(egui::Sense::click()))
                            .on_hover_text("Open")
                            .clicked()
                        {
                            jump = Some((loc.0.to_owned(), loc.1, loc.2));
                        }
                    } else {
                        ui.add(egui::Label::new(text).extend());
                    }
                }
            });
        if let Some((file, line, col)) = jump {
            let path = std::path::Path::new(&file);
            let path = match &ide.root {
                Some(root) if path.is_relative() => root.join(path),
                _ => path.to_path_buf(),
            };
            ide.open_at(&path, line, col, state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rustc_locations() {
        assert_eq!(
            location(" --> src/main.rs:3:18"),
            Some(("src/main.rs", 3, 18))
        );
        assert_eq!(location("--> C:/x/a.rs:1:2"), Some(("C:/x/a.rs", 1, 2)));
        assert_eq!(location("error: x"), None);
    }
}

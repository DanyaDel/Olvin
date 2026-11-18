//! Editor element: console with the editor log (saves, errors, plugin messages).

use olvin_editor::egui::{self, RichText, TextEdit};
use olvin_editor::{EditorPlugin, EditorState, LogLevel, Panel, PanelKind, Registry};

pub struct ConsolePlugin;

impl EditorPlugin for ConsolePlugin {
    fn name(&self) -> &'static str {
        "Console"
    }

    fn build(&self, registry: &mut Registry) {
        registry.add_panel(PanelKind {
            id: "console.log",
            title: "🖵 Console",
            factory: || {
                Box::new(Console {
                    info: true,
                    warn: true,
                    error: true,
                    filter: String::new(),
                })
            },
        });
    }
}

struct Console {
    info: bool,
    warn: bool,
    error: bool,
    filter: String,
}

impl Panel for Console {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let count = |level| state.log.iter().filter(|l| l.level == level).count();
        let (n_info, n_warn, n_error) = (
            count(LogLevel::Info),
            count(LogLevel::Warn),
            count(LogLevel::Error),
        );

        ui.horizontal(|ui| {
            if ui.button("Clear").clicked() {
                state.log.clear();
            }
            ui.separator();
            ui.toggle_value(&mut self.info, format!("ℹ {n_info}"));
            ui.toggle_value(
                &mut self.warn,
                RichText::new(format!("⚠ {n_warn}")).color(ui.visuals().warn_fg_color),
            );
            ui.toggle_value(
                &mut self.error,
                RichText::new(format!("⛔ {n_error}")).color(ui.visuals().error_fg_color),
            );
            ui.add(
                TextEdit::singleline(&mut self.filter)
                    .hint_text("🔍 Filter")
                    .desired_width(f32::INFINITY),
            );
        });
        ui.separator();

        let filter = self.filter.to_lowercase();
        let visible: Vec<_> = state
            .log
            .iter()
            .filter(|l| match l.level {
                LogLevel::Info => self.info,
                LogLevel::Warn => self.warn,
                LogLevel::Error => self.error,
            })
            .filter(|l| filter.is_empty() || l.text.to_lowercase().contains(&filter))
            .collect();

        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show_rows(ui, row_height, visible.len(), |ui, range| {
                for line in &visible[range] {
                    let color = match line.level {
                        LogLevel::Info => ui.visuals().text_color(),
                        LogLevel::Warn => ui.visuals().warn_fg_color,
                        LogLevel::Error => ui.visuals().error_fg_color,
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{:>8.2}", line.time))
                                .monospace()
                                .weak(),
                        );
                        ui.label(RichText::new(&line.text).monospace().color(color));
                    });
                }
            });
    }
}

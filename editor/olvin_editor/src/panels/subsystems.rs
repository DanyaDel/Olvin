//! Subsystems window: what crashed and a Restart for it, like `systemctl
//! status` for the game in Play and for the editor's own panels.

use egui::RichText;

use crate::{EditorState, Panel, theme};

#[derive(Default)]
pub struct Subsystems;

impl Panel for Subsystems {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let palette = theme::palette(ui.ctx());
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.heading("Game");
                match &mut state.play {
                    None => {
                        ui.weak("Not running. Crashes of the game during Play show up here.");
                    }
                    Some(play) => {
                        let mut faults = play.app.resources.fetch_mut::<olvin::core::Faults>();
                        let down: Vec<&'static str> = faults.down().to_vec();
                        if down.is_empty() {
                            ui.colored_label(palette.accent, "✔ All subsystems are running");
                        }
                        for owner in down {
                            ui.horizontal(|ui| {
                                ui.colored_label(palette.error, format!("● {owner}"));
                                ui.weak("stopped");
                                if ui
                                    .button("⟲ Restart")
                                    .on_hover_text(
                                        "Runs its systems again from the next frame (Startup systems do not run twice)",
                                    )
                                    .clicked()
                                {
                                    faults.restart(owner);
                                }
                            });
                        }
                        if !faults.list().is_empty() {
                            ui.add_space(6.0);
                            ui.label(RichText::new("Crash history").strong());
                            for f in faults.list().iter().rev() {
                                ui.label(format!(
                                    "frame {} · {} · {} ({:?})",
                                    f.frame, f.owner, f.system, f.stage
                                ));
                                ui.weak(&f.message);
                            }
                        }
                    }
                }

                ui.add_space(10.0);
                ui.heading("Editor panels");
                if state.crashed_panels.is_empty() {
                    ui.colored_label(palette.accent, "✔ No panel has crashed");
                }
                let mut restart = None;
                for (id, (title, message)) in &state.crashed_panels {
                    ui.horizontal(|ui| {
                        ui.colored_label(palette.error, format!("● {title}"));
                        if ui.button("⟲ Restart").clicked() {
                            restart = Some(*id);
                        }
                    });
                    ui.weak(message);
                }
                if let Some(id) = restart
                    && let Some((title, _)) = state.crashed_panels.remove(&id)
                {
                    state.warn(format!("{title}: restarted"));
                }

                ui.add_space(10.0);
                ui.heading("Editor elements");
                let crashed = state.ext.viewport.iter().any(|s| s.crashed.is_some())
                    || state.ext.hooks.iter().any(|h| h.crashed.is_some());
                if !crashed {
                    ui.colored_label(palette.accent, "✔ All viewport tools and frame hooks are running");
                }
                let mut restarted = Vec::new();
                for slot in &mut state.ext.viewport {
                    let Some(message) = slot.crashed.clone() else {
                        continue;
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(palette.error, format!("● {} · viewport", slot.owner));
                        if ui.button("⟲ Restart").clicked() {
                            slot.restart();
                            restarted.push(slot.owner);
                        }
                    });
                    ui.weak(message);
                }
                for hook in &mut state.ext.hooks {
                    let Some(message) = hook.crashed.clone() else {
                        continue;
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(palette.error, format!("● {} · frame hook", hook.owner));
                        if ui.button("⟲ Restart").clicked() {
                            hook.crashed = None;
                            restarted.push(hook.owner);
                        }
                    });
                    ui.weak(message);
                }
                for owner in restarted {
                    state.warn(format!("element {owner}: restarted"));
                }
            });
    }
}

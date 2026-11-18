//! Lighting window: presets and every light in the scene at a glance. The
//! environment itself is edited in Settings → Rendering (`environment_ui`).

use egui::{DragValue, RichText};
use olvin_scene::file::{Environment, FogDesc, LightKind};

use super::inspector::{drag, grid, row, section};
use crate::{EditorState, Panel};

#[derive(Default)]
pub struct Lighting;

impl Panel for Lighting {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                if ui
                    .button("⚙ Render Settings…")
                    .on_hover_text(
                        "Environment, exposure, fog, shadows and viewport quality (Ctrl+,)",
                    )
                    .clicked()
                {
                    state.open_settings(crate::settings::Page::Rendering);
                }
                ui.add_space(4.0);
                presets(ui, state);
                lights(ui, state);
            });
    }
}

/// The scene's environment: ambient, camera, fog and shadows (Settings → Rendering).
pub(crate) fn environment_ui(ui: &mut egui::Ui, env: &mut Environment) {
    section(ui, "🌤 Environment", None, |ui| {
        grid(ui, "env", |ui| {
            row(ui, "Sky", |ui| {
                ui.color_edit_button_rgb(&mut env.sky);
            });
            row(ui, "Ground", |ui| {
                ui.color_edit_button_rgb(&mut env.ground);
            });
            row(ui, "Intensity", |ui| {
                drag(ui, &mut env.ambient_intensity, 0.0..=8.0, 0.01, "")
            });
            row(ui, "Background", |ui| {
                ui.color_edit_button_rgb(&mut env.background);
            });
        });
    });
    section(ui, "📷 Camera", None, |ui| {
        grid(ui, "cam", |ui| {
            row(ui, "Exposure", |ui| {
                ui.add(egui::Slider::new(&mut env.exposure, 0.05..=8.0).logarithmic(true));
            });
            row(ui, "Tonemapping", |ui| {
                ui.selectable_value(&mut env.filmic, true, "ACES Filmic");
                ui.selectable_value(&mut env.filmic, false, "None");
            });
        });
    });
    section(ui, "🌫 Fog", None, |ui| {
        let mut on = env.fog.is_some();
        if ui.checkbox(&mut on, "Enabled").changed() {
            env.fog = on.then_some(FogDesc {
                color: env.background,
                start: 15.0,
                end: 120.0,
            });
        }
        if let Some(fog) = &mut env.fog {
            grid(ui, "fog", |ui| {
                row(ui, "Color", |ui| {
                    ui.color_edit_button_rgb(&mut fog.color);
                    if ui.small_button("= Background").clicked() {
                        fog.color = env.background;
                    }
                });
                row(ui, "Start", |ui| {
                    drag(ui, &mut fog.start, 0.0..=10_000.0, 0.2, " m")
                });
                row(ui, "End", |ui| {
                    drag(ui, &mut fog.end, 0.0..=10_000.0, 0.5, " m")
                });
            });
            fog.end = fog.end.max(fog.start + 0.1);
        }
    });
    section(ui, "◼ Shadows", None, |ui| {
        grid(ui, "shadows", |ui| {
            row(ui, "Distance", |ui| {
                drag(ui, &mut env.shadow_distance, 5.0..=1000.0, 0.5, " m")
            });
            row(ui, "Resolution", |ui| {
                for r in [512, 1024, 2048, 4096] {
                    ui.selectable_value(&mut env.shadow_resolution, r, r.to_string());
                }
            });
        });
        ui.weak("Shorter distance = sharper shadows; the sun's Shadows toggle is in its Light.");
    });
}

pub(crate) fn presets(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Presets").strong());
        for (name, make) in Environment::PRESETS {
            if ui.button(name).clicked() {
                state.scene.environment = make();
            }
        }
        if ui
            .button("Reset")
            .on_hover_text("Default environment")
            .clicked()
        {
            state.scene.environment = Environment::default();
        }
    });
    ui.add_space(4.0);
}

/// Every light in the scene: click to select, quick colour/intensity/toggle.
fn lights(ui: &mut egui::Ui, state: &mut EditorState) {
    let ids: Vec<u64> = state
        .scene
        .entities
        .iter()
        .filter(|e| e.light.is_some())
        .map(|e| e.id)
        .collect();
    let mut select = None;
    section(ui, &format!("💡 Lights ({})", ids.len()), None, |ui| {
        if ids.is_empty() {
            ui.weak(
                "No lights: GameObject → Light. Without a Directional light a default sun is used.",
            );
        }
        let mut sun_seen = false;
        for id in ids {
            let selected = state.selection == Some(id);
            let Some(e) = state.scene.get_mut(id) else {
                continue;
            };
            let Some(l) = &mut e.light else { continue };
            ui.horizontal(|ui| {
                ui.checkbox(&mut e.active, "");
                let icon = match l.kind {
                    LightKind::Directional => "☀",
                    LightKind::Point => "💡",
                    LightKind::Spot => "🔦",
                };
                let mut label = format!("{icon} {}", e.name);
                if l.kind == LightKind::Directional && e.active && !sun_seen {
                    sun_seen = true;
                    label.push_str("  (sun)");
                }
                if ui.selectable_label(selected, label).clicked() {
                    select = Some(id);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        DragValue::new(&mut l.intensity)
                            .range(0.0..=100.0)
                            .speed(0.02)
                            .max_decimals(2),
                    );
                    ui.color_edit_button_rgb(&mut l.color);
                });
            });
        }
    });
    if let Some(id) = select {
        state.select(Some(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewEntity;

    #[test]
    fn draws_headless_with_fog_and_lights() {
        let mut state = EditorState::new(None);
        state.create(NewEntity::Light(LightKind::Spot), None);
        state.scene.environment = (Environment::PRESETS[2].1)();
        let ctx = egui::Context::default();
        let mut panel = Lighting;
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 900.0),
                )),
                ..Default::default()
            };
            ctx.run_ui(input, |ui| panel.ui(ui, &mut state))
                .textures_delta
                .clear();
        }
        assert!(state.scene.environment.fog.is_some());
    }
}

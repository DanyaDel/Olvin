//! Lighting window: presets and every light in the scene at a glance. The
//! environment itself is edited in Settings → Rendering (`environment_ui`).

use egui::{DragValue, RichText};
use olvin_scene::file::{Environment, FogDesc, LightKind, Sky};

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
pub(crate) fn environment_ui(ui: &mut egui::Ui, env: &mut Environment, textures: &[String]) {
    sky_ui(ui, env, textures);
    section(ui, "🌤 Environment", None, |ui| {
        grid(ui, "env", |ui| {
            row(ui, "Sky", |ui| {
                crate::widgets::color_rgb(ui, &mut env.sky);
            });
            row(ui, "Ground", |ui| {
                crate::widgets::color_rgb(ui, &mut env.ground);
            });
            row(ui, "Intensity", |ui| {
                drag(ui, &mut env.ambient_intensity, 0.0..=8.0, 0.01, "")
            });
            row(ui, "Background", |ui| {
                crate::widgets::color_rgb(ui, &mut env.background);
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
                    crate::widgets::color_rgb(ui, &mut fog.color);
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
                    crate::widgets::color_rgb(ui, &mut l.color);
                });
            });
        }
    });
    if let Some(id) = select {
        state.select(Some(id));
    }
}

/// Skybox: flat colour, procedural gradient with a sun, or a panorama texture.
fn sky_ui(ui: &mut egui::Ui, env: &mut Environment, textures: &[String]) {
    section(ui, "🌌 Skybox", None, |ui| {
        ui.horizontal(|ui| {
            let kind = match env.skybox {
                Sky::Color => 0,
                Sky::Procedural { .. } => 1,
                Sky::Texture { .. } => 2,
            };
            if ui.selectable_label(kind == 0, "Color").clicked() && kind != 0 {
                env.skybox = Sky::Color;
            }
            if ui.selectable_label(kind == 1, "Procedural").clicked() && kind != 1 {
                env.skybox = Sky::procedural();
            }
            if ui.selectable_label(kind == 2, "Panorama").clicked() && kind != 2 {
                let path = textures
                    .iter()
                    .find(|t| t.ends_with(".hdr") || t.ends_with(".exr"))
                    .or(textures.first());
                env.skybox = Sky::Texture {
                    path: path.cloned().unwrap_or_default(),
                    rotation_deg: 0.0,
                    intensity: 1.0,
                };
            }
        });
        match &mut env.skybox {
            Sky::Color => {
                ui.weak("Uses the Background colour.");
            }
            Sky::Procedural {
                zenith,
                horizon,
                ground,
                sun_size,
            } => {
                grid(ui, "sky-proc", |ui| {
                    row(ui, "Zenith", |ui| {
                        crate::widgets::color_rgb(ui, zenith);
                    });
                    row(ui, "Horizon", |ui| {
                        crate::widgets::color_rgb(ui, horizon);
                    });
                    row(ui, "Ground", |ui| {
                        crate::widgets::color_rgb(ui, ground);
                    });
                    row(ui, "Sun size", |ui| {
                        drag(ui, sun_size, 0.0..=20.0, 0.05, "°")
                    });
                });
                ui.weak("The sun disc follows the first Directional light.");
            }
            Sky::Texture {
                path,
                rotation_deg,
                intensity,
            } => {
                grid(ui, "sky-tex", |ui| {
                    row(ui, "Panorama", |ui| {
                        egui::ComboBox::from_id_salt("sky-texture")
                            .width(200.0)
                            .selected_text(if path.is_empty() {
                                "(pick an image)"
                            } else {
                                path.as_str()
                            })
                            .show_ui(ui, |ui| {
                                for t in textures {
                                    ui.selectable_value(path, t.clone(), t.as_str());
                                }
                            });
                    });
                    row(ui, "Rotation", |ui| {
                        drag(ui, rotation_deg, -360.0..=360.0, 0.5, "°")
                    });
                    row(ui, "Intensity", |ui| {
                        drag(ui, intensity, 0.0..=16.0, 0.01, "")
                    });
                });
                ui.weak("Equirectangular .hdr / .exr / .png / .jpg from assets/ (Poly Haven HDRIs work great).");
            }
        }
    });
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
        crate::theme::apply(&ctx);
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

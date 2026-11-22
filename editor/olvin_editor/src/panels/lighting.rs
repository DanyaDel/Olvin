//! Lighting window: presets and every light in the scene at a glance. The
//! environment itself is edited in Settings → Rendering (`environment_ui`).

use egui::{DragValue, RichText};
use olvin_scene::file::{
    AoDesc, Environment, FogDesc, HeightFogDesc, LightKind, PostDesc, ShadowFilter, Sky,
};

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
    section(ui, "⛰ Height Fog", None, |ui| {
        let mut on = env.height_fog.is_some();
        if ui.checkbox(&mut on, "Enabled").changed() {
            env.height_fog = on.then(|| HeightFogDesc {
                color: env.fog.map_or(HeightFogDesc::default().color, |f| f.color),
                ..Default::default()
            });
        }
        if let Some(fog) = &mut env.height_fog {
            grid(ui, "height-fog", |ui| {
                row(ui, "Color", |ui| {
                    crate::widgets::color_rgb(ui, &mut fog.color);
                });
                row(ui, "Density", |ui| {
                    drag(ui, &mut fog.density, 0.0..=2.0, 0.001, " /m")
                });
                row(ui, "Falloff", |ui| {
                    drag(ui, &mut fog.falloff, 0.0..=2.0, 0.005, " /m")
                });
                row(ui, "Base height", |ui| {
                    drag(ui, &mut fog.height, -1000.0..=1000.0, 0.2, " m")
                });
            });
            ui.weak("Thickest at the base height, thinner above it. Fades into the sky horizon.");
        }
    });
    post_ui(ui, &mut env.post, textures);
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
            row(ui, "Filter", |ui| {
                egui::ComboBox::from_id_salt("shadow-filter")
                    .selected_text(filter_label(env.shadow_filter))
                    .show_ui(ui, |ui| {
                        for f in [
                            ShadowFilter::Hard,
                            ShadowFilter::Pcf,
                            ShadowFilter::Soft,
                            ShadowFilter::Pcss,
                        ] {
                            ui.selectable_value(&mut env.shadow_filter, f, filter_label(f));
                        }
                    })
                    .response
                    .on_hover_text(
                        "PCSS: soft edges that widen with the distance from the occluder \
                         (the sun's Size sets how much). Needs a Vulkan / Metal / DirectX \
                         GPU; on OpenGL it falls back to PCF.",
                    );
            });
            row(ui, "Cascades", |ui| {
                for c in 1..=4 {
                    ui.selectable_value(&mut env.shadow_cascades, c, c.to_string());
                }
            });
            row(ui, "Spot Shadows", |ui| {
                for n in 0..=4 {
                    ui.selectable_value(&mut env.max_spot_shadows, n, n.to_string());
                }
            });
            row(ui, "Point Shadows", |ui| {
                for n in 0..=2 {
                    ui.selectable_value(&mut env.max_point_shadows, n, n.to_string());
                }
            });
            row(ui, "Local Res.", |ui| {
                for r in [1024, 2048, 4096] {
                    ui.selectable_value(&mut env.local_shadow_resolution, r, r.to_string());
                }
            });
        });
        ui.weak(
            "Shorter distance = sharper shadows. Lights cast shadows with their own Cast \
             Shadows toggle; only the most important spot / point lights up to the limits \
             above get one (fewer on OpenGL).",
        );
    });
}

fn filter_label(f: ShadowFilter) -> &'static str {
    match f {
        ShadowFilter::Hard => "Hard",
        ShadowFilter::Pcf => "PCF",
        ShadowFilter::Soft => "Soft",
        ShadowFilter::Pcss => "PCSS",
    }
}

/// Post-processing: FXAA, ambient occlusion and colour grading. Drawn by the
/// game window on Full-tier GPUs; the editor viewport shows none of it.
fn post_ui(ui: &mut egui::Ui, post: &mut PostDesc, textures: &[String]) {
    section(ui, "✨ Post-processing", None, |ui| {
        ui.checkbox(&mut post.fxaa, "FXAA anti-aliasing");
        let mut ao = post.ao.is_some();
        if ui
            .checkbox(&mut ao, "Ambient occlusion (GTAO)")
            .on_hover_text("Darkens creases and contact points; costs two fullscreen passes")
            .changed()
        {
            post.ao = ao.then(AoDesc::default);
        }
        if let Some(a) = &mut post.ao {
            grid(ui, "post-ao", |ui| {
                row(ui, "Radius", |ui| {
                    drag(ui, &mut a.radius, 0.05..=10.0, 0.01, " m")
                });
                row(ui, "Strength", |ui| {
                    drag(ui, &mut a.strength, 0.1..=4.0, 0.01, "")
                });
                row(ui, "Bias", |ui| drag(ui, &mut a.bias, 0.0..=0.5, 0.002, ""));
            });
        }
        ui.separator();
        ui.label(RichText::new("Color grading").strong());
        grid(ui, "post-grade", |ui| {
            row(ui, "Saturation", |ui| {
                drag(ui, &mut post.saturation, 0.0..=2.0, 0.01, "")
            });
            row(ui, "Contrast", |ui| {
                drag(ui, &mut post.contrast, 0.0..=2.0, 0.01, "")
            });
            row(ui, "Lift", |ui| {
                crate::widgets::color_rgb(ui, &mut post.lift);
            });
            row(ui, "Gamma", |ui| {
                for g in &mut post.gamma {
                    ui.add(
                        DragValue::new(g)
                            .range(0.1..=4.0)
                            .speed(0.01)
                            .max_decimals(2),
                    );
                }
            });
            row(ui, "Gain", |ui| {
                for g in &mut post.gain {
                    ui.add(
                        DragValue::new(g)
                            .range(0.0..=4.0)
                            .speed(0.01)
                            .max_decimals(2),
                    );
                }
            });
            row(ui, "LUT", |ui| {
                let shown = post.lut.clone().unwrap_or_default();
                egui::ComboBox::from_id_salt("post-lut")
                    .width(200.0)
                    .selected_text(if shown.is_empty() {
                        "(none)"
                    } else {
                        shown.as_str()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut post.lut, None, "(none)");
                        for t in textures {
                            ui.selectable_value(&mut post.lut, Some(t.clone()), t.as_str());
                        }
                    });
            });
            if post.lut.is_some() {
                row(ui, "LUT strength", |ui| {
                    drag(ui, &mut post.lut_strength, 0.0..=1.0, 0.01, "")
                });
            }
        });
        if ui.small_button("Reset post-processing").clicked() {
            *post = PostDesc::default();
        }
        ui.weak("LUT: a strip of N slices of N×N (e.g. 1024×32) in an image file, imported as plain data.");
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

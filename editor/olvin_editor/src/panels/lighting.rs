//! Lighting window: presets and every light in the scene at a glance. The
//! environment itself is edited in Settings → Rendering (`environment_ui`).

use egui::{DragValue, RichText};
use olvin_scene::file::{
    AoDesc, BloomDesc, Environment, FogDesc, HeightFogDesc, LightKind, PostDesc, RetroDesc,
    ShadowFilter, Sky,
};
use olvin_scene::look::LookDesc;

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
    retro_ui(ui, &mut env.retro);
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

/// The retro (PS1) render style. Snapping, affine textures, colour depth and
/// dither, vertex lighting and the draw distance live in the mesh shader and
/// show in the editor viewport too; the low internal resolution is a
/// game-window (Full tier) effect.
fn retro_ui(ui: &mut egui::Ui, retro: &mut Option<RetroDesc>) {
    section(ui, "🕹 Retro / PS1", None, |ui| {
        ui.horizontal(|ui| {
            let mut on = retro.is_some();
            if ui
                .checkbox(&mut on, "Enabled")
                .on_hover_text("The PlayStation 1 look for every material that doesn't opt out")
                .changed()
            {
                *retro = on.then(RetroDesc::ps1);
            }
            if ui.button("PS1 preset").clicked() {
                *retro = Some(RetroDesc::ps1());
            }
            if ui.button("Off").clicked() {
                *retro = None;
            }
        });
        let Some(r) = retro else {
            ui.weak("Vertex jitter, affine textures, 15-bit dithered colour, Gouraud lighting.");
            return;
        };
        let int2 = |ui: &mut egui::Ui, v: &mut [u32; 2]| {
            ui.add(DragValue::new(&mut v[0]).range(0..=4096).prefix("w "));
            ui.add(DragValue::new(&mut v[1]).range(0..=4096).prefix("h "));
        };
        grid(ui, "retro", |ui| {
            row(ui, "Snap resolution", |ui| int2(ui, &mut r.snap_resolution));
            row(ui, "Snap grid", |ui| {
                drag(ui, &mut r.snap_grid_m, 0.0..=4.0, 0.01, " m")
            });
            row(ui, "Affine strength", |ui| {
                drag(ui, &mut r.affine_strength, 0.0..=1.0, 0.01, "")
            });
            row(ui, "Affine fade", |ui| {
                drag(ui, &mut r.affine_fade_near, 0.0..=50.0, 0.1, " m")
            });
            row(ui, "Colour bits", |ui| {
                ui.add(DragValue::new(&mut r.color_bits).range(0..=8))
                    .on_hover_text(
                        "Per channel: 5 = the PS1's 15-bit colour; 0 or 8 = full colour",
                    );
            });
            row(ui, "Dither", |ui| {
                drag(ui, &mut r.dither_strength, 0.0..=2.0, 0.01, "")
            });
            row(ui, "Vertex lighting", |ui| {
                ui.checkbox(&mut r.vertex_lighting, "")
                    .on_hover_text("Gouraud: light per vertex, diffuse only, no normal maps");
            });
            row(ui, "Sun shadows", |ui| {
                ui.checkbox(&mut r.shadows, "")
                    .on_hover_text("Shadow map lookups per pixel in vertex-lit mode");
            });
            row(ui, "Draw distance", |ui| {
                drag(ui, &mut r.draw_distance, 0.0..=2000.0, 0.5, " m")
            });
            row(ui, "Dissolve", |ui| {
                ui.checkbox(&mut r.dissolve, "")
                    .on_hover_text("Dither the last tenth before the draw distance");
            });
            row(ui, "Cull small", |ui| {
                drag(ui, &mut r.cull_small_px, 0.0..=20.0, 0.1, " px")
            });
            row(ui, "Force nearest", |ui| {
                ui.checkbox(&mut r.force_nearest, "")
                    .on_hover_text("Nearest filtering, no mipmaps for retro materials");
            });
            row(ui, "Internal res.", |ui| {
                int2(ui, &mut r.internal_resolution)
            });
            row(ui, "Pixel scale", |ui| {
                ui.add(DragValue::new(&mut r.pixel_scale).range(0..=32))
                    .on_hover_text("Window divided by this instead of the internal resolution");
            });
            row(ui, "Letterbox", |ui| {
                ui.checkbox(&mut r.letterbox, "")
                    .on_hover_text("Keep the internal aspect with black bars");
            });
        });
        ui.weak(
            "Internal resolution and pixel scale show only in the game window on Full-tier \
             GPUs (not the editor viewport, not OpenGL); everything else shows in both. \
             The fog hides the draw distance: set it to the background colour.",
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
        let mut bloom = post.bloom.is_some();
        if ui
            .checkbox(&mut bloom, "Bloom")
            .on_hover_text("Glow around areas brighter than the threshold")
            .changed()
        {
            post.bloom = bloom.then(BloomDesc::default);
        }
        if let Some(b) = &mut post.bloom {
            grid(ui, "post-bloom", |ui| {
                row(ui, "Threshold", |ui| {
                    drag(ui, &mut b.threshold, 0.0..=8.0, 0.01, "")
                });
                row(ui, "Intensity", |ui| {
                    drag(ui, &mut b.intensity, 0.0..=4.0, 0.01, "")
                });
            });
        }
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

/// What the Look selector remembers between frames.
#[derive(Clone, Default)]
struct LookUi {
    /// `builtin:<name>` or an asset path; empty = nothing picked yet.
    picked: String,
    /// The environment before / after the last Apply, for the compare toggle.
    compare: Option<(Environment, Environment, bool)>,
    /// Name being typed for "Save current as Look…" (`Some` = the row is open).
    naming: Option<String>,
}

/// The looks the selector offers: the built-in ones, then the project's.
fn look_choices(state: &EditorState) -> Vec<(String, String)> {
    let mut all: Vec<(String, String)> = Vec::new();
    // The project's own looks first: they are the ones being worked on.
    for path in &state.materials.looks {
        let name = path.rsplit('/').next().unwrap_or(path);
        let name = name.strip_suffix(".look.ron").unwrap_or(name);
        all.push((path.clone(), format!("{name}  ({path})")));
    }
    all.extend(LookDesc::built_in().into_iter().map(|l| {
        (
            format!("builtin:{}", l.name),
            format!("{} (built-in)", l.name),
        )
    }));
    all
}

fn load_look(state: &EditorState, key: &str) -> Result<LookDesc, String> {
    if let Some(name) = key.strip_prefix("builtin:") {
        return LookDesc::built_in()
            .into_iter()
            .find(|l| l.name == name)
            .ok_or_else(|| format!("no built-in look {name}"));
    }
    let assets = state.assets_dir().ok_or("no project")?;
    LookDesc::load(assets.join(key))
}

/// Settings → Rendering: pick a picture look (a named set of exposure,
/// tonemapping, bloom, grading and fog settings) and apply it to the scene.
/// Applying is an ordinary scene edit, so it is undoable.
pub(crate) fn look_ui(ui: &mut egui::Ui, state: &mut EditorState) {
    let id = ui.id().with("look-ui");
    let mut ui_state: LookUi = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let choices = look_choices(state);
    section(ui, "🎞 Look", None, |ui| {
        let shown = choices
            .iter()
            .find(|(k, _)| *k == ui_state.picked)
            .map_or("(pick a look)", |(_, label)| label.as_str());
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("look-pick")
                .width(240.0)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (key, label) in &choices {
                        ui.selectable_value(&mut ui_state.picked, key.clone(), label);
                    }
                });
            let can_apply = !ui_state.picked.is_empty();
            if ui
                .add_enabled(can_apply, egui::Button::new("Apply"))
                .on_hover_text(
                    "Sets exposure, tonemapping, bloom, grading and fog; lighting and sky stay",
                )
                .clicked()
            {
                match load_look(state, &ui_state.picked) {
                    Ok(look) => {
                        let before = state.scene.environment.clone();
                        look.apply_to(&mut state.scene.environment);
                        ui_state.compare = Some((before, state.scene.environment.clone(), true));
                    }
                    Err(e) => state.warn(format!("look: {e}")),
                }
            }
            if ui
                .button("Save current as Look…")
                .on_hover_text("Writes the scene's current look to assets/looks/")
                .clicked()
            {
                ui_state.naming.get_or_insert_with(|| "My Look".into());
            }
        });
        let mut done = false;
        if let Some(name) = &mut ui_state.naming {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(name).desired_width(160.0));
                let ok = !name.trim().is_empty() && state.assets_dir().is_some();
                if ui.add_enabled(ok, egui::Button::new("Save")).clicked() {
                    let look = LookDesc::from_environment(name.trim(), &state.scene.environment);
                    match save_look(state, &look) {
                        Ok(rel) => {
                            state.info(format!("saved look {rel}"));
                            ui_state.picked = rel;
                        }
                        Err(e) => state.warn(format!("look: {e}")),
                    }
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        }
        if done {
            ui_state.naming = None;
        }
        if let Some((before, after, showing_after)) = &mut ui_state.compare {
            ui.horizontal(|ui| {
                ui.label("Compare");
                let mut now = *showing_after;
                ui.selectable_value(&mut now, false, "Before");
                ui.selectable_value(&mut now, true, "After");
                if now != *showing_after {
                    *showing_after = now;
                    state.scene.environment = if now { after.clone() } else { before.clone() };
                }
            });
        }
        ui.weak(
            "Picture filters only change the camera and post part of the environment. \
             Bloom, grading, LUT, FXAA and AO show in the game window on Full-tier GPUs, \
             not in the editor viewport.",
        );
    });
    ui.data_mut(|d| d.insert_temp(id, ui_state));
}

/// Writes `look` as `assets/looks/<name>.look.ron` (a free name); returns the asset path.
fn save_look(state: &mut EditorState, look: &LookDesc) -> Result<String, String> {
    let assets = state.assets_dir().ok_or("no project")?;
    let stem: String = look
        .name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let stem = stem.trim_matches('-');
    let stem = if stem.is_empty() { "look" } else { stem };
    let mut rel = format!("looks/{stem}.look.ron");
    let mut n = 2;
    while assets.join(&rel).exists() {
        rel = format!("looks/{stem}-{n}.look.ron");
        n += 1;
    }
    look.save(assets.join(&rel))?;
    state.materials.forget(&|_| false);
    Ok(rel)
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

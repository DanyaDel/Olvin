//! The Settings window (Edit → Settings…, Ctrl+, or ⚙ in the top bar): all
//! rendering in one place, camera controls, interface and the keymap.

use egui::{RichText, Slider};

use crate::EditorState;
use crate::panels::inspector::{grid, row, section};
use crate::panels::lighting::{environment_ui, presets};
use crate::prefs::Preferences;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Rendering,
    Camera,
    Interface,
    Keymap,
}

impl Page {
    const ALL: [(Page, &'static str); 4] = [
        (Page::Rendering, "☀  Rendering"),
        (Page::Camera, "🎥  Scene Camera"),
        (Page::Interface, "▣  Interface"),
        (Page::Keymap, "⌨  Keymap"),
    ];
}

/// Draws the window while it's open; saves preferences when they change.
pub(crate) fn window(ctx: &egui::Context, state: &mut EditorState) {
    let Some(mut page) = state.settings_page else {
        return;
    };
    let before = state.prefs.clone();
    let mut open = true;
    egui::Window::new("⚙ Settings")
        .open(&mut open)
        .default_size([780.0, 560.0])
        .min_size([560.0, 360.0])
        .collapsible(false)
        .show(ctx, |ui| {
            egui::Panel::left("settings-pages")
                .resizable(false)
                .exact_size(170.0)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    for (p, label) in Page::ALL {
                        let text = RichText::new(label).size(14.0);
                        if ui
                            .add_sized([160.0, 28.0], egui::Button::selectable(page == p, text))
                            .clicked()
                        {
                            page = p;
                        }
                    }
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        if ui.button("Reset editor settings").clicked() {
                            state.prefs = Preferences::default();
                        }
                    });
                });
            egui::CentralPanel::default().show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| match page {
                        Page::Rendering => rendering(ui, state),
                        Page::Camera => camera(ui, &mut state.prefs),
                        Page::Interface => interface(ui, &mut state.prefs),
                        Page::Keymap => keymap(ui),
                    });
            });
        });
    state.settings_page = open.then_some(page);
    if state.prefs != before
        && let Err(e) = state.prefs.save()
    {
        state.warn(format!("settings: {e}"));
    }
}

fn heading(ui: &mut egui::Ui, text: &str, sub: &str) {
    ui.label(RichText::new(text).heading());
    ui.weak(sub);
    ui.add_space(6.0);
}

fn rendering(ui: &mut egui::Ui, state: &mut EditorState) {
    heading(
        ui,
        "Scene",
        "Saved in the scene: the game looks exactly like this.",
    );
    presets(ui, state);
    environment_ui(ui, &mut state.scene.environment);
    ui.add_space(8.0);
    heading(
        ui,
        "Editor views",
        "Only the editor's Scene and Game views; the built game is unaffected.",
    );
    let p = &mut state.prefs;
    section(ui, "Viewport quality", None, |ui| {
        ui.checkbox(&mut p.viewport_shadows, "Shadows");
        ui.checkbox(&mut p.viewport_fog, "Fog");
        ui.checkbox(
            &mut p.show_stats,
            "Render stats overlay (draw calls, objects, frame time)",
        );
        ui.horizontal(|ui| {
            ui.label("New views start in");
            ui.selectable_value(&mut p.solid_by_default, false, "☀ Rendered");
            ui.selectable_value(&mut p.solid_by_default, true, "◐ Solid");
        });
    });
}

fn camera(ui: &mut egui::Ui, p: &mut Preferences) {
    heading(
        ui,
        "Scene Camera",
        "RMB look + WASD/QE fly, Alt+LMB orbit, MMB pan, wheel zoom, F frame.",
    );
    section(ui, "Navigation", None, |ui| {
        grid(ui, "camera", |ui| {
            row(ui, "Look sensitivity", |ui| {
                ui.add(Slider::new(&mut p.look_sensitivity, 0.2..=3.0));
            });
            row(ui, "Invert Y", |ui| {
                ui.checkbox(&mut p.invert_y, "");
            });
            row(ui, "Fly speed", |ui| {
                ui.add(
                    Slider::new(&mut p.fly_speed, 0.5..=50.0)
                        .logarithmic(true)
                        .suffix(" m/s"),
                );
            });
            row(ui, "Zoom speed", |ui| {
                ui.add(Slider::new(&mut p.zoom_speed, 0.2..=4.0));
            });
            row(ui, "Field of view", |ui| {
                ui.add(Slider::new(&mut p.fov_deg, 20.0..=110.0).suffix("°"));
            });
        });
    });
}

fn interface(ui: &mut egui::Ui, p: &mut Preferences) {
    heading(ui, "Interface", "Theme: Olvin Night.");
    section(ui, "Scale", None, |ui| {
        ui.add(
            Slider::new(&mut p.ui_scale, 0.75..=2.0)
                .step_by(0.05)
                .text("UI scale"),
        );
        ui.horizontal(|ui| {
            for s in [0.9, 1.0, 1.25, 1.5] {
                if ui.button(format!("{}%", (s * 100.0) as u32)).clicked() {
                    p.ui_scale = s;
                }
            }
        });
    });
}

const KEYS: [(&str, &[(&str, &str)]); 4] = [
    (
        "Scene",
        &[
            ("Q / W / E / R", "Hand / Move / Rotate / Scale"),
            ("X", "Global / Local gizmo"),
            ("F", "Frame selection"),
            ("Shift+Z", "Solid / Rendered"),
            ("Ctrl+click", "Add to / remove from selection"),
        ],
    ),
    (
        "Edit Mode (Tab)",
        &[
            ("1 / 2 / 3", "Vertex / Edge / Face"),
            (
                "G / R / S",
                "Move / rotate / scale; X Y Z axis, type a value",
            ),
            ("E / I", "Extrude / Inset"),
            ("Ctrl+R / K", "Loop cut / Knife"),
            ("F / J", "Make face / Connect vertices"),
            ("X / M", "Delete / Merge"),
            ("Alt+click", "Edge or face loop"),
            ("Alt+Z", "X-ray"),
            ("H / Alt+H", "Hide / Reveal"),
        ],
    ),
    ("CSG", &[("B", "Draw brush"), ("T", "Resize brush faces")]),
    (
        "Editor",
        &[
            ("Ctrl+S", "Save scene"),
            ("Ctrl+Z / Ctrl+Shift+Z", "Undo / Redo"),
            ("Ctrl+D / Del", "Duplicate / Delete"),
            ("Ctrl+P", "Play / Stop"),
            ("Ctrl+,", "Settings"),
        ],
    ),
];

fn keymap(ui: &mut egui::Ui) {
    heading(
        ui,
        "Keymap",
        "Unity-style object keys, Blender-style Edit Mode.",
    );
    for (title, keys) in KEYS {
        section(ui, title, None, |ui| {
            egui::Grid::new(title)
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for (k, what) in keys {
                        ui.label(RichText::new(*k).monospace().strong());
                        ui.label(*what);
                        ui.end_row();
                    }
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_draws_headless() {
        let mut state = EditorState::new(None);
        let ctx = egui::Context::default();
        for (page, _) in Page::ALL {
            state.settings_page = Some(page);
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 800.0),
                    )),
                    ..Default::default()
                };
                ctx.run_ui(input, |ui| window(&ui.ctx().clone(), &mut state))
                    .textures_delta
                    .clear();
            }
            assert_eq!(state.settings_page, Some(page), "window stays open");
        }
    }
}

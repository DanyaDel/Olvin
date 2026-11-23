//! 2D overlays drawn over the 3D image: toolbar, scene (view) gizmo,
//! light / audio icons, stats and hints.

use std::f32::consts::{FRAC_PI_2, PI};

use egui::{Align2, Color32, FontId, PointerButton, Pos2, Sense, Stroke, vec2};
use glam::Vec3;
use olvin_scene::file::LightKind;

use super::{AUDIO_GIZMO, AXES, HOT, LIGHT_GIZMO, STEPS, Viewport, has_icon};
use crate::ext::{ViewCamera, linear_color};
use crate::state::tools;
use crate::{EditorState, theme};

impl Viewport {
    /// Unity's scene gizmo: click an axis to look along it, the centre toggles
    /// perspective / orthographic.
    fn view_gizmo(&mut self, ui: &mut egui::Ui, rect: egui::Rect, camera: &ViewCamera) {
        let center = rect.right_top() + vec2(-52.0, 58.0);
        let area = egui::Rect::from_center_size(center, vec2(96.0, 96.0));
        let response = ui.interact(area, egui::Id::new(("view-gizmo", self.id)), Sense::click());
        let pointer = response.hover_pos();
        let painter = ui.painter_at(rect);
        painter.circle_filled(
            center,
            40.0,
            Color32::from_black_alpha(if response.hovered() { 70 } else { 40 }),
        );

        // (direction, axis index, positive); camera ends up on that side looking at the target.
        let mut tips: Vec<(Vec3, usize, bool, Vec3)> = Vec::new();
        for (i, axis) in AXES.iter().enumerate() {
            for positive in [true, false] {
                let dir = if positive { *axis } else { -*axis };
                tips.push((dir, i, positive, camera.view.transform_vector3(dir)));
            }
        }
        tips.sort_by(|a, b| a.3.z.total_cmp(&b.3.z));
        let mut clicked_axis = None;
        for (dir, i, positive, v) in &tips {
            let tip = center + vec2(v.x, -v.y) * 28.0;
            let hot = pointer.is_some_and(|p| p.distance(tip) < 9.0);
            let color = if hot { HOT } else { theme::AXES[*i] };
            if *positive {
                painter.line_segment([center, tip], (2.0, color));
                painter.circle_filled(tip, 8.5, color);
                painter.text(
                    tip,
                    Align2::CENTER_CENTER,
                    ["X", "Y", "Z"][*i],
                    FontId::proportional(11.0),
                    Color32::BLACK,
                );
            } else {
                painter.circle(tip, 6.0, Color32::from_gray(60), (1.5, color));
            }
            if hot && response.clicked() {
                clicked_axis = Some(*dir);
            }
        }
        let center_hot = pointer.is_some_and(|p| p.distance(center) < 8.0);
        painter.circle_filled(
            center,
            6.0,
            if center_hot {
                HOT
            } else {
                Color32::from_gray(200)
            },
        );
        painter.text(
            center + vec2(0.0, 50.0),
            Align2::CENTER_CENTER,
            if self.orthographic {
                "◀ Iso"
            } else {
                "◀ Persp"
            },
            FontId::proportional(11.0),
            Color32::from_white_alpha(170),
        );

        if let Some(dir) = clicked_axis {
            (self.orbit.yaw, self.orbit.pitch) = match dir {
                d if d == Vec3::X => (FRAC_PI_2, 0.0),
                d if d == -Vec3::X => (-FRAC_PI_2, 0.0),
                d if d == Vec3::Y => (0.0, 1.55),
                d if d == -Vec3::Y => (0.0, -1.55),
                d if d == Vec3::Z => (0.0, 0.0),
                _ => (PI, 0.0),
            };
        } else if response.clicked() && center_hot {
            self.orthographic = !self.orthographic;
        }
    }

    pub(super) fn overlay(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        camera: &ViewCamera,
        state: &mut EditorState,
        grid_step: f32,
    ) {
        // The toolbar wraps before the view gizmo in the top right corner.
        let mut bar_rect = rect.shrink(6.0);
        bar_rect.max.x = bar_rect
            .max
            .x
            .min(rect.right() - 108.0)
            .max(bar_rect.min.x + 160.0);
        let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(bar_rect));
        self.toolbar_bottom = egui::Frame::new()
            .fill(Color32::from_black_alpha(110))
            .corner_radius(5.0)
            .inner_margin(egui::Margin::symmetric(6, 3))
            .show(&mut bar, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (id, label, key) in [
                        (tools::VIEW, "Hand", "Q"),
                        (tools::MOVE, "Move", "W"),
                        (tools::ROTATE, "Rotate", "E"),
                        (tools::SCALE, "Scale", "R"),
                    ] {
                        if ui
                            .selectable_label(state.tool == id, label)
                            .on_hover_text(format!("{label} ({key})"))
                            .clicked()
                        {
                            state.tool = id.to_owned();
                        }
                    }
                    let ext_tools: Vec<_> = state
                        .ext
                        .viewport
                        .iter()
                        .flat_map(|e| e.tools().iter().copied())
                        .collect();
                    if !ext_tools.is_empty() {
                        ui.separator();
                    }
                    for t in ext_tools {
                        let hint = match t.key {
                            Some(k) => format!("{} ({})", t.tooltip, k.name()),
                            None => t.tooltip.to_owned(),
                        };
                        if ui
                            .selectable_label(state.tool == t.id, t.label)
                            .on_hover_text(hint)
                            .clicked()
                        {
                            state.tool = t.id.to_owned();
                        }
                    }
                    ui.separator();
                    let space = if state.local_space { "Local" } else { "Global" };
                    if ui.button(space).on_hover_text("Gizmo axes (X)").clicked() {
                        state.local_space = !state.local_space;
                    }
                    ui.separator();
                    ui.toggle_value(&mut state.snap.enabled, "Snap")
                        .on_hover_text("Grid snapping (hold Ctrl to invert)");
                    let step_label = |s: f32| {
                        if s < 1.0 {
                            format!("1/{}", (1.0 / s).round())
                        } else {
                            format!("{s}")
                        }
                    };
                    egui::ComboBox::from_id_salt(("snap-step", self.id))
                        .width(78.0)
                        .selected_text(match state.snap.step {
                            None => format!("Auto {}", step_label(grid_step)),
                            Some(s) => step_label(s),
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut state.snap.step, None, "Auto (adaptive)");
                            for s in STEPS {
                                ui.selectable_value(&mut state.snap.step, Some(s), step_label(s));
                            }
                        });
                    egui::ComboBox::from_id_salt(("snap-angle", self.id))
                        .width(48.0)
                        .selected_text(format!("{}°", state.snap.angle_deg))
                        .show_ui(ui, |ui| {
                            for a in [1.0, 5.0, 15.0, 30.0, 45.0, 90.0] {
                                ui.selectable_value(&mut state.snap.angle_deg, a, format!("{a}°"));
                            }
                        });
                    ui.separator();
                    ui.toggle_value(&mut self.show_grid, "Grid");
                    ui.toggle_value(&mut self.wireframe, "Wire");
                    ui.separator();
                    use crate::engine::Shading;
                    for (mode, label, tip) in [
                        (
                            Shading::Solid,
                            "◐ Solid",
                            "Studio light from the view, scene lights off (Shift+Z)",
                        ),
                        (
                            Shading::Rendered,
                            "☀ Rendered",
                            "Scene lights, shadows, environment and fog (Shift+Z)",
                        ),
                    ] {
                        ui.selectable_value(&mut self.shading, mode, label)
                            .on_hover_text(tip);
                    }
                    if ui
                        .button("Frame")
                        .on_hover_text("Frame selection (F)")
                        .clicked()
                    {
                        state.frame_selection = true;
                    }
                    model_preview_button(ui, state);
                });
            })
            .response
            .rect
            .bottom();

        let painter = ui.painter_at(rect);
        draw_icons(&painter, camera, state);
        self.view_gizmo(ui, rect, camera);
        if self.prefs.show_stats
            && let Some(stats) = state.render_stats.get(&self.id)
        {
            let dt = ui.input(|i| i.stable_dt).max(1e-4);
            let text = format!(
                "{} draw calls · {} objects · {} culled · {:.1} ms ({:.0} fps)",
                stats.draw_calls,
                stats.instances,
                stats.culled,
                dt * 1000.0,
                1.0 / dt
            );
            let galley = painter.layout_no_wrap(
                text,
                FontId::monospace(11.5),
                Color32::from_white_alpha(220),
            );
            let at = rect.right_bottom() - vec2(galley.size().x + 14.0, galley.size().y + 12.0);
            painter.rect_filled(
                egui::Rect::from_min_size(at - vec2(6.0, 3.0), galley.size() + vec2(12.0, 6.0)),
                6.0,
                Color32::from_black_alpha(120),
            );
            painter.galley(at, galley, Color32::WHITE);
        }
        if ui.input(|i| i.pointer.button_down(PointerButton::Secondary)) && self.fly_speed != 1.0 {
            painter.text(
                rect.center_bottom() + vec2(0.0, -28.0),
                Align2::CENTER_CENTER,
                format!("fly speed ×{:.2}", self.fly_speed),
                FontId::proportional(12.0),
                Color32::from_white_alpha(170),
            );
        }
        if !state.preview.is_empty() {
            painter.text(
                rect.center_top() + vec2(0.0, 44.0),
                Align2::CENTER_CENTER,
                "• animation preview",
                FontId::proportional(13.0),
                theme::SELECTED,
            );
        }
        painter.text(
            rect.left_bottom() + vec2(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            "Alt+LMB orbit · MMB pan · RMB look + WASD/QE fly · Alt+RMB / wheel zoom · F frame",
            FontId::proportional(11.0),
            Color32::from_white_alpha(90),
        );
    }
}

/// A light's icon: a disc in the light's colour with a glyph for its type,
/// plus a short ray showing where directional and spot lights point.
fn light_icon(
    painter: &egui::Painter,
    camera: &ViewCamera,
    p: Pos2,
    pos: Vec3,
    forward: Vec3,
    light: &olvin_scene::file::LightDesc,
    selected: bool,
) {
    let tint = linear_color([light.color[0], light.color[1], light.color[2], 1.0]);
    let ring = if selected {
        theme::SELECTED
    } else {
        Color32::from_gray(20)
    };
    let glyph = Color32::from_gray(25);
    if light.kind != LightKind::Point
        && let Some(tip) = camera.project(pos + forward * camera.world_per_point(pos) * 34.0)
    {
        let dir = (tip - p).normalized();
        painter.line_segment(
            [p + dir * 11.0, tip],
            (2.0, LIGHT_GIZMO.gamma_multiply(0.8)),
        );
        painter.circle_filled(tip, 2.5, LIGHT_GIZMO);
    }
    if light.kind == LightKind::Directional {
        for i in 0..8 {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            let d = vec2(a.cos(), a.sin());
            painter.line_segment([p + d * 12.0, p + d * 16.0], (2.0, tint));
        }
    }
    painter.circle(p, 9.5, tint, (if selected { 2.0 } else { 1.5 }, ring));
    match light.kind {
        LightKind::Directional => {
            painter.circle_filled(p, 3.5, glyph);
        }
        LightKind::Point => {
            painter.circle_stroke(p - vec2(0.0, 1.5), 3.5, (1.5, glyph));
            painter.line_segment([p + vec2(-2.0, 4.0), p + vec2(2.0, 4.0)], (1.5, glyph));
        }
        LightKind::Spot => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    p + vec2(-2.0, -4.5),
                    p + vec2(2.0, -4.5),
                    p + vec2(4.5, 3.5),
                    p + vec2(-4.5, 3.5),
                ],
                glyph,
                Stroke::NONE,
            ));
        }
    }
}

/// Unity-style icons for things without a surface.
fn draw_icons(painter: &egui::Painter, camera: &ViewCamera, state: &EditorState) {
    let worlds = state.scene.world_matrices();
    for e in &state.scene.entities {
        if !has_icon(e) || e.camera.is_some() || !state.scene.active_in_hierarchy(e.id) {
            continue;
        }
        let Some(p) = worlds
            .get(&e.id)
            .and_then(|m| camera.project(m.w_axis.truncate()))
        else {
            continue;
        };
        let selected = state.selection == Some(e.id);
        if let Some(light) = &e.light {
            let forward = worlds
                .get(&e.id)
                .map(|m| -m.z_axis.truncate().normalize_or_zero())
                .unwrap_or(Vec3::NEG_Z);
            let pos = worlds[&e.id].w_axis.truncate();
            light_icon(painter, camera, p, pos, forward, light, selected);
        } else if e.audio_source.is_some() {
            let color = if selected {
                theme::SELECTED
            } else {
                AUDIO_GIZMO
            };
            painter.rect_filled(
                egui::Rect::from_center_size(p - vec2(4.0, 0.0), vec2(5.0, 7.0)),
                1.0,
                color,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    p + vec2(-2.0, -3.5),
                    p + vec2(5.0, -8.0),
                    p + vec2(5.0, 8.0),
                    p + vec2(-2.0, 3.5),
                ],
                color,
                Stroke::NONE,
            ));
        }
    }
}

/// "▶ Preview" for the selected model with animation clips: plays the clip
/// chosen in the inspector on it in this view (not saved, not an undo step).
fn model_preview_button(ui: &mut egui::Ui, state: &mut EditorState) {
    if state.is_playing() {
        return;
    }
    let Some(id) = state.selection else {
        return;
    };
    let Some(path) = state
        .scene
        .get(id)
        .and_then(|e| e.model.as_ref())
        .map(|m| m.path.clone())
    else {
        return;
    };
    let assets = state.assets_dir();
    if state.engine_scene.model_clips(assets, &path).is_empty() {
        return;
    }
    ui.separator();
    let preview = state.model_previews.entry(id).or_default();
    let label = if preview.playing {
        "■ Stop"
    } else {
        "▶ Preview"
    };
    if ui
        .selectable_label(preview.playing, label)
        .on_hover_text("Play the model's animation here (clip and speed in the inspector)")
        .clicked()
    {
        preview.playing = !preview.playing;
    }
}

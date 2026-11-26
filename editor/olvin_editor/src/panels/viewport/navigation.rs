//! Camera and navigation: the orbit camera, Unity scene-view controls and
//! the viewport hotkeys.

use egui::{PointerButton, Vec2};
use glam::camera::rh::{proj::directx, view};
use glam::{Mat4, Vec3};

use super::{STEPS, Viewport};
use crate::EditorState;
use crate::ext::ViewCamera;
use crate::keymap::Command;
use crate::state::tools;

const NEAR: f32 = 0.05;
const FAR: f32 = 4000.0;

impl Viewport {
    fn eye_height(&self) -> f32 {
        self.orbit
            .eye()
            .y
            .abs()
            .max(self.orbit.distance * 0.3)
            .max(0.05)
    }

    /// Power-of-two grid step that keeps roughly the same on-screen density.
    pub(super) fn grid_step(&self) -> f32 {
        let raw = self.eye_height() / 14.0;
        2f32.powf(raw.log2().round()).clamp(STEPS[0], 64.0)
    }

    pub(super) fn fade_distance(&self) -> f32 {
        (self.eye_height() * 60.0).clamp(30.0, FAR * 0.8)
    }

    pub(super) fn camera(&self, rect: egui::Rect) -> ViewCamera {
        let eye = self.orbit.eye();
        // Pitch is clamped short of ±90°, so Y stays a valid up vector.
        let view = view::look_at_mat4(eye, self.orbit.target, Vec3::Y);
        let aspect = rect.width() / rect.height();
        let proj = if self.orthographic {
            let h = self.orbit.distance * (self.prefs.fov_deg.to_radians() * 0.5).tan();
            directx::orthographic(-h * aspect, h * aspect, -h, h, NEAR, FAR)
        } else {
            directx::perspective(self.prefs.fov_deg.to_radians(), aspect, NEAR, FAR)
        };
        ViewCamera {
            view,
            proj,
            view_proj: proj * view,
            eye,
            forward: -self.orbit.back(),
            rect,
            near: NEAR,
            orthographic: self.orthographic,
        }
    }

    pub(super) fn hotkeys(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        state: &mut EditorState,
    ) {
        let flying = ui.input(|i| i.pointer.button_down(PointerButton::Secondary));
        if !response.hovered()
            || flying
            || ui.ctx().egui_wants_keyboard_input()
            || self.drag.is_some()
            // A tool such as Edit Mode uses plain keys (E, R, X, F...) itself.
            || state.tool_owns_keys
        {
            return;
        }
        if state.capturing_key {
            return;
        }
        let keys = state.prefs.keymap.clone();
        let pressed = |k| ui.input(|i| i.key_pressed(k) && i.modifiers.is_none());
        let command = |c| ui.input_mut(|i| keys.pressed(i, c));
        if command(Command::ToggleShading) {
            use crate::engine::Shading;
            self.shading = if self.shading == Shading::Solid {
                Shading::Rendered
            } else {
                Shading::Solid
            };
        }
        for (cmd, tool) in [
            (Command::ToolView, tools::VIEW),
            (Command::ToolMove, tools::MOVE),
            (Command::ToolRotate, tools::ROTATE),
            (Command::ToolScale, tools::SCALE),
        ] {
            if command(cmd) {
                state.tool = tool.to_owned();
            }
        }
        for slot in &state.ext.viewport {
            for t in slot.tools() {
                if t.key.is_some_and(pressed) {
                    state.tool = t.id.to_owned();
                }
            }
        }
        if command(Command::ToggleSpace) {
            state.local_space = !state.local_space;
        }
        if command(Command::FrameSelection) {
            state.frame_selection = true;
        }
        // Blender: I keys Location, Rotation and Scale of the selection at
        // the current frame, Alt+I removes the keys on that frame.
        // Delete keys first: its chord has Alt, the other's is a bare key.
        let alt_i = command(Command::DeleteKeys);
        if command(Command::KeyTransform) || alt_i {
            let frame = state.anim_frame as f32;
            let ids = state.selected_ids();
            let mut count = 0;
            for id in &ids {
                count += if alt_i {
                    state.scene.animation.delete_keys_at(*id, None, frame)
                } else {
                    state.scene.insert_keys(*id, frame, Default::default())
                };
            }
            if count > 0 {
                let verb = if alt_i { "deleted" } else { "inserted" };
                state.info(format!("{verb} {count} keys at frame {}", state.anim_frame));
            }
        }
    }

    /// Unity scene-view controls: MMB pan, Alt+LMB orbit, Alt+RMB zoom,
    /// RMB look + WASD/QE fly (scroll changes speed), wheel zoom, F frame.
    pub(super) fn navigate(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        state: &mut EditorState,
        worlds: &std::collections::HashMap<u64, Mat4>,
    ) {
        let delta = response.drag_delta();
        let alt = ui.input(|i| i.modifiers.alt);
        let middle = response.dragged_by(PointerButton::Middle);
        let right = response.dragged_by(PointerButton::Secondary);
        let left = response.dragged_by(PointerButton::Primary);
        let hand = left && state.tool == tools::VIEW && !alt;

        if middle || hand {
            let view = view::look_at_mat4(self.orbit.eye(), self.orbit.target, Vec3::Y);
            let (r, u) = (view.row(0).truncate(), view.row(1).truncate());
            let scale = self.orbit.distance * 0.0016;
            self.orbit.target += (-r * delta.x + u * delta.y) * scale;
        } else if left && alt {
            let (k, dy) = self.look(delta);
            self.orbit.yaw -= delta.x * 0.008 * k;
            self.orbit.pitch = (self.orbit.pitch + dy * 0.008 * k).clamp(-1.55, 1.55);
        } else if right && alt {
            self.zoom((delta.x - delta.y) * 0.01);
        } else if right {
            let eye = self.orbit.eye();
            let (k, dy) = self.look(delta);
            self.orbit.yaw -= delta.x * 0.005 * k;
            self.orbit.pitch = (self.orbit.pitch + dy * 0.005 * k).clamp(-1.55, 1.55);
            self.orbit.target = eye - self.orbit.back() * self.orbit.distance;
        }

        let flying = ui.input(|i| i.pointer.button_down(PointerButton::Secondary)) && !alt;
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if flying && (response.hovered() || right) {
            let (dt, boost) = ui.input(|i| {
                (
                    i.stable_dt.min(0.1),
                    if i.modifiers.shift { 4.0 } else { 1.0 },
                )
            });
            if scroll != 0.0 {
                self.fly_speed = (self.fly_speed * (scroll * 0.004).exp()).clamp(0.05, 50.0);
            }
            let forward = -self.orbit.back();
            let right_v = forward.cross(Vec3::Y).normalize_or_zero();
            let axis = |a: egui::Key, b: egui::Key| {
                ui.input(|i| i.key_down(a) as i32 as f32 - i.key_down(b) as i32 as f32)
            };
            let mv = forward * axis(egui::Key::W, egui::Key::S)
                + right_v * axis(egui::Key::D, egui::Key::A)
                + Vec3::Y * axis(egui::Key::E, egui::Key::Q);
            if mv != Vec3::ZERO {
                self.orbit.target +=
                    mv.normalize() * self.prefs.fly_speed * self.fly_speed * boost * dt;
                ui.ctx().request_repaint();
            }
        } else if scroll != 0.0 && response.hovered() {
            self.zoom(-scroll * 0.0015 * self.prefs.zoom_speed);
        }

        if state.frame_selection {
            // The request is consumed even with nothing to frame, so it does
            // not fire on the next selection.
            state.frame_selection = false;
            if let Some(sel) = state.selection
                && let Some(m) = worlds.get(&sel)
            {
                let (scale, _, translation) = m.to_scale_rotation_translation();
                self.orbit.target = translation;
                self.orbit.distance = (scale.max_element() * 2.5).clamp(2.0, 800.0);
            }
        }
    }

    /// Look sensitivity and the (optionally inverted) vertical delta.
    fn look(&self, delta: Vec2) -> (f32, f32) {
        let dy = if self.prefs.invert_y {
            -delta.y
        } else {
            delta.y
        };
        (self.prefs.look_sensitivity, dy)
    }

    fn zoom(&mut self, amount: f32) {
        self.orbit.distance = (self.orbit.distance * amount.exp()).clamp(0.1, FAR * 0.5);
    }
}

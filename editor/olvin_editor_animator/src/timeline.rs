//! The Timeline: playback controls, current frame, start / end / fps, and a
//! strip of the selected objects' keyframes under the frame ruler.

use olvin_editor::egui::{self, Color32, DragValue, Rect, Sense, pos2, vec2};
use olvin_editor::{EditorState, Panel};

use crate::clock::{self, Dir};
use crate::ops;
use crate::view::{self, Axis, KEY_IDLE, KEY_SELECTED, RULER_H};

/// Frame rates Blender offers as presets.
const FPS_PRESETS: [f32; 8] = [12.0, 15.0, 24.0, 25.0, 30.0, 50.0, 60.0, 120.0];

pub struct Timeline {
    view: Axis,
    scrubbing: bool,
    fitted: bool,
    only_selected: bool,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            view: Axis {
                origin: -5.0,
                scale: 4.0,
            },
            scrubbing: false,
            fitted: false,
            only_selected: true,
        }
    }
}

impl Panel for Timeline {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        self.header(ui, state);
        let rect = ui.available_rect_before_wrap();
        if rect.width() < 60.0 || rect.height() < RULER_H + 10.0 {
            return;
        }
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        if !self.fitted {
            self.fitted = true;
            self.fit(state, rect);
        }
        view::navigate_x(ui, &response, rect, &mut self.view, true);
        if response.hovered() {
            let a = &state.scene.animation;
            if ui.input(|i| i.key_pressed(egui::Key::Home)) {
                self.view = Axis::fit(
                    a.frame_start as f32,
                    a.frame_end as f32,
                    rect.width(),
                    20.0,
                    false,
                );
            }
        }

        let ruler_rect = Rect::from_min_size(rect.min, vec2(rect.width(), RULER_H));
        if response.drag_started_by(egui::PointerButton::Primary)
            && ui
                .input(|i| i.pointer.press_origin())
                .is_some_and(|p| ruler_rect.contains(p))
        {
            self.scrubbing = true;
        }
        if self.scrubbing {
            if let Some(p) = response.interact_pointer_pos() {
                state.anim_frame = view::scrub_frame(&self.view, rect, p.x);
            }
            if response.drag_stopped() {
                self.scrubbing = false;
            }
        }
        let curves = ops::visible_curves(state, self.only_selected);
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
        {
            if ruler_rect.contains(p) {
                state.anim_frame = view::scrub_frame(&self.view, rect, p.x);
            } else {
                // Click a key to select it, empty space to deselect.
                let shift = ui.input(|i| i.modifiers.shift);
                let a = &mut state.scene.animation;
                let hit = ops::frames_of(a, &curves)
                    .into_iter()
                    .find(|(f, _)| (self.view.px(*f, rect.left()) - p.x).abs() <= 7.0)
                    .map(|(f, _)| ops::keys_at(a, &curves, f))
                    .unwrap_or_default();
                ops::click_select(a, &hit, shift);
            }
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        let a = &state.scene.animation;
        view::ruler(
            ui,
            &painter,
            ruler_rect,
            rect,
            &self.view,
            (a.frame_start, a.frame_end),
            state.anim_frame,
        );
        let y = rect.top() + RULER_H + (rect.height() - RULER_H) / 2.0;
        let y = y.min(rect.top() + RULER_H + 26.0);
        for (f, sel) in ops::frames_of(a, &curves) {
            let x = self.view.px(f, rect.left());
            if (rect.left()..=rect.right()).contains(&x) {
                view::diamond(
                    &painter,
                    pos2(x, y),
                    5.0,
                    if sel { KEY_SELECTED } else { KEY_IDLE },
                );
            }
        }
        // Redraw the playhead over the keys.
        let x = self.view.px(state.anim_frame as f32, rect.left());
        painter.line_segment(
            [pos2(x, rect.top() + RULER_H), pos2(x, rect.bottom())],
            (1.5, view::PLAYHEAD),
        );
    }
}

impl Timeline {
    fn fit(&mut self, state: &EditorState, rect: Rect) {
        let a = &state.scene.animation;
        self.view = Axis::fit(
            a.frame_start as f32,
            a.frame_end as f32,
            rect.width(),
            20.0,
            false,
        );
    }

    fn header(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        ui.horizontal_wrapped(|ui| {
            let playing = clock::with(|s| s.play.dir);
            let (start, end) = (
                state.scene.animation.frame_start,
                state.scene.animation.frame_end,
            );
            if ui
                .button("⏮")
                .on_hover_text("Jump to start  (Shift+Left)")
                .clicked()
            {
                state.anim_frame = start;
            }
            if ui
                .button("◀◆")
                .on_hover_text("Previous keyframe  (Down)")
                .clicked()
            {
                clock::jump_keyframe(state, false);
            }
            let reverse = if playing == Some(Dir::Reverse) {
                "⏸"
            } else {
                "◀"
            };
            if ui
                .button(reverse)
                .on_hover_text("Play reverse  (Shift+Ctrl+Space)")
                .clicked()
            {
                clock::with(|s| s.play.toggle(Dir::Reverse, state.anim_frame));
            }
            let forward = if playing == Some(Dir::Forward) {
                "⏸"
            } else {
                "▶"
            };
            if ui
                .button(forward)
                .on_hover_text("Play / pause  (Space); Esc stops and returns")
                .clicked()
            {
                clock::with(|s| s.play.toggle(Dir::Forward, state.anim_frame));
            }
            if ui
                .button("◆▶")
                .on_hover_text("Next keyframe  (Up)")
                .clicked()
            {
                clock::jump_keyframe(state, true);
            }
            if ui
                .button("⏭")
                .on_hover_text("Jump to end  (Shift+Right)")
                .clicked()
            {
                state.anim_frame = end;
            }
            ui.separator();
            ui.label("Frame");
            ui.add(DragValue::new(&mut state.anim_frame).speed(0.25));
            let a = &mut state.scene.animation;
            ui.label("Start");
            ui.add(DragValue::new(&mut a.frame_start).speed(0.25));
            ui.label("End");
            ui.add(DragValue::new(&mut a.frame_end).speed(0.25));
            a.frame_end = a.frame_end.max(a.frame_start);
            ui.label("FPS");
            egui::ComboBox::from_id_salt("fps")
                .width(54.0)
                .selected_text(format!("{}", a.fps))
                .show_ui(ui, |ui| {
                    for p in FPS_PRESETS {
                        ui.selectable_value(&mut a.fps, p, format!("{p}"));
                    }
                });
            ui.separator();
            ui.checkbox(&mut self.only_selected, "Only Show Selected");
            ui.menu_button("Game", |ui| {
                ui.checkbox(&mut a.autoplay, "Play on start");
                ui.checkbox(&mut a.looping, "Loop");
            })
            .response
            .on_hover_text("How the running game plays this animation");
            let _ = Color32::WHITE;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_presets_include_blenders_default() {
        assert!(FPS_PRESETS.contains(&24.0));
        assert!(FPS_PRESETS.windows(2).all(|w| w[0] < w[1]));
    }
}

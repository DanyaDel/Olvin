//! The Game view: what the scene's camera sees. While playing, the live game.

use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Align2, Color32, FontId, Sense};
use olvin::render::camera_view_proj;

use crate::render::Frame;
use crate::{EditorState, Panel, theme};

static NEXT_ID: AtomicU64 = AtomicU64::new(1 << 32);

pub struct GameView {
    id: u64,
}

impl Default for GameView {
    fn default() -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl Panel for GameView {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click());
        if rect.width() < 8.0 || rect.height() < 8.0 {
            return;
        }
        // The view is always dark, so its overlays keep the Dark theme's
        // widgets whatever the chrome uses.
        *ui.visuals_mut() = olvin_ui::visuals(&theme::Palette::DARK);
        // The game gets the keyboard and mouse while the pointer is over this view.
        if let Some(play) = &mut state.play {
            let focused = response.hovered() && !ui.ctx().egui_wants_keyboard_input();
            state.panel_owns_edit_keys = response.hovered();
            ui.input(|i| play.feed_input(i, focused));
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::BLACK);
        let aspect = rect.width() / rect.height();
        let frame = ui.ctx().cumulative_frame_nr();
        match crate::engine::prepare_view(
            state,
            self.id,
            frame,
            crate::engine::Shading::Rendered,
            |world| camera_view_proj(world, aspect),
        ) {
            Some(source) => {
                let frame = Frame {
                    viewport: self.id,
                    uniforms: bytemuck::Zeroable::zeroed(),
                    vertices: Vec::new(),
                    lines: Vec::new(),
                    overlay: Vec::new(),
                    grid: false,
                    engine: Some(source),
                };
                painter.add(eframe::egui_wgpu::Callback::new_paint_callback(rect, frame));
            }
            None => {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "No camera in the scene",
                    FontId::proportional(14.0),
                    Color32::GRAY,
                );
            }
        }
        if state.is_playing() {
            ui.ctx().request_repaint();
            painter.text(
                rect.left_bottom() + egui::vec2(8.0, -8.0),
                Align2::LEFT_BOTTOM,
                "Game input: hover this view (WASD, mouse, Space)",
                FontId::proportional(11.0),
                Color32::from_white_alpha(120),
            );
        }
    }
}

//! Editor element: transform keyframe animator. Clips live in the scene file
//! (`extensions["animator"]`), so they save, load and undo with the scene.

use std::collections::BTreeMap;

use olvin_editor::egui::{self, Color32, DragValue, Pos2, Sense, pos2, vec2};
use olvin_editor::egui_dock::{DockState, NodeIndex};
use olvin_editor::glam::Vec3;
use olvin_editor::olvin_scene::Transform;
use olvin_editor::olvin_scene::file::euler_to_quat;
use olvin_editor::{EditorPlugin, EditorState, Panel, PanelKind, Registry, Tab, TabAlloc, WorkspaceTemplate, panels, theme};
use serde::{Deserialize, Serialize};

const EXT: &str = "animator";
const TIMELINE: &str = "animator.timeline";
const NAMES_W: f32 = 150.0;
const ROW_H: f32 = 22.0;
const RULER_H: f32 = 24.0;

pub struct AnimatorPlugin;

impl EditorPlugin for AnimatorPlugin {
    fn name(&self) -> &'static str {
        "Animator"
    }

    fn build(&self, registry: &mut Registry) {
        registry
            .add_panel(PanelKind { id: TIMELINE, title: "⏱ Timeline", factory: || Box::<Timeline>::default() })
            .add_workspace_template(WorkspaceTemplate { name: "Animation", build: animation_layout });
    }
}

fn animation_layout(a: &mut TabAlloc) -> DockState<Tab> {
    let mut dock = DockState::new(vec![a.tab(panels::VIEWPORT)]);
    let tree = dock.main_surface_mut();
    let [center, _] = tree.split_right(NodeIndex::root(), 0.8, vec![a.tab(panels::INSPECTOR)]);
    let [center, _] = tree.split_left(center, 0.2, vec![a.tab(panels::HIERARCHY)]);
    tree.split_below(center, 0.6, vec![a.tab(TIMELINE)]);
    dock
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Clip {
    length: f32,
    /// Entity id -> keys, kept sorted by time.
    tracks: BTreeMap<u64, Vec<Key>>,
}

impl Default for Clip {
    fn default() -> Self {
        Self { length: 2.0, tracks: BTreeMap::new() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Key {
    t: f32,
    position: Vec3,
    rotation: Vec3,
    scale: Vec3,
}

impl Clip {
    fn load(state: &EditorState) -> Self {
        state.scene.extensions.get(EXT).and_then(|text| ron::from_str(text).ok()).unwrap_or_default()
    }

    fn store(&self, state: &mut EditorState) {
        if self.tracks.is_empty() && *self == Self::default() {
            state.scene.extensions.remove(EXT);
        } else if let Ok(text) = ron::to_string(self) {
            state.scene.extensions.insert(EXT.to_owned(), text);
        }
    }
}

/// Linear position/scale, spherical rotation; clamps outside the key range.
fn sample(keys: &[Key], t: f32) -> Option<Transform> {
    let first = keys.iter().min_by(|a, b| a.t.total_cmp(&b.t))?;
    let before = keys.iter().filter(|k| k.t <= t).max_by(|a, b| a.t.total_cmp(&b.t)).unwrap_or(first);
    let after = keys.iter().filter(|k| k.t > t).min_by(|a, b| a.t.total_cmp(&b.t)).unwrap_or(before);
    let span = after.t - before.t;
    let f = if span > 1e-6 { ((t - before.t) / span).clamp(0.0, 1.0) } else { 0.0 };
    Some(Transform {
        translation: before.position.lerp(after.position, f),
        rotation: euler_to_quat(before.rotation).slerp(euler_to_quat(after.rotation), f),
        scale: before.scale.lerp(after.scale, f),
    })
}

struct Timeline {
    time: f32,
    playing: bool,
    looping: bool,
    /// Keep showing the pose at the playhead while paused.
    hold_pose: bool,
    dragging_key: Option<(u64, usize)>,
    scrubbing: bool,
    applied: bool,
}

impl Default for Timeline {
    fn default() -> Self {
        Self { time: 0.0, playing: false, looping: true, hold_pose: false, dragging_key: None, scrubbing: false, applied: false }
    }
}

impl Panel for Timeline {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let mut clip = Clip::load(state);
        let original = clip.clone();

        self.transport(ui, state, &mut clip);
        ui.separator();

        if self.playing {
            self.time += ui.input(|i| i.stable_dt).min(0.1);
            if self.time > clip.length {
                if self.looping {
                    self.time %= clip.length.max(1e-3);
                } else {
                    self.time = clip.length;
                    self.playing = false;
                }
            }
            ui.ctx().request_repaint();
        }
        self.time = self.time.clamp(0.0, clip.length);

        self.tracks(ui, state, &mut clip);

        // Preview pose: while playing, scrubbing, or when asked to hold it.
        if self.playing || self.scrubbing || self.hold_pose {
            state.preview = clip
                .tracks
                .iter()
                .filter_map(|(id, keys)| Some((*id, sample(keys, self.time)?)))
                .collect();
            self.applied = true;
        } else if self.applied {
            state.preview.clear();
            self.applied = false;
        }

        if clip != original {
            clip.store(state);
        }
    }
}

impl Timeline {
    fn transport(&mut self, ui: &mut egui::Ui, state: &mut EditorState, clip: &mut Clip) {
        ui.horizontal(|ui| {
            if ui.button("⏮").on_hover_text("To start").clicked() {
                self.time = 0.0;
            }
            let play = if self.playing { "⏸" } else { "▶" };
            if ui.button(play).on_hover_text("Play / pause").clicked() {
                self.playing = !self.playing;
            }
            if ui.button("⏹").on_hover_text("Stop").clicked() {
                self.playing = false;
                self.time = 0.0;
            }
            ui.add(DragValue::new(&mut self.time).range(0.0..=clip.length).speed(0.01).max_decimals(2).suffix(" s"));
            ui.label("of");
            ui.add(DragValue::new(&mut clip.length).range(0.1..=600.0).speed(0.05).max_decimals(2).suffix(" s"));
            ui.checkbox(&mut self.looping, "Loop");
            ui.checkbox(&mut self.hold_pose, "Hold pose").on_hover_text("Keep showing the pose while paused");
            ui.separator();

            let selected = state.selection.and_then(|id| state.scene.get(id).map(|e| (id, e.clone())));
            let key_button = ui
                .add_enabled(selected.is_some(), egui::Button::new("◆ Key"))
                .on_hover_text("Record the selected object's Transform at the playhead");
            if key_button.clicked()
                && let Some((id, e)) = &selected
            {
                let key = Key { t: self.time, position: e.position, rotation: e.rotation, scale: e.scale };
                let keys = clip.tracks.entry(*id).or_default();
                keys.retain(|k| (k.t - self.time).abs() > 1.0 / 120.0);
                keys.push(key);
                keys.sort_by(|a, b| a.t.total_cmp(&b.t));
                state.info(format!("key {:.2} s: {}", self.time, e.name));
            }
            if let Some((id, _)) = &selected
                && clip.tracks.contains_key(id)
                && ui.button("Delete Track").clicked()
            {
                clip.tracks.remove(id);
            }
        });
    }

    fn tracks(&mut self, ui: &mut egui::Ui, state: &mut EditorState, clip: &mut Clip) {
        let height = (RULER_H + ROW_H * clip.tracks.len().max(1) as f32).max(ui.available_height());
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);

        let x0 = rect.left() + NAMES_W;
        let zoom = ((rect.right() - 12.0 - x0) / clip.length).max(1.0);
        let x_of = |t: f32| x0 + t * zoom;
        let t_of = |x: f32| ((x - x0) / zoom).clamp(0.0, clip.length);

        // Ruler.
        let ruler = egui::Rect::from_min_max(pos2(x0, rect.top()), pos2(rect.right(), rect.top() + RULER_H));
        painter.rect_filled(ruler, 0.0, ui.visuals().panel_fill);
        let step = [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0].into_iter().find(|s| s * zoom >= 40.0).unwrap_or(60.0);
        let mut t = 0.0;
        while t <= clip.length + 1e-4 {
            let x = x_of(t);
            painter.line_segment([pos2(x, ruler.bottom() - 6.0), pos2(x, rect.bottom())], (1.0, Color32::from_white_alpha(14)));
            painter.text(pos2(x + 3.0, ruler.top() + 3.0), egui::Align2::LEFT_TOP, format!("{t:.2}"), egui::FontId::monospace(10.0), ui.visuals().weak_text_color());
            t += step;
        }

        // Rows and keys.
        let pointer = response.hover_pos();
        let mut hovered_key = None;
        if clip.tracks.is_empty() {
            painter.text(
                pos2(x0 + 12.0, ruler.bottom() + 14.0),
                egui::Align2::LEFT_CENTER,
                "Select an object, press ◆ Key, move the playhead, change the object, press ◆ Key again",
                egui::FontId::proportional(12.0),
                ui.visuals().weak_text_color(),
            );
        }
        for (row, (id, keys)) in clip.tracks.iter().enumerate() {
            let y = ruler.bottom() + row as f32 * ROW_H;
            let selected = state.selection == Some(*id);
            if selected {
                painter.rect_filled(
                    egui::Rect::from_min_max(pos2(rect.left(), y), pos2(rect.right(), y + ROW_H)),
                    0.0,
                    theme::ACCENT.gamma_multiply(0.25),
                );
            }
            let name = state.scene.get(*id).map(|e| e.name.as_str()).unwrap_or("(deleted)");
            painter.text(pos2(rect.left() + 8.0, y + ROW_H / 2.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(13.0), ui.visuals().text_color());
            for (i, key) in keys.iter().enumerate() {
                let c = pos2(x_of(key.t), y + ROW_H / 2.0);
                let hot = pointer.is_some_and(|p| p.distance(c) < 7.0);
                if hot {
                    hovered_key = Some((*id, i));
                }
                let color = if hot || self.dragging_key == Some((*id, i)) {
                    Color32::WHITE
                } else if selected {
                    theme::SELECTED
                } else {
                    Color32::from_gray(190)
                };
                diamond(&painter, c, 5.5, color);
            }
        }
        painter.line_segment([pos2(x0, rect.top()), pos2(x0, rect.bottom())], (1.0, Color32::from_gray(70)));

        // Interaction: drag keys, scrub elsewhere, click names to select.
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.dragging_key = hovered_key;
            self.scrubbing = hovered_key.is_none() && pointer.is_some_and(|p| p.x >= x0);
        }
        if let Some(p) = response.interact_pointer_pos() {
            if let Some((id, i)) = self.dragging_key {
                if let Some(key) = clip.tracks.get_mut(&id).and_then(|k| k.get_mut(i)) {
                    key.t = t_of(p.x);
                }
            } else if self.scrubbing {
                self.time = t_of(p.x);
            }
        }
        if response.drag_stopped() {
            if let Some((id, _)) = self.dragging_key.take()
                && let Some(keys) = clip.tracks.get_mut(&id)
            {
                keys.sort_by(|a, b| a.t.total_cmp(&b.t));
            }
            self.scrubbing = false;
        }
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(p) = pointer
        {
            if p.x < x0 {
                let row = ((p.y - ruler.bottom()) / ROW_H).floor();
                if row >= 0.0
                    && let Some(id) = clip.tracks.keys().nth(row as usize)
                {
                    state.selection = Some(*id);
                }
            } else if hovered_key.is_none() {
                self.time = t_of(p.x);
            }
        }
        if response.clicked_by(egui::PointerButton::Secondary)
            && let Some((id, i)) = hovered_key
            && let Some(keys) = clip.tracks.get_mut(&id)
        {
            keys.remove(i);
            if keys.is_empty() {
                clip.tracks.remove(&id);
            }
        }

        // Playhead.
        let x = x_of(self.time);
        painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], (2.0, theme::SELECTED));
        painter.rect_filled(egui::Rect::from_center_size(pos2(x, ruler.top() + 6.0), vec2(10.0, 12.0)), 2.0, theme::SELECTED);
        response.on_hover_text_at_pointer("LMB: set time / drag key · RMB on a key: delete");
    }
}

fn diamond(painter: &egui::Painter, c: Pos2, r: f32, color: Color32) {
    let pts = vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)];
    painter.add(egui::Shape::convex_polygon(pts, color, (1.0, Color32::BLACK)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(t: f32, x: f32) -> Key {
        Key { t, position: Vec3::new(x, 0.0, 0.0), rotation: Vec3::ZERO, scale: Vec3::ONE }
    }

    #[test]
    fn samples_between_and_outside_keys() {
        let keys = [key(0.0, 0.0), key(2.0, 10.0)];
        assert!((sample(&keys, 1.0).unwrap().translation.x - 5.0).abs() < 1e-5);
        assert_eq!(sample(&keys, -1.0).unwrap().translation.x, 0.0);
        assert_eq!(sample(&keys, 5.0).unwrap().translation.x, 10.0);
        assert!(sample(&[], 0.0).is_none());
    }
}

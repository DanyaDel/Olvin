//! Key selection helpers shared by the editors: what a click or a box picks,
//! which curves are visible, popup menus.

use olvin_editor::EditorState;
use olvin_editor::egui::{self, Color32, Pos2};
use olvin_editor::olvin_scene::action::{Action, FRAME_EPS, SEL_ALL};

/// A key by (curve index, key index).
pub type KeyRef = (usize, usize);

/// Curves shown in an editor: those of the selected objects, or all of
/// them without "Only Show Selected".
pub fn visible_curves(state: &EditorState, only_selected: bool) -> Vec<usize> {
    let selected = state.selected_ids();
    state
        .scene
        .animation
        .curves
        .iter()
        .enumerate()
        .filter(|(_, c)| !only_selected || selected.contains(&c.object))
        .map(|(i, _)| i)
        .collect()
}

/// The keys of `curves` on `frame`.
pub fn keys_at(action: &Action, curves: &[usize], frame: f32) -> Vec<KeyRef> {
    let mut out = Vec::new();
    for &c in curves {
        if let Some(curve) = action.curves.get(c) {
            for (k, key) in curve.keys.iter().enumerate() {
                if (key.frame - frame).abs() < FRAME_EPS {
                    out.push((c, k));
                }
            }
        }
    }
    out
}

/// Distinct key frames of `curves`, ascending, with whether any key of that
/// frame is selected.
pub fn frames_of(action: &Action, curves: &[usize]) -> Vec<(f32, bool)> {
    let mut out: Vec<(f32, bool)> = Vec::new();
    for &c in curves {
        let Some(curve) = action.curves.get(c) else {
            continue;
        };
        for key in &curve.keys {
            match out
                .iter_mut()
                .find(|(f, _)| (f - key.frame).abs() < FRAME_EPS)
            {
                Some(e) => e.1 |= key.is_selected(),
                None => out.push((key.frame, key.is_selected())),
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// A click on keys: replaces the selection, or with `extend` (Shift)
/// toggles them. Clicking nothing clears the selection.
pub fn click_select(action: &mut Action, hit: &[KeyRef], extend: bool) {
    if hit.is_empty() {
        if !extend {
            action.select_all(false);
        }
        return;
    }
    let all = hit
        .iter()
        .all(|&(c, k)| action.curves[c].keys[k].is_selected());
    if !extend {
        action.select_all(false);
    }
    for &(c, k) in hit {
        action.curves[c].keys[k].select = if extend && all { 0 } else { SEL_ALL };
    }
}

/// Selects the keys of `curves` between two frames (a box selection).
pub fn box_select(action: &mut Action, curves: &[usize], frames: (f32, f32), extend: bool) {
    if !extend {
        action.select_all(false);
    }
    let (lo, hi) = (
        frames.0.min(frames.1) - FRAME_EPS,
        frames.0.max(frames.1) + FRAME_EPS,
    );
    for &c in curves {
        if let Some(curve) = action.curves.get_mut(c) {
            for key in curve
                .keys
                .iter_mut()
                .filter(|k| k.frame >= lo && k.frame <= hi)
            {
                key.select = SEL_ALL;
            }
        }
    }
}

pub fn invert_selection(action: &mut Action) {
    for key in action.curves.iter_mut().flat_map(|c| &mut c.keys) {
        key.select = if key.is_selected() { 0 } else { SEL_ALL };
    }
}

/// A small popup menu at `pos`; returns the chosen item, or `Some(None)`
/// when it was dismissed.
pub fn popup_menu<T: Copy>(
    ctx: &egui::Context,
    id: &str,
    pos: Pos2,
    items: &[(&str, T)],
) -> Option<Option<T>> {
    let mut chosen = None;
    let area = egui::Area::new(egui::Id::new(("animator-popup", id)))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(110.0);
                for (label, value) in items {
                    if ui.button(*label).clicked() {
                        chosen = Some(*value);
                    }
                }
            });
        });
    if chosen.is_some() {
        return Some(chosen);
    }
    let dismissed = ctx.input(|i| i.key_pressed(egui::Key::Escape))
        || (ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer());
    dismissed.then_some(None)
}

/// A dim colour for text that explains what to do.
pub fn hint_color(ui: &egui::Ui) -> Color32 {
    ui.visuals().weak_text_color()
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_editor::olvin_scene::action::Prop;

    fn action() -> Action {
        let mut a = Action::default();
        for i in 0..3 {
            for f in [1.0, 10.0, 20.0] {
                a.insert_key(1, Prop::Location, i, f, f);
            }
        }
        a
    }

    #[test]
    fn click_replaces_and_shift_click_toggles() {
        let mut a = action();
        let all = [0, 1, 2];
        let hit = keys_at(&a, &all, 10.0);
        assert_eq!(hit.len(), 3);
        click_select(&mut a, &hit, false);
        assert_eq!(a.selected_count(), 3);
        let other = keys_at(&a, &all, 20.0);
        click_select(&mut a, &other, true);
        assert_eq!(a.selected_count(), 6);
        click_select(&mut a, &other, true);
        assert_eq!(a.selected_count(), 3, "second shift-click deselects");
        click_select(&mut a, &[], false);
        assert_eq!(a.selected_count(), 0);
    }

    #[test]
    fn box_select_and_invert() {
        let mut a = action();
        box_select(&mut a, &[0, 1], (0.0, 12.0), false);
        assert_eq!(a.selected_count(), 4);
        box_select(&mut a, &[2], (15.0, 25.0), true);
        assert_eq!(a.selected_count(), 5);
        invert_selection(&mut a);
        assert_eq!(a.selected_count(), 4);
        let frames = frames_of(&a, &[0, 1, 2]);
        assert_eq!(
            frames.iter().map(|f| f.0).collect::<Vec<_>>(),
            [1.0, 10.0, 20.0]
        );
        assert!(frames.iter().all(|f| f.1));
    }
}

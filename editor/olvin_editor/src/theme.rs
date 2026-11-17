//! Look: Blender's neutral grays with Unity's blue selection; orange for
//! selected objects in the viewport. Plugins use these constants too.

use egui::{Color32, CornerRadius, Visuals};

pub const ACCENT: Color32 = Color32::from_rgb(71, 114, 179);
pub const SELECTED: Color32 = Color32::from_rgb(255, 160, 51);
pub const AXIS_X: Color32 = Color32::from_rgb(232, 72, 85);
pub const AXIS_Y: Color32 = Color32::from_rgb(122, 196, 64);
pub const AXIS_Z: Color32 = Color32::from_rgb(64, 132, 232);
pub const VIEWPORT_BG: Color32 = Color32::from_gray(58);
pub const AXES: [Color32; 3] = [AXIS_X, AXIS_Y, AXIS_Z];

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    let gray = Color32::from_gray;
    v.panel_fill = gray(43);
    v.window_fill = gray(48);
    v.extreme_bg_color = gray(29);
    v.faint_bg_color = gray(48);
    v.code_bg_color = gray(36);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke.color = Color32::WHITE;
    v.hyperlink_color = Color32::from_rgb(110, 160, 230);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(5);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = gray(43);
    w.noninteractive.weak_bg_fill = gray(43);
    w.noninteractive.bg_stroke.color = gray(32);
    for (state, fill) in [(&mut w.inactive, 62), (&mut w.hovered, 78), (&mut w.active, 92), (&mut w.open, 70)] {
        state.bg_fill = gray(fill);
        state.weak_bg_fill = gray(fill);
        state.corner_radius = CornerRadius::same(3);
    }
    w.inactive.bg_stroke.color = gray(30);

    ctx.set_visuals(v);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(6.0, 2.0);
        style.spacing.interact_size.y = 20.0;
    });
}

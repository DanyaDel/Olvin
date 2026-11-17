//! "Olvin Night": cool blue-grey surfaces, one blue accent, orange for the
//! viewport selection (Blender's habit), Unity's axis colours. Plugins use
//! these constants too.

use egui::{Color32, CornerRadius, FontId, Shadow, Stroke, TextStyle, Visuals};

pub const ACCENT: Color32 = Color32::from_rgb(88, 132, 255);
pub const SELECTED: Color32 = Color32::from_rgb(255, 160, 51);
pub const AXIS_X: Color32 = Color32::from_rgb(232, 72, 85);
pub const AXIS_Y: Color32 = Color32::from_rgb(122, 196, 64);
pub const AXIS_Z: Color32 = Color32::from_rgb(64, 132, 232);
pub const VIEWPORT_BG: Color32 = Color32::from_rgb(52, 56, 64);
pub const AXES: [Color32; 3] = [AXIS_X, AXIS_Y, AXIS_Z];

/// Window background, behind the panels (the gaps between docked panels).
pub const BACKDROP: Color32 = Color32::from_rgb(18, 20, 25);
/// Panel surfaces.
pub const SURFACE: Color32 = Color32::from_rgb(29, 32, 39);
/// Raised blocks inside panels (inspector components, cards).
pub const RAISED: Color32 = Color32::from_rgb(36, 40, 49);
pub const TEXT: Color32 = Color32::from_rgb(218, 223, 232);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(134, 142, 158);

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = SURFACE;
    v.window_fill = RAISED;
    v.extreme_bg_color = Color32::from_rgb(22, 25, 31);
    v.faint_bg_color = RAISED;
    v.code_bg_color = Color32::from_rgb(24, 27, 33);
    v.override_text_color = None;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.85);
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = Color32::from_rgb(130, 170, 255);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(52, 58, 70));
    v.window_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(110) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(100) };
    v.warn_fg_color = Color32::from_rgb(255, 196, 92);
    v.error_fg_color = Color32::from_rgb(255, 110, 110);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(44, 49, 59));
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    let states = [
        (&mut w.inactive, Color32::from_rgb(44, 49, 60), TEXT),
        (&mut w.hovered, Color32::from_rgb(56, 63, 78), Color32::WHITE),
        (&mut w.active, Color32::from_rgb(68, 78, 98), Color32::WHITE),
        (&mut w.open, Color32::from_rgb(50, 57, 70), Color32::WHITE),
    ];
    for (state, fill, text) in states {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.corner_radius = CornerRadius::same(6);
        state.fg_stroke = Stroke::new(1.0, text);
        state.bg_stroke = Stroke::NONE;
        state.expansion = 0.0;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(76, 86, 108));

    ctx.set_visuals(v);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(7.0, 5.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.interact_size.y = 22.0;
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.window_margin = egui::Margin::same(12);
        use egui::FontFamily::{Monospace, Proportional};
        style.text_styles = [
            (TextStyle::Heading, FontId::new(19.0, Proportional)),
            (TextStyle::Body, FontId::new(13.5, Proportional)),
            (TextStyle::Button, FontId::new(13.5, Proportional)),
            (TextStyle::Small, FontId::new(11.0, Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, Monospace)),
        ]
        .into();
    });
}

/// Dock look: pill tabs on a dark backdrop, panels separated by gaps.
pub fn dock_style(style: &egui::Style) -> egui_dock::Style {
    let mut d = egui_dock::Style::from_egui(style);
    d.dock_area_padding = Some(egui::Margin::same(4));
    d.main_surface_border_stroke = Stroke::NONE;
    d.separator.width = 4.0;
    d.separator.color_idle = BACKDROP;
    d.separator.color_hovered = ACCENT.gamma_multiply(0.6);
    d.separator.color_dragged = ACCENT;

    d.tab_bar.bg_fill = BACKDROP;
    d.tab_bar.height = 30.0;
    d.tab_bar.hline_color = Color32::TRANSPARENT;
    d.tab_bar.corner_radius = CornerRadius::ZERO;

    let tab = |bg: Color32, text: Color32| egui_dock::TabInteractionStyle {
        outline_color: Color32::TRANSPARENT,
        corner_radius: CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 },
        bg_fill: bg,
        text_color: text,
    };
    d.tab.active = tab(SURFACE, TEXT);
    d.tab.focused = tab(SURFACE, Color32::WHITE);
    d.tab.inactive = tab(Color32::TRANSPARENT, TEXT_WEAK);
    d.tab.hovered = tab(Color32::from_rgb(26, 29, 36), TEXT);
    d.tab.active_with_kb_focus = d.tab.active.clone();
    d.tab.focused_with_kb_focus = d.tab.focused.clone();
    d.tab.inactive_with_kb_focus = d.tab.inactive.clone();
    d.tab.hline_below_active_tab_name = true;
    d.tab.spacing = 2.0;
    d.tab.tab_body.bg_fill = SURFACE;
    d.tab.tab_body.stroke = Stroke::NONE;
    d.tab.tab_body.corner_radius = CornerRadius { nw: 0, ne: 8, sw: 8, se: 8 };
    d.tab.tab_body.inner_margin = egui::Margin::same(6);

    d.buttons.close_tab_color = TEXT_WEAK;
    d.buttons.close_tab_active_color = Color32::WHITE;
    d.buttons.add_tab_color = TEXT_WEAK;
    d.buttons.add_tab_active_color = Color32::WHITE;
    d.buttons.add_tab_bg_fill = Color32::from_rgb(44, 49, 60);
    d.overlay.selection_color = ACCENT.gamma_multiply(0.35);
    d
}

//! Olvin's look for the editor, a thin layer over `olvin_ui` (the UI kit the
//! hub shares): themes, fonts, logo, motion. See STYLE.md.
//!
//! The colour constants below are the **Dark** theme's tokens, kept so
//! existing panels and plugins compile unchanged. Code that should follow the
//! selected theme (the chrome: top bar, status bar, dock, menus) reads
//! [`palette`] instead; egui widgets follow it on their own through the
//! visuals that [`apply`] installs. Viewport and gizmo colours are scene
//! drawing, not UI, and don't change with the theme.

use egui::{Color32, CornerRadius, Stroke};

pub use olvin_ui::{
    FS_2XL, FS_LG, FS_MD, FS_SM, FS_XL, FS_XS, Palette, RADIUS_LG, RADIUS_MD, RADIUS_PILL,
    RADIUS_SM, RADIUS_XS, ThemeKind, anim, bold, caps, fonts, kind, logo, menu_title, mono_bold,
    palette, reduce_motion, set_reduce_motion,
};

const DARK: Palette = Palette::DARK;

// --- Dark tokens (STYLE.md §1), for code without a context at hand ---

/// `--bg`: window background, the gaps between docked panels.
pub const BG: Color32 = DARK.bg;
/// `--surface`: editor panels.
pub const SURFACE: Color32 = DARK.surface;
/// `--surface-raised`: popup menus, dialogs.
pub const SURFACE_RAISED: Color32 = DARK.surface_raised;
/// `--glass`: glass panels and secondary buttons.
pub const GLASS: Color32 = DARK.glass;
/// `--glass-strong`: the header and floating panels.
pub const GLASS_STRONG: Color32 = DARK.glass_strong;
/// `--footer`: the status bar.
pub const FOOTER: Color32 = DARK.footer;
/// `--text`: main text, headings.
pub const TEXT: Color32 = DARK.text;
/// `--text-2`: descriptions, field captions.
pub const TEXT_2: Color32 = DARK.text_2;
/// `--muted`: secondary text, labels.
pub const MUTED: Color32 = DARK.muted;
/// `--faint`: decorative text only, never information.
pub const FAINT: Color32 = DARK.faint;
/// `--ghost`: separators, not text.
pub const GHOST: Color32 = DARK.ghost;
/// `--accent`: selected, active, cursor, "ok" status. The only accent.
pub const ACCENT: Color32 = DARK.accent;
/// `--accent-glow`: glow around the accent.
pub const ACCENT_GLOW: Color32 = DARK.accent_glow;
/// `--accent-border`: outline on hover.
pub const ACCENT_BORDER: Color32 = DARK.accent_border;
/// `--warn`: warnings (not green, so it isn't mistaken for "ok").
pub const WARN: Color32 = DARK.warn;
/// `--error`: errors, red in Problems.
pub const ERROR: Color32 = DARK.error;
/// `--info`: information, hints.
pub const INFO: Color32 = DARK.info;
/// `--line`: ordinary separators.
pub const LINE: Color32 = DARK.line;
/// `--line-top`: light top edge of glass panels.
pub const LINE_TOP: Color32 = DARK.line_top;
/// `--line-soft`: card borders.
pub const LINE_SOFT: Color32 = DARK.line_soft;

// --- Not in STYLE.md: viewport and gizmo colours (scene drawing, not UI) ---

/// Viewport selection (Blender's habit). Viewport only; UI selection is [`ACCENT`].
pub const SELECTED: Color32 = Color32::from_rgb(255, 160, 51);
pub const AXIS_X: Color32 = Color32::from_rgb(232, 72, 85);
pub const AXIS_Y: Color32 = Color32::from_rgb(122, 196, 64);
pub const AXIS_Z: Color32 = Color32::from_rgb(64, 132, 232);
pub const VIEWPORT_BG: Color32 = Color32::from_rgb(52, 56, 64);
pub const AXES: [Color32; 3] = [AXIS_X, AXIS_Y, AXIS_Z];

// --- Old names, kept so plugins keep compiling ---

/// Old name of [`BG`].
pub const BACKDROP: Color32 = BG;
/// Old name of [`SURFACE_RAISED`].
pub const RAISED: Color32 = SURFACE_RAISED;
/// Old name of [`MUTED`].
pub const TEXT_WEAK: Color32 = MUTED;

/// Re-applies the current theme (Dark until [`apply_kind`] picks another):
/// fonts once, visuals, spacing. Headless tests call this too.
pub fn apply(ctx: &egui::Context) {
    olvin_ui::apply(ctx, olvin_ui::kind(ctx));
}

/// Switches to `kind`.
pub fn apply_kind(ctx: &egui::Context, kind: ThemeKind) {
    olvin_ui::apply(ctx, kind);
}

/// Dock look: pill tabs on the backdrop, panels separated by gaps.
pub fn dock_style(style: &egui::Style, p: &Palette) -> egui_dock::Style {
    let mut d = egui_dock::Style::from_egui(style);
    d.dock_area_padding = Some(egui::Margin::same(4));
    d.main_surface_border_stroke = Stroke::NONE;
    d.separator.width = 4.0;
    d.separator.color_idle = p.bg;
    d.separator.color_hovered = p.accent_border;
    d.separator.color_dragged = p.accent;

    d.tab_bar.bg_fill = p.bg;
    d.tab_bar.height = 30.0;
    d.tab_bar.hline_color = Color32::TRANSPARENT;
    d.tab_bar.corner_radius = CornerRadius::ZERO;

    // STYLE.md §6: the focused tab is inverted (text colour fill, bg text),
    // everywhere. The editor eases between these states per tab.
    let tab = |bg: Color32, text: Color32| egui_dock::TabInteractionStyle {
        outline_color: Color32::TRANSPARENT,
        corner_radius: CornerRadius::same(RADIUS_PILL),
        bg_fill: bg,
        text_color: text,
    };
    d.tab.active = tab(p.surface_raised, p.text);
    d.tab.focused = tab(p.text, p.bg);
    d.tab.inactive = tab(Color32::TRANSPARENT, p.muted);
    d.tab.hovered = tab(p.glass, p.text);
    d.tab.active_with_kb_focus = d.tab.active.clone();
    d.tab.focused_with_kb_focus = d.tab.focused.clone();
    d.tab.inactive_with_kb_focus = d.tab.inactive.clone();
    d.tab.hline_below_active_tab_name = false;
    d.tab.spacing = 2.0;
    d.tab.tab_body.bg_fill = p.surface;
    d.tab.tab_body.stroke = Stroke::NONE;
    d.tab.tab_body.corner_radius = CornerRadius::same(RADIUS_MD);
    d.tab.tab_body.inner_margin = egui::Margin::same(6);

    d.buttons.close_tab_color = p.muted;
    d.buttons.close_tab_active_color = p.text;
    // Under the hovered ✕, so it reads on the inverted focused tab too.
    d.buttons.close_tab_bg_fill = p.ghost;
    d.buttons.add_tab_color = p.muted;
    d.buttons.add_tab_active_color = p.text;
    d.buttons.add_tab_bg_fill = p.glass;
    d.overlay.selection_color = p.accent_border;
    d
}

/// Tab colours eased between inactive → active (front tab of its leaf) →
/// focused (front tab of the focused leaf). `active_t`/`focused_t` are 0..1.
pub fn eased_tab_style(
    global: &egui_dock::TabStyle,
    active_t: f32,
    focused_t: f32,
) -> egui_dock::TabStyle {
    let lerp = anim::lerp_color;
    let blend = |a: &egui_dock::TabInteractionStyle, b: &egui_dock::TabInteractionStyle, t| {
        egui_dock::TabInteractionStyle {
            bg_fill: lerp(a.bg_fill, b.bg_fill, t),
            text_color: lerp(a.text_color, b.text_color, t),
            ..a.clone()
        }
    };
    let now = blend(
        &blend(&global.inactive, &global.active, active_t),
        &global.focused,
        focused_t,
    );
    let mut s = global.clone();
    s.active = now.clone();
    s.focused = now.clone();
    s.inactive = now.clone();
    s.active_with_kb_focus = now.clone();
    s.focused_with_kb_focus = now.clone();
    s.inactive_with_kb_focus = now.clone();
    // Background tabs still light up on hover; front tabs keep their colours.
    if active_t > 0.5 {
        s.hovered = now;
    }
    s
}

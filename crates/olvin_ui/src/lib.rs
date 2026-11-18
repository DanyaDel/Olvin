//! Olvin's look, shared by the editor and the hub: the tokens from
//! `STYLE.md` as [`Palette`]s for four themes, the bundled fonts, the logo
//! painted from geometry ([`logo`]) and the motion curves ([`anim`]).
//!
//! Every theme has the same token names (STYLE.md "Themes"); UI code asks for
//! the current one with [`palette`] instead of hard-coding colours. egui has no
//! backdrop blur, so the "glass" surfaces fall back to opaque colours (the
//! Compat recipe in STYLE.md §4).
//!
//! ```ignore
//! olvin_ui::apply(ctx, olvin_ui::ThemeKind::Dark);
//! let p = olvin_ui::palette(ctx);
//! olvin_ui::logo::wordmark(ui, 18.0);
//! ```

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Id, RichText, Shadow,
    Stroke, TextStyle, Visuals,
};
use serde::{Deserialize, Serialize};

pub mod anim;
pub mod logo;

// --- Radii (STYLE.md §3); egui radii are u8 ---

/// `--radius-xs`: badges, status tags.
pub const RADIUS_XS: u8 = 4;
/// `--radius-sm`: buttons, inputs, list items.
pub const RADIUS_SM: u8 = 8;
/// `--radius-md`: editor panels, popup menus.
pub const RADIUS_MD: u8 = 12;
/// `--radius-lg`: dialogs, launcher cards.
pub const RADIUS_LG: u8 = 20;
/// `--radius-pill`: 999px in the spec; u8 tops out at 255, which is the same pill.
pub const RADIUS_PILL: u8 = 255;

// --- Font sizes (STYLE.md §2), px ---

/// `--fs-xs`: labels, statuses, menu groups (UPPERCASE, see [`caps`]).
pub const FS_XS: f32 = 10.0;
/// `--fs-sm`: field captions, hints.
pub const FS_SM: f32 = 12.0;
/// `--fs-md`: main editor text.
pub const FS_MD: f32 = 13.0;
/// `--fs-lg`: main menu items (bold, UPPERCASE, see [`menu_title`]).
pub const FS_LG: f32 = 14.0;
/// `--fs-xl`: window titles (bold).
pub const FS_XL: f32 = 20.0;
/// `--fs-2xl`: launcher and "About" titles (bold).
pub const FS_2XL: f32 = 28.0;

/// The colour tokens of one theme. Field names are the `--tokens` of STYLE.md
/// (`--text-2` → `text_2`); every theme fills all of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// `--bg`: window background, the gaps between docked panels.
    pub bg: Color32,
    /// `--surface`: editor panels.
    pub surface: Color32,
    /// `--surface-raised`: popup menus, dialogs.
    pub surface_raised: Color32,
    /// `--glass`: glass panels and secondary buttons.
    pub glass: Color32,
    /// `--glass-strong`: the header and floating panels.
    pub glass_strong: Color32,
    /// `--footer`: the status bar.
    pub footer: Color32,
    /// `--text`: main text, headings.
    pub text: Color32,
    /// `--text-2`: descriptions, field captions.
    pub text_2: Color32,
    /// `--muted`: secondary text, labels.
    pub muted: Color32,
    /// `--faint`: decorative text only, never information.
    pub faint: Color32,
    /// `--ghost`: separators, not text.
    pub ghost: Color32,
    /// `--accent`: selected, active, cursor, "ok" status. The only accent.
    pub accent: Color32,
    /// `--accent-glow`: glow around the accent.
    pub accent_glow: Color32,
    /// `--accent-border`: outline on hover.
    pub accent_border: Color32,
    /// Text on an `accent` fill (the primary button).
    pub on_accent: Color32,
    /// `--warn`: warnings (not green, so it isn't mistaken for "ok").
    pub warn: Color32,
    /// `--error`: errors.
    pub error: Color32,
    /// `--info`: information, hints, links.
    pub info: Color32,
    /// `--line`: ordinary separators.
    pub line: Color32,
    /// `--line-top`: the stronger top edge of glass panels.
    pub line_top: Color32,
    /// `--line-soft`: card borders.
    pub line_soft: Color32,
    /// The logo's O and letters (the cursor is always `accent`).
    pub logo_ring: Color32,
    /// Text selection and selected rows: the accent, dimmed so accent text
    /// on top stays readable.
    pub selection: Color32,
    /// Panel shadow colour (`0 10px 40px`, STYLE.md §5).
    pub shadow: Color32,
    /// Popup menu shadow colour.
    pub shadow_popup: Color32,
    /// Dark theme: egui's dark defaults underneath, light text on dark.
    pub dark: bool,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const fn rgba(hex: u32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied_const((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, alpha)
}

impl Palette {
    /// Olvin Night, the STYLE.md values one to one.
    pub const DARK: Palette = Palette {
        bg: rgb(0x000000),
        surface: rgb(0x0f0f0f),
        surface_raised: rgb(0x1c1c1e),
        glass: rgba(0xffffff, 8),          // 0.03
        glass_strong: rgba(0x0f0f0f, 153), // 0.6
        footer: rgb(0x020202),
        text: rgb(0xf5f5f7),
        text_2: rgb(0xcccccc),
        muted: rgb(0x999999),
        faint: rgb(0x666666),
        ghost: rgb(0x333333),
        accent: rgb(0x4cd964),
        accent_glow: rgba(0x4cd964, 102),  // 0.4
        accent_border: rgba(0x4cd964, 77), // 0.3
        on_accent: rgb(0x000000),
        warn: rgb(0xffb020),
        error: rgb(0xff5a5a),
        info: rgb(0x4da3ff),
        line: rgba(0xffffff, 20),      // 0.08
        line_top: rgba(0xffffff, 38),  // 0.15
        line_soft: rgba(0xffffff, 13), // 0.05
        logo_ring: rgb(0xf5f5f7),
        selection: rgba(0x4cd964, 46),
        shadow: rgba(0x000000, 153),
        shadow_popup: rgba(0x000000, 204),
        dark: true,
    };

    /// Olvin Day: near-white panels on a light grey backdrop, ink text, the
    /// darker logo green `#1F9D3A` (on white `#4cd964` is too faint).
    pub const LIGHT: Palette = Palette {
        bg: rgb(0xeeeef1),
        surface: rgb(0xfbfbfc),
        surface_raised: rgb(0xffffff),
        glass: rgba(0x000000, 8),
        glass_strong: rgba(0xfbfbfc, 217), // 0.85
        footer: rgb(0xf3f3f5),
        text: rgb(0x1b1e24),
        text_2: rgb(0x3c3f46),
        muted: rgb(0x5e626b),
        faint: rgb(0x9a9ca3),
        ghost: rgb(0xd0d1d6),
        accent: rgb(0x1f9d3a),
        accent_glow: rgba(0x1f9d3a, 77),
        accent_border: rgba(0x1f9d3a, 115),
        on_accent: rgb(0x000000),
        warn: rgb(0x8a5300),
        error: rgb(0xb3261e),
        info: rgb(0x0b5cbf),
        line: rgba(0x000000, 26),
        line_top: rgba(0x000000, 41),
        line_soft: rgba(0x000000, 15),
        logo_ring: rgb(0x1b1e24),
        selection: rgba(0x1f9d3a, 46),
        shadow: rgba(0x000000, 40),
        shadow_popup: rgba(0x000000, 64),
        dark: false,
    };

    /// Graphite: the dark theme on soft greys instead of black, for long
    /// sessions and monitors with crushed blacks.
    pub const GRAPHITE: Palette = Palette {
        bg: rgb(0x1a1a1c),
        surface: rgb(0x232326),
        surface_raised: rgb(0x2c2c30),
        glass: rgba(0xffffff, 10),
        glass_strong: rgba(0x232326, 217),
        footer: rgb(0x18181a),
        text: rgb(0xededf0),
        text_2: rgb(0xc8c8cc),
        muted: rgb(0xa1a1a8),
        faint: rgb(0x6e6e75),
        ghost: rgb(0x3d3d42),
        accent: rgb(0x4cd964),
        accent_glow: rgba(0x4cd964, 102),
        accent_border: rgba(0x4cd964, 89),
        on_accent: rgb(0x000000),
        warn: rgb(0xffb020),
        error: rgb(0xff6b6b),
        info: rgb(0x5eaaff),
        line: rgba(0xffffff, 22),
        line_top: rgba(0xffffff, 40),
        line_soft: rgba(0xffffff, 14),
        logo_ring: rgb(0xededf0),
        selection: rgba(0x4cd964, 50),
        shadow: rgba(0x000000, 140),
        shadow_popup: rgba(0x000000, 190),
        dark: true,
    };

    /// High contrast: pure black and white, solid borders, a brighter green.
    /// Every text token is at least 10:1.
    pub const HIGH_CONTRAST: Palette = Palette {
        bg: rgb(0x000000),
        surface: rgb(0x050505),
        surface_raised: rgb(0x101010),
        glass: rgba(0xffffff, 20),
        glass_strong: rgba(0x000000, 242),
        footer: rgb(0x000000),
        text: rgb(0xffffff),
        text_2: rgb(0xf0f0f0),
        muted: rgb(0xd0d0d0),
        faint: rgb(0xa8a8a8),
        ghost: rgb(0x707070),
        accent: rgb(0x6bff85),
        accent_glow: rgba(0x6bff85, 120),
        accent_border: rgba(0x6bff85, 200),
        on_accent: rgb(0x000000),
        warn: rgb(0xffd23f),
        error: rgb(0xff8080),
        info: rgb(0x7cc0ff),
        line: rgba(0xffffff, 110),
        line_top: rgba(0xffffff, 170),
        line_soft: rgba(0xffffff, 80),
        logo_ring: rgb(0xffffff),
        selection: rgba(0x6bff85, 70),
        shadow: rgba(0x000000, 200),
        shadow_popup: rgba(0x000000, 230),
        dark: true,
    };
}

impl Default for Palette {
    fn default() -> Self {
        Self::DARK
    }
}

/// Which palette the UI uses. Stored in user preferences (serde).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ThemeKind {
    #[default]
    Dark,
    Light,
    Graphite,
    HighContrast,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 4] = [
        ThemeKind::Dark,
        ThemeKind::Light,
        ThemeKind::Graphite,
        ThemeKind::HighContrast,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Dark => "Dark",
            ThemeKind::Light => "Light",
            ThemeKind::Graphite => "Graphite",
            ThemeKind::HighContrast => "High Contrast",
        }
    }

    pub fn palette(self) -> Palette {
        match self {
            ThemeKind::Dark => Palette::DARK,
            ThemeKind::Light => Palette::LIGHT,
            ThemeKind::Graphite => Palette::GRAPHITE,
            ThemeKind::HighContrast => Palette::HIGH_CONTRAST,
        }
    }

    fn from_u8(v: u8) -> Self {
        Self::ALL.get(v as usize).copied().unwrap_or_default()
    }
}

// --- Current theme ---

/// What [`apply`] stored in the context.
#[derive(Clone, Copy, Debug, Default)]
struct Current {
    kind: ThemeKind,
    palette: Palette,
}

fn current_id() -> Id {
    Id::new("olvin_ui::theme")
}

fn fonts_id() -> Id {
    Id::new("olvin_ui::fonts")
}

fn motion_id() -> Id {
    Id::new("olvin_ui::reduce_motion")
}

/// Last applied theme, process-wide, for helpers without a context
/// ([`caps`]). One UI per process in practice; the context copy wins.
static LAST_KIND: AtomicU8 = AtomicU8::new(0);
static LAST_REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

/// The palette [`apply`] installed, or [`Palette::DARK`] before that.
pub fn palette(ctx: &egui::Context) -> Palette {
    ctx.data(|d| d.get_temp::<Current>(current_id()))
        .unwrap_or_default()
        .palette
}

/// The theme [`apply`] installed, or [`ThemeKind::Dark`] before that.
pub fn kind(ctx: &egui::Context) -> ThemeKind {
    ctx.data(|d| d.get_temp::<Current>(current_id()))
        .unwrap_or_default()
        .kind
}

/// The palette of the last [`apply`] anywhere in the process, for code that
/// has no context at hand. Prefer [`palette`].
pub fn last_palette() -> Palette {
    ThemeKind::from_u8(LAST_KIND.load(Ordering::Relaxed)).palette()
}

/// STYLE.md §7 "reduce motion": no transitions, no blinking cursor. Also
/// sets egui's own `animation_time` (menu fade-in, collapsing headers) to 0.
pub fn set_reduce_motion(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(motion_id(), on));
    LAST_REDUCE_MOTION.store(on, Ordering::Relaxed);
    ctx.all_styles_mut(|s| s.animation_time = animation_time(on));
}

pub fn reduce_motion(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(motion_id()))
        .unwrap_or_else(|| LAST_REDUCE_MOTION.load(Ordering::Relaxed))
}

/// egui's built-in fades (popups, windows, collapsing): a quick ease-out.
fn animation_time(reduce_motion: bool) -> f32 {
    if reduce_motion { 0.0 } else { 0.18 }
}

// --- Fonts ---

/// `--font-ui` and `--font-mono`, bundled in `assets/fonts/` (OFL).
const INTER: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/Inter-Regular.ttf"
));
const INTER_BOLD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/Inter-Bold.ttf"
));
const MONO: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/JetBrainsMono-Regular.ttf"
));
const MONO_BOLD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/JetBrainsMono-Bold.ttf"
));

/// Inter Bold. egui's `.strong()` only brightens the colour, so real bold
/// text goes through this family.
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// JetBrains Mono Bold.
pub fn mono_bold() -> FontFamily {
    FontFamily::Name("mono-bold".into())
}

/// Inter for UI text, JetBrains Mono for code, egui's defaults behind them
/// for the glyphs they lack (emoji, icons).
pub fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    for (name, bytes) in [
        ("Inter", INTER),
        ("Inter Bold", INTER_BOLD),
        ("JetBrains Mono", MONO),
        ("JetBrains Mono Bold", MONO_BOLD),
    ] {
        f.font_data
            .insert(name.into(), FontData::from_static(bytes).into());
    }
    let fallback =
        |f: &FontDefinitions, family| f.families.get(&family).cloned().unwrap_or_default();
    let ui = fallback(&f, FontFamily::Proportional);
    let mono = fallback(&f, FontFamily::Monospace);
    let chain = |first: &str, rest: &[String]| {
        std::iter::once(first.to_owned())
            .chain(rest.iter().cloned())
            .collect()
    };
    f.families
        .insert(FontFamily::Proportional, chain("Inter", &ui));
    f.families.insert(bold(), chain("Inter Bold", &ui));
    f.families
        .insert(FontFamily::Monospace, chain("JetBrains Mono", &mono));
    f.families
        .insert(mono_bold(), chain("JetBrains Mono Bold", &mono));
    f
}

// --- Text helpers ---

/// `--fs-xs` label: UPPERCASE, bold, 0.06em tracking, `--text-2` of the
/// current theme. Panel section headings.
pub fn caps(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .size(FS_XS)
        .family(bold())
        .extra_letter_spacing(FS_XS * 0.06)
        .color(last_palette().text_2)
}

/// `--fs-lg` main menu item: bold, UPPERCASE.
pub fn menu_title(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .size(FS_LG)
        .family(bold())
}

// --- Applying ---

/// `0 10px 40px <color>`, STYLE.md §5.
const fn shadow(color: Color32) -> Shadow {
    Shadow {
        offset: [0, 10],
        blur: 40,
        spread: 0,
        color,
    }
}

/// egui visuals for a palette: STYLE.md §6 buttons, §3 radii, §5 shadows.
pub fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.dark_mode = p.dark;
    v.panel_fill = p.surface;
    v.window_fill = p.surface_raised;
    v.extreme_bg_color = p.bg;
    v.text_edit_bg_color = Some(p.bg);
    v.faint_bg_color = p.glass;
    v.code_bg_color = p.bg;
    v.override_text_color = None;
    v.weak_text_color = Some(p.muted);
    v.selection.bg_fill = p.selection;
    // Also the focus outline of text fields (STYLE.md §11: accent, 2px).
    v.selection.stroke = Stroke::new(2.0, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.hyperlink_color = p.info;
    v.window_corner_radius = CornerRadius::same(RADIUS_MD);
    v.menu_corner_radius = CornerRadius::same(RADIUS_MD);
    v.window_stroke = Stroke::new(1.0, p.line);
    v.window_shadow = shadow(p.shadow);
    v.popup_shadow = shadow(p.shadow_popup);
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.error;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = CornerRadius::same(RADIUS_SM);
    // Secondary button (STYLE.md §6): glass fill, line border, text-2;
    // hover lights the border, press/focus turns it accent.
    let states = [
        (&mut w.inactive, p.glass, p.line, p.text_2),
        (&mut w.hovered, p.line, p.accent_border, p.text),
        (&mut w.active, p.line_top, p.accent, p.text),
        (&mut w.open, p.line, p.line_top, p.text),
    ];
    for (state, fill, border, text) in states {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.corner_radius = CornerRadius::same(RADIUS_SM);
        state.fg_stroke = Stroke::new(1.0, text);
        state.bg_stroke = Stroke::new(1.0, border);
        state.expansion = 0.0;
    }
    v
}

/// Installs the fonts (once per context), then the theme's visuals, spacing
/// and text styles for both egui themes, and remembers the palette for
/// [`palette`]. Cheap enough to call when the theme changes.
pub fn apply(ctx: &egui::Context, kind: ThemeKind) {
    let has_fonts = ctx
        .data(|d| d.get_temp::<bool>(fonts_id()))
        .unwrap_or(false);
    if !has_fonts {
        ctx.set_fonts(fonts());
        ctx.data_mut(|d| d.insert_temp(fonts_id(), true));
    }
    let p = kind.palette();
    ctx.data_mut(|d| d.insert_temp(current_id(), Current { kind, palette: p }));
    LAST_KIND.store(kind as u8, Ordering::Relaxed);

    // Same visuals whatever egui thinks the system theme is.
    let v = visuals(&p);
    ctx.set_theme(if p.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.set_visuals_of(egui::Theme::Dark, v.clone());
    ctx.set_visuals_of(egui::Theme::Light, v);
    let motion = reduce_motion(ctx);
    ctx.all_styles_mut(|style| {
        style.animation_time = animation_time(motion);
        style.spacing.item_spacing = egui::vec2(7.0, 5.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.interact_size.y = 22.0;
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.window_margin = egui::Margin::same(12);
        use egui::FontFamily::{Monospace, Proportional};
        style.text_styles = [
            (TextStyle::Heading, FontId::new(FS_XL, bold())),
            (TextStyle::Body, FontId::new(FS_MD, Proportional)),
            (TextStyle::Button, FontId::new(FS_MD, Proportional)),
            (TextStyle::Small, FontId::new(FS_SM, Proportional)),
            (TextStyle::Monospace, FontId::new(FS_MD, Monospace)),
        ]
        .into();
    });
}

// --- Contrast (STYLE.md §11) ---

/// WCAG relative luminance of an opaque colour.
pub fn luminance(c: Color32) -> f32 {
    let ch = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
}

/// WCAG contrast ratio between two opaque colours, 1..=21.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// One headless frame at `time` on a 400×200 screen, textures discarded.
#[cfg(test)]
pub(crate) fn test_frame(
    ctx: &egui::Context,
    time: f64,
    f: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let input = egui::RawInput {
        time: Some(time),
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(400.0, 200.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, f);
    out.textures_delta.clear();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_tokens_meet_4_5_to_1_on_every_surface() {
        for kind in ThemeKind::ALL {
            let p = kind.palette();
            let backgrounds = [
                ("bg", p.bg),
                ("surface", p.surface),
                ("surface_raised", p.surface_raised),
                ("footer", p.footer),
            ];
            let texts = [
                ("text", p.text),
                ("text_2", p.text_2),
                ("muted", p.muted),
                ("warn", p.warn),
                ("error", p.error),
                ("info", p.info),
                ("logo_ring", p.logo_ring),
            ];
            for (bn, b) in backgrounds {
                for (tn, t) in texts {
                    let c = contrast(t, b);
                    assert!(c >= 4.5, "{kind:?}: {tn} on {bn} is {c:.2}:1");
                }
                // The accent marks state (selected tab, cursor, status dot):
                // 3:1 is the non-text minimum.
                let c = contrast(p.accent, b);
                assert!(c >= 3.0, "{kind:?}: accent on {bn} is {c:.2}:1");
            }
            let c = contrast(p.on_accent, p.accent);
            assert!(c >= 4.5, "{kind:?}: on_accent on accent is {c:.2}:1");
        }
    }

    #[test]
    fn stark_contrast_is_21() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
    }

    #[test]
    fn apply_installs_palette_and_motion() {
        let ctx = egui::Context::default();
        assert_eq!(palette(&ctx), Palette::DARK);
        set_reduce_motion(&ctx, true);
        apply(&ctx, ThemeKind::Light);
        assert_eq!(kind(&ctx), ThemeKind::Light);
        assert_eq!(palette(&ctx), Palette::LIGHT);
        assert!(reduce_motion(&ctx));
        assert_eq!(ctx.global_style().animation_time, 0.0);
        assert_eq!(
            ctx.global_style().visuals.panel_fill,
            Palette::LIGHT.surface
        );
        set_reduce_motion(&ctx, false);
        assert!(ctx.global_style().animation_time > 0.0);
    }

    #[test]
    fn theme_kind_indices_and_labels() {
        for kind in ThemeKind::ALL {
            assert_eq!(ThemeKind::from_u8(kind as u8), kind);
            assert!(!kind.label().is_empty());
        }
    }
}

//! Editor preferences (the Settings window), per user in
//! `<config>/olvin-editor/preferences.ron`. Scene lighting lives in the scene.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Interface zoom.
    pub ui_scale: f32,
    /// Mouse look / orbit speed multiplier.
    pub look_sensitivity: f32,
    pub invert_y: bool,
    /// Base fly speed (m/s), before the scroll multiplier and Shift boost.
    pub fly_speed: f32,
    pub zoom_speed: f32,
    pub fov_deg: f32,
    /// New scene views start in Solid instead of Rendered.
    pub solid_by_default: bool,
    /// Shadows and fog in editor views (the game is unaffected).
    pub viewport_shadows: bool,
    pub viewport_fog: bool,
    /// Draw calls, objects and frame time in the scene view corner.
    pub show_stats: bool,
    /// Interface palette (STYLE.md "Themes"). Scene views keep their colours.
    pub theme: olvin_ui::ThemeKind,
    /// No transitions or blinking cursor (STYLE.md §7).
    pub reduce_motion: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            ui_scale: 1.0,
            look_sensitivity: 1.0,
            invert_y: false,
            fly_speed: 6.0,
            zoom_speed: 1.0,
            fov_deg: 55.0,
            solid_by_default: false,
            viewport_shadows: true,
            viewport_fog: true,
            show_stats: false,
            theme: olvin_ui::ThemeKind::default(),
            reduce_motion: false,
        }
    }
}

impl Preferences {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("olvin-editor/preferences.ron"))
    }

    /// Saved preferences, or defaults when missing or unreadable.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| ron::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no config directory")?;
        let dir = path.parent().ok_or("config path has no parent")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let text =
            ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new().struct_names(false))
                .map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

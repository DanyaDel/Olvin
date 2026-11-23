//! Window mode and resolution: a plain-data request ([`WindowSettings`]) that
//! the runner applies after each frame, like [`crate::CursorOptions`].
//! Everything here is pure data and logic, so it works without a window.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const DEFAULT_PATH: &str = "assets/window.ron";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowMode {
    #[default]
    Windowed,
    /// Borderless window over the whole monitor at the desktop resolution:
    /// instant to enter and leave, `width`/`height` are ignored.
    Borderless,
    /// Real video-mode change to the closest mode to `width` × `height`
    /// (physical pixels). Falls back to borderless when the monitor lists none.
    Exclusive,
}

impl WindowMode {
    pub fn is_fullscreen(self) -> bool {
        self != Self::Windowed
    }
}

fn default_width() -> u32 {
    1280
}

fn default_height() -> u32 {
    720
}

/// What the game wants the window to be. Insert/change it as a resource
/// (`olvin_window::WindowSettings`); the window follows after the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSettings {
    pub mode: WindowMode,
    /// Windowed: inner size in logical pixels. Exclusive: the video mode in
    /// physical pixels. Borderless ignores it, but it is kept for the way back.
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            mode: WindowMode::Windowed,
            width: default_width(),
            height: default_height(),
        }
    }
}

impl WindowSettings {
    pub fn set_mode(&mut self, mode: WindowMode) -> &mut Self {
        self.mode = mode;
        self
    }

    /// Changes the resolution (see the field docs for what it means per mode).
    /// Zero sizes are ignored.
    pub fn set_resolution(&mut self, width: u32, height: u32) -> &mut Self {
        if width > 0 && height > 0 {
            self.width = width;
            self.height = height;
        }
        self
    }

    /// Windowed ↔ borderless fullscreen (the F11 key).
    pub fn toggle_fullscreen(&mut self) -> &mut Self {
        self.mode = if self.mode.is_fullscreen() {
            WindowMode::Windowed
        } else {
            WindowMode::Borderless
        };
        self
    }

    pub fn from_ron(text: &str) -> Result<Self, String> {
        ron::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_ron(&self) -> String {
        // invariant: plain data (enums, strings, numbers) always serializes.
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .expect("window settings always serialize")
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_ron(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, self.to_ron()).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The settings at `path` if it exists and parses (errors are logged), else `fallback`.
    pub fn load_or(path: impl AsRef<Path>, fallback: Self) -> Self {
        let path = path.as_ref();
        if !path.exists() {
            return fallback;
        }
        Self::load(path).unwrap_or_else(|err| {
            log::error!("window settings: {err}; using the defaults");
            fallback
        })
    }
}

/// A video mode of the monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
    /// Refresh rate in millihertz (60000 = 60 Hz).
    pub refresh_mhz: u32,
}

/// Video modes of the monitor the window is on, for a settings menu. Filled
/// by the runner once the window exists; empty before that and headless.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Monitors {
    pub modes: Vec<Resolution>,
}

impl Monitors {
    /// Distinct sizes, largest first (refresh rates collapsed).
    pub fn resolutions(&self) -> Vec<(u32, u32)> {
        let mut sizes: Vec<_> = self.modes.iter().map(|m| (m.width, m.height)).collect();
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        sizes.dedup();
        sizes
    }

    /// Index of the mode closest to `width` × `height`, preferring the highest
    /// refresh rate among equals.
    pub fn closest(&self, width: u32, height: u32) -> Option<usize> {
        let area = |w: u32, h: u32| i64::from(w) * i64::from(h);
        let want = area(width, height);
        self.modes
            .iter()
            .enumerate()
            .min_by_key(|(_, m)| {
                (
                    (i64::from(m.width) - i64::from(width)).abs()
                        + (i64::from(m.height) - i64::from(height)).abs(),
                    (area(m.width, m.height) - want).abs(),
                    std::cmp::Reverse(m.refresh_mhz),
                )
            })
            .map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(width: u32, height: u32, hz: u32) -> Resolution {
        Resolution {
            width,
            height,
            refresh_mhz: hz * 1000,
        }
    }

    #[test]
    fn toggle_and_resolution() {
        let mut s = WindowSettings::default();
        s.set_resolution(0, 600).set_resolution(1920, 1080);
        assert_eq!((s.width, s.height), (1920, 1080), "zero size is ignored");
        s.toggle_fullscreen();
        assert_eq!(s.mode, WindowMode::Borderless);
        assert_eq!((s.width, s.height), (1920, 1080), "kept for the way back");
        s.set_mode(WindowMode::Exclusive).toggle_fullscreen();
        assert_eq!(s.mode, WindowMode::Windowed);
    }

    #[test]
    fn ron_round_trip_defaults_and_file() {
        let s = WindowSettings {
            mode: WindowMode::Exclusive,
            width: 1600,
            height: 900,
        };
        assert_eq!(WindowSettings::from_ron(&s.to_ron()).unwrap(), s);
        let partial = WindowSettings::from_ron("(mode: Borderless)").unwrap();
        assert_eq!(partial.mode, WindowMode::Borderless);
        assert_eq!((partial.width, partial.height), (1280, 720));

        let dir = std::env::temp_dir().join(format!("olvin-window-{}", std::process::id()));
        let path = dir.join("window.ron");
        s.save(&path).unwrap();
        assert_eq!(WindowSettings::load_or(&path, WindowSettings::default()), s);
        std::fs::write(&path, "garbage").unwrap();
        let fallback = WindowSettings::default();
        assert_eq!(WindowSettings::load_or(&path, fallback), fallback);
        assert_eq!(
            WindowSettings::load_or(dir.join("none.ron"), s),
            s,
            "missing file gives the fallback"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn plugin_inserts_settings_from_its_fields() {
        use olvin_core::{App, Plugin};
        let plugin = crate::WindowPlugin {
            width: 800,
            height: 600,
            mode: WindowMode::Borderless,
            ..Default::default()
        };
        let mut app = App::new();
        plugin.build(&mut app);
        let s = *app.resources.fetch::<WindowSettings>();
        assert_eq!(
            (s.mode, s.width, s.height),
            (WindowMode::Borderless, 800, 600)
        );
        let from = crate::WindowPlugin::default().with_settings(s);
        assert_eq!((from.mode, from.width), (WindowMode::Borderless, 800));
    }

    #[test]
    fn resolutions_and_closest_mode() {
        let m = Monitors {
            modes: vec![
                res(1920, 1080, 60),
                res(1920, 1080, 144),
                res(1280, 720, 60),
                res(2560, 1440, 60),
            ],
        };
        assert_eq!(m.resolutions(), [(2560, 1440), (1920, 1080), (1280, 720)]);
        // Exact size: the highest refresh rate wins.
        assert_eq!(m.closest(1920, 1080), Some(1));
        // No exact match: the nearest size.
        assert_eq!(m.closest(1366, 768), Some(2));
        assert_eq!(m.closest(3840, 2160), Some(3));
        assert_eq!(Monitors::default().closest(800, 600), None);
    }
}

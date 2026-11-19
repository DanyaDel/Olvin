//! Action maps: action name → bindings, saved as RON (`assets/input.ron`).

use std::collections::BTreeMap;
use std::path::Path;

use olvin_window::{KeyCode, MouseButton};
use serde::{Deserialize, Serialize};

use crate::{Binding, GamepadButton, Stick};

pub const DEFAULT_PATH: &str = "assets/input.ron";

fn default_deadzone() -> f32 {
    0.15
}

fn default_threshold() -> f32 {
    0.5
}

/// Rebindable actions. An action's value is that of its strongest binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputMap {
    pub actions: BTreeMap<String, Vec<Binding>>,
    /// Stick and axis dead zone (0..1).
    #[serde(default = "default_deadzone")]
    pub deadzone: f32,
    /// Magnitude at which an action counts as pressed.
    #[serde(default = "default_threshold")]
    pub press_threshold: f32,
}

impl Default for InputMap {
    /// Move, Look, Jump, Sprint, Fire, Interact, Pause (+ Ascend for fly cameras),
    /// for keyboard + mouse and gamepad.
    fn default() -> Self {
        use Binding::*;
        let mut map = Self::empty();
        map.bind(
            "Move",
            [
                Binding::wasd(),
                Binding::arrows(),
                GamepadStick {
                    stick: Stick::Left,
                    scale: 1.0,
                },
            ],
        );
        // Mouse pixels per frame; the stick is scaled to feel similar at 60 fps.
        map.bind(
            "Look",
            [
                MouseMotion { scale: 1.0 },
                GamepadStick {
                    stick: Stick::Right,
                    scale: 12.0,
                },
            ],
        );
        map.bind(
            "Jump",
            [
                Key(KeyCode::Space),
                GamepadButton(self::GamepadButton::South),
            ],
        );
        map.bind(
            "Sprint",
            [
                Key(KeyCode::ShiftLeft),
                GamepadButton(self::GamepadButton::LeftStick),
            ],
        );
        map.bind(
            "Fire",
            [
                Mouse(MouseButton::Left),
                GamepadButton(self::GamepadButton::RightTrigger),
            ],
        );
        map.bind(
            "Interact",
            [Key(KeyCode::KeyE), GamepadButton(self::GamepadButton::West)],
        );
        map.bind(
            "Pause",
            [
                Key(KeyCode::Escape),
                GamepadButton(self::GamepadButton::Start),
            ],
        );
        map.bind(
            "Ascend",
            [
                Binding::keys_1d(KeyCode::ControlLeft, KeyCode::Space),
                Axis1D {
                    negative: Box::new(GamepadButton(self::GamepadButton::LeftBumper)),
                    positive: Box::new(GamepadButton(self::GamepadButton::RightBumper)),
                },
            ],
        );
        map
    }
}

impl InputMap {
    pub fn empty() -> Self {
        Self {
            actions: BTreeMap::new(),
            deadzone: default_deadzone(),
            press_threshold: default_threshold(),
        }
    }

    /// Replaces the bindings of an action.
    pub fn bind(&mut self, action: &str, bindings: impl IntoIterator<Item = Binding>) -> &mut Self {
        self.actions
            .insert(action.to_owned(), bindings.into_iter().collect());
        self
    }

    /// Adds one binding to an action (creating it).
    pub fn add(&mut self, action: &str, binding: Binding) -> &mut Self {
        self.actions
            .entry(action.to_owned())
            .or_default()
            .push(binding);
        self
    }

    /// Replaces the binding at `index` (appends if it is past the end).
    pub fn set_binding(&mut self, action: &str, index: usize, binding: Binding) -> &mut Self {
        let list = self.actions.entry(action.to_owned()).or_default();
        match list.get_mut(index) {
            Some(slot) => *slot = binding,
            None => list.push(binding),
        }
        self
    }

    /// Removes the binding at `index`; false if there is none.
    pub fn remove_binding(&mut self, action: &str, index: usize) -> bool {
        match self.actions.get_mut(action) {
            Some(list) if index < list.len() => {
                list.remove(index);
                true
            }
            _ => false,
        }
    }

    /// Puts one action back to what `defaults` has (removes it if absent there).
    pub fn reset_action(&mut self, action: &str, defaults: &InputMap) {
        match defaults.actions.get(action) {
            Some(list) => {
                self.actions.insert(action.to_owned(), list.clone());
            }
            None => {
                self.actions.remove(action);
            }
        }
    }

    /// Other actions that already use `binding` as one of their bindings
    /// (`(action, index)`), skipping `except`.
    pub fn conflicts(&self, binding: &Binding, except: Option<&str>) -> Vec<(String, usize)> {
        let mut found = Vec::new();
        for (name, list) in &self.actions {
            if Some(name.as_str()) == except {
                continue;
            }
            found.extend(
                list.iter()
                    .enumerate()
                    .filter(|(_, b)| *b == binding)
                    .map(|(i, _)| (name.clone(), i)),
            );
        }
        found
    }

    /// Adds the actions of `defaults` that this map lacks (a saved user map
    /// after the game gained a new action).
    pub fn fill_missing(&mut self, defaults: &InputMap) {
        for (name, list) in &defaults.actions {
            self.actions
                .entry(name.clone())
                .or_insert_with(|| list.clone());
        }
    }

    pub fn from_ron(text: &str) -> Result<Self, String> {
        ron::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .expect("input maps always serialize")
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

    /// The map at `path` if it exists and parses (errors are logged), else the default.
    pub fn load_or_default(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        if !path.exists() {
            return Self::default();
        }
        Self::load(path).unwrap_or_else(|err| {
            log::error!("input map: {err}; using the default map");
            Self::default()
        })
    }
}

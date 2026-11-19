//! Per-frame action values with press/release edges.

use std::collections::HashMap;

use glam::Vec2;
use olvin_window::Input;

use crate::binding::Raw;
use crate::{Device, GamepadAxis, GamepadButton, GamepadState, InputMap};

/// One action this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActionValue {
    /// Strongest binding's value (buttons: `(1, 0)`).
    pub value: Vec2,
    pub pressed: bool,
    pub just_pressed: bool,
    pub just_released: bool,
}

/// Read side of input for gameplay: actions by name, plus raw mouse data and
/// cursor requests. Unknown action names read as idle.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionState {
    actions: HashMap<String, ActionValue>,
    /// Cursor position in physical pixels.
    pub mouse_position: Vec2,
    /// Raw mouse movement this frame in pixels (+Y down, like the screen).
    pub mouse_delta: Vec2,
    pub scroll: Vec2,
    /// Request: lock the cursor to the window (mouse look). Applied after the
    /// frame when a window exists (`olvin_window::CursorOptions`).
    pub cursor_locked: bool,
    pub cursor_visible: bool,
    /// Device touched last, for choosing prompts ([`InputMap::hint`]).
    pub device: Device,
}

impl Default for ActionState {
    fn default() -> Self {
        Self {
            actions: HashMap::new(),
            mouse_position: Vec2::ZERO,
            mouse_delta: Vec2::ZERO,
            scroll: Vec2::ZERO,
            cursor_locked: false,
            cursor_visible: true,
            device: Device::default(),
        }
    }
}

impl ActionState {
    /// Recomputes every action from raw devices; edges compare with the previous call.
    pub fn update(&mut self, map: &InputMap, input: &Input, gamepads: &GamepadState) {
        let raw = Raw {
            input,
            gamepads,
            deadzone: map.deadzone,
            threshold: map.press_threshold,
        };
        self.mouse_position = input.cursor;
        self.mouse_delta = input.mouse_delta;
        self.scroll = input.scroll;
        self.track_device(&raw);
        for (name, bindings) in &map.actions {
            let value = bindings
                .iter()
                .map(|b| b.value(&raw))
                .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
                .unwrap_or_default();
            let pressed = value.length() >= map.press_threshold;
            let state = self.actions.entry(name.clone()).or_default();
            let was = state.pressed;
            *state = ActionValue {
                value,
                pressed,
                just_pressed: pressed && !was,
                just_released: !pressed && was,
            };
        }
        // Actions removed from the map go idle.
        self.actions
            .retain(|name, _| map.actions.contains_key(name));
    }

    /// Switches `device` when something on the other one is used this frame.
    fn track_device(&mut self, raw: &Raw) {
        let input = raw.input;
        let keyboard_mouse = input.pressed_keys().next().is_some()
            || input.pressed_mouse().next().is_some()
            || input.mouse_delta != Vec2::ZERO
            || input.scroll != Vec2::ZERO;
        let pads = raw.gamepads;
        let gamepad = GamepadButton::ALL.iter().any(|b| pads.pressed(*b))
            || GamepadAxis::ALL
                .iter()
                .any(|a| pads.axis(*a).abs() > raw.deadzone);
        // Both at once (a held key while the pad drifts): keep the current one.
        match (keyboard_mouse, gamepad) {
            (true, false) => self.device = Device::KeyboardMouse,
            (false, true) => self.device = Device::Gamepad,
            _ => {}
        }
    }

    pub fn get(&self, action: &str) -> ActionValue {
        self.actions.get(action).copied().unwrap_or_default()
    }

    pub fn pressed(&self, action: &str) -> bool {
        self.get(action).pressed
    }

    pub fn just_pressed(&self, action: &str) -> bool {
        self.get(action).just_pressed
    }

    pub fn just_released(&self, action: &str) -> bool {
        self.get(action).just_released
    }

    /// 1D value (`x`): -1..1 for axes, 0..1 for buttons and triggers.
    pub fn value(&self, action: &str) -> f32 {
        self.get(action).value.x
    }

    /// 2D value: composites and sticks (+Y up / forward).
    pub fn axis2d(&self, action: &str) -> Vec2 {
        self.get(action).value
    }

    /// Locks and hides the cursor for mouse look, or frees it.
    pub fn lock_cursor(&mut self, locked: bool) {
        self.cursor_locked = locked;
        self.cursor_visible = !locked;
    }

    pub fn actions(&self) -> impl Iterator<Item = (&str, &ActionValue)> {
        self.actions.iter().map(|(k, v)| (k.as_str(), v))
    }
}

use std::collections::HashSet;

use glam::Vec2;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// Keyboard and mouse state for the current frame.
#[derive(Debug, Default)]
pub struct Input {
    keys: HashSet<KeyCode>,
    keys_pressed: HashSet<KeyCode>,
    keys_released: HashSet<KeyCode>,
    buttons: HashSet<MouseButton>,
    buttons_pressed: HashSet<MouseButton>,
    /// Cursor position in physical pixels.
    pub cursor: Vec2,
    /// Raw mouse movement accumulated this frame.
    pub mouse_delta: Vec2,
}

impl Input {
    pub fn pressed(&self, key: KeyCode) -> bool {
        self.keys.contains(&key)
    }

    pub fn just_pressed(&self, key: KeyCode) -> bool {
        self.keys_pressed.contains(&key)
    }

    pub fn just_released(&self, key: KeyCode) -> bool {
        self.keys_released.contains(&key)
    }

    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.buttons.contains(&button)
    }

    pub fn mouse_just_pressed(&self, button: MouseButton) -> bool {
        self.buttons_pressed.contains(&button)
    }

    pub(crate) fn set_key(&mut self, key: KeyCode, down: bool) {
        if down {
            if self.keys.insert(key) {
                self.keys_pressed.insert(key);
            }
        } else if self.keys.remove(&key) {
            self.keys_released.insert(key);
        }
    }

    pub(crate) fn set_button(&mut self, button: MouseButton, down: bool) {
        if down {
            if self.buttons.insert(button) {
                self.buttons_pressed.insert(button);
            }
        } else {
            self.buttons.remove(&button);
        }
    }

    /// Focus loss: the window never sees the key-up events.
    pub(crate) fn release_all(&mut self) {
        self.keys_released.extend(self.keys.drain());
        self.buttons.clear();
    }

    pub(crate) fn end_frame(&mut self) {
        self.keys_pressed.clear();
        self.keys_released.clear();
        self.buttons_pressed.clear();
        self.mouse_delta = Vec2::ZERO;
    }
}

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
    buttons_released: HashSet<MouseButton>,
    /// Cursor position in physical pixels.
    pub cursor: Vec2,
    /// Raw mouse movement accumulated this frame.
    pub mouse_delta: Vec2,
    /// Wheel movement this frame in lines (notches); +y is away from the user.
    pub scroll: Vec2,
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

    pub fn mouse_just_released(&self, button: MouseButton) -> bool {
        self.buttons_released.contains(&button)
    }

    /// Keys held right now (e.g. for a rebinding UI).
    pub fn pressed_keys(&self) -> impl Iterator<Item = KeyCode> + '_ {
        self.keys.iter().copied()
    }

    /// Mouse buttons held right now.
    pub fn pressed_mouse(&self) -> impl Iterator<Item = MouseButton> + '_ {
        self.buttons.iter().copied()
    }

    // The setters below are what the platform layer calls; they are public so
    // other platforms (SDL, web) and tests can feed synthetic input.

    pub fn set_key(&mut self, key: KeyCode, down: bool) {
        if down {
            if self.keys.insert(key) {
                self.keys_pressed.insert(key);
            }
        } else if self.keys.remove(&key) {
            self.keys_released.insert(key);
        }
    }

    pub fn set_button(&mut self, button: MouseButton, down: bool) {
        if down {
            if self.buttons.insert(button) {
                self.buttons_pressed.insert(button);
            }
        } else if self.buttons.remove(&button) {
            self.buttons_released.insert(button);
        }
    }

    /// Focus loss: the window never sees the key-up events.
    pub fn release_all(&mut self) {
        self.keys_released.extend(self.keys.drain());
        self.buttons_released.extend(self.buttons.drain());
    }

    /// Clears per-frame edges and deltas (`Stage::Last`).
    pub fn end_frame(&mut self) {
        self.keys_pressed.clear();
        self.keys_released.clear();
        self.buttons_pressed.clear();
        self.buttons_released.clear();
        self.mouse_delta = Vec2::ZERO;
        self.scroll = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_last_one_frame() {
        let mut input = Input::default();
        input.set_key(KeyCode::Space, true);
        input.set_button(MouseButton::Left, true);
        assert!(input.just_pressed(KeyCode::Space) && input.pressed(KeyCode::Space));
        input.end_frame();
        assert!(!input.just_pressed(KeyCode::Space) && input.pressed(KeyCode::Space));
        input.set_button(MouseButton::Left, false);
        assert!(input.mouse_just_released(MouseButton::Left));
        input.release_all();
        assert!(input.just_released(KeyCode::Space));
        input.end_frame();
        assert!(!input.just_released(KeyCode::Space) && !input.pressed(KeyCode::Space));
    }
}

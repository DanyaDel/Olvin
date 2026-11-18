//! Gamepad state merged over every connected pad, and the gilrs backend.

use std::collections::{HashMap, HashSet};

use crate::{GamepadAxis, GamepadButton};

/// Buttons and axes of all connected gamepads merged (any pad can play).
/// Filled by gilrs with the `gamepad` feature; tests and other backends can
/// write it directly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GamepadState {
    buttons: HashSet<GamepadButton>,
    axes: HashMap<GamepadAxis, f32>,
    /// Connected gamepads.
    pub connected: usize,
}

impl GamepadState {
    pub fn pressed(&self, button: GamepadButton) -> bool {
        self.buttons.contains(&button)
    }

    pub fn axis(&self, axis: GamepadAxis) -> f32 {
        self.axes.get(&axis).copied().unwrap_or(0.0)
    }

    pub fn set_button(&mut self, button: GamepadButton, down: bool) {
        if down {
            self.buttons.insert(button);
        } else {
            self.buttons.remove(&button);
        }
    }

    pub fn set_axis(&mut self, axis: GamepadAxis, value: f32) {
        self.axes.insert(axis, value.clamp(-1.0, 1.0));
    }

    pub fn clear(&mut self) {
        self.buttons.clear();
        self.axes.clear();
        self.connected = 0;
    }
}

#[cfg(feature = "gamepad")]
pub(crate) mod backend {
    use gilrs::{Axis, Button, Gilrs};

    use super::*;

    /// The gilrs context; `None` when no gamepad subsystem is available.
    pub(crate) struct Gamepads(pub Option<Gilrs>);

    impl Gamepads {
        pub(crate) fn new() -> Self {
            match Gilrs::new() {
                Ok(g) => Self(Some(g)),
                Err(gilrs::Error::NotImplemented(g)) => {
                    log::info!("gamepads are not supported on this platform");
                    Self(Some(g))
                }
                Err(err) => {
                    log::warn!("gamepads unavailable: {err}");
                    Self(None)
                }
            }
        }

        /// Drains events and rebuilds the merged state.
        pub(crate) fn poll(&mut self, state: &mut GamepadState) {
            let Some(gilrs) = &mut self.0 else { return };
            while gilrs.next_event().is_some() {}
            state.clear();
            for (_, pad) in gilrs.gamepads() {
                state.connected += 1;
                for b in GamepadButton::ALL {
                    if pad.is_pressed(button(b)) {
                        state.set_button(b, true);
                    }
                }
                for a in GamepadAxis::ALL {
                    let v = match a {
                        GamepadAxis::LeftTrigger => {
                            trigger(&pad, Button::LeftTrigger2, Axis::LeftZ)
                        }
                        GamepadAxis::RightTrigger => {
                            trigger(&pad, Button::RightTrigger2, Axis::RightZ)
                        }
                        _ => pad.value(axis(a)),
                    };
                    // Strongest pad wins.
                    if v.abs() > state.axis(a).abs() {
                        state.set_axis(a, v);
                    }
                }
            }
        }
    }

    fn trigger(pad: &gilrs::Gamepad<'_>, b: Button, a: Axis) -> f32 {
        let analog = pad.button_data(b).map_or(0.0, |d| d.value());
        analog.max(pad.value(a).max(0.0))
    }

    fn button(b: GamepadButton) -> Button {
        match b {
            GamepadButton::South => Button::South,
            GamepadButton::East => Button::East,
            GamepadButton::North => Button::North,
            GamepadButton::West => Button::West,
            GamepadButton::LeftBumper => Button::LeftTrigger,
            GamepadButton::RightBumper => Button::RightTrigger,
            GamepadButton::LeftTrigger => Button::LeftTrigger2,
            GamepadButton::RightTrigger => Button::RightTrigger2,
            GamepadButton::Select => Button::Select,
            GamepadButton::Start => Button::Start,
            GamepadButton::Mode => Button::Mode,
            GamepadButton::LeftStick => Button::LeftThumb,
            GamepadButton::RightStick => Button::RightThumb,
            GamepadButton::DPadUp => Button::DPadUp,
            GamepadButton::DPadDown => Button::DPadDown,
            GamepadButton::DPadLeft => Button::DPadLeft,
            GamepadButton::DPadRight => Button::DPadRight,
        }
    }

    fn axis(a: GamepadAxis) -> Axis {
        match a {
            GamepadAxis::LeftStickX => Axis::LeftStickX,
            GamepadAxis::LeftStickY => Axis::LeftStickY,
            GamepadAxis::RightStickX => Axis::RightStickX,
            GamepadAxis::RightStickY => Axis::RightStickY,
            GamepadAxis::LeftTrigger => Axis::LeftZ,
            GamepadAxis::RightTrigger => Axis::RightZ,
        }
    }
}

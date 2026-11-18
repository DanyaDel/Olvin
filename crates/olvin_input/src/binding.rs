//! What a physical control is: keys, buttons, axes and composites of them.

use glam::Vec2;
use olvin_window::{Input, KeyCode, MouseButton};
use serde::{Deserialize, Serialize};

use crate::GamepadState;

/// Gamepad buttons in a layout-neutral naming (Xbox A = `South`, PS Cross = `South`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GamepadButton {
    South,
    East,
    North,
    West,
    LeftBumper,
    RightBumper,
    /// Pressed past half travel; the analog value is [`GamepadAxis::LeftTrigger`].
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    Mode,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

impl GamepadButton {
    pub const ALL: [GamepadButton; 17] = [
        Self::South,
        Self::East,
        Self::North,
        Self::West,
        Self::LeftBumper,
        Self::RightBumper,
        Self::LeftTrigger,
        Self::RightTrigger,
        Self::Select,
        Self::Start,
        Self::Mode,
        Self::LeftStick,
        Self::RightStick,
        Self::DPadUp,
        Self::DPadDown,
        Self::DPadLeft,
        Self::DPadRight,
    ];
}

/// Analog gamepad inputs. Sticks are -1..1 with +Y up, triggers 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GamepadAxis {
    LeftStickX,
    LeftStickY,
    RightStickX,
    RightStickY,
    LeftTrigger,
    RightTrigger,
}

impl GamepadAxis {
    pub const ALL: [GamepadAxis; 6] = [
        Self::LeftStickX,
        Self::LeftStickY,
        Self::RightStickX,
        Self::RightStickY,
        Self::LeftTrigger,
        Self::RightTrigger,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Stick {
    Left,
    Right,
}

fn one() -> f32 {
    1.0
}

fn is_one(v: &f32) -> bool {
    *v == 1.0
}

/// One way to drive an action. Every binding yields a 2D value: buttons give
/// `(1, 0)` while held, 1D axes use `x`, sticks and composites use both.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Binding {
    Key(KeyCode),
    Mouse(MouseButton),
    /// Raw mouse movement this frame in pixels × `scale`, +Y up.
    MouseMotion {
        #[serde(default = "one", skip_serializing_if = "is_one")]
        scale: f32,
    },
    /// Wheel movement this frame in notches × `scale`, +Y away from the user.
    Scroll {
        #[serde(default = "one", skip_serializing_if = "is_one")]
        scale: f32,
    },
    GamepadButton(GamepadButton),
    GamepadAxis {
        axis: GamepadAxis,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        invert: bool,
    },
    /// Both axes of a stick (with the map's dead zone) × `scale`.
    GamepadStick {
        stick: Stick,
        #[serde(default = "one", skip_serializing_if = "is_one")]
        scale: f32,
    },
    /// A 1D axis from two controls: `positive - negative`.
    Axis1D {
        negative: Box<Binding>,
        positive: Box<Binding>,
    },
    /// A 2D axis from four controls (WASD, arrows, d-pad), normalised so
    /// diagonals are not faster.
    Axis2D {
        up: Box<Binding>,
        down: Box<Binding>,
        left: Box<Binding>,
        right: Box<Binding>,
    },
    /// `binding` counts only while every modifier is held (Ctrl+S, Shift+click).
    Chord {
        modifiers: Vec<Binding>,
        binding: Box<Binding>,
    },
}

impl Binding {
    /// `Axis2D` from four keys.
    pub fn keys_2d(up: KeyCode, down: KeyCode, left: KeyCode, right: KeyCode) -> Self {
        Self::Axis2D {
            up: Box::new(Self::Key(up)),
            down: Box::new(Self::Key(down)),
            left: Box::new(Self::Key(left)),
            right: Box::new(Self::Key(right)),
        }
    }

    pub fn wasd() -> Self {
        Self::keys_2d(KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyA, KeyCode::KeyD)
    }

    pub fn arrows() -> Self {
        Self::keys_2d(
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
        )
    }

    pub fn keys_1d(negative: KeyCode, positive: KeyCode) -> Self {
        Self::Axis1D {
            negative: Box::new(Self::Key(negative)),
            positive: Box::new(Self::Key(positive)),
        }
    }

    pub fn chord(modifiers: impl IntoIterator<Item = KeyCode>, binding: Binding) -> Self {
        Self::Chord {
            modifiers: modifiers.into_iter().map(Self::Key).collect(),
            binding: Box::new(binding),
        }
    }

    /// Current value from raw device state.
    pub fn value(&self, raw: &Raw) -> Vec2 {
        let button = |down: bool| if down { Vec2::X } else { Vec2::ZERO };
        match self {
            // A tap shorter than a frame still counts for the frame it happened in.
            Self::Key(k) => button(raw.input.pressed(*k) || raw.input.just_pressed(*k)),
            Self::Mouse(b) => {
                button(raw.input.mouse_pressed(*b) || raw.input.mouse_just_pressed(*b))
            }
            Self::MouseMotion { scale } => {
                Vec2::new(raw.input.mouse_delta.x, -raw.input.mouse_delta.y) * *scale
            }
            Self::Scroll { scale } => raw.input.scroll * *scale,
            Self::GamepadButton(b) => button(raw.gamepads.pressed(*b)),
            Self::GamepadAxis { axis, invert } => {
                let v = raw.gamepads.axis(*axis);
                let v = if v.abs() < raw.deadzone { 0.0 } else { v };
                Vec2::new(if *invert { -v } else { v }, 0.0)
            }
            Self::GamepadStick { stick, scale } => {
                let (x, y) = match stick {
                    Stick::Left => (GamepadAxis::LeftStickX, GamepadAxis::LeftStickY),
                    Stick::Right => (GamepadAxis::RightStickX, GamepadAxis::RightStickY),
                };
                let v = Vec2::new(raw.gamepads.axis(x), raw.gamepads.axis(y));
                radial_deadzone(v, raw.deadzone) * *scale
            }
            Self::Axis1D { negative, positive } => Vec2::new(
                positive.value(raw).length() - negative.value(raw).length(),
                0.0,
            ),
            Self::Axis2D {
                up,
                down,
                left,
                right,
            } => {
                let v = Vec2::new(
                    right.value(raw).length() - left.value(raw).length(),
                    up.value(raw).length() - down.value(raw).length(),
                );
                if v.length_squared() > 1.0 {
                    v.normalize()
                } else {
                    v
                }
            }
            Self::Chord { modifiers, binding } => {
                let held = modifiers
                    .iter()
                    .all(|m| m.value(raw).length() >= raw.threshold);
                if held { binding.value(raw) } else { Vec2::ZERO }
            }
        }
    }
}

/// Rescales so the stick reads 0 inside the dead zone and 1 at full tilt.
fn radial_deadzone(v: Vec2, deadzone: f32) -> Vec2 {
    let len = v.length();
    if len <= deadzone || len == 0.0 {
        return Vec2::ZERO;
    }
    let scaled = ((len - deadzone) / (1.0 - deadzone).max(1e-3)).min(1.0);
    v / len * scaled
}

/// Everything bindings read in one frame.
pub struct Raw<'a> {
    pub input: &'a Input,
    pub gamepads: &'a GamepadState,
    pub deadzone: f32,
    pub threshold: f32,
}

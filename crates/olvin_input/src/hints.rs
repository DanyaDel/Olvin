//! Button prompts: what to call a control on the player's current device
//! ("Space", "A", "Cross", "Ctrl+S").

use olvin_window::{KeyCode, MouseButton};

use crate::{Binding, GamepadAxis, GamepadButton, InputMap, Stick};

/// Which device the player last used; prompts follow it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Device {
    #[default]
    KeyboardMouse,
    Gamepad,
}

/// Face button naming of a gamepad family. The layout-neutral
/// [`GamepadButton::South`] is A on Xbox, Cross on PlayStation, B on Nintendo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadStyle {
    #[default]
    Xbox,
    PlayStation,
    Nintendo,
}

impl PadStyle {
    /// Guesses the family from the USB vendor id and the device name.
    pub fn detect(vendor_id: Option<u16>, name: &str) -> Self {
        let name = name.to_lowercase();
        match vendor_id {
            Some(0x054c) => Self::PlayStation,
            Some(0x057e) => Self::Nintendo,
            Some(0x045e) => Self::Xbox,
            _ if name.contains("dualshock")
                || name.contains("dualsense")
                || name.contains("playstation")
                || name.contains("wireless controller") =>
            {
                Self::PlayStation
            }
            _ if name.contains("nintendo") || name.contains("switch") => Self::Nintendo,
            _ => Self::Xbox,
        }
    }
}

impl GamepadButton {
    /// The name printed on the button for this family.
    pub fn label(self, style: PadStyle) -> &'static str {
        use GamepadButton::*;
        use PadStyle::*;
        match (self, style) {
            (South, Xbox) => "A",
            (East, Xbox) => "B",
            (West, Xbox) => "X",
            (North, Xbox) => "Y",
            (South, PlayStation) => "Cross",
            (East, PlayStation) => "Circle",
            (West, PlayStation) => "Square",
            (North, PlayStation) => "Triangle",
            (South, Nintendo) => "B",
            (East, Nintendo) => "A",
            (West, Nintendo) => "Y",
            (North, Nintendo) => "X",
            (LeftBumper, Xbox) => "LB",
            (RightBumper, Xbox) => "RB",
            (LeftBumper, PlayStation) => "L1",
            (RightBumper, PlayStation) => "R1",
            (LeftBumper, Nintendo) => "L",
            (RightBumper, Nintendo) => "R",
            (LeftTrigger, Xbox) => "LT",
            (RightTrigger, Xbox) => "RT",
            (LeftTrigger, PlayStation) => "L2",
            (RightTrigger, PlayStation) => "R2",
            (LeftTrigger, Nintendo) => "ZL",
            (RightTrigger, Nintendo) => "ZR",
            (Select, Xbox) => "View",
            (Select, PlayStation) => "Share",
            (Select, Nintendo) => "-",
            (Start, Xbox) => "Menu",
            (Start, PlayStation) => "Options",
            (Start, Nintendo) => "+",
            (Mode, Xbox) => "Xbox",
            (Mode, PlayStation) => "PS",
            (Mode, Nintendo) => "Home",
            (LeftStick, PlayStation) => "L3",
            (RightStick, PlayStation) => "R3",
            (LeftStick, _) => "LS Click",
            (RightStick, _) => "RS Click",
            (DPadUp, _) => "D-pad Up",
            (DPadDown, _) => "D-pad Down",
            (DPadLeft, _) => "D-pad Left",
            (DPadRight, _) => "D-pad Right",
        }
    }
}

impl GamepadAxis {
    pub fn label(self, style: PadStyle) -> &'static str {
        match self {
            Self::LeftStickX => "Left Stick X",
            Self::LeftStickY => "Left Stick Y",
            Self::RightStickX => "Right Stick X",
            Self::RightStickY => "Right Stick Y",
            Self::LeftTrigger => GamepadButton::LeftTrigger.label(style),
            Self::RightTrigger => GamepadButton::RightTrigger.label(style),
        }
    }
}

/// Human name of a key: `KeyW` → "W", `Digit1` → "1", `ControlLeft` → "Left Ctrl".
pub fn key_label(key: KeyCode) -> String {
    let name = format!("{key:?}");
    if let Some(rest) = name
        .strip_prefix("Key")
        .or_else(|| name.strip_prefix("Digit"))
        && rest.len() == 1
    {
        return rest.to_owned();
    }
    let special = match key {
        KeyCode::ControlLeft => "Left Ctrl",
        KeyCode::ControlRight => "Right Ctrl",
        KeyCode::ShiftLeft => "Left Shift",
        KeyCode::ShiftRight => "Right Shift",
        KeyCode::AltLeft => "Left Alt",
        KeyCode::AltRight => "Right Alt",
        KeyCode::SuperLeft | KeyCode::SuperRight => "Super",
        KeyCode::Escape => "Esc",
        KeyCode::ArrowUp => "Up",
        KeyCode::ArrowDown => "Down",
        KeyCode::ArrowLeft => "Left",
        KeyCode::ArrowRight => "Right",
        KeyCode::Backquote => "`",
        KeyCode::Minus => "-",
        KeyCode::Equal => "=",
        KeyCode::Comma => ",",
        KeyCode::Period => ".",
        KeyCode::Slash => "/",
        KeyCode::Backslash => "\\",
        KeyCode::Semicolon => ";",
        KeyCode::Quote => "'",
        KeyCode::BracketLeft => "[",
        KeyCode::BracketRight => "]",
        _ => return spaced(&name),
    };
    special.to_owned()
}

/// `PageUp` → "Page Up", `Numpad5` → "Numpad 5".
fn spaced(name: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        let boundary = prev.is_some_and(|p| {
            (c.is_uppercase() && !p.is_uppercase() && !p.is_ascii_digit())
                || (c.is_ascii_digit() && p.is_lowercase())
        });
        if boundary {
            out.push(' ');
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

pub fn mouse_label(button: MouseButton) -> String {
    match button {
        MouseButton::Left => "Left Click".into(),
        MouseButton::Right => "Right Click".into(),
        MouseButton::Middle => "Middle Click".into(),
        MouseButton::Back => "Mouse Back".into(),
        MouseButton::Forward => "Mouse Forward".into(),
        MouseButton::Other(n) => format!("Mouse {n}"),
    }
}

impl Binding {
    /// Which device this binding is for; `None` for a composite mixing both
    /// or one with nothing in it.
    pub fn device(&self) -> Option<Device> {
        match self {
            Self::Key(_) | Self::Mouse(_) | Self::MouseMotion { .. } | Self::Scroll { .. } => {
                Some(Device::KeyboardMouse)
            }
            Self::GamepadButton(_) | Self::GamepadAxis { .. } | Self::GamepadStick { .. } => {
                Some(Device::Gamepad)
            }
            Self::Axis1D { negative, positive } => merge([&**negative, &**positive]),
            Self::Axis2D {
                up,
                down,
                left,
                right,
            } => merge([&**up, &**down, &**left, &**right]),
            Self::Chord { modifiers, binding } => merge(modifiers.iter().chain([&**binding])),
        }
    }

    /// Prompt text: "Space", "Left Click", "Cross", "W/A/S/D", "Left Ctrl+S".
    /// `style` only matters for gamepad controls.
    pub fn label(&self, style: PadStyle) -> String {
        match self {
            Self::Key(k) => key_label(*k),
            Self::Mouse(b) => mouse_label(*b),
            Self::MouseMotion { .. } => "Mouse".into(),
            Self::Scroll { .. } => "Mouse Wheel".into(),
            Self::GamepadButton(b) => b.label(style).into(),
            Self::GamepadAxis { axis, .. } => axis.label(style).into(),
            Self::GamepadStick { stick, .. } => match stick {
                Stick::Left => "Left Stick",
                Stick::Right => "Right Stick",
            }
            .into(),
            Self::Axis1D { negative, positive } => {
                format!("{}/{}", negative.label(style), positive.label(style))
            }
            // Up, left, down, right: reads as WASD.
            Self::Axis2D {
                up,
                down,
                left,
                right,
            } => [up, left, down, right].map(|b| b.label(style)).join("/"),
            Self::Chord { modifiers, binding } => modifiers
                .iter()
                .map(|m| m.label(style))
                .chain([binding.label(style)])
                .collect::<Vec<_>>()
                .join("+"),
        }
    }
}

fn merge<'a>(parts: impl IntoIterator<Item = &'a Binding>) -> Option<Device> {
    let mut found = None;
    for part in parts {
        match (found, part.device()) {
            (_, None) => return None,
            (None, d) => found = d,
            (Some(a), Some(b)) if a != b => return None,
            _ => {}
        }
    }
    found
}

impl InputMap {
    /// Prompts for an action on `device`, in binding order. Bindings of the
    /// other device are skipped.
    pub fn hints(&self, action: &str, device: Device, style: PadStyle) -> Vec<String> {
        self.actions
            .get(action)
            .into_iter()
            .flatten()
            .filter(|b| b.device() == Some(device))
            .map(|b| b.label(style))
            .collect()
    }

    /// The first prompt for an action on `device`, if it has one there.
    pub fn hint(&self, action: &str, device: Device, style: PadStyle) -> Option<String> {
        self.hints(action, device, style).into_iter().next()
    }
}

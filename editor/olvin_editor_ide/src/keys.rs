//! Keyboard routing. The base editor handles Ctrl+S (save scene) and Ctrl+P
//! (play, also matching Ctrl+Shift+P because egui ignores extra Shift) before
//! any panel runs. This egui plugin takes those keys out of the raw input
//! while the code editor has focus (or, for the palette, while an IDE panel
//! is on screen) and hands them to the IDE instead.

use olvin_editor::egui::{self, Event, Key, Modifiers, RawInput};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Captured {
    Save,
    SaveAll,
    NewFile,
    Palette,
    Build,
}

#[derive(Default)]
pub struct KeyRouter {
    /// Set during a frame by the code editor when it has keyboard focus.
    pub code_focused: bool,
    /// Set during a frame when any IDE panel was drawn.
    pub ide_visible: bool,
    captured: Vec<Captured>,
}

/// Which IDE command a key event is, given who had focus last frame.
pub fn route(key: Key, m: Modifiers, code_focused: bool, ide_visible: bool) -> Option<Captured> {
    if !m.command || m.alt {
        return None;
    }
    match (key, m.shift) {
        (Key::S, false) if code_focused => Some(Captured::Save),
        (Key::S, true) if code_focused => Some(Captured::SaveAll),
        (Key::N, false) if code_focused => Some(Captured::NewFile),
        (Key::B, true) if code_focused => Some(Captured::Build),
        (Key::P, true) if ide_visible || code_focused => Some(Captured::Palette),
        _ => None,
    }
}

impl KeyRouter {
    pub fn take(&mut self) -> Vec<Captured> {
        std::mem::take(&mut self.captured)
    }
}

impl egui::Plugin for KeyRouter {
    fn debug_name(&self) -> &'static str {
        "olvin_ide_keys"
    }

    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut RawInput) {
        let code = std::mem::take(&mut self.code_focused);
        let visible = std::mem::take(&mut self.ide_visible);
        let captured = &mut self.captured;
        input.events.retain(|e| {
            let Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } = e
            else {
                return true;
            };
            match route(*key, *modifiers, code, visible) {
                Some(c) => {
                    if *pressed {
                        captured.push(c);
                    }
                    false
                }
                None => true,
            }
        });
    }
}

/// Removes key presses matching exactly (no extra Shift/Alt); returns how many.
pub fn consume_exact(input: &mut egui::InputState, modifiers: Modifiers, key: Key) -> usize {
    let mut n = 0;
    input.events.retain(|e| match e {
        Event::Key {
            key: k,
            pressed: true,
            modifiers: m,
            ..
        } if *k == key && m.matches_exact(modifiers) => {
            n += 1;
            false
        }
        _ => true,
    });
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_editor::egui::Plugin;

    #[test]
    fn routes_only_with_focus() {
        let ctrl = Modifiers::COMMAND;
        let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        assert_eq!(route(Key::S, ctrl, true, true), Some(Captured::Save));
        assert_eq!(route(Key::S, ctrl, false, true), None);
        assert_eq!(
            route(Key::S, ctrl_shift, true, true),
            Some(Captured::SaveAll)
        );
        assert_eq!(
            route(Key::P, ctrl_shift, false, true),
            Some(Captured::Palette)
        );
        assert_eq!(route(Key::P, ctrl, true, true), None, "Ctrl+P stays Play");
        assert_eq!(route(Key::P, ctrl_shift, false, false), None);
        assert_eq!(route(Key::S, Modifiers::NONE, true, true), None);
    }

    #[test]
    fn hook_removes_captured_keys() {
        let ctx = egui::Context::default();
        let mut router = KeyRouter {
            code_focused: true,
            ..Default::default()
        };
        let key = |key, pressed, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        let mut input = RawInput {
            events: vec![
                key(Key::S, true, Modifiers::COMMAND),
                key(Key::S, false, Modifiers::COMMAND),
                key(Key::A, true, Modifiers::NONE),
            ],
            ..Default::default()
        };
        router.input_hook(&ctx, &mut input);
        assert_eq!(input.events.len(), 1);
        assert_eq!(router.take(), vec![Captured::Save]);
        assert!(
            !router.code_focused,
            "focus must be re-reported every frame"
        );
    }
}

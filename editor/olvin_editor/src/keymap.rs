//! The editor's configurable keys: scene and project commands, each with a
//! default chord that the Settings window can change.
//!
//! Only commands listed in [`Command`] are configurable. The keys of Edit
//! Mode, the UV editor, the animation timeline and text fields are fixed
//! (see the Keymap page of Settings). Overrides are stored per user in the
//! preferences as `command id → "Ctrl+Shift+Z"`; a missing or unreadable
//! entry means the default, so a new default reaches everyone who did not
//! change that command.

use std::collections::BTreeMap;
use std::fmt;

use egui::{InputState, Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};

/// Something a key can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Command {
    Save,
    NewScene,
    Undo,
    Redo,
    Duplicate,
    Delete,
    Play,
    Settings,
    ToolView,
    ToolMove,
    ToolRotate,
    ToolScale,
    ToggleSpace,
    FrameSelection,
    ToggleShading,
    KeyTransform,
    DeleteKeys,
}

impl Command {
    pub const ALL: [Command; 17] = [
        Command::Save,
        Command::NewScene,
        Command::Undo,
        Command::Redo,
        Command::Duplicate,
        Command::Delete,
        Command::Play,
        Command::Settings,
        Command::ToolView,
        Command::ToolMove,
        Command::ToolRotate,
        Command::ToolScale,
        Command::ToggleSpace,
        Command::FrameSelection,
        Command::ToggleShading,
        Command::KeyTransform,
        Command::DeleteKeys,
    ];

    /// The name in the preferences file.
    pub fn id(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::NewScene => "new_scene",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Duplicate => "duplicate",
            Self::Delete => "delete",
            Self::Play => "play",
            Self::Settings => "settings",
            Self::ToolView => "tool_view",
            Self::ToolMove => "tool_move",
            Self::ToolRotate => "tool_rotate",
            Self::ToolScale => "tool_scale",
            Self::ToggleSpace => "toggle_space",
            Self::FrameSelection => "frame_selection",
            Self::ToggleShading => "toggle_shading",
            Self::KeyTransform => "key_transform",
            Self::DeleteKeys => "delete_keys",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Save => "Save scene",
            Self::NewScene => "New scene",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Duplicate => "Duplicate selection",
            Self::Delete => "Delete selection",
            Self::Play => "Play / Stop",
            Self::Settings => "Settings",
            Self::ToolView => "Hand tool",
            Self::ToolMove => "Move tool",
            Self::ToolRotate => "Rotate tool",
            Self::ToolScale => "Scale tool",
            Self::ToggleSpace => "Global / Local space",
            Self::FrameSelection => "Frame selection",
            Self::ToggleShading => "Solid / Rendered",
            Self::KeyTransform => "Key Location, Rotation, Scale",
            Self::DeleteKeys => "Delete keys on this frame",
        }
    }

    /// The group it is listed under in Settings.
    pub fn group(self) -> &'static str {
        match self {
            Self::Save
            | Self::NewScene
            | Self::Undo
            | Self::Redo
            | Self::Duplicate
            | Self::Delete
            | Self::Play
            | Self::Settings => "Editor",
            _ => "Scene view",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    /// The keys this does until the user changes it (Redo has two, as in most editors).
    pub fn defaults(self) -> Vec<Chord> {
        let plain = |key| Chord::plain(key);
        let cmd = |key| Chord {
            command: true,
            ..Chord::plain(key)
        };
        match self {
            Self::Save => vec![cmd(Key::S)],
            Self::NewScene => vec![cmd(Key::N)],
            Self::Undo => vec![cmd(Key::Z)],
            Self::Redo => vec![
                Chord {
                    shift: true,
                    ..cmd(Key::Z)
                },
                cmd(Key::Y),
            ],
            Self::Duplicate => vec![cmd(Key::D)],
            Self::Delete => vec![plain(Key::Delete)],
            Self::Play => vec![cmd(Key::P)],
            Self::Settings => vec![cmd(Key::Comma)],
            Self::ToolView => vec![plain(Key::Q)],
            Self::ToolMove => vec![plain(Key::W)],
            Self::ToolRotate => vec![plain(Key::E)],
            Self::ToolScale => vec![plain(Key::R)],
            Self::ToggleSpace => vec![plain(Key::X)],
            Self::FrameSelection => vec![plain(Key::F)],
            Self::ToggleShading => vec![Chord {
                shift: true,
                ..plain(Key::Z)
            }],
            Self::KeyTransform => vec![plain(Key::I)],
            Self::DeleteKeys => vec![Chord {
                alt: true,
                ..plain(Key::I)
            }],
        }
    }
}

/// A key with modifiers. `command` is Ctrl (Cmd on a Mac).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    pub fn plain(key: Key) -> Self {
        Self {
            key,
            command: false,
            shift: false,
            alt: false,
        }
    }

    fn modifiers(&self) -> Modifiers {
        Modifiers {
            command: self.command,
            shift: self.shift,
            alt: self.alt,
            ..Modifiers::NONE
        }
    }

    /// Whether the chord was pressed this frame. A chord with a modifier is
    /// consumed (so a text field or another handler does not see it); a bare
    /// key is only looked at, like the tool keys always were.
    pub fn pressed(&self, input: &mut InputState) -> bool {
        if self.command || self.shift || self.alt {
            input.consume_shortcut(&KeyboardShortcut::new(self.modifiers(), self.key))
        } else {
            let key = self.key;
            input.events.iter().any(|e| {
                matches!(e, egui::Event::Key { key: k, pressed: true, repeat: false, modifiers, .. }
                    if *k == key && modifiers.is_none())
            })
        }
    }

    /// `Ctrl+Shift+Z`; `None` when the text is not a chord.
    pub fn parse(text: &str) -> Option<Self> {
        let mut chord = Self::plain(Key::A);
        let mut key = None;
        // The key itself may be `+` (Ctrl++): split on the last separator only.
        let parts: Vec<&str> = if let Some(head) = text.strip_suffix("++") {
            head.split('+')
                .filter(|p| !p.is_empty())
                .chain(std::iter::once("+"))
                .collect()
        } else {
            text.split('+').collect()
        };
        for part in parts {
            match part.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "command" => chord.command = true,
                "shift" => chord.shift = true,
                "alt" => chord.alt = true,
                _ => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(Key::from_name(part.trim())?);
                }
            }
        }
        chord.key = key?;
        Some(chord)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.command {
            f.write_str("Ctrl+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        f.write_str(self.key.name())
    }
}

/// The user's changes to the default keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keymap {
    keys: BTreeMap<String, String>,
}

impl Keymap {
    /// The chords of `command`: the user's one, else the defaults.
    pub fn chords(&self, command: Command) -> Vec<Chord> {
        match self.keys.get(command.id()).and_then(|t| Chord::parse(t)) {
            Some(chord) => vec![chord],
            None => command.defaults(),
        }
    }

    /// Whether `command` was pressed this frame.
    pub fn pressed(&self, input: &mut InputState, command: Command) -> bool {
        // Not `any`: every chord is tried, so a consumed shortcut is not left behind.
        self.chords(command)
            .iter()
            .fold(false, |hit, c| c.pressed(input) | hit)
    }

    /// Binds `command` to `chord` (the default is stored as no override).
    pub fn set(&mut self, command: Command, chord: Chord) {
        if command.defaults() == [chord] {
            self.keys.remove(command.id());
        } else {
            self.keys.insert(command.id().to_owned(), chord.to_string());
        }
    }

    pub fn reset(&mut self, command: Command) {
        self.keys.remove(command.id());
    }

    pub fn reset_all(&mut self) {
        self.keys.clear();
    }

    pub fn is_default(&self, command: Command) -> bool {
        !self.keys.contains_key(command.id())
    }

    /// Other commands that use one of this command's chords.
    pub fn conflicts(&self, command: Command) -> Vec<Command> {
        let mine = self.chords(command);
        Command::ALL
            .into_iter()
            .filter(|&c| c != command)
            .filter(|&c| self.chords(c).iter().any(|x| mine.contains(x)))
            .collect()
    }

    /// Entries naming a command that no longer exists are dropped on load.
    pub fn cleaned(mut self) -> Self {
        self.keys.retain(|k, _| Command::from_id(k).is_some());
        self
    }
}

#[cfg(test)]
mod tests;

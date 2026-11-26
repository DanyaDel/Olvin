//! The editor's configurable keys: scene and project commands, each with a
//! default chord that the Settings window can change.
//!
//! User overrides use stable command IDs and fall back to defaults.

use std::collections::BTreeMap;
use std::fmt;

use egui::{InputState, Key, Modifiers};
use serde::{Deserialize, Serialize};

/// Where a command receives keyboard input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Editor,
    Scene,
    Edit,
    Uv,
    Playback,
    Anim,
}

macro_rules! commands {
    ($($name:ident, $id:literal, $label:literal, $scope:ident, [$($key:expr),+];)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Command { $($name),* }

        impl Command {
            pub const ALL: &'static [Self] = &[$(Self::$name),*];

            pub fn id(self) -> &'static str {
                match self { $(Self::$name => $id),* }
            }

            pub fn label(self) -> &'static str {
                match self { $(Self::$name => $label),* }
            }

            fn scope(self) -> Scope {
                match self { $(Self::$name => Scope::$scope),* }
            }

            pub fn defaults(self) -> Vec<Chord> {
                match self { $(Self::$name => vec![$($key),+]),* }
            }
        }
    };
}

commands! {
    Save, "save", "Save scene", Editor, [Chord::with_modifiers(Key::S, Modifiers::COMMAND)];
    NewScene, "new_scene", "New scene", Editor, [Chord::with_modifiers(Key::N, Modifiers::COMMAND)];
    Undo, "undo", "Undo", Editor, [Chord::with_modifiers(Key::Z, Modifiers::COMMAND)];
    Redo, "redo", "Redo", Editor, [Chord::with_modifiers(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT), Chord::with_modifiers(Key::Y, Modifiers::COMMAND)];
    Duplicate, "duplicate", "Duplicate selection", Editor, [Chord::with_modifiers(Key::D, Modifiers::COMMAND)];
    Delete, "delete", "Delete selection", Editor, [Chord::plain(Key::Delete)];
    Play, "play", "Play / Stop", Editor, [Chord::with_modifiers(Key::P, Modifiers::COMMAND)];
    Settings, "settings", "Settings", Editor, [Chord::with_modifiers(Key::Comma, Modifiers::COMMAND)];
    ToolView, "tool_view", "Hand tool", Scene, [Chord::plain(Key::Q)];
    ToolMove, "tool_move", "Move tool", Scene, [Chord::plain(Key::W)];
    ToolRotate, "tool_rotate", "Rotate tool", Scene, [Chord::plain(Key::E)];
    ToolScale, "tool_scale", "Scale tool", Scene, [Chord::plain(Key::R)];
    ToggleSpace, "toggle_space", "Global / Local space", Scene, [Chord::plain(Key::X)];
    FrameSelection, "frame_selection", "Frame selection", Scene, [Chord::plain(Key::F)];
    ToggleShading, "toggle_shading", "Solid / Rendered", Scene, [Chord::with_modifiers(Key::Z, Modifiers::SHIFT)];
    KeyTransform, "key_transform", "Key Location, Rotation, Scale", Scene, [Chord::plain(Key::I)];
    DeleteKeys, "delete_keys", "Delete keys on this frame", Scene, [Chord::with_modifiers(Key::I, Modifiers::ALT)];
    EditToggle, "edit_toggle", "Enter/leave Edit Mode", Edit, [Chord::plain(Key::Tab)];
    EditVertex, "edit_vertex", "Vertex selection", Edit, [Chord::plain(Key::Num1)];
    EditEdge, "edit_edge", "Edge selection", Edit, [Chord::plain(Key::Num2)];
    EditFace, "edit_face", "Face selection", Edit, [Chord::plain(Key::Num3)];
    EditXray, "edit_xray", "X-ray", Edit, [Chord::with_modifiers(Key::Z, Modifiers::ALT)];
    EditSelectAll, "edit_select_all", "Select all vertices", Edit, [Chord::plain(Key::A)];
    EditSelectNone, "edit_select_none", "Deselect all vertices", Edit, [Chord::with_modifiers(Key::A, Modifiers::ALT)];
    EditInvert, "edit_invert", "Invert mesh selection", Edit, [Chord::with_modifiers(Key::I, Modifiers::COMMAND)];
    EditLinked, "edit_linked", "Select linked geometry", Edit, [Chord::with_modifiers(Key::L, Modifiers::COMMAND)];
    EditGrow, "edit_grow", "Grow selection", Edit, [Chord::with_modifiers(Key::Plus, Modifiers::COMMAND), Chord::with_modifiers(Key::Equals, Modifiers::COMMAND), Chord::with_modifiers(Key::Plus, Modifiers::COMMAND | Modifiers::SHIFT), Chord::with_modifiers(Key::Equals, Modifiers::COMMAND | Modifiers::SHIFT)];
    EditShrink, "edit_shrink", "Shrink selection", Edit, [Chord::with_modifiers(Key::Minus, Modifiers::COMMAND), Chord::with_modifiers(Key::Minus, Modifiers::COMMAND | Modifiers::SHIFT)];
    EditHide, "edit_hide", "Hide selection", Edit, [Chord::plain(Key::H)];
    EditHideOthers, "edit_hide_others", "Hide unselected", Edit, [Chord::with_modifiers(Key::H, Modifiers::SHIFT)];
    EditReveal, "edit_reveal", "Reveal hidden geometry", Edit, [Chord::with_modifiers(Key::H, Modifiers::ALT)];
    EditFill, "edit_fill", "Make face", Edit, [Chord::plain(Key::F)];
    EditConnect, "edit_connect", "Connect vertices", Edit, [Chord::plain(Key::J)];
    EditDelete, "edit_delete", "Delete mesh selection", Edit, [Chord::plain(Key::X), Chord::plain(Key::Delete)];
    EditMerge, "edit_merge", "Merge vertices", Edit, [Chord::plain(Key::M)];
    EditNormals, "edit_normals", "Recalculate normals", Edit, [Chord::with_modifiers(Key::N, Modifiers::SHIFT)];
    EditPickLinked, "edit_pick_linked", "Select linked under pointer", Edit, [Chord::plain(Key::L)];
    EditUnpickLinked, "edit_unpick_linked", "Deselect linked under pointer", Edit, [Chord::with_modifiers(Key::L, Modifiers::SHIFT)];
    EditLoopCut, "edit_loop_cut", "Loop cut", Edit, [Chord::with_modifiers(Key::R, Modifiers::COMMAND)];
    EditKnife, "edit_knife", "Knife", Edit, [Chord::plain(Key::K)];
    EditMove, "edit_move", "Move mesh selection", Edit, [Chord::plain(Key::G)];
    EditRotate, "edit_rotate", "Rotate mesh selection", Edit, [Chord::plain(Key::R)];
    EditScale, "edit_scale", "Scale mesh selection", Edit, [Chord::plain(Key::S)];
    EditDuplicate, "edit_duplicate", "Duplicate mesh selection", Edit, [Chord::with_modifiers(Key::D, Modifiers::SHIFT)];
    EditExtrude, "edit_extrude", "Extrude", Edit, [Chord::plain(Key::E)];
    EditInset, "edit_inset", "Inset", Edit, [Chord::plain(Key::I)];
    UvVertex, "uv_vertex", "Vertex selection", Uv, [Chord::plain(Key::Num1)];
    UvEdge, "uv_edge", "Edge selection", Uv, [Chord::plain(Key::Num2)];
    UvFace, "uv_face", "Face selection", Uv, [Chord::plain(Key::Num3)];
    UvIsland, "uv_island", "Island selection", Uv, [Chord::plain(Key::Num4)];
    UvSelectAll, "uv_select_all", "Select all UVs", Uv, [Chord::plain(Key::A)];
    UvSelectNone, "uv_select_none", "Deselect all UVs", Uv, [Chord::with_modifiers(Key::A, Modifiers::ALT)];
    UvInvert, "uv_invert", "Invert UV selection", Uv, [Chord::with_modifiers(Key::I, Modifiers::COMMAND)];
    UvLinked, "uv_linked", "Select linked UVs", Uv, [Chord::with_modifiers(Key::L, Modifiers::COMMAND)];
    UvUnwrap, "uv_unwrap", "Unwrap", Uv, [Chord::plain(Key::U)];
    UvSeam, "uv_seam", "Mark seam", Uv, [Chord::with_modifiers(Key::E, Modifiers::COMMAND)];
    UvPack, "uv_pack", "Pack islands", Uv, [Chord::with_modifiers(Key::P, Modifiers::COMMAND)];
    UvFrame, "uv_frame", "Frame UV selection", Uv, [Chord::plain(Key::F)];
    UvViewAll, "uv_view_all", "Frame all UVs", Uv, [Chord::plain(Key::Home)];
    UvPickLinked, "uv_pick_linked", "Select island under pointer", Uv, [Chord::plain(Key::L)];
    UvUnpickLinked, "uv_unpick_linked", "Deselect island under pointer", Uv, [Chord::with_modifiers(Key::L, Modifiers::SHIFT)];
    UvMove, "uv_move", "Move UV selection", Uv, [Chord::plain(Key::G)];
    UvRotate, "uv_rotate", "Rotate UV selection", Uv, [Chord::plain(Key::R)];
    UvScale, "uv_scale", "Scale UV selection", Uv, [Chord::plain(Key::S)];
    AnimForward, "anim_forward", "Play animation forward", Playback, [Chord::plain(Key::Space)];
    AnimReverse, "anim_reverse", "Play animation in reverse", Playback, [Chord::with_modifiers(Key::Space, Modifiers::COMMAND | Modifiers::SHIFT)];
    AnimPreviousFrame, "anim_previous_frame", "Previous frame", Playback, [Chord::plain(Key::ArrowLeft)];
    AnimNextFrame, "anim_next_frame", "Next frame", Playback, [Chord::plain(Key::ArrowRight)];
    AnimStart, "anim_start", "First frame", Playback, [Chord::with_modifiers(Key::ArrowLeft, Modifiers::SHIFT)];
    AnimEnd, "anim_end", "Last frame", Playback, [Chord::with_modifiers(Key::ArrowRight, Modifiers::SHIFT)];
    AnimNextKey, "anim_next_key", "Next keyframe", Playback, [Chord::plain(Key::ArrowUp)];
    AnimPreviousKey, "anim_previous_key", "Previous keyframe", Playback, [Chord::plain(Key::ArrowDown)];
    AnimSelectAll, "anim_select_all", "Select all keyframes", Anim, [Chord::plain(Key::A)];
    AnimSelectNone, "anim_select_none", "Deselect all keyframes", Anim, [Chord::with_modifiers(Key::A, Modifiers::ALT)];
    AnimInvert, "anim_invert", "Invert keyframe selection", Anim, [Chord::with_modifiers(Key::I, Modifiers::COMMAND)];
    AnimBox, "anim_box", "Box select keyframes", Anim, [Chord::plain(Key::B)];
    AnimGrab, "anim_grab", "Move keyframes", Anim, [Chord::plain(Key::G)];
    AnimScale, "anim_scale", "Scale keyframes", Anim, [Chord::plain(Key::S)];
    AnimDuplicate, "anim_duplicate", "Duplicate keyframes", Anim, [Chord::with_modifiers(Key::D, Modifiers::SHIFT)];
    AnimDelete, "anim_delete", "Delete selected keyframes", Anim, [Chord::plain(Key::X), Chord::plain(Key::Delete)];
    AnimInsert, "anim_insert", "Insert keyframes", Anim, [Chord::plain(Key::I)];
    AnimViewAll, "anim_view_all", "Frame animation", Anim, [Chord::plain(Key::Home)];
    AnimInterpolation, "anim_interpolation", "Interpolation menu", Anim, [Chord::plain(Key::T)];
    AnimCopy, "anim_copy", "Copy keyframes", Anim, [Chord::with_modifiers(Key::C, Modifiers::COMMAND)];
    AnimPaste, "anim_paste", "Paste keyframes", Anim, [Chord::with_modifiers(Key::V, Modifiers::COMMAND)];
    AnimHandle, "anim_handle", "Handle type menu", Anim, [Chord::plain(Key::V)];
}

impl Command {
    pub const GROUPS: [&'static str; 5] = [
        "Editor",
        "Scene view",
        "Edit Mode",
        "UV Editor",
        "Animation",
    ];

    pub fn group(self) -> &'static str {
        match self.scope() {
            Scope::Editor => "Editor",
            Scope::Scene => "Scene view",
            Scope::Edit => "Edit Mode",
            Scope::Uv => "UV Editor",
            Scope::Playback | Scope::Anim => "Animation",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.id() == id)
    }

    fn overlaps(self, other: Self) -> bool {
        use Scope::*;
        let (a, b) = (self.scope(), other.scope());
        a == b
            || matches!(a, Editor | Playback)
            || matches!(b, Editor | Playback)
            || matches!((a, b), (Scene, Edit) | (Edit, Scene))
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

    fn with_modifiers(key: Key, modifiers: Modifiers) -> Self {
        Self {
            key,
            command: modifiers.command,
            shift: modifiers.shift,
            alt: modifiers.alt,
        }
    }

    fn modifiers(self) -> Modifiers {
        Modifiers {
            command: self.command,
            shift: self.shift,
            alt: self.alt,
            ..Modifiers::NONE
        }
    }

    fn consume(self, input: &mut InputState, allow_repeat: bool, pattern: Modifiers) -> bool {
        let mut pressed = false;
        input.events.retain(|event| {
            let matches = matches!(event, egui::Event::Key {
                key, pressed: true, repeat, modifiers, ..
            } if *key == self.key && (allow_repeat || !repeat)
                && modifiers.matches_exact(pattern));
            pressed |= matches;
            !matches
        });
        pressed
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

    pub fn shortcut_text(&self, command: Command) -> String {
        self.chords(command)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// Whether `command` was pressed this frame.
    pub fn pressed(&self, input: &mut InputState, command: Command) -> bool {
        // Not `any`: every chord is tried, so a consumed shortcut is not left behind.
        self.chords(command).iter().fold(false, |hit, c| {
            let repeat = !matches!(command.scope(), Scope::Editor | Scope::Scene)
                || c.command
                || c.shift
                || c.alt;
            let mut modifiers = c.modifiers();
            // These Blender defaults used the physical Control key on macOS.
            if self.is_default(command)
                && matches!(command, Command::AnimReverse | Command::AnimInvert)
            {
                modifiers.command = false;
                modifiers.ctrl = true;
            }
            c.consume(input, repeat, modifiers) | hit
        })
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
            .iter()
            .copied()
            .filter(|&c| c != command && command.overlaps(c))
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

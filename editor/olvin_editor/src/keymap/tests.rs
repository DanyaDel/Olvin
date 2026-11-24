use egui::{Event, Key, Modifiers, RawInput};

use super::*;

fn input_with(key: Key, modifiers: Modifiers) -> egui::InputState {
    let raw = RawInput {
        events: vec![Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }],
        ..Default::default()
    };
    egui::InputState::default().begin_pass(raw, false, 1.0, Default::default())
}

#[test]
fn chords_print_and_parse_back() {
    for command in Command::ALL {
        for chord in command.defaults() {
            let text = chord.to_string();
            assert_eq!(Chord::parse(&text), Some(chord), "{command:?}: {text}");
        }
    }
    assert_eq!(
        Chord::parse("ctrl+shift+z"),
        Some(Chord {
            key: Key::Z,
            command: true,
            shift: true,
            alt: false
        })
    );
    assert!(Chord::parse("").is_none());
    assert!(Chord::parse("Ctrl+").is_none());
    assert!(Chord::parse("Ctrl+Nonsense").is_none());
    assert!(Chord::parse("A+B").is_none(), "two keys are not a chord");
}

#[test]
fn defaults_match_the_old_hard_coded_keys_and_do_not_clash() {
    let map = Keymap::default();
    assert_eq!(
        map.chords(Command::Save),
        [Chord {
            command: true,
            ..Chord::plain(Key::S)
        }]
    );
    assert_eq!(map.chords(Command::Redo).len(), 2);
    assert_eq!(map.chords(Command::ToolMove), [Chord::plain(Key::W)]);
    for command in Command::ALL {
        assert!(
            map.conflicts(command).is_empty(),
            "{command:?} clashes by default"
        );
        assert!(map.is_default(command));
    }
}

#[test]
fn rebinding_changes_what_fires_and_defaults_are_stored_as_nothing() {
    let mut map = Keymap::default();
    map.set(Command::ToolMove, Chord::plain(Key::M));
    assert!(!map.is_default(Command::ToolMove));
    assert_eq!(map.chords(Command::ToolMove), [Chord::plain(Key::M)]);
    let mut m_pressed = input_with(Key::M, Modifiers::NONE);
    assert!(map.pressed(&mut m_pressed, Command::ToolMove));
    let mut w_pressed = input_with(Key::W, Modifiers::NONE);
    assert!(
        !map.pressed(&mut w_pressed, Command::ToolMove),
        "the old key no longer does it"
    );
    // Putting the default back leaves no override behind.
    map.set(Command::ToolMove, Chord::plain(Key::W));
    assert!(map.is_default(Command::ToolMove));
    map.set(Command::ToolMove, Chord::plain(Key::M));
    map.reset(Command::ToolMove);
    assert_eq!(map, Keymap::default());
}

#[test]
fn modifier_chords_are_exact_and_consumed() {
    let map = Keymap::default();
    let mut ctrl_z = input_with(Key::Z, Modifiers::COMMAND);
    assert!(map.pressed(&mut ctrl_z, Command::Undo));
    assert!(
        !map.pressed(&mut ctrl_z, Command::Undo),
        "consumed: a second handler does not see it"
    );
    // Ctrl+Shift+Z is Redo, and must not also be Undo.
    let both = Modifiers::COMMAND | Modifiers::SHIFT;
    let mut redo = input_with(Key::Z, both);
    assert!(map.pressed(&mut redo, Command::Redo));
    // A bare key does not fire when a modifier is held.
    let mut shift_w = input_with(Key::W, Modifiers::SHIFT);
    assert!(!map.pressed(&mut shift_w, Command::ToolMove));
}

#[test]
fn conflicts_are_reported_and_junk_in_the_file_is_ignored() {
    let mut map = Keymap::default();
    map.set(Command::ToolScale, Chord::plain(Key::W));
    assert_eq!(map.conflicts(Command::ToolScale), [Command::ToolMove]);
    assert_eq!(map.conflicts(Command::ToolMove), [Command::ToolScale]);
    let text = r#"(keys: {"tool_move": "Ctrl+Banana", "old_command": "F9", "save": "F5"})"#;
    let loaded: Keymap = ron::from_str(text).unwrap();
    assert_eq!(
        loaded.chords(Command::ToolMove),
        Command::ToolMove.defaults(),
        "unreadable: default"
    );
    assert_eq!(loaded.chords(Command::Save), [Chord::plain(Key::F5)]);
    let cleaned = loaded.cleaned();
    assert!(!cleaned.keys.contains_key("old_command"));
    let again: Keymap = ron::from_str(&ron::to_string(&cleaned).unwrap()).unwrap();
    assert_eq!(again, cleaned);
}

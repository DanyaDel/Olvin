//! Rebinding at run time: listen for the next control the player touches and
//! put it into the [`InputMap`] (the "press a key..." screen of a settings menu).

use std::collections::HashSet;
use std::path::PathBuf;

use olvin_window::{Input, KeyCode};

use crate::{Binding, GamepadButton, GamepadState, InputMap};

/// Which binding of the action a rebind changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Replaces the binding at this index (appends if it is past the end).
    Replace(usize),
    /// Appends a new binding.
    Add,
}

/// What a finished [`Rebind::poll`] did.
#[derive(Clone, Debug, PartialEq)]
pub enum RebindEvent {
    /// `binding` is now bound; `conflicts` are the other actions that already
    /// use it (`(action, index)`), left in place for the UI to resolve.
    Bound {
        action: String,
        binding: Binding,
        conflicts: Vec<(String, usize)>,
    },
    /// Escape was pressed (or `cancel` called): nothing changed.
    Cancelled,
}

/// Wait-for-input state. Call [`Rebind::listen`] from the UI; the plugin polls
/// it each frame in `Stage::First`, and while it listens actions read idle, so
/// the captured press does not also trigger gameplay.
#[derive(Debug, Default)]
pub struct Rebind {
    target: Option<(String, Slot)>,
    /// Pad buttons held last poll, to see new presses (`GamepadState` has no edges).
    pads_down: HashSet<GamepadButton>,
    /// Written after every successful rebind when set. `InputPlugin` turns a
    /// relative path into one inside the project folder (`ProjectPaths::resolve`).
    pub autosave: Option<PathBuf>,
    /// Outcome of the last poll that finished a rebind; take it with [`Rebind::take_event`].
    event: Option<RebindEvent>,
    /// This frame's input went to the rebind (set by `update_actions`).
    took_input: bool,
}

impl Rebind {
    /// Starts listening for the action's `slot`.
    pub fn listen(&mut self, action: &str, slot: Slot) {
        self.target = Some((action.to_owned(), slot));
        self.event = None;
    }

    pub fn cancel(&mut self) {
        if self.target.take().is_some() {
            self.event = Some(RebindEvent::Cancelled);
        }
    }

    /// This frame's raw input was taken by the rebind (it was listening when
    /// the frame began, so the press that finished it is not a click either).
    /// Readers of raw `Input` outside `ActionState` (the UI) should ignore the frame.
    pub fn took_input(&self) -> bool {
        self.took_input
    }

    pub(crate) fn set_took_input(&mut self, took: bool) {
        self.took_input = took;
    }

    pub fn is_listening(&self) -> bool {
        self.target.is_some()
    }

    /// The action and slot being rebound.
    pub fn target(&self) -> Option<(&str, Slot)> {
        self.target.as_ref().map(|(a, s)| (a.as_str(), *s))
    }

    pub fn take_event(&mut self) -> Option<RebindEvent> {
        self.event.take()
    }

    /// Looks at this frame's raw input; on a press applies it to `map`.
    pub fn poll(&mut self, map: &mut InputMap, input: &Input, pads: &GamepadState) {
        let down: HashSet<_> = GamepadButton::ALL
            .into_iter()
            .filter(|b| pads.pressed(*b))
            .collect();
        let pad_pressed = GamepadButton::ALL
            .into_iter()
            .find(|b| down.contains(b) && !self.pads_down.contains(b));
        self.pads_down = down;
        let Some((action, slot)) = self.target.clone() else {
            return;
        };
        if input.just_pressed(KeyCode::Escape) {
            self.cancel();
            return;
        }
        let Some(binding) = capture(input, pad_pressed) else {
            return;
        };
        let conflicts = map.conflicts(&binding, Some(&action));
        match slot {
            Slot::Replace(i) => {
                map.set_binding(&action, i, binding.clone());
            }
            Slot::Add => {
                map.add(&action, binding.clone());
            }
        }
        if let Some(path) = &self.autosave
            && let Err(err) = map.save(path)
        {
            log::warn!("input map: {err}");
        }
        self.target = None;
        self.event = Some(RebindEvent::Bound {
            action,
            binding,
            conflicts,
        });
    }
}

/// The first thing pressed this frame: a key, then a mouse button, then a pad button.
fn capture(input: &Input, pad: Option<GamepadButton>) -> Option<Binding> {
    let mut keys: Vec<_> = input
        .pressed_keys()
        .filter(|k| input.just_pressed(*k))
        .collect();
    // Stable pick when several land in one frame.
    keys.sort_by_key(|k| format!("{k:?}"));
    if let Some(k) = keys.first() {
        return Some(Binding::Key(*k));
    }
    input
        .pressed_mouse()
        .find(|b| input.mouse_just_pressed(*b))
        .map(Binding::Mouse)
        .or(pad.map(Binding::GamepadButton))
}

//! Input actions, Unity Input System style: an [`InputMap`] names actions
//! (Move, Jump, ...) and binds them to keys, mouse, gamepads and composites;
//! [`ActionState`] answers `pressed / just_pressed / value / axis2d` per frame.
//!
//! Reads `olvin_window::Input` (works headless too: the plugin creates an
//! empty one). Gamepads come from gilrs behind the `gamepad` feature.

mod binding;
mod gamepad;
mod hints;
mod map;
mod rebind;
mod state;

pub use binding::{Binding, GamepadAxis, GamepadButton, Raw, Stick};
pub use gamepad::{GamepadState, Haptics, Rumble};
pub use hints::{Device, PadStyle, key_label, mouse_label};
pub use map::{DEFAULT_PATH, FILE_NAME, InputMap};
pub use olvin_window::{CursorOptions, Input, KeyCode, MouseButton};
pub use rebind::{Rebind, RebindEvent, Slot};
pub use state::{ActionState, ActionValue};

use olvin_core::{App, Plugin, ProjectPaths, Resources, Stage};

/// Inserts [`InputMap`] (from `map`, else `input.ron` in the project's
/// assets, else the default map), [`ActionState`] and [`GamepadState`];
/// updates actions in `Stage::First`, so `PreUpdate` (physics fixed steps)
/// already sees them. A relative [`Rebind::autosave`] is inside the project folder.
#[derive(Default)]
pub struct InputPlugin {
    pub map: Option<InputMap>,
}

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        let map = self.map.clone().unwrap_or_else(|| {
            let paths = app.resources.fetch::<ProjectPaths>();
            InputMap::load_or_default(paths.asset(map::FILE_NAME))
        });
        app.insert_resource(map)
            .init_resource::<Input>()
            .init_resource::<ActionState>()
            .init_resource::<GamepadState>()
            .init_resource::<Haptics>()
            .init_resource::<Rebind>();
        #[cfg(feature = "gamepad")]
        {
            app.insert_resource(gamepad::backend::Gamepads::new());
            app.add_system(Stage::First, |res| {
                let mut pads = res.fetch_mut::<gamepad::backend::Gamepads>();
                pads.poll(
                    &mut res.fetch_mut::<GamepadState>(),
                    &mut res.fetch_mut::<Haptics>(),
                );
            });
        }
        app.add_system(Stage::First, update_actions)
            .add_system(Stage::PostUpdate, apply_cursor);
        if !app.has_plugin::<olvin_window::WindowPlugin>() {
            // Headless or another platform: someone must still clear the
            // per-frame edges. (Clearing twice is harmless if a window plugin
            // is added later.)
            app.add_system(Stage::Last, |res| res.fetch_mut::<Input>().end_frame());
        }
    }
}

/// Recomputes [`ActionState`] from the raw devices (what the plugin runs each frame).
/// While [`Rebind`] listens, it takes the input (so the captured press does
/// not also fire gameplay) and actions read idle.
pub fn update_actions(res: &mut Resources) {
    let mut map = res.fetch_mut::<InputMap>();
    let input = res.fetch::<Input>();
    let pads = res.fetch::<GamepadState>();
    let mut rebind = res.fetch_mut::<Rebind>();
    if let Some(path) = rebind.autosave.as_mut().filter(|p| p.is_relative())
        && let Some(paths) = res.get::<ProjectPaths>()
    {
        *path = paths.resolve(&*path);
    }
    let was_listening = rebind.is_listening();
    rebind.poll(&mut map, &input, &pads);
    let mut actions = res.fetch_mut::<ActionState>();
    if was_listening {
        actions.update(&map, &Input::default(), &GamepadState::default());
    } else {
        actions.update(&map, &input, &pads);
    }
}

fn apply_cursor(res: &mut Resources) {
    let state = res.fetch::<ActionState>();
    if let Some(mut cursor) = res.get_mut::<CursorOptions>() {
        cursor.grabbed = state.cursor_locked;
        cursor.visible = state.cursor_visible;
    }
}

#[cfg(test)]
mod tests;

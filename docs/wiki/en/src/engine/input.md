# Input and rebinding

Two layers: `Input` in `olvin_window` (keys, mouse, raw `mouse_delta`, `scroll`, cursor) and action maps in `olvin_input` (the `input` module): the game reads actions, not keys.

## Action maps

`InputMap { actions: name → [Binding] }`. Bindings: keys, mouse buttons and wheel, gamepad buttons, `Axis1D`, `Axis2D`, `Chord`. The default map: Move, Look, Jump, Sprint, Fire, Interact, Pause, Ascend. A saved map lives in `assets/input.ron` (`ProjectPaths::asset`).

```rust
let a = res.fetch::<ActionState>();
if a.just_pressed("Jump") { /* ... */ }
let mv = a.axis2d("Move");          // Vec2
```

`ActionState`: `pressed`, `just_pressed`, `just_released`, `value`, `axis2d`, `lock_cursor(bool)`. Updated in `Stage::First`. Gamepads: gilrs (feature `gamepad`). Your own actions: `InputMap::bind(name, [Binding::Key(KeyCode::F5)])`, and `fill_missing(&map)` adds them to a map the player already saved without overwriting rebinds.

## Rebinding at runtime

The `Rebind` resource: `listen(action, Slot::Replace(i) | Slot::Add)`. The plugin catches the first key, mouse button or gamepad button pressed (`Esc` cancels), writes it into `InputMap` and sends `RebindEvent::{Bound { conflicts }, Cancelled}` (overlaps with other actions are only reported). While waiting, actions read as inactive. `Rebind.autosave` saves the map to RON after every change. Also: `set_binding`, `remove_binding`, `reset_action`, `conflicts`.

## Hints and rumble

`Device::{KeyboardMouse, Gamepad}` (what the player touched last), `PadStyle::{Xbox, PlayStation, Nintendo}`; `InputMap::hint(action, device, style)` returns a label ("Space", "Cross", "W/A/S/D"). Rumble: the `Haptics { enabled, intensity, supported }` resource: `rumble(strong, weak, duration)`, `pulse`, `stop`. Without the `gamepad` feature the queue is just dropped.

## From Lua

`ctx:pressed`, `ctx:just_pressed`, `ctx:just_released`, `ctx:value`, `ctx:axis2d` ([reference](../reference/lua-api.md#input)).

## UI and input

The game UI "eats" input under the pointer and on a focused widget: see [Game UI](game-ui.md).

## See also

- [Game UI](game-ui.md)
- [Scripting](scripting.md)
- [Tutorial: step 4](../tutorial/04-player.md)
- [Editor hotkeys](../editor/hotkeys.md)

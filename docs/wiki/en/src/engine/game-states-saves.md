# Game states and saves

`olvin_game` (the `game` module): states with enter and exit, background scene loading, save slots. No GPU or window.

## States

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GameState { Loading, Playing, Paused }

app.add_state(GameState::Loading)
    .on_enter(GameState::Paused, |res| res.fetch_mut::<Time>().pause())
    .on_enter(GameState::Playing, |res| res.fetch_mut::<Time>().resume())
    .add_system_in_state(Stage::Update, GameState::Playing, player_control);
```

`State<S>` (`get`, `is`, `set`). `set` only requests a transition: it is applied once per frame at the start of `PreUpdate` (the last request wins): `on_exit(old)`, the switch, `on_enter(new)`, the `StateChanged<S>` event. Transition chains are cut at 8 steps; `on_enter(initial)` runs in `Startup`.

## Background scene loading

`SceneLoaderPlugin` + `SceneLoader`: `request(path) -> LoadHandle` reads and parses a scene (with all migrations) on a worker thread; `status` (`Pending`, `Ready`, `Failed`, `Gone`), `take(handle)`. A loading screen is a `Loading` state that waits for `Ready` and `olvin::models::pending_count(res) == 0` and then sets `Playing`; spawning: `olvin::scene_file::spawn_loaded(res, handle)`. A model: `apps/vertical_slice/src/level.rs`.

## Saves

A save is session state, not a scene. `SaveRegistry`: `register_component::<T>(name)`, `register_resource::<T>(name)` (serde + RON), `register_external` for state outside ECS. Entities with `SaveId(u64)` and scene objects (by `SceneId`) are saved.

`SavePlugin::new(dir)` registers engine state: `transform`, `motor`, `velocity` (physics), `scripts` (`Behaviour::save_state` / `load_state`). `without_engine()` turns it off.

- `capture(res, meta) -> SaveData { version, meta, entities, resources }`, `restore(res, &data)`: entities with `SaveId` are updated or created, those absent from the save are deleted, registered components absent from the save are removed. Everything is parsed before the world is touched. References between entities are not remapped: store `SaveId`.
- `SaveSlots`: `<dir>/<slot>.save.ron`, `save` (tmp + rename), `load`, `list` (newest first), `delete`, `exists`; a slot name cannot leave the directory.
- Signed saves (`SavePlugin::signed(SaveSigner::new(key).with_policy(..))` or `SaveSlots::with_signing`): the last line of the file is `// olvin-save-hmac: <hex>` (HMAC-SHA256 of the text); saves without it still load. Policies: `Warn` (default, a log line), `Enforce` (refuses a mismatch), `Strict` (refuses a mismatch and an unsigned save; removing the line cannot bypass it). `load_checked` returns a `SaveStatus` (`Valid`, `Unsigned`, `Tampered`). **This is a signal and a deterrent, not protection:** the key is part of the game, so anyone who takes it out of the binary can sign their own edits; it does stop an edit in a text editor.
- Versioning: `SAVE_VERSION` + `SAVE_MIGRATIONS`, a future version is an error.

> **Status:** the state of Lua `self` tables is not saved (the hooks are wired only for Rust `Behaviour`); use `ScriptVars` and components.

## See also

- [Tutorial: step 8](../tutorial/08-saves.md)
- [Scene format](scene-format.md)
- [Time and events](time-events.md)
- [Scripting](scripting.md)

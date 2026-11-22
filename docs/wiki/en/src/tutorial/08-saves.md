# Step 8. Saving and loading

`F5` saves the game state to a slot, `F9` loads it. The `game` module is needed (`olvin_game::save`). Saves are separate from scenes: a scene is a level as the editor made it, a save is the state of a session.

## How it works

- `SavePlugin::new(dir)` puts slots into `<dir>/<slot>.save.ron` (written through a temp file and renamed) and by default registers the engine state: `Transform`, velocities of dynamic bodies, `CharacterMotor`, the state of Rust scripts (`Behaviour::save_state` / `load_state`).
- Scene entities are saved under their `SceneId`, so nothing needs registering for the crates and the player. Your own components and resources are registered: `register_component::<T>(name)`, `register_resource::<T>(name)` (serde + RON).
- `restore` puts bodies at the saved poses and brings velocities back. Everything is parsed before the world is touched, so an error changes nothing.

## Code

The plugin and actions (in `main`, after `DefaultPlugins`):

```rust
use olvin::game::{SaveMeta, SavePlugin, SaveRegistry, SaveSlots};

let saves = app.resources.fetch::<ProjectPaths>().resolve("saves");
let mut keys = InputMap::default();
keys.bind("QuickSave", [Binding::Key(KeyCode::F5)]);
keys.bind("QuickLoad", [Binding::Key(KeyCode::F9)]);
app.resources.fetch_mut::<InputMap>().fill_missing(&keys);
app.add_plugin(SavePlugin::new(saves))
    .add_system(Stage::Update, quick_slot);
```

The system (a simplified `apps/vertical_slice/src/save.rs`):

```rust
fn quick_slot(res: &mut Resources) {
    let (save, load) = {
        let a = res.fetch::<ActionState>();
        (a.just_pressed("QuickSave"), a.just_pressed("QuickLoad"))
    };
    if save {
        let meta = SaveMeta {
            title: "Quick save".into(),
            scene: "scenes/main.scene.ron".into(),
            playtime_secs: res.fetch::<Time>().elapsed(),
            saved_at: 0,
        };
        let data = res.fetch::<SaveRegistry>().capture(res, meta).expect("capture");
        res.fetch::<SaveSlots>().save("quick", &data).expect("write slot");
        log::info!("saved: {} entities", data.entities.len());
    }
    if load {
        match res.fetch::<SaveSlots>().load("quick") {
            Ok(data) => res.fetch::<SaveRegistry>().restore(res, &data).expect("restore"),
            Err(e) => log::warn!("no quick save: {e}"),
        }
    }
}
```

After `restore` the slice also syncs the shared Lua variables: the saved `Progress { door_opened }` goes back into `ScriptVars` (`door::restore_vars`), because the state of Lua scripts does not go into saves yet.

> **Status:** the state of Lua `self` tables is not saved (the `save_state`/`load_state` hooks are not wired for Lua); keep what you need in components, resources or `ScriptVars` and register them.

## Check

Move a crate, `F5`, move it again, `F9`: the crate and the player come back. The file is `saves/quick.save.ron` in the project folder (`/saves` is in the template's `.gitignore`).

## See also

- [Game states and saves](../engine/game-states-saves.md)
- [Input](../engine/input.md)
- [Scripting](../engine/scripting.md)
- [Next: step 9, build](09-build.md)

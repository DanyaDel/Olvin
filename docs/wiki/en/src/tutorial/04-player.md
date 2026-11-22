# Step 4. Player

A first-person character on the built-in `SimplePlayerController` behaviour (WASD, mouse, Space, Shift). The `input` and `physics` modules are needed.

## In the editor

1. **GameObject → Create Empty**, name `Player`, Position `0, 1, 10`.
2. **Add Component → Physics → Character Controller** (a capsule: Height 2, Radius 0.5; step and slope are set there too).
3. **Add Component → Scripts → New Script**, name `SimplePlayerController`, parameters (key–value):

   | Key | Value | Meaning |
   |---|---|---|
   | `speed` | 4 | walking, m/s |
   | `sprint_speed` | 7 | with Shift held |
   | `jump_height` | 1.2 | jump height, m |
   | `sensitivity` | 0.12 | degrees per mouse pixel |
   | `camera` | `Player Camera` | name of the child camera (pitch up and down) |

4. With `Player` selected, **GameObject → Camera**, rename it `Player Camera`, drag it onto `Player` in the Hierarchy (it becomes a child), Position `0, 0.7, 0`. A camera is created together with an Audio Listener: sound is heard from it.

The `Move`, `Look`, `Jump` and `Sprint` actions are in the default action map (`InputMap`), see [Input](../engine/input.md).

## In code

We lock the cursor on startup and add `LuaPlugin` (needed in the next step). The project's `src/main.rs`:

```rust
use olvin::prelude::*;
use olvin::script::lua::{LuaLimits, LuaPlugin};

fn main() {
    olvin::core::Logging::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")
        .file("logs/game.log")
        .init();

    let mut app = App::new();
    app.add_plugin(DefaultPlugins {
        window: WindowPlugin { title: "first_game".into(), ..Default::default() },
        ..Default::default()
    });
    let assets = app.resources.fetch::<ProjectPaths>().asset("scripts");
    app.add_plugin(LuaPlugin { dir: assets, hot_reload: cfg!(debug_assertions), limits: LuaLimits::default() })
        .add_system(Stage::Startup, |res| res.fetch_mut::<ActionState>().lock_cursor(true))
        .add_system(Stage::Startup, setup);   // setup from the template: loads the scene
    app.run();
}
```

`setup`, `Spin` and `spin` stay from the template (loading `scenes/main.scene.ron`); in the template `Esc` closes the game.

The template loads the scene through `olvin::scene_file::load`; physics, input and scripts are on because `DefaultPlugins` installs everything the features enable.

## Check

`olvin-hub run --release`: walk with WASD, look with the mouse, jump with Space. The character pushes dynamic bodies (`push_mass` in the Character Controller, 80 kg by default).

## See also

- [Physics: character controller](../engine/physics.md#character-controller)
- [Scripting](../engine/scripting.md)
- [Input](../engine/input.md)
- [Next: step 5, plate and door](05-door.md)

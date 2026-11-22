# Project layout

Where a game's files are and how the engine finds them: the `ProjectPaths` resource from `olvin_core`.

## Project folders

```
my_game/
  Cargo.toml          the olvin dependency with module features, deps always -O3
  src/main.rs         entry point (hub template)
  assets/             everything the game needs
    scenes/           *.scene.ron (main one: scenes/main.scene.ron)
    prefabs/          *.prefab.ron
    materials/        *.mat.ron, *.wgsl
    models/           glTF, <model>_textures/ next to it
    scripts/          *.lua
    sfx/, music/      sounds (paths in code are relative to assets/)
    input.ron         saved action map (if any)
    window.ron        window mode and resolution (if saved)
  saves/              save slots *.save.ron (in .gitignore)
  logs/               game.log, crash-<time>.txt (in .gitignore)
  dist/               output of olvin-hub build (in .gitignore)
```

Next to every asset the editor keeps `<name>.meta` with a GUID ([Assets](assets.md)).

## ProjectPaths

`ProjectPaths { root, assets, source }`: a resource inserted by `App::new`. It is read by `AudioPlugin`, `InputPlugin`, `Logging` and the hub template. Methods: `asset(p)` (a path inside `assets/`) and `resolve(p)` (a path from the project root); absolute paths are left alone.

`ProjectPaths::detect()` is computed once per process, the root search order:

| # | Source | When |
|---|---|---|
| 1 | `OLVIN_PROJECT_DIR` | an explicitly set folder |
| 2 | the exe's folder, if it has `assets/` | the `olvin-hub build` layout |
| 3 | `CARGO_MANIFEST_DIR` | `cargo run`, `cargo test` |
| 4 | the working directory | fallback |

`OLVIN_ASSETS_DIR` changes only `assets`. What fired is reported by `PathsSource`. That is why a built folder runs by double-click or from any working directory.

`LuaPlugin` and `SavePlugin` take their path as is: give them paths from `ProjectPaths` (`paths.asset("scripts")`, `paths.resolve("saves")`).

## Editor and hub files

| File | What |
|---|---|
| `~/.config/olvin-hub/config.toml` | project list, engine source, hub theme |
| `~/.local/share/olvin-hub/editor` | the built editor |
| `<config>/olvin-editor/{preferences,layout}.ron` | editor settings |

## See also

- [Hub](../start/hub.md)
- [Assets](assets.md)
- [Scene format](scene-format.md)
- [Environment variables](../reference/env-vars.md)

# Step 1. Project

Create a project in the hub, install the modules you need and open the editor.

## In the hub

1. Start `olvin-hub-gui`, press `Ctrl+N` on the Projects page.
2. Template **3D + Physics**, name `first_game`, a folder of your choice.
3. Switch on the **Audio**, **Lua scripts** and **Game runtime** modules (toggle cards; Lua pulls Scripts in by itself). Create.

The same on the console:

```sh
olvin-hub new first_game && cd first_game
olvin-hub add physics audio lua game
```

## The gui feature

The game UI ([step 7](07-ui.md)) is not in the hub catalog; add the feature to `Cargo.toml` by hand:

```toml
olvin = { path = "…", default-features = false, features = ["window", "render", "scene", "physics", "script", "input", "audio", "lua", "game", "gui"] }
```

(take the path and the list from your own file: the hub has already written `olvin` there, you only append `"gui"`). `serde` and `ron` are not needed for the saves in step 8: the facade is enough.

## The editor

```sh
olvin-hub editor install      # if you have not yet
olvin-hub editor add animator # optional for the tutorial
olvin-hub editor open
```

The Layout workspace opens: Scene, Game, Hierarchy, Inspector, Assets, see [First editor run](../start/editor-first-run.md).

Put the assets into `first_game/assets/`: `sfx/door.wav`, `sfx/thud.wav`, `music/theme.wav` (the Assets panel will see them and create `.meta` files with GUIDs).

## Check

`olvin-hub run` builds the game and shows a spinning cube: there is no scene yet, so the template draws its fallback cube. Exit with `Esc`.

## See also

- [Hub](../start/hub.md)
- [olvin features](../reference/olvin-features.md)
- [Project layout](../engine/project-layout.md)
- [Next: step 2, scene](02-scene.md)

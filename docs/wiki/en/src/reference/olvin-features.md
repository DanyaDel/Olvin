# Features of the olvin facade

The `olvin` crate (`crates/olvin`) re-exports the subsystems; each is enabled by a feature. `window` and `render` are on by default. When it creates a project the hub sets `default-features = false` and lists features itself ([Hub](../start/hub.md)).

| Feature | What it adds | Enables | Hub module |
|---|---|---|---|
| `window` | window, `Input`, runner (winit) | | `window` |
| `render` | wgpu rendering; `GuiRenderPlugin` if `gui` is on | | `render` |
| `scene` | loading `*.scene.ron`, models, CSG, `RenderAssets` | `render`, `anim`, `assets` | `scenes` |
| `anim` | skeletons, clips, `Animator`, `AnimPlugin` (no rendering) | | none |
| `assets` | `.meta`, GUIDs, glTF, `AssetManager` | | none |
| `physics` | Rapier: bodies, colliders, controller | | `physics` |
| `script` | behaviours in Rust (`Behaviour`) | | `scripts` |
| `lua` | Lua 5.4 (vendored, no system library) | `script` | `lua` |
| `input` | action maps, gamepads | | `input` |
| `audio` | sound on kira | | `audio` |
| `game` | states, `SceneLoader`, saves | | `game` |
| `gui` | game UI and text (`olvin::gui`, `olvin::text`) | `input` | none |
| `net` | netcode (`olvin::net`): sessions, replication, messages, prediction; Lua bindings with `lua` | | nothing |
| `net-quic` | QUIC sockets for `net` (TLS 1.3, room codes) | `net` | nothing |
| `profile` | puffin marks for systems, rendering, physics, scripts | | none |
| `profile-http` | a server for `puffin_viewer` (`OLVIN_PROFILE_HTTP`) | | none |

Features link up: `physics` turns physics on in `olvin_script` and `olvin_game`, `audio` and `input` turn on the Lua bindings of the same name.

## Plugin sets

- `DefaultPlugins { window, audio, .. }`: everything enabled (window, rendering, physics, input, scripts, audio, GUI, animation).
- `HeadlessPlugins { physics, input, script, audio }`: the same without window and GPU (audio is off by default), for game tests and servers. It is not installed together with `DefaultPlugins`.
- `prelude`: `App`, `Stage`, `Resources`, `Time`, `ProjectPaths`, `Transform`, `Camera`, `Input`, `Meshes`, … and the types of enabled modules.

## Checking

A hub test checks catalog modules against the facade's features; the features `gui`, `anim`, `assets`, `profile*` are not in the catalog yet.

## See also

- [Hub](../start/hub.md)
- [olvin-hub CLI](hub-cli.md)
- [Scripting](../engine/scripting.md)
- [Profiler](../engine/profiler.md)

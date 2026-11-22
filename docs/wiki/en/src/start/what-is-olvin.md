# What is Olvin

Olvin is a modular game engine in Rust (edition 2024) with a project hub and a scene editor. Every subsystem lives in its own crate and plugs in as a plugin.

## Principles

- **Modularity at the subsystem level.** A plugin registers systems, a system walks all data in one call. No virtual call per object, and what you do not need is not compiled.
- **Flat arrays in hot paths.** The renderer sorts instances per mesh and issues one instanced draw per mesh; a steady-state frame allocates nothing.
- **The core knows nothing about GPU or window.** Rendering and the platform meet at `SurfaceTarget` (raw-window-handle).
- **Minimum hardware: WebGL2 / GLES 3.0.** Anything stronger is switched on by the `Full` tier, see [Rendering](../engine/rendering.md).

## What it consists of

| Part | What it does | Where |
|---|---|---|
| Engine | `olvin_core` (App, Plugin, Resources, Schedule), ECS on `hecs`, window on winit, rendering on wgpu, Rapier physics, kira audio, input, Rust and Lua scripts, game UI | `crates/` |
| `olvin` facade | re-exports, `DefaultPlugins`, `prelude`; subsystems are switched on by features | [olvin features](../reference/olvin-features.md) |
| Hub | creates projects, installs modules, builds the game | [Hub](hub.md) |
| Editor | scene, hierarchy, inspector, assets, animation, CSG, modeling, UV, IDE | [Editor](../editor/overview.md) |

## What works today

- `*.scene.ron` scenes with versions and migrations, `*.prefab.ron` prefabs, `*.mat.ron` materials, glTF with animation.
- PBR (GGX), cascaded shadows, spot and point shadows, transparency, fog, sky, bloom/GTAO/FXAA/LUT on the Full tier, a Compat tier for weak GPUs.
- Rapier physics: bodies, colliders, triggers, character controller, raycasts.
- Audio with buses, 3D, reverb, occlusion; input with action maps and rebinding; game UI with real text.
- Scripts in Rust (`Behaviour`) and Lua 5.4 with hot reload and a sandbox.
- Game saves (slots), game states, background scene loading.
- One-command game build: `olvin-hub build --release`.

> **Status:** not done, for example: GPU skinning ([#8](https://github.com/DanyaDel/Olvin/issues/8)), particles in scenes and the editor ([#11](https://github.com/DanyaDel/Olvin/issues/11)), joints ([#2](https://github.com/DanyaDel/Olvin/issues/2)), web ([#51](https://github.com/DanyaDel/Olvin/issues/51)) and Android ([#52](https://github.com/DanyaDel/Olvin/issues/52)). The whole plan: [ROADMAP](../dev/roadmap.md).

## See also

- [Installation](installation.md)
- [Tutorial: your first game](../tutorial/index.md)
- [Architecture](../dev/architecture.md)
- [Glossary](../reference/glossary.md)

# Architecture: overview

A short map. The full description of every crate, format and decision: [ARCHITECTURE.en.md](https://github.com/DanyaDel/Olvin/blob/main/ARCHITECTURE.en.md) (the Russian original is [ARCHITECTURE.md](https://github.com/DanyaDel/Olvin/blob/main/ARCHITECTURE.md)). Read it instead of scanning the code.

## Layers

```
apps/ (sandbox, vertical_slice, editor_dev)     games and examples
editor/ (olvin_editor + elements)  hub/         tools, not part of the engine
crates/olvin                                    facade: DefaultPlugins, prelude, features
crates/                                         subsystem plugins
  olvin_core      App, Plugin, Resources, Schedule, Time, Events, ProjectPaths, logging, profile
  olvin_ecs       hecs
  olvin_scene     Transform, Camera, scene format, prefabs, Action
  olvin_transform hierarchy
  olvin_window    winit, Input, runner
  olvin_render    wgpu: PBR, shadows, post, particles, sprites, overlay
  olvin_physics   Rapier
  olvin_audio     kira
  olvin_input     action maps
  olvin_script    Behaviour, Lua
  olvin_game      states, scene loading, saves
  olvin_gui, olvin_text   game UI and text
  olvin_assets, olvin_anim, olvin_csg, olvin_mesh   assets, animation, CSG, meshes
  olvin_ui        the shared UI kit of hub and editor (egui)
```

## Principles

- A plugin registers systems; subsystems talk through resources, not directly.
- The core knows nothing about GPU and window (`SurfaceTarget`).
- Minimum hardware: WebGL2/GLES3, the rest through the `Full` tier.
- Data (`*Desc`, serde) is separate from the runtime: scene, editor and game share the same types.
- Editor plugins are linked statically.

## Where to look

| Question | Page |
|---|---|
| How a frame, the schedule and events work | [Time and events](../engine/time-events.md) |
| What is in a scene | [Scene format](../engine/scene-format.md) |
| How rendering works | [Rendering](../engine/rendering.md) |
| How scripts work | [Scripting](../engine/scripting.md) |
| How the editor hooks up elements | [Contributing guide](contributing.md#add-an-editor-element) |

The state of the code in ARCHITECTURE.md is marked with a date and a commit; a mismatch with the code is a documentation bug, file an issue.

## See also

- [Roadmap](roadmap.md)
- [Contributing guide](contributing.md)
- [What is Olvin](../start/what-is-olvin.md)
- [Glossary](../reference/glossary.md)

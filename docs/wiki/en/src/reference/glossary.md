# Glossary

| Term | Meaning |
|---|---|
| **Action** | a named game input (`Jump`, `Move`) in `InputMap`, independent of the key |
| **Asset** | a file in `assets/` with a `.meta`: image, model, material, prefab, sound, scene |
| **Behaviour** | an object behaviour in Rust: `start`, `update`, collision callbacks |
| **Brush** | a convex volume for a CSG level; operation Add / Subtract / Intersect |
| **CSG** | constructive solid geometry: a level made of boolean operations over brushes |
| **Compat** | the render tier for GL/WebGL2 ([Rendering](../engine/rendering.md)) |
| **DefaultPlugins** | the engine's plugin set by enabled features |
| **ECS** | entity-component-system; the backend is `hecs`, types are re-exported by `olvin_ecs` |
| **Element** | an editor plugin: console, profiler, animator, csg, modeling, ide, uv |
| **Environment** | sky, light and post-processing settings in a scene |
| **Facade** | the `olvin` crate, re-exports subsystems by feature |
| **Full** | the render tier for Vulkan/Metal/DX12 |
| **F-curve** | an animation curve of an object property (as in Blender) |
| **GUID** | a permanent asset identifier in `.meta` |
| **Headless** | running without a window and GPU (`HeadlessPlugins`) |
| **Hub** | `olvin-hub`: projects, modules, build, editor installation |
| **Module** | a hub catalog entry: an `olvin` feature or an external crate |
| **Override** | a difference of a prefab instance from the asset |
| **Plugin** | `impl Plugin`: registers resources and systems in `App` |
| **PolyMesh** | an editable polygon mesh (Edit Mode) |
| **Prefab** | `*.prefab.ron`: a scene with one root for reuse |
| **ProjectPaths** | a resource with the project root and the `assets/` folder |
| **Resource** | an application singleton by `TypeId` |
| **SceneId** | a scene object's id, put on the entity at spawn |
| **Stage** | a schedule stage: Startup, First, PreUpdate, Update, PostUpdate, Render, Last |
| **Slice** | `apps/vertical_slice`, a finished game for checking the engine |
| **Tier** | a rendering capability level: Full or Compat |
| **Trigger** | a collider without collision, gives enter and exit events |
| **Workspace** | an editor tab with a panel layout |

## See also

- [What is Olvin](../start/what-is-olvin.md)
- [Architecture](../dev/architecture.md)
- [olvin features](olvin-features.md)
- [Lua API](lua-api.md)

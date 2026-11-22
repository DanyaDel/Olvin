# Inspector

The Inspector panel shows the components of the selected object. The header: the Active flag, name, Tag, Layer. Below are collapsible sections; **Add Component** opens a menu with search and categories.

## Components

| Category | Component | Data in the scene |
|---|---|---|
| Rendering | Transform | position, rotation (degrees, Euler YXZ), scale |
| Rendering | Mesh Renderer | primitive and color, material |
| Rendering | Model | a glTF model, animation clip or graph |
| Rendering | Camera | perspective camera |
| Rendering | Light | `LightDesc`: Directional / Point / Spot, color, intensity, range, Cast Shadows |
| Physics | Box / Sphere / Capsule / Mesh Collider | `ColliderDesc` (center, trigger, friction, bounciness) |
| Physics | Rigidbody | Dynamic / Kinematic, mass, drag, gravity, rotation freeze, CCD |
| Physics | Character Controller | capsule, step, slope, `push_mass` |
| Audio | Audio Source / Audio Listener | clip, volume, looping, 3D |
| Scripts | New Script | script name and key–value parameters |
| Elements | element components | CSG Brush etc. |

Primitives are created with their collider, a camera with an Audio Listener.

## Good to know

- **Light**: quick colour temperatures (Candle… Sky), range and cone handles right in the viewport.
- **Material**: choose and create `*.mat.ron`, edit all fields, saved at once ([Materials](materials.md)).
- **Animation keys**: Transform fields are coloured: yellow = a key on this frame, green = animated, orange = changed by hand. Right-click: Insert / Delete / Clear Keyframes; `I` / `Alt+I` on hover ([Animation](animation.md)).
- **Prefab**: a block with Open / Select / Revert All / Apply All / Unpack and a list of overrides with `↺` ([Prefabs in the editor](prefabs.md)).
- **Scripts**: parameters are typed as strings; Lua scripts do not show a parameter schema in the inspector yet (the editor does not load Lua), for Rust behaviours the schema comes from the registry.
- Wide widgets adapt to the panel: in a narrow inspector the X/Y/Z labels disappear.

## See also

- [Hierarchy](hierarchy.md)
- [Materials](materials.md)
- [Scripting](../engine/scripting.md)
- [Scene format](../engine/scene-format.md)

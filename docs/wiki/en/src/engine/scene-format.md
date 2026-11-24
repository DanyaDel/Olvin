# Scene format

A scene is a `*.scene.ron` (RON) shared by the editor and games. Code: `olvin_scene::file` (feature `serde`). A game loads it through `olvin::scene_file` (feature `scene`).

## Structure

```ron
(
    version: 4,
    entities: [ ... ],
    // environment, animation, extensions: written only if not default
)
```

`SceneFile { version, entities, environment, animation, extensions }`:

| Field | What |
|---|---|
| `entities` | a list of `EntityDesc`, parents always before children |
| `environment` | sky, ambient, exposure, fog, shadows, post-processing ([Rendering](rendering.md)) |
| `animation` | F-curves of object transforms (`Action`, [Animation](animation.md)) |
| `extensions` | data of editor elements |

## EntityDesc

Empty fields are not written to the file.

| Field | Meaning |
|---|---|
| `id`, `name`, `active`, `tag`, `layer`, `parent` | identity and hierarchy |
| `position`, `rotation`, `scale` | local Transform; rotation in degrees, Euler YXZ |
| `mesh` | `MeshDesc { shape: Cube/Sphere/Capsule/Cylinder/Plane/Quad, color }` |
| `poly` | an edited `PolyMesh` (instead of `mesh.shape`) |
| `brush` | a CSG brush `{ shape, op, color, vertices }` |
| `material` | a `*.mat.ron` path from `assets/` |
| `model` | `ModelDesc { path, animation }`: a glTF under the object |
| `camera` | `CameraDesc { fov_deg, near, far, .. }` |
| `light` | `LightDesc { kind, color, intensity, range, spot_angle, shadows, shadow_bias, size }` |
| `collider` | `ColliderDesc { shape: Box/Sphere/Capsule/Mesh, center, is_trigger, friction, restitution }` |
| `rigidbody` | `RigidbodyDesc { kind: Dynamic/Kinematic, mass, drag, angular_drag, gravity_scale, freeze_rotation, ccd }` |
| `character_controller` | `CharacterControllerDesc { height, radius, step_offset, slope_limit_deg, push_mass }` |
| `audio_source`, `audio_listener` | sound |
| `scripts` | `[ScriptDesc { name, params }]`, parameters as strings |
| `components` | `BTreeMap<type name, RON text>`: game components |
| `prefab` | a prefab instance ([Prefabs](prefabs.md)) |

An example from `apps/vertical_slice/assets/scenes/level.scene.ron`:

```ron
(id: 31, name: "Plate Trigger", position: (4.0, 1.0, 6.0), rotation: (0.0, 0.0, 0.0), scale: (2.0, 2.0, 2.0),
    collider: Some((shape: Box(size: (1.0, 1.0, 1.0)), center: (0.0, 0.0, 0.0), is_trigger: true, friction: 0.6, restitution: 0.0)),
    scripts: [(name: "plate", params: {"door": "Door", "message": "Дверь открыта: зайди к роботу"})]),
```

## Game components from a scene

Register a type in `olvin::scene_file::SceneComponents` (`register::<T>(name)` for `T: Component + Deserialize`) and a scene can carry it in `components`:

```ron
components: {"Player": "()", "PlayerLook": "(yaw: 0.0, pitch: 0.0)"}
```

Unknown names and broken values are skipped with a warning, the editor keeps them. On spawn the entity gets `SceneId(id)`; `scene_file::find(world, id)` and `spawn_ids` return an entity by id.

## Versions and migrations

`FORMAT_VERSION` is 6 now and is always written; a file without the field is version 0. `SceneFile::parse` / `load` reject a version newer than supported and run the `MIGRATIONS` chain (`fn(&mut SceneFile)`, entry `n` takes `n → n+1`).

| Version | What changed |
|---|---|
| v1 → v2 | the migration is empty, but the `model` field appeared (old builds will not open the file) |
| v2 → v3 | the clip of the old timeline (seconds) is turned into F-curves at 24 fps; the `animator` extension is removed |
| v3 → v4 | the migration is empty, `EntityDesc::prefab` was added |

Renames and new fields are covered by `#[serde(alias/default)]`, a migration fixes the meaning of data. A breaking change: `FORMAT_VERSION += 1` and a new migration. Fixtures of old versions: `crates/olvin_scene/tests/fixtures/`. The slice is written as `version: 2` and migrates fine.

## Loading

- `olvin::scene_file::load(res, path)` reads and spawns (active objects only): components move over as data, a runtime `Light` is added next to lights, CSG is built on load, `apply_lighting` moves the sun and `Environment` into `RenderSettings`.
- `SceneLoader` (`olvin_game`) reads and parses a scene on a worker thread, `scene_file::spawn_loaded` spawns the result ([States and saves](game-states-saves.md)).

> **Status:** particles (`ParticleEmitter`) are not part of the scene format yet: planned, see [#11](https://github.com/DanyaDel/Olvin/issues/11). The same for sprites and tilemaps.

## See also

- [Prefabs](prefabs.md)
- [Assets](assets.md)
- [Rendering](rendering.md)
- [Inspector](../editor/inspector.md)

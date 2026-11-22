# Prefabs (format and overrides)

A prefab is a `*.prefab.ron`: an ordinary `SceneFile` with one root and its children (own `id`s, own migrations). In a scene an instance is one root entity with `EntityDesc::prefab`.

The slice's prefab file (`apps/vertical_slice/assets/prefabs/crate.prefab.ron`):

```ron
(
    version: 4,
    entities: [
        (id: 1, name: "Crate", position: (0.0, 0.0, 0.0), rotation: (0.0, 0.0, 0.0), scale: (1.0, 1.0, 1.0),
            mesh: Some((shape: Cube, color: (0.6, 0.42, 0.22, 1.0))),
            collider: Some((shape: Box(size: (1.0, 1.0, 1.0)), center: (0.0, 0.0, 0.0), is_trigger: false, friction: 0.5, restitution: 0.0)),
            rigidbody: Some((kind: Dynamic, mass: 2.0, drag: 0.1, angular_drag: 0.1, gravity_scale: 1.0, freeze_rotation: (false, false, false), ccd: false)),
            scripts: [(name: "Thud")]),
    ],
)
```

## PrefabInstance

`PrefabInstance { asset, overrides, removed }`:

| Field | Meaning |
|---|---|
| `asset` | a path from `assets/`, like materials and models |
| `overrides` | `BTreeMap<node path, BTreeMap<field, RON text>>`: the difference from the prefab |
| `removed` | paths of removed nodes |

An override field is a top-level `EntityDesc` field by name (`position`, `mesh`, `scripts`, `material`…) or `components.<Type>` (empty text means the component is absent on the instance). Granularity: the whole field. A node path is the prefab's `id`s joined with `/`: `""` the root, `"3"` node 3, `"3/2"` node 2 of the nested prefab that is node 3. The root owns its name, active flag, parent and transform itself, that is not an override.

In the slice the third crate differs in colour: that is `overrides` on the node `""`. Children added to an instance stay ordinary scene entities with `parent` on the instance node.

## Expansion

`SceneFile::expand_prefabs(loader)` expands instances into ordinary entities; `collapse_prefabs(loader)` does the opposite (the difference becomes overrides). `member_id` gives stable node ids (a hash with `MEMBER_BIT`). A prefab that cannot be read or is cyclic (depth guard `MAX_DEPTH` = 16) gives a warning, the object stays empty, no panic.

In a game: `spawn_ids` expands instances itself; the root gets `PrefabInstanceOf(path)`. During play a prefab can be created like this:

```rust
let e = olvin::scene_file::instantiate_prefab(res, "prefabs/crate.prefab.ron", Transform::from_xyz(0.0, 2.0, 0.0))?;
```

Also: `prefab_pristine`, `prefab_diff`, `revert_prefab[_field]`, `unpack_prefab`, `apply_prefab`.

## See also

- [Prefabs in the editor](../editor/prefabs.md)
- [Scene format](scene-format.md)
- [Assets](assets.md)
- [Tutorial: step 3](../tutorial/03-physics.md)

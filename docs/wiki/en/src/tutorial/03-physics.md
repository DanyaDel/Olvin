# Step 3. Physics

Crates and a ball you can push. The `physics` module is needed (it is in the 3D + Physics template).

## A crate

1. **GameObject → 3D Object → Cube**, name `Crate`, Position `-4, 0.5, 5`.
2. Inspector → **Add Component → Physics → Rigidbody**: Kind Dynamic, Mass 2, Drag 0.1. The cube already has a Box Collider.

The ball: a Sphere at `2, 0.5, 8.5` with the same Rigidbody. Bodies are created at runtime from the `ColliderDesc` and `RigidbodyDesc` components, the object's scale is baked into the collider shape, see [Physics](../engine/physics.md).

## A prefab

The four crates of the level are instances of one prefab, `prefabs/crate.prefab.ron`. To do the same:

1. Select `Crate` → **GameObject → Create Prefab** (or the Hierarchy context menu). `prefabs/Crate.prefab.ron` appears and the object becomes an instance (a blue ◆ icon).
2. Drag the prefab from Assets into the viewport (it lands on the ground under the cursor) or onto a Hierarchy row (it becomes a child).
3. Editing an instance node is an override: the **Prefab** block of the Inspector lists them, `↺` reverts a field, **Apply All** writes the changes back to the asset.

In the scene an instance takes one entry:

```ron
(id: 41, name: "Crate 2", position: (-5.3, 0.5, 5.2), rotation: (0.0, -5.0, 0.0), scale: (1.0, 1.0, 1.0),
    prefab: Some((asset: "prefabs/crate.prefab.ron"))),
```

More: [Prefabs](../engine/prefabs.md), [Prefabs in the editor](../editor/prefabs.md).

## Check

In the editor press ▶ (`Ctrl+P`): the crates fall and settle on the floor. Stop returns the scene as it was.

## See also

- [Physics](../engine/physics.md)
- [Prefabs](../engine/prefabs.md)
- [Inspector](../editor/inspector.md)
- [Next: step 4, player](04-player.md)

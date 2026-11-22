# Step 2. Scene

We build a level: floor, four walls, a partition with a doorway, a sun. This is how `assets/scenes/level.scene.ron` of the slice is made.

## Objects

Create them with **GameObject → 3D Object → Cube** (a primitive gets its own Box Collider) and edit the Transform in the Inspector:

| Name | Position | Scale |
|---|---|---|
| Floor | 0, -0.5, 0 | 24, 1, 24 |
| Wall North | 0, 2, -12.25 | 24, 4, 0.5 |
| Wall South | 0, 2, 12.25 | 24, 4, 0.5 |
| Wall East | 12.25, 2, 0 | 0.5, 4, 25 |
| Wall West | -12.25, 2, 0 | 0.5, 4, 25 |
| Divider West | -6.625, 2, 0 | 10.75, 4, 0.5 |
| Divider East | 6.625, 2, 0 | 10.75, 4, 0.5 |
| Lintel | 0, 3.5, 0 | 2.5, 1, 0.5 |

A 2.5 m opening remains between Divider West and East: the door goes there (step 5). Color is set in the Mesh Renderer section.

Double-click a name in the Hierarchy to rename, `Ctrl+D` duplicates (Transform values are copied).

## Light

**GameObject → Light → Directional Light**, Rotation `-50, 30, 0`, Cast Shadows on. The first directional light becomes the sun and casts the shadows. The environment (sky, fog, exposure): **Edit → Settings… → Rendering**, with the Day / Sunset / Night / Studio presets, see [Rendering](../engine/rendering.md).

## Save

`Ctrl+S`, path `scenes/level.scene.ron` (or `scenes/main.scene.ron`, which is what the template loads). The RON looks like this (an excerpt from the slice):

```ron
(id: 10, name: "Floor", position: (0.0, -0.5, 0.0), rotation: (0.0, 0.0, 0.0), scale: (24.0, 1.0, 24.0),
    mesh: Some((shape: Cube, color: (0.32, 0.34, 0.36, 1.0))),
    collider: Some((shape: Box(size: (1.0, 1.0, 1.0)), center: (0.0, 0.0, 0.0), is_trigger: false, friction: 0.8, restitution: 0.0))),
```

The format in detail: [Scene format](../engine/scene-format.md).

## Check

`olvin-hub run`: the game loads the template's `assets/scenes/main.scene.ron`. There is no camera in the scene yet, we create it together with the player in [step 4](04-player.md).

## See also

- [Scene view](../editor/scene-view.md)
- [Inspector](../editor/inspector.md)
- [Hierarchy](../editor/hierarchy.md)
- [Next: step 3, physics](03-physics.md)

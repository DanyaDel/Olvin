# Hierarchy

The tree of scene objects (the Hierarchy panel). The order of parents and children matters: CSG operations are ordered the same way.

## Actions

| What | How |
|---|---|
| Select | click; `Ctrl+click` adds; `Shift+click` a range |
| Change parent | drag a row onto another |
| Rename | double-click or context menu → Rename |
| Create | context menu → Create child (the same items as in GameObject) |
| Duplicate / delete | `Ctrl+D` / `Del` or the context menu (a selected parent and child are copied once) |
| Order | context menu → Move Up / Move Down |
| Unparent | context menu → Unparent |
| Find | the search field at the top of the panel |

From Assets you can drop onto a hierarchy row a model (becomes a child), a material (is assigned) or a prefab (becomes a child instance).

## Prefabs

Prefab instances are painted in the theme's `info` colour with a ◆ icon (an amber ⚠ if the prefab could not be read). Instance nodes cannot be dragged, duplicated or reordered: they stay where the prefab put them. More: [Prefabs in the editor](prefabs.md).

## Format

Parents always come before children in the scene file; the hierarchy is stored through `parent` (the parent's id), see [Scene format](../engine/scene-format.md).

> **Status:** there is no scene search besides the hierarchy field; multi-scene is not done, see [#44](https://github.com/DanyaDel/Olvin/issues/44).

## See also

- [Inspector](inspector.md)
- [Scene view](scene-view.md)
- [Prefabs in the editor](prefabs.md)
- [Scene format](../engine/scene-format.md)

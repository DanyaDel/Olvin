# Prefabs in the editor

A prefab is a `*.prefab.ron`: an ordinary scene with one root and its children. In a scene an instance takes one entry; the format: [Prefabs (engine)](../engine/prefabs.md).

## How it works in the editor

The in-memory scene keeps instances already expanded: every node is an ordinary object, so the hierarchy, viewport and gizmos know nothing about prefabs. **Editing a node is the override.** On save the scene is collapsed: the file gets the root and the difference.

## Actions

| What | How |
|---|---|
| Create a prefab | GameObject → Create Prefab or the Hierarchy context menu: the subtree goes to `prefabs/<Name>.prefab.ron`, the object becomes an instance |
| Place an instance | drag a `.prefab.ron` from Assets into the viewport or onto a Hierarchy row |
| Open a prefab | double-click in Assets or Open in the inspector (opens as a scene) |
| Revert | `↺` on a field, Revert All on an instance |
| Apply | Apply All: the asset is overwritten, other instances are rebuilt |
| Unpack | Unpack: the instance becomes ordinary objects |

The **Prefab** block in the inspector: Open / Select / Revert All / Apply All / Unpack, a list of overrides. Overridden fields have a blue header and a label with `↺`.

A prefab edited outside (in a text editor) is picked up: mtime and size are checked every 0.4 s. Moving a prefab in Assets rewrites paths; deleting one first unpacks instances, the objects stay.

Limits: instance nodes cannot be dragged, duplicated or reordered. Override granularity is the whole field (`position`: the whole vector; `scripts`: the whole list).

## See also

- [Prefabs (engine)](../engine/prefabs.md)
- [Hierarchy](hierarchy.md)
- [Assets](assets.md)
- [Tutorial: step 3](../tutorial/03-physics.md)

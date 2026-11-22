# Assets panel

A browser of the project's `assets/` folder: breadcrumbs, tiles with thumbnails, a filter, creating a folder or a scene. Double-click on a scene opens it, on a `.gltf`/`.glb` imports the model.

## Actions

| What | How |
|---|---|
| Rename | `F2` or the context menu |
| Delete | `Del` (asks first; the `.meta` goes with the file) |
| Move | drag a tile onto a folder, a breadcrumb or the ⬆ button |
| Create a material | context menu of an image → Create material |
| Import a model | double-click a `.gltf` / `.glb` or "Import into scene" |

Moves and renames go through `AssetRoot::move_asset`: the GUIDs in `.meta` survive and paths are rewritten in the open scene, its undo history and in `.scene.ron`, `.mat.ron`, `.prefab.ron` files. A file operation cannot be undone.

## Drag and drop

- A **model** onto the viewport: imported at the ground point under the cursor; onto a Hierarchy row: as a child.
- A **material** onto an object: assigned.
- A **prefab** into the viewport or onto a Hierarchy row: an instance.
- A **scene**: opened.

While the cursor is over Assets, `Del` and `Ctrl+D` act on files, not scene objects.

## GUID and `.meta`

Every asset gets a `<name>.meta` (RON) next to it with a GUID and importer settings (for textures: sRGB, max size, mips, point filter). A copy of a file together with the old `.meta` gets a new GUID. More: [Assets](../engine/assets.md).

## See also

- [Assets (engine)](../engine/assets.md)
- [Materials](materials.md)
- [Prefabs in the editor](prefabs.md)
- [Project layout](../engine/project-layout.md)

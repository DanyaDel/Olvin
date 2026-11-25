# Assets panel

A browser of the project's `assets/` folder: breadcrumbs, tiles with thumbnails, a filter, creating a folder or a scene. Double-click on a scene opens it, on a `.gltf`/`.glb` imports the model.

## Actions

| What | How |
|---|---|
| Rename | `F2` or the context menu |
| Delete | `Del` (asks first; the file and its `.meta` go to the project's trash `.olvin/trash`, `Ctrl+Z` brings them back) |
| Move | drag a tile onto a folder, a breadcrumb or the ⬆ button |
| Create a material | context menu of an image → Create material |
| Import a model | double-click a `.gltf` / `.glb` or "Import into scene" |

Moves and renames go through `AssetRoot::move_asset`: the GUIDs in `.meta` survive and paths are rewritten in the open scene, its undo history and in `.scene.ron`, `.mat.ron`, `.prefab.ron` files. Renames, moves and deletes are ordinary `Ctrl+Z`/`Ctrl+Y` steps: a move is undone by the opposite move (references follow back), a deleted file comes back from the trash with the same GUID, and the instances of a deleted prefab become instances again (in the open scene and in files). The trash keeps a session's files for a day and is emptied the next time the project opens. Opening another scene clears the history, but the files stay in `.olvin/trash` for a day.

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

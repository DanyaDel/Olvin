# Editor: overview

The scene editor `olvin-editor`: Blender and Unity style, on egui 0.36 and egui_dock, English interface. It is installed by the hub and its elements plug in as crates linked statically (no dylibs).

![The editor](../images/editor-dark.png)

## Workspaces

Tabs on top work like in a browser: click selects, double-click renames, middle-click or × closes, right-click duplicates or moves, "+" creates a new one (empty, a copy or a template). Inside, a workspace is a dock: panels are dragged, split and torn into windows, every tab group has its own "+".

| Template | From | Contents |
|---|---|---|
| Layout | base | Scene, Game, Hierarchy, Inspector, Assets |
| Lighting | base | environment, presets, light list ([Settings](settings.md)) |
| Animation | Animator element | Timeline, Dope Sheet, Graph Editor ([Animation](animation.md)) |
| Mapping | CSG element | levels from brushes ([CSG](csg.md)) |
| Modeling | Modeling element | Edit Mode ([Modeling](modeling.md)) |
| UV Editing | UV element | UV Editor next to the scene view ([UV](uv.md)) |
| Code | IDE element | code and shaders ([IDE](ide.md)) |

The first template (Layout) opens by default. The layout is saved to `<config>/olvin-editor/layout.ron` on exit. **Window → Add Panel** adds any panel, **Window → Reset Layout** resets.

## Menus and the top bar

- **File**: New Scene `Ctrl+N`, Save `Ctrl+S`, Save As…, Quit.
- **Edit**: Undo/Redo, Duplicate, Delete, Settings… `Ctrl+,`.
- **GameObject**: Create Empty, 3D Object, Light, Audio, Camera, Create Prefab and the elements' items (CSG/…, Mesh/…).
- **Window**: Add Panel, New Workspace, Reset Layout.
- In the middle ▶ ⏸ ⏭: Play (`Ctrl+P`), Pause, frame step. While playing the interface is tinted and the status bar shows `● PLAYING`.

## Play Mode

Play starts a real `App` with physics, input and Rust scripts on a copy of the scene. Stop returns the scene and the undo history to the moment of the start. While playing you edit the "pre-Play" scene, undo/redo are frozen and Save writes it. The Game window forwards keyboard and mouse to the game while the cursor is over it.

> **Status:** Lua scripts do not run in Play (the editor does not load `LuaPlugin`, so Lua script parameters are not visible in the inspector either, see the ROADMAP item on Lua) and particles and sprites are not drawn. glTF models stand in their default pose in Play.

## Undo, selection, saving

- Undo/redo works with snapshots: a change is committed when mouse and keyboard are released, a drag is one step. Material and prefab edits are in the history too (files are rewritten on undo). File operations in Assets cannot be undone.
- Selection: an active object and additional ones (`Ctrl+click` in the hierarchy and viewport, `Shift` in the hierarchy selects a range). Move, Delete and `Ctrl+D` act on the whole selection.
- Unsaved changes: the editor asks before closing and before replacing the scene.

## See also

- [Hierarchy](hierarchy.md)
- [Inspector](inspector.md)
- [Scene view](scene-view.md)
- [Hotkeys](hotkeys.md)
- [First editor run](../start/editor-first-run.md)

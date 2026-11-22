# First editor run

The editor is built by the hub from the engine sources and opens a project folder. The interface is in English (STYLE.md, section 10), so this wiki names things as they appear in the UI.

## Launching

```sh
olvin-hub editor install       # once: a few minutes of building
cd my_game && olvin-hub editor open
```

In the GUI hub: Editor page → Install, then Open on a project. To develop the editor itself: `cargo run -p editor_dev -- <project folder>` (all elements at once).

## What is on screen

![The editor](../images/editor-dark.png)

The default **Layout** workspace:

| Panel | Where | What for |
|---|---|---|
| Scene (viewport) | center | view of the scene, gizmos, picking |
| Game | tab next to it | the view from the scene's first camera; the live game in Play |
| Hierarchy | left | object tree |
| Inspector | right | components of the selected object |
| Assets | bottom | the project's `assets/` folder |

On top: the **File / Edit / GameObject / Window** menus, workspace tabs, ▶ ⏸ ⏭ in the middle (Play, Pause, frame step), ⚙ (Settings) on the right. At the bottom a status bar: `● PLAYING` or `PAUSED`, the latest log message.

## First steps

1. **GameObject → 3D Object → Cube**: a cube with its own collider is created.
2. Select it in the Hierarchy, move it with `W` (Move) and the gizmo arrows.
3. `Ctrl+S`: the editor asks for a path, `scenes/main.scene.ron` by default. The hub template's game loads exactly that.
4. `olvin-hub run` in the project folder shows the scene in the game.

Edits are saved explicitly; Undo/Redo works on snapshots: `Ctrl+Z`, `Ctrl+Shift+Z` (or `Ctrl+Y`). The editor will not close with unsaved changes without asking.

## Editor files

| File | What |
|---|---|
| `<config>/olvin-editor/preferences.ron` | theme, camera, UI scale, default shading |
| `<config>/olvin-editor/layout.ron` | tabs and workspace layouts, written on exit |
| `<project>/logs/editor.log` | the editor log |

Reset the layout: **Window → Reset Layout**.

## See also

- [Editor: overview](../editor/overview.md)
- [Hotkeys](../editor/hotkeys.md)
- [Tutorial: your first game](../tutorial/index.md)
- [Black window and GL](../troubleshooting/black-window.md)

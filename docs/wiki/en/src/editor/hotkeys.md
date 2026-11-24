# Hotkeys

The editor and scene-view commands (save, new, undo, redo, duplicate, delete, play, settings, the four tools, Global/Local, frame, Solid/Rendered, keying) can be rebound in **Edit → Settings… → Keymap**: click a key, press the new one (`Esc` cancels); a clash with another command is shown next to it, *Reset* puts the default back, the changes are saved per user in `preferences.ron`. The keys of Edit Mode, the UV editor, the animation timeline and text fields are fixed. The tables below are the defaults. On macOS `Ctrl` is `Cmd` in combinations with `Ctrl+S/N/Z/D/P/,`.

## Editor

| Keys | Action |
|---|---|
| `Ctrl+S` / `Ctrl+N` | save / new scene |
| `Ctrl+Z` / `Ctrl+Shift+Z` (`Ctrl+Y`) | undo / redo |
| `Ctrl+D` / `Del` | duplicate / delete the selection (over Assets: files) |
| `Ctrl+P` | Play / Stop |
| `Ctrl+,` | Settings |
| `F2` | rename a file in Assets |

## Scene

| Keys | Action |
|---|---|
| `Q` / `W` / `E` / `R` | Hand / Move / Rotate / Scale |
| `X` | Global / Local |
| `F` | frame the selection |
| `Shift+Z` | Solid / Rendered |
| `Ctrl+click` | add to / remove from the selection |
| `I` / `Alt+I` | key Location, Rotation, Scale / delete keys on the frame |
| `B` / `T` | CSG: Draw Brush / Resize |

## Edit Mode (`Tab`)

| Keys | Action |
|---|---|
| `Tab` | enter / leave (a primitive or brush is converted to a `PolyMesh`) |
| `1` / `2` / `3` | vertices / edges / faces |
| `A` / `Alt+A` / `Ctrl+I` | select all / none / invert |
| `G` / `R` / `S` | move / rotate / scale; `X` `Y` `Z` axis (again: local), `Shift+axis` plane, a number, `Ctrl` snap, `Shift` precision ×0.1 |
| `E` / `I` | extrude (move at once) / inset |
| `Shift+D` | duplicate |
| `Ctrl+R` / `K` | loop cut / knife |
| `F` / `J` | make face / connect vertices |
| `X` / `M` | delete / merge |
| `Shift+N` | recalculate normals outside |
| `Alt+click` / `Ctrl+Alt+click` | edge loop / ring (in face mode a face loop) |
| `L` / `Shift+L` / `Ctrl+L` | select linked |
| `Ctrl++` / `Ctrl+-` | grow / shrink the selection |
| `H` / `Shift+H` / `Alt+H` | hide / hide others / reveal |
| `Alt+Z` | X-ray |

## UV Editor

| Keys | Action |
|---|---|
| `U` | unwrap |
| `Ctrl+E` | seams |
| `Ctrl+P` | pack |
| `G` / `R` / `S` | move / rotate / scale, `X` / `Y`, numbers, `Ctrl` snap |

## Animation

Global: `Space` play/pause, `Shift+Ctrl+Space` backwards, `Esc` stop and return to the start, `←` `→` frame, `Shift+←/→` start/end, `↑` `↓` next/previous key. In the Dope Sheet and Graph Editor: `A`, `Alt+A`, `B`, `Ctrl+I`, `G`, `S`, `Shift+D`, `X`/`Del`, `Ctrl+C`/`Ctrl+V`, `T` (interpolation), `V` (handle type), `Home`.

## IDE

`Ctrl+Shift+P` palette, `Ctrl+E` go to file, `Ctrl+G` line, `Ctrl+/` comment, `Ctrl+S` save the file, `Ctrl+W` close a tab.

## The game in the slice

`WASD`, mouse, `Space` jump, `Shift` sprint, `Esc` pause (`Q` quits while paused), `F5` / `F9` quick save / load.

## See also

- [Scene view](scene-view.md)
- [Modeling](modeling.md)
- [Animation](animation.md)
- [Settings](settings.md)

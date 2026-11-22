# FAQ and troubleshooting

Short answers; details on separate pages.

| Symptom | Where to look |
|---|---|
| The window opens black, or no frame is shown | [Black window and GL](black-window.md) |
| The game is silent | [No sound](sound.md) |
| I changed a script or asset and nothing changed | [Hot reload](hot-reload.md) |
| Low fps | [Slow frames](performance.md) |
| `cargo` complains | [Build errors](build-errors.md) |
| Windows, macOS | [Windows and macOS](platforms.md) |

## Questions

**How do I see the log?** It goes to stderr and to `logs/<name>.log` next to the project; level: `OLVIN_LOG=debug` or `OLVIN_LOG=info,olvin_render=trace`. On a panic `logs/crash-<time>.txt` appears with a backtrace and the last log lines. The editor log: `<project>/logs/editor.log` and the Console panel.

**The game cannot find the scene or assets.** The project root is determined by `ProjectPaths`: `OLVIN_PROJECT_DIR`, the exe's folder with `assets/`, `CARGO_MANIFEST_DIR`, the working directory. Run with `OLVIN_LOG=info`: the game logs where it took `assets` from ([Project layout](../engine/project-layout.md)).

**A Lua script does not run in the editor.** The editor does not load Lua, scripts work in the game (`olvin-hub run`). You need the `lua` module and `LuaPlugin` in `main.rs`.

**A script turned off and no longer works.** A failed callback disables the script until the file is reloaded, the error is in the log. Fix and save the file (with `hot_reload`) or restart.

**The editor asks to rebuild.** The engine changed: the "The engine changed…" bar in the hub or an automatic rebuild on `editor open`.

**A save does not restore Lua state.** This is a known limitation ([States and saves](../engine/game-states-saves.md)).

**An object with a collider does not move with its Transform.** Fixed and Dynamic bodies are teleported when `Transform` changes; for a collider that moves constantly use a Kinematic Rigidbody.

**Shadows vanished on a weak GPU.** On Compat there are at most 2 cascades and 2 + 1 light shadows; Soft and PCSS are replaced by PCF.

**Where do I ask or report a bug?** In [issues](https://github.com/DanyaDel/Olvin/issues) (in Russian, see [languages](../dev/language-policy.md)). A docs-versus-code mismatch is a documentation bug.

## See also

- [Black window and GL](black-window.md)
- [Build errors](build-errors.md)
- [Installation](../start/installation.md)
- [Environment variables](../reference/env-vars.md)

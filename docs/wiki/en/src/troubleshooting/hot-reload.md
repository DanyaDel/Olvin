# Hot reload does not work

In Olvin data is reloaded hot, not game code.

| What | Works | How |
|---|---|---|
| Lua scripts | yes | `LuaPlugin { hot_reload: true, .. }` (in the template and slice only in debug: `cfg!(debug_assertions)`); `LuaScripts::poll` |
| Materials, textures, shaders, sky | yes | `RenderAssets` checks mtime every 0.5 s; `AssetManager::with_hot_reload()` |
| Prefabs in the editor | yes | every 0.4 s |
| Rust code | no | rebuild, see [#70](https://github.com/DanyaDel/Olvin/issues/70) |
| A scene in a running game | no | restart |

## Lua does not update

- The game is built `--release` while `hot_reload: cfg!(debug_assertions)`: turn it on explicitly.
- The file is not in the plugin's `dir` (`assets/scripts`) or `dir` is relative to the working directory: pass `paths.asset("scripts")`.
- A syntax error: the last working version is kept, the error is in the log.
- A callback failed: the script is disabled until the file is reloaded; save the file again.
- Lua is re-read by polling (`LuaScripts::poll`, by file modification time), and assets through `AssetWatcher` (notify): on a network folder and in some containers file system events do not arrive and asset reload will not work.

After a reload `self` survives and `start` is not called again. New `params` fields do not appear in already created objects until a restart.

## The editor

The editor does not load Lua, so there is nothing to reload: check scripts in the game.

## See also

- [Scripting](../engine/scripting.md)
- [Assets](../engine/assets.md)
- [FAQ](faq.md)
- [Lua API](../reference/lua-api.md)

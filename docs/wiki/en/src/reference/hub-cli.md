# olvin-hub CLI

`olvin-hub [command]`. Without arguments it prints help with a text logo. The `-p, --project <path>` option names a project; by default the project is looked up from the current folder upward. Messages are in English.

| Command | What it does |
|---|---|
| `new <name> [--dir <folder>]` | create a game project (in the current folder by default) |
| `list [-p]` | the module catalog, `[x]` marks installed ones |
| `add <module>…` | install modules together with what they require |
| `remove <module>… [--force]` | remove modules; without `--force` you cannot remove what others depend on |
| `run [--release]` | build and run the game |
| `build [--release] [-o <folder>] [-a tar.gz\|zip]` | build the game and lay out a folder for players |
| `projects` | projects the hub knows about |
| `register <path>` / `forget <path>` | add to / remove from the list (files stay) |
| `engine [--path <folder> \| --git <url>]` | show or change the engine source for new projects |
| `editor install` | build the editor |
| `editor list` | elements, `[x]` installed |
| `editor add <element>…` / `editor remove <element>…` | install / remove elements and rebuild |
| `editor open [project]` | open a project in the editor (from the current folder by default); a stale editor is rebuilt |

## build

```sh
olvin-hub build --release                       # dist/<name>/
olvin-hub build --release -o ~/out/mygame       # your own folder
olvin-hub build --release -a tar.gz             # + <folder>.tar.gz
```

It runs `cargo build --message-format=json-render-diagnostics` (cargo's output goes to the terminal, the exe path is taken from `compiler-artifact`), puts the exe, a copy of `assets/` (without hidden files and `*~`) and `licenses/` into `<project>/dist/<name>`. The folder is rebuilt only if the hub created it (the `.olvin-build` marker). When building into the default folder `/dist` is appended to an existing `.gitignore`. It works the same for crate projects inside a workspace (`--project apps/vertical_slice`).

Not done: `--target` (cross-compilation), asset stripping and compression, installers.

## Modules and elements

The module catalog: [Hub](../start/hub.md#modules). Editor elements: `console`, `profiler`, `animator`, `csg`, `modeling`, `ide`, `uv`; `console`, `csg`, `modeling`, `uv` are installed by default.

## Files and environment

`~/.config/olvin-hub/config.toml` (`XDG_CONFIG_HOME`), `~/.local/share/olvin-hub/editor`. `NO_COLOR` removes the logo colour.

## See also

- [Hub](../start/hub.md)
- [Installation](../start/installation.md)
- [olvin features](olvin-features.md)
- [Environment variables](env-vars.md)

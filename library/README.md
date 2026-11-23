# Olvin preset library

Ready-made packs for Olvin projects: characters, shaders, materials, looks, prefabs. Think of the old Unity Asset Store, offline and small. Everything in this folder is compiled into the editor and `olvin-hub`, so it is there without a download.

Using it:

- Editor: **Window > Add Panel > Library**, pick a pack, **Import**.
- Command line: `olvin-hub library list`, `olvin-hub library show <id>`, `olvin-hub library add <id> [--project DIR]`, `olvin-hub library installed`.
- Your own packs: put them in `<project>/library/<pack-id>/` or in a folder listed in `$OLVIN_LIBRARY` (a path list, `:` on Linux and macOS, `;` on Windows). They show up next to the built-in ones.

## A pack

```
library/<pack-id>/pack.ron        manifest
library/<pack-id>/assets/...      files laid out exactly as they appear in a project's assets/
library/<pack-id>/preview.png     optional, at most 256 KB
```

`pack.ron` is RON; unknown fields are an error, so a typo does not silently drop `requires`:

```ron
(
  id: "fps-controller",            // kebab-case, unique, equals the folder name
  name: "First Person Controller",
  version: "1.0.0",                // major.minor.patch
  category: Characters,            // Characters, Gameplay, Shaders, Materials, Looks, Prefabs, Audio, Misc
  description: "Mouse look, WASD, jump, sprint ...",   // English, 1-3 sentences
  tags: ["player", "fps"],
  author: "NLPA Collab",
  license: "MIT OR Apache-2.0",    // an SPDX expression: "CC0-1.0", "CC-BY-4.0", ...
  requires: [],                    // other pack ids, imported first
  provides: ["prefabs/fps-controller/Player.prefab.ron"],  // headline assets, shown in the UI
  notes: "How to use after import ...",   // optional, shown after the import
)
```

- `provides` must exist in `assets/`. The first `*.prefab.ron` is what the editor's **Add to scene** instantiates; a `*.scene.ron` is offered as the demo to open.
- `notes` says what to do next: which scene to open, which parameters to tune.

## What the import does

1. Resolves `requires` (cycles and missing packs are errors) and imports them first.
2. Copies `assets/**` into the project's `assets/` and creates a `.meta` with a new GUID for every file, through `olvin_assets`. Packs never ship `.meta` files.
3. Never overwrites a different file silently. Identical files are skipped. Differing files are reported as conflicts, and the user picks **Replace**, **Keep both** (the pack's copy gets a `-2` suffix before the extension) or **Skip**. The command line refuses until you pass `--replace`, `--keep-both` or `--skip`.
4. Records the pack in `<project>/library.lock.ron`: id, version, whether you asked for it or it came as a dependency, and every file with its SHA-256. This is what "Installed" in the UI reads, and what Update and Remove will be built on.

## Namespacing

Two packs must not write the same path, and neither should collide with a user's files. Put every file under its pack id, either directly or one folder below the kind of asset:

```
assets/<pack-id>/...                      for example assets/fps-controller/notes.txt
assets/<kind>/<pack-id>/...               scripts/fps-controller/player.lua
                                          prefabs/fps-controller/Player.prefab.ron
                                          scenes/fps-controller/demo.scene.ron
```

Files outside both shapes are not an error but `Pack::lint` reports them. References inside a pack are written against the final paths: a prefab says `scripts: [(name: "fps-controller/player")]`, a material says `"textures/pbr-stone/albedo.png"`.

Lua scripts in subfolders of `assets/scripts/` are named by their path under `scripts/`, without `.lua`: `scripts/fps-controller/player.lua` is the script `fps-controller/player`. (A file directly in `scripts/` keeps its plain name.)

## Rules the loader checks

A pack that breaks one is skipped, listed in `Library::problems()`, and fails the `the_embedded_library_is_valid` test.

- `id` is kebab-case (`a-z`, `0-9`, single dashes) and equals the folder name. `name`, `description`, `author`, `license` are not empty. `version` is `major.minor.patch`. At most 12 tags.
- File paths are relative, use `/`, have no `..`, no hidden files, only letters, digits and `-_. ()`.
- Allowed extensions: `ron lua wgsl png jpg jpeg tga webp hdr exr ogg wav flac mp3 gltf glb bin json toml txt md`. No `.meta`, no executables, no Rust source (a pack must not change what a game compiles).
- At most 16 MiB per file, 64 MiB and 1024 files per pack, 256 KB for `preview.png`.
- No two paths that differ only in case.

## Testing a pack

1. `cargo test -p olvin_library`: loads every pack in this folder and checks the rules above.
2. Import it into a throwaway project and play it headless, like `crates/olvin_library/tests/characters.rs`: `Library::embedded().import(id, project_dir, &ImportOptions::default())`, then `HeadlessPlugins` + `LuaPlugin` + `olvin::scene_file::load` of the demo scene, feed keys through the `Input` resource, step frames, assert, and check that nothing was logged at warning level.
3. A pack with scripts or prefabs should ship a demo scene (`scenes/<pack-id>/demo.scene.ron`) that opens and plays with no setup.
4. In the editor: Window > Add Panel > Library, import, open the demo scene, press Play.

## Gameplay packs

Category Gameplay: Lua scripts and prefabs that work together. Every pack has a demo scene (`scenes/<id>/demo.scene.ron`; open it and press Play) and requires `fps-controller` for the player (an entity named `Player` or tagged `Player`). Scripts show their parameters in the Inspector. Sound is never shipped: every sound hook is an optional path parameter, empty by default.

| Pack | What it gives |
|---|---|
| `interactive-door` | `Door` (slide or hinge; proximity, key press or message; lockable), `Switch` plate |
| `pickups` | `Coin`, `HealthPack`, `Key` |
| `health-damage` | `health.lua`, `HazardZone`, `SpikeTrap`, `PlayerHealth` (the fps player with health) |
| `checkpoints` | `Checkpoint`, `KillPlane` |
| `moving-platforms` | `MovingPlatform` (waypoints, carries the player), `Spinner` |
| `enemy-ai` | `Enemy` (patrol, sight, chase, attack) |
| `spawners` | `Spawner` (interval, message, waves), `Crate`, `lifetime.lua` |
| `camera-effects` | `camera_shake`, `smooth_follow`, `look_at_target`, `trigger_zone` (+ prefabs) |
| `gameplay-demo` | an arena using all of them |

Messages (`ctx:send(name, value[, target])`; "to X" means send it to that entity, "broadcast" means no target):

| Message | Value | Sent by | Understood by |
|---|---|---|---|
| `open`, `close`, `toggle`, `lock`, `unlock` | door id (string) or nothing | `door_switch`, `trigger_zone`, anything | doors (to the door, or broadcast with its `id`) |
| `door_opened`, `door_closed`, `door_locked` | door id or name | doors | anyone |
| `picked_up` | kind (`coin`, `health`, `key`, `custom`) | pickups | anyone |
| `damage` | amount | hazards, `touch_damage`, enemies, kill plane | `health.lua` (to the victim) |
| `heal` | amount | health packs | `health.lua` (to the target) |
| `kill`, `revive` | | anyone | `health.lua` |
| `damaged`, `healed` | amount | `health.lua` (broadcast) | camera shake (`listen`) |
| `died`, `revived` | entity name | `health.lua` (broadcast) | enemies |
| `respawn` | entity id | `health.lua` after `respawn_delay`, kill plane | the active checkpoint |
| `checkpoint_reached`, `respawned` | checkpoint name | checkpoints | anyone |
| `enemy_alert`, `enemy_lost` | enemy name | enemies | anyone |
| `spawn`, `start`, `stop`, `clear` | | anyone | spawners (to the spawner); `start`/`stop` also platforms |
| `spawned` | new root's entity id | the engine (`ctx:spawn_prefab`) | the requesting spawner |
| `wave_started`, `waves_done` | wave number / spawner name | spawners | anyone |
| `shake` | 0..1 or `vec3(strength, seconds, 0)` | `trigger_zone`, anyone | `camera_shake` (to the camera) |

Shared variables (`ctx:var` / `ctx:set_var`; a game can mirror them into its HUD): `score` (coins), `keys` (count), `key_<id>` (true once picked up), `health` and `health_max` (`health.lua`, param `var`), `checkpoint` (name of the active checkpoint) and `checkpoint_pos` (vector).

A health bar or score text is drawn by the game, not by Lua: read `ScriptVars` in a Rust system and call `gui.ui.set_text(node, ...)`, as `apps/vertical_slice/src/hud.rs` does for messages.

Limits worth knowing: a character cannot walk on a Kinematic body (the physics controller stalls), so `MovingPlatform` moves a plain collider and carries the player in its script; scripts see the player by name or tag only.

## Licences

Everything shipped here must be original work or under a licence compatible with MIT OR Apache-2.0 (CC0-1.0, MIT, Apache-2.0, CC-BY with attribution, OFL for fonts). Put the SPDX expression in `license`. Anything not written by the NLPA Collab goes into `THIRD_PARTY.md` at the repository root with its source and licence text, and the pack's `license` says so. Do not copy from the Unity Asset Store, Unreal Marketplace, Sketchfab downloads under non-free terms, or any "free for personal use" source.

## Adding a pack to the repository

Create `library/<pack-id>/`, add the files, run `cargo test -p olvin_library`. There is nothing to register: `crates/olvin_library/build.rs` makes cargo rebuild when anything under `library/` changes, and `include_dir` bakes the folder into the binary.

# Contributing guide

How to build, check and submit a change to Olvin. Closes [#62](https://github.com/DanyaDel/Olvin/issues/62). In short: `fmt`, `clippy`, `test`, a commit by the rules, updated documentation.

## The repository

```
crates/   the engine (olvin_core … olvin is the facade)
editor/   olvin_editor and elements
hub/      olvin_hub (core), olvin_hub_cli, olvin_hub_gui
apps/     sandbox, script_demo, editor_dev, vertical_slice
docs/     wiki, adr, screenshots
```

Before working read [ARCHITECTURE.md](https://github.com/DanyaDel/Olvin/blob/main/ARCHITECTURE.en.md): what is in each crate. It is also the source of truth when it disagrees with the code, and it is extended after every significant change (the Russian and English files together, [languages](language-policy.md)).

## Checks before a commit

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

CI runs the same (`.github/workflows/ci.yml`). Rust 1.95+, edition 2024. Keep heavy builds in a `CARGO_TARGET_DIR` on disk, not in `/tmp` (a tmpfs on Arch).

## Tests and GPU

Most tests are ordinary. Render tests draw into an offscreen texture and read pixels: they need an adapter. In CI and locally without a GPU they run on Mesa llvmpipe:

```sh
export __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl
cargo test -p olvin_render
cargo test -p olvin_gui --features render
```

With Vulkan (the default, no variables) the same tests run on the real GPU. Notes:

- PCSS and other Full-tier tests need a native adapter (Vulkan / Metal / DX12): with `WGPU_BACKEND=gl` they are skipped.
- Tests marked `#[ignore]` draw screenshots and UI with egui_kittest and wgpu; run with `-- --ignored`:

  ```sh
  cargo test -p editor_dev --test screenshots -- --ignored     # docs/screenshots/editor-*.png
  cargo test -p editor_dev --test ide -- --ignored             # the IDE with real clicks
  cargo test -p editor_dev --test animator -- --ignored
  cargo test -p olvin_hub_gui -- --ignored screenshots         # OLVIN_HUB_SHOTS=<folder>
  ```

- `cargo test -p vertical_slice`: 7 headless tests play the real level without a window or GPU (through `HeadlessPlugins`).
- Format fixtures live in `crates/olvin_scene/tests/fixtures/` and `crates/olvin_game/tests/fixtures/`.

In egui 0.36 headless tests must clear `textures_delta`.

## Code style

- `cargo fmt` without exceptions; clippy without warnings.
- Identifiers, comments, doc comments, log and error messages: English. Errors: what happened and what to do.
- Hot paths without allocations in steady state; flat arrays, not objects.
- User data (paths, files) is handled through `Result`, not `unwrap()`. `unwrap()` is fine in tests.
- Do not add `TODO`/`todo!`: file an issue.
- Visual style of the editor and hub: [STYLE.md](https://github.com/DanyaDel/Olvin/blob/main/STYLE.en.md); the interface is English, with no exclamation marks.
- A breaking change of the scene or save format: `FORMAT_VERSION += 1` (`SAVE_VERSION`), a migration, a fixture of the old version.

## Commits

The message is in English: `Area: what was done (#number)`.

```
Editor: prefabs (#24)

Create Prefab from the selection, drag .prefab.ron into the scene ...

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_...
```

- One commit, one logical change; in the body: what and why.
- If the code was written with an AI agent, the `Co-Authored-By:` and `Claude-Session:` trailers stay at the end of the message (Claude Code adds them; do not remove them).
- The issue number in the title (`(#24)`) if the change closes or advances it.
- Issues and discussions: in Russian.

## Add an engine plugin

1. A crate `crates/olvin_<name>` depending on `olvin_core`; `pub struct XPlugin; impl Plugin for XPlugin { fn build(&self, app: &mut App) { ... } }`. A plugin registers resources and systems (`add_system(Stage::Update, f)`), events (`add_event::<T>()`) and talks to others only through resources. It adds its own dependencies: `app.add_plugin(OtherPlugin)` (repeats are skipped).
2. In the workspace `Cargo.toml`: `olvin_<name>` in `[workspace.dependencies]`.
3. The `crates/olvin` facade: an optional dependency, a feature (`name = ["dep:olvin_<name>"]`), a re-export module, wiring into `DefaultPlugins` (and `HeadlessPlugins` if it needs no window), types in `prelude`.
4. The hub: a `[[module]]` entry in `hub/olvin_hub/catalog.toml` (`kind = "olvin"`, `feature`, `requires`); a test checks the catalog against the facade's features.
5. Tests in the crate (without GPU if possible), a line in ARCHITECTURE.md and ROADMAP.md, a page in this wiki.

## Add an editor element

1. A crate `editor/olvin_editor_<name>` depending on `olvin_editor`.
2. `impl EditorPlugin { fn build(&self, r: &mut Registry) }`: `add_panel(PanelKind { id, title, factory })` (a panel implements `Panel::ui(ui, &mut EditorState)`), `add_workspace_template`, `add_viewport_extension` (gizmos, tools), `add_component` (a component editor), `add_create_item` (a path `"CSG/Box"` makes a submenu), `add_frame_hook`.
3. A `[[editor_module]]` entry in the hub's `catalog.toml`; the hub writes the plugin into the generated editor `main.rs` itself.
4. A UI test in `apps/editor_dev/tests/` (kittest) and a line in ARCHITECTURE.md.

## Documentation

- Documentation is Russian first with an English copy beside it (`*.en.md`; in the wiki: `ru/` and `en/` with identical file names).
- ARCHITECTURE.md and ROADMAP.md are updated together with code: ticks are set in the same commit.
- The wiki: if you added or changed a feature, update its page; a new Lua `ctx:` method must get into the [reference](../reference/lua-api.md) or CI goes red. Page rules: `docs/wiki/README.md` in the repository.
- Screenshots are rendered windowless (llvmpipe), see `docs/wiki/README.md`.

## See also

- [Architecture](architecture.md)
- [Roadmap](roadmap.md)
- [Documentation languages](language-policy.md)
- [Build errors](../troubleshooting/build-errors.md)

# Olvin: the plan towards a real engine

A document for people and AI agents. The current state is described in [ARCHITECTURE.md](ARCHITECTURE.en.md); this file lists only what still needs doing.

<p align="center"><a href="ROADMAP.md">Русский</a></p>

State of the code as of 2026-11-22 (commit `0e3451e`). ROADMAP ticks are set together with the code; a mismatch is a documentation bug, please open an issue.

Priority labels: **P0** fix now, **P1** needed for the first game, **P2** for "Godot/Unity-lite" level, **P3** for completeness.

Marks: `[x]` done, `[~]` partial (what is left is noted at the end of the item), `[ ]` not started.
Size: **S** up to a week, **M** several weeks, **L** a month or more.

## The main principle

The engine should grow from a game, not from a list of features. First we need a **vertical slice**: one small game (for example, first-person with physics, triggers, sound, one character with an animation and saving) that builds and runs without manual code edits. Each item below is checked against whether this game needs it. Everything the vertical slice does not need goes to P2–P3.

Not to do now: rewriting the ECS (hecs is fine), supporting WebGPU and Android before the first stable desktop build, writing our own scripting language.

---

## Phase 0. Fix what already exists (P0)

Goal: what has been written is reliable, and the documentation does not lie.

- [x] **CI on GitHub Actions**: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, hub and editor builds. (S) — `.github/workflows/ci.yml`: rustfmt, clippy `-D warnings`, `cargo test --workspace` with rendering on llvmpipe, binary builds, `check_unwrap`, a window smoke test under xvfb, `supply-chain` (cargo deny and audit); the runs on GitHub are green. Windows and macOS: see #50.
- [x] **Headless render tests** through llvmpipe (`__EGL_VENDOR_LIBRARY_FILENAMES=.../50_mesa.json WGPU_BACKEND=gl`), so CI does not depend on a GPU. (M) — CI installs Mesa and sets the llvmpipe variables, `olvin_render` (112 tests) runs on the GPU-less runner; the same tests run on real Vulkan locally.
- [x] **Audit of `unwrap()` calls.** Of ~609 calls about 21 are left outside tests, each with an invariant comment or a message saying why it cannot fail; user paths and files return `Result`; `tools/check_unwrap.py` with per-crate allowances stops the count from growing and runs in CI (#56). (M)
- [x] **Tests where there are none at all**: `olvin_physics` (24), `olvin_ecs` (3), `olvin_transform` (5), a full round trip in `olvin_scene::file`, `olvin_script` (33 with Lua), `olvin_window` (6) and a real-window smoke test (hidden window, resize, exit) under xvfb in CI, `olvin_render` (112) on llvmpipe; the editor is covered by kittest suites (#58). (M)
- [x] **Split large files**: `olvin_render/src/core.rs` (2251 lines) into subsystems (device, pipelines, shadows, materials, frame), `editor/olvin_editor/src/panels/viewport.rs` (1743), `olvin_editor_uv/src/panel.rs` (1262), `hub/olvin_hub_gui/src/main.rs` (1187). (M) — `core.rs` (which had grown to 2989) became `core/` + `pipelines.rs`, the largest non-test file is 389 lines; `viewport.rs` → `panels/viewport/` (max 481); `panel.rs` → `panel/` (max 287); the hub's `main.rs` is already 760 lines and was left alone.
- [x] **Sync `ARCHITECTURE.md`**: the "Assets" item of the plan is partly done (glTF, textures, materials); `olvin_hub_gui` is listed on egui 0.35 but `Cargo.toml` has 0.36; check the line "Rust ≥ 1.95 (1.100 is used)". (S). egui 0.36 / egui_dock 0.21 corrected; 1.95 is the MSRV of eframe and egui_dock, and that is now written down; "Assets" in ARCHITECTURE is described as done.
- [x] **Document known limitations**: GL on proprietary NVIDIA under Wayland does not work (a wgpu limitation); written in the README next to the run examples, with the Vulkan workaround. (S)
- [x] **Log and panics**: a single logger with levels and a file, panic capture with a report saved next to the project. (S). `olvin_core::logging`, wired into `sandbox`, `script_demo`, the game template and the editor (`<project>/logs/editor.log`).
- [x] **Check for `TODO`/`todo!`** (there are none now, but make sure none appeared after the latest commits). (S). Checked: no `TODO`/`FIXME`/`todo!`/`unimplemented!` in the code.

---

## Phase 1. Core: what a game cannot be built without (P1)

- [x] **Asset systems with handles and reference counting**: unloading unused assets, async loading, dependencies (mesh → material → texture), rebuilding on hot reload. `olvin_assets` with `.meta` and GUIDs already exists, but there is no runtime manager. (L) — `AssetManager` (handles, refcounts, worker threads, dependencies, hot reload); `olvin::models::ModelPlugin` loads scene models through it, renderer textures still go through `render_assets`.
- [x] **Prefabs**: `olvin_scene` has no notion of a prefab yet (2 mentions). Needed: save an object as a prefab, instantiate it, override fields, update all instances. (L) — scene format v4: `*.prefab.ron` (an ordinary `SceneFile`), `EntityDesc::prefab` with per-field overrides (`overrides`, `removed`), nested prefabs with a cycle guard, `scene_file::instantiate_prefab`; instances follow changes to the prefab. Overrides are per whole field, not sub-field.
- [x] **Versioned scene format**: a version field, migrations when the format changes, a separate test on old files. (M) — `FORMAT_VERSION`, `MIGRATIONS` in `olvin_scene::file`, tests on old files.
- [x] **Scene and state management**: loading a scene in the background, transitions between scenes (menu, game, pause), states with enter and exit. (M) — `olvin_game`: `State<S>` with enter/exit, background `SceneLoader`.
- [x] **Time**: fixed step for physics, time scale, pause, a deterministic step for tests. `Time` exists, but without a fixed step. (S) — `olvin_core::time`: scale, pause, `step`, fixed delta.
- [x] **Events between systems**: a typed event queue (currently, it seems, done manually through resources). (S) — `Events<T>` + `EventCursor`, `App::add_event`.
- [ ] **Parallel scheduler** (item 4 of the old plan, rayon) for systems with no resource overlap. Do it once real systems exist and profiling shows the bottleneck. (M)
- [x] **Profiler**: integration with tracy or puffin, a frame panel in the editor (system times, draw calls, triangles). (M) — puffin behind the `profile` feature (scopes on systems, stages, render, physics, scripts), `OLVIN_PROFILE_HTTP` for `puffin_viewer`; the panel is in the editor (see below).
- [x] **Saving and loading the game** (a layer separate from scenes): component serialisation through a type registry, save slots. (M) — `olvin_game::save`: `SaveRegistry`, `SaveSlots`. Since 2026-11-21 engine state registers itself (`register_engine`: `Transform`, velocities, the controller, `Behaviour::save_state`), keyed by `SceneId` by default; Lua state is not saved.
- [x] **Localisation**: string tables with keys and a fallback language. (S) — `olvin_locale` (feature `locale`): `assets/locales/<language>.ron` tables, a fallback language, `{name}`, plurals by language rules (ru/uk/be/pl/cs/fr/pt/ja and more), the language from `OLVIN_LANG`/`LANG` or the player's choice, `missing_in` for translators; a wiki page. Not yet wired into Lua or `gui`.

---

## Phase 2. Rendering up to the level of a modern engine (P1–P2)

- [~] **Skeletal animation**: glTF skins are currently ignored (`olvin_assets/model.rs`, "skin ignored (static pose)"). Needed: GPU skinning, animation import, clips, playback. (L) [P1] — partial: models and animators in the scene format (v2, `ModelPlugin`), the editor shows the model in its default pose; skinning runs on the CPU, models don't animate in the editor or Play.
- [x] **Blending and animation graph** at runtime: `Animator` currently exists only in the editor. Transitions, layers, parameters, per-frame events. (L) [P2] — `olvin_anim::graph`: transitions, layers, params, clip events.
- [ ] **Morph targets** (blend shapes) for character faces. (M) [P2]
- [~] **Shadows**: currently one shadow map (`shadow_map`). Cascades for directional light, PCF/PCSS, soft shadows for point and spot lights. (M) [P1] — partial: sun cascades (up to 4) with PCF/Soft/PCSS, spot shadows (up to 4) and point shadows (up to 2; 2 and 1 on Compat) through a map atlas with no cube arrays, `Light.shadows`/`shadow_bias`; PCSS is sun-only and Full-only (PCF on Compat), noisy in wide penumbras, `Light.size` has no effect on spot and point lights.
- [ ] **Clustered lighting**: currently up to 16 sources. Grid clusters in the view volume, so that scenes with hundreds of lights do not fall over. (L) [P2]
- [~] **Post-processing**: a separate stack of passes. Bloom, SSAO or GTAO, TAA or FXAA, colour grading through a LUT, height fog. ACES already exists. (M) [P1] — partial: HDR, bloom, ACES, FXAA 3.11 (on), GTAO (off by default), colour grading (lift/gamma/gain, saturation, contrast, LUT), height fog (every tier); all but the fog are Full-only and not visible in the editor viewport (it draws inside egui's single MSAA pass); no TAA.
- [x] **Transparency**: sorting and a separate pass for translucent objects, alpha test mask. (M) [P1] — `BlendMode` with a separate sorted pass, `alpha_cutoff`.
- [ ] **LOD and instancing**: distance LOD, simplification through meshoptimizer (item 3 of the old plan). Compute culling and indirect draw for the Full tier. (L) [P2]
- [ ] **Reflections and environment**: IBL from HDR (HDR/EXR loading already exists, a convolution into a cubemap is needed), reflection probes. (L) [P2]
- [~] **Particles**: CPU particles first, GPU particles later. Emitters with parameters that are edited in the editor. (L) [P1] — partial: CPU `ParticleEmitter`; not in the scene format or editor, no GPU particles.
- [ ] **Decals** and **surfaces with several material layers**. (M) [P3]
- [ ] **Terrain**: a height map, texture layers, chunk LOD, brushes in the editor. (L) [P2]
- [~] **2D pipeline**: sprites, atlases, tilemaps, text. Needed both for the in-game UI and for 2D games (item 7 of the old plan). (L) [P1 for UI] — partial: sprites, atlases, tilemaps, `Camera2d`, text (`olvin_text`: Inter with Cyrillic, a glyph atlas); no wrapping or clipping.
- [~] **In-game UI at runtime**: currently egui works only in the editor. Needed: buttons, text, layout, gamepad focus. Separate from egui, so that a game does not pull in editor dependencies. (L) [P1] — partial: `olvin_gui` is drawn over the frame (`GuiRenderPlugin`, an overlay in `olvin_render`, works on Compat), buttons and text, gamepad focus, re-exported by `olvin`; no image nodes, text fields, sliders or scrolling, and the UI doesn't swallow input (a button click reaches the game as `Fire`).
- [~] **Shader graph**, or at least shader editing with hot reload (the IDE already has it; connect it to rendering). (L) [P3] — partial: shader hot reload is wired (`Shaders::set`); no graph.

---

## Phase 3. Gameplay and systems (P1–P2)

- [ ] **Navigation (navmesh)**: baking from geometry, A* or funnel, agents that steer around obstacles. Nothing mentions it yet. (L) [P2]
- [~] **Physics**: joints, collision layers and masks, sweep and shape-cast queries, continuous collision detection for fast bodies; the character already exists. (M) [P1] — partial: `spherecast`, `overlap_sphere`, CCD, the controller pushes bodies and emits `ControllerHit`, `Physics::teleport`, static bodies follow their `Transform`; no joints, layers/masks or general shape cast.
- [x] **Audio**: mixing with buses and effects (reverb, equaliser), occlusion through obstacles, streaming of long files, source priority. Currently only kira with 3D and music. (M) [P1] — buses with EQ and a reverb send, occlusion, priorities, streaming.
- [~] **Scripts in Lua 5.4 (`mlua`, a `lua` module in the hub)**: the decision is made (fast, familiar, easy to embed). Behaviours are written as Lua tables with the same callbacks as `Behaviour`: `start`, `update`, `fixed_update`, collisions and triggers, `on_destroy`. None of this is implemented yet. (L) [P1] — partial: the runtime with sound, other entities, messages and the `*_xyz` fast path, a `lua` module in the hub catalog, the slice runs its door and plate in Lua; the editor doesn't load Lua, Lua state isn't saved.
  - [x] An optional `lua` feature in `olvin_script`, built with `vendored`, so that C code is not pulled in unless needed. — `lua = ["dep:mlua"]`, `lua54` + `vendored`.
  - [x] `ScriptCtx` bindings: input actions, `Transform`, `Rigidbody` (after the physics force API), `raycast`. — input, `Transform`, forces and velocity, `raycast`.
  - [~] Loading `*.lua` from the project, and hot reload without restarting the editor. — partial: `LuaScripts::load_dir`/`poll` with tests; the editor doesn't load Lua.
  - [x] Sandbox: a memory limit and an instruction-count limit per call. Script errors do not bring the frame down; they go to the console. — `LuaLimits`: memory and instruction limits, trimmed libraries, errors go to the log.
  - [~] Editor parameters (`FromParams`) for Lua scripts. — partial: the `params` schema is parsed and registered; it won't show in the inspector until the editor loads Lua.
  - [x] Benchmark: 5 000 `update` calls per frame, Lua against Rust behaviours. Record the numbers here. — `cargo bench -p olvin_script --features lua --bench lua_vs_rust`. Measured (i5-11400F, release, 5,000 entities, median of 300 frames): empty `update` — Rust 0.05 ms/frame (10 ns/call), Lua 1.2 ms (≈240 ns); "walker" (`axis2d` + `dt` + `translate`) — Rust 0.19 ms (38 ns), Lua 11.7 ms (≈2.3 µs, 70% of a 16.6 ms frame). The cost is vectors passed as tables across the Rust↔Lua boundary (`translate(vec3())` ≈1.6 µs, `axis2d` ≈0.95 µs) and garbage collection, not the interpreter or the instruction-limit hook (3–7%).
  - [x] Make the Rust↔Lua boundary cheaper: vectors as userdata or plain numbers (`translate_xyz`, `axis2d` returning two values), cache the `x/y/z` key strings and callback names. Target: the walker at 0.5 µs or less. (S) — the `*_xyz`/`axis2d_xy` fast path: the walker at 0.42 µs (was 2.3 µs), the old table API down to 1.1 µs (i5-11400F, measured under load).
  - [ ] Web: decide what happens to scripts in the browser. Does not block the native version.
  - [x] ADR (`docs/adr/`): why Lua 5.4 and not Rhai, Luau, JS, WASM, Python, C#. — `docs/adr/0001-lua-scripting.en.md`.
  - [ ] The gameplay node editor compiles to Lua. Shader nodes compile to WGSL (a separate item).
  - [x] Hub: the `lua` module appears in the catalogue only once the crate is ready. — `hub/olvin_hub/catalog.toml`: `lua` and `game` modules.
- [ ] **Timeline and cutscenes** (sequencer): tracks for animation, sound, camera, events. (L) [P2]
- [x] **Input**: rebinding keys at runtime, saving bindings, gamepad prompts, vibration. Action maps already exist. (S) [P1] — `rebind.rs` with autosave, `hints.rs`, `Haptics` via gilrs.
- [ ] **Networking**: transport (UDP through `quinn` or `renet`), component replication, client-side prediction, interpolation. A separate `olvin_net` crate. (XL) [P3]
- [x] **Timer systems**: timer events, coroutines or async tasks for delays in behaviours. (S) [P2] — `olvin_core::Timers` (`after`/`every`, scaled and `_unscaled`, `cancel`, the `TimerFired` event), in Lua `ctx:after`/`every`/`cancel_timer` with `on_message`, limits of 32 per object and 4096 in all; a "Timers" section in the wiki. Coroutines (`wait`) for scripts are left out: timers cover delays without them.

---

## Phase 4. Build and platforms (P1–P2)

- [x] **Game build command**: `olvin-hub build --release` builds the game, copies the assets, packs them and produces a folder or an archive. There is no build command at all yet. (M) [P1] — `olvin-hub build --release [--archive]`, copies assets, tar.gz/zip.
- [ ] **Asset packing**: texture compression (KTX2 with BC7 or ASTC), mesh optimisation, one archive file with a table of contents. (L) [P2]
- [~] **Desktop**: Windows and macOS in CI with build artefacts, not only Linux. (M) [P1] — partial: a `desktop` job in `ci.yml` for Windows and macOS (tests, release builds of six binaries, zip artefacts), branch `feat-ci`; not yet run on real runners.
- [ ] **Web**: wasm and WebGL2 (the architecture plans for it, but there is no build yet): `web-time` instead of `std::time`, asset loading over HTTP. (L) [P2]
- [ ] **Android**: recreating the surface on suspend and resume, touch input, a build through cargo-ndk. (XL) [P3]
- [ ] **Engine auto-update** and project version compatibility (migrations when Olvin is updated). (M) [P3]

---

## Phase 5. Editor (P1–P2)

- [x] **Undo and redo for all operations**: currently present in part of the editor. Check that dragging, the inspector, Edit Mode, CSG and UV are covered. (M) [P1] — verified by editor tests: hierarchy drag, gizmos, the inspector, materials (files roll back with the scene), Edit Mode, CSG, UV, animation; history is frozen during Play. Asset file operations (rename, move, delete) are not undoable.
- [x] **Play Mode that does not lose edits**: what happens to the scene when playback stops matters. A snapshot of the scene at start and a rollback on stop. (M) [P1] — `start_play`/`stop_play` save and restore the scene (`PlaySession`).
- [x] **Prefabs in the editor**: creating, overriding, applying back. Depends on phase 1. (M) [P1] — "Create Prefab" (menu and hierarchy), dragging from the Assets panel into the scene and hierarchy, a Prefab block in the inspector (Open, Select, Revert All, Apply All, Unpack, overridden fields with revert), undo, instances follow the asset, moving and deleting assets keeps references. Viewport picking selects the node, not the instance root.
- [~] **Material editor**: the `*.mat.ron` contracts exist (M1), a visual editor with a preview is needed. (M) [P2] — partial: in-inspector material editor with presets; no preview.
- [x] **Profiler and statistics panel** (see phase 1). (S) [P2] — the Profiler panel: frame graph, draw calls, instances, culled, triangles, a systems table (`olvin_editor_profiler`).
- [x] **Animation editor**: connect Animator to the runtime after phase 2. (L) [P2] — a Blender-like animator: F-curves with Auto Clamped Bezier (Blender's maths ported), Timeline, Dope Sheet, Graph Editor, I/Alt+I and key colours in the inspector; scene format v3 (`SceneFile::animation`), played in games and Play by `olvin_anim::ActionPlayer`. Missing: auto-keying, markers, Normalize, animating anything but Transform; rotation order is YXZ, not XYZ.
- [~] **Search in the scene and assets**, filters in the console. (S) [P2] — partial: filters in the assets panel and the console; no hierarchy search.
- [ ] **Multi-scene**: open several scenes at once and switch between them. (M) [P3]
- [ ] **Configurable hotkeys** (currently hard-coded). (S) [P3]

---

## Phase 6. Ecosystem and documentation (P2–P3)

- [ ] **Documentation**: an mdbook. A "first game in an hour" tutorial, a description of each subsystem, a table of "what lives where". (M) [P1]
- [ ] **Examples**: one per subsystem (physics, sound, animation, UI, networking), each runnable with one command. Currently only `sandbox` and `script_demo`. (M) [P2]
- [ ] **Versioning and changelog**: semver, `CHANGELOG.md`, release tags. The version is 0.1.0 with no release history. (S) [P2]
- [ ] **Publishing crates** on crates.io once the API is stable. (M) [P3]
- [ ] **Contributor guide**: how to run tests, how to add a new plugin, code style, how commits are organised. (S) [P2]
- [ ] **Benchmarks**: a separate set with the frame cost on reference scenes, results recorded in CI so regressions are visible. (M) [P2]

---

## Phase 7. Growth: library, wiki, collaboration (team ideas)

Suggestions from users and the NLPA team. The first two are being built now, the rest are dreams for later; keep them in mind while designing (stable `SceneId`s, a serialisable scene, events through `Events<T>`).

- [ ] **Preset library** (like the Unity Asset Store, but built in and offline): packs in a `library/` folder with a `pack.ron` manifest; a "Library" panel in the editor (search, categories, previews, "Import") copies files into the project's `assets/`, keeping GUIDs and dependencies; a `library` module in the hub. First packs: character controllers (first person, third person, top down) with Lua scripts, gameplay scripts (door, pickup, health and damage, checkpoint, moving platform, patrol and chase, spawner, camera shake), ready shaders (toon, outline, hologram, dissolve, water), materials, picture "filters" (looks). (L) [P1 for the first release]
- [ ] **Wiki** in the spirit of the Arch Wiki and Hyprland Wiki: one page per topic, cross-links, search, FAQ and troubleshooting, a Lua API reference, a "first game in an hour" guide; Russian is the primary language with an English copy. Book engine: mdBook, published to GitHub Pages. (M) [P1]
- [ ] **Online package registry**: browse and install library packs over the network, versions and dependencies, signatures, ratings. After the local library settles. (L) [P3]
- [ ] **Collaborative editing like Roblox Studio**: several people edit one scene in real time, teammates' cursors and selections, per-object locks, change history. The groundwork exists: scene operations are `SceneFile` snapshots with stable `SceneId`s; what is missing is network synchronisation of operations (CRDT or operational transforms) and a session server. (XL) [P3]
- [ ] **Easy multiplayer**: ready "network object" and "owner" components, replication through `Events<T>` and the save registry, lobbies and joining by room code, presets for typical games (co-op, arena). Tied to the "Networking" item of phase 3 (`olvin_net`). (XL) [P3]

## Phase 8. Security and build protection (P0–P3)

Goal: cover the whole threat model, not one case at a time. Assets: money and access, game balance, assets and logic, player data, secrets, build integrity. A client that is in the player's hands can never be fully protected, so each item states what it gives: **protection** (the threat is closed), **deterrence** (raises the cost but does not lock anything), or **signal** (tampering becomes visible). Codes (B1, L2, …) are for linking from issues and commits. Out of scope: physical access to the player's machine, legal measures (EULA, store), compromise of the developer's machine (needs 2FA and signing keys kept off the working machine).

### Build and distribution

- [ ] **Strip in release (B1)**: `strip` in `[profile.release]` or a flag for `olvin-hub build`; debug symbols in a separate file so crash reports stay readable. Gives deterrence. (S) [P0]
- [ ] **Hash manifest and archive signature (B2)**: SHA-256 of every build file, a signature (minisign or Ed25519), verified at startup. Gives protection against a swapped binary and a signal on patching. (M) [P0]
- [ ] **Supply chain (B4)**: `Cargo.lock` in releases, `cargo audit` and `cargo deny` in CI. Gives protection against a swapped dependency. (S) [P0]
- [ ] **Build paths and user name in the binary (B3)**: `--remap-path-prefix` in the build, a string check on the finished binary in CI. (S) [P1]
- [ ] **Warning about third-party `build.rs` (B5)**: the hub warns when a crates.io dependency is added, because its `build.rs` runs on the developer's machine; optionally build in a container. (M) [P2]
- [ ] **Signed `catalog.toml` (B6)**: the catalog is built into the binary now, but it needs a signature once it becomes downloadable. (S) [P2]

### Binary

- [ ] **String encryption (E1)**: sensitive literals (keys, addresses, check messages) are not plain text in `.rodata`. Gives deterrence. (S) [P1]
- [ ] **Integrity check at startup (E2)**: hash comparison, and a signal when checks are patched. A patch is one jump, so this is only a signal. (S) [P2]
- [ ] **Wiping keys from memory (E3)**: `zeroize` for keys and passwords, keep their lifetime short. Gives protection against a simple memory dump. (S) [P1]
- [ ] **Audit of `unsafe` (E5)**: `olvin_script/src/lua/api.rs` (`with` and `lend`, two blocks with `transmute` and `NonNull`); Miri in CI, fuzzing of callback paths. Gives protection against undefined behaviour. (M) [P1]
- [ ] **Debugger detection (E4)**: deterrence, not protection. (S) [P3]

### Assets

- [ ] **Encrypted asset packs (A1)**: AES-GCM, a hash of each file in the pack's table of contents. Gives protection against reading and tampering with scenes, prefabs and shaders. (L) [P1]
- [ ] **Key outside the binary (A2)**: the key is derived from an online token or the licence; offline it stays deterrence. A built-in key is not protection. (M) [P1]
- [ ] **On-demand loading (A3)**: decrypt as needed instead of holding the whole pack in memory. Gives deterrence against a memory dump. (M) [P2]

### Lua scripts

- [x] **Close `load` and binary chunks (L2)**: remove `load`, `loadstring`, `collectgarbage`, or restrict `mode = "t"` in `crates/olvin_script/src/lua/mod.rs`; a test that a binary chunk is rejected. Gives protection against escaping the sandbox through bytecode. (S) [P0] — done: `load` is forced to text (the caller's mode is ignored, `env` works), `string.dump` is removed, `collectgarbage` allows only `collect` and `count`; the test `load_is_text_only_and_bytecode_sources_are_gone` was checked with a negative control (without the sandbox 26 of 63 pass). Callback fuzzing (L3, L4) remains.
- [ ] **Encrypt or move logic (L1)**: scripts are encrypted, or critical parts (checks, economy) move to Rust or the server. Gives deterrence. (M) [P1]
- [ ] **Library pack verification (L5)**: signed `pack.ron` and files, review before install, a warning about a pack's Lua scripts. Gives protection against a malicious pack. (M) [P1]
- [ ] **Lua limits and fuzzing (L3, L4)**: fuzz the callbacks, check that `string.dump` and neighbouring functions give no way out of the sandbox. Gives protection against resource exhaustion and escape. (M) [P2]

### Saves and player data

- [x] **HMAC-signed saves (S1)**: the key is built in, so this is deterrence. Gives a signal that a save was edited. (S) [P1] — done: `SaveSigner` and the `Warn`/`Enforce`/`Strict` policies, the signature as the last comment line of the same file (old saves load), `SaveSlots::with_signing`, `SavePlugin::signed`, `load_checked` returns a `SaveStatus`; the game has to embed its own key (not switched on in the slice yet). S2 (binding to an account) remains.
- [ ] **Save binding (S2)**: to an account or machine, once there is online play. (M) [P2]
- [ ] **Redacting paths and names in logs and crash files (S3)**: in `logging.rs` the backtrace and log may contain the path to the home folder. Reports are sent only with explicit consent. Gives protection of player data. (S) [P1]

### Online (phase 3, `olvin_net`)

- [ ] **Server is authoritative (N1)**: the client only requests actions, the server computes the state. Gives protection against cheating. (L) [P0 for co-op]
- [ ] **TLS and signed requests (N2)**: the channel is encrypted and signed. Gives protection against interception. (M) [P0 for co-op]
- [ ] **Nonces and timestamps (N3)**: protection against replayed requests. (S) [P1]
- [ ] **Server secrets stay off the client (N4)**: no server key in any build, checked in CI. Gives protection against leaks. (S) [P0 for co-op]

### Olvin's own tools

- [ ] **Warning when opening a foreign project (T1)**: the editor and hub run `cargo` on the project, so they ask first whether the project is trusted. (S) [P2]
- [ ] **`OLVIN_LIBRARY` as trusted input (T2)**: document that the library path is trusted. (S) [P3]

## Phase 9. The 2D engine and a separate 2D editor

Owner's decision (2026-11-23): first **finish the 3D editor**, then take on 2D. The engine is built from separate elements, so **the 2D editor is a separate editor element** (`olvin_editor_2d`, its own workspaces and window theme), not a mode of the 3D viewport. The feel to aim for is **Adobe Flash/Animate and Clickteam Fusion**: a friendly interface for people who are not programmers, with a frame timeline, a symbol library and a "condition -> action" event editor; a game is built with the mouse.

What exists: sprites, atlases, tilemaps, `Camera2d`, text (`olvin_render::sprite`, `olvin_text`). What does not: 2D components in the scene format and the inspector, 2D physics, 2D editors, 2D animation, library packs.

- [ ] **Finish the 3D editor (prerequisite)**: material previews (#41), several scenes at once (#44), configurable hotkeys (#45), skinned animation in the editor (#8), picking the prefab root in the viewport, undo for asset file operations, post-processing effects in the editor viewport (visible only in the game today), a shader graph editor (#22), terrain (#21). (L) [P1]
- [ ] **2D core**: `Sprite`, `Tilemap`, `SpriteAnimator` components in the scene format (serde defaults), `Camera2d` with pixel-perfect mode, layers and sorting, Lua bindings (`ctx:` for sprites and animation). (M) [P1]
- [ ] **2D physics** on `rapier2d`: `Rigidbody2D`, `Collider2D` (circle, box, capsule, polygon, tilemap colliders), triggers and events, a character controller for platformers and top-down; shares the event machinery with 3D (`CollisionEvents`). (M) [P1]
- [ ] **The 2D editor as a separate element**: an orthographic scene with a pixel grid, snapping and guides; a layer hierarchy; an inspector for 2D components; a symbol library (reusable clips and prefabs, like Flash symbols). (L) [P1]
- [ ] **Sprite editor**: slicing a sheet by grid or by outline, pivot, 9-slice, PNG/GIF/Aseprite import, batch processing. (M)
- [ ] **Tilemap editor**: brushes, fill, layers, auto-tiling (neighbour rules), colliders from tiles, obstacle and trigger layers. (L)
- [ ] **A 2D animation editor with a timeline** (like Flash): frames and keyframes, layers, tweening, frame-by-frame animation, sound on the timeline, frame events, nested clips, onion skin; shares the format with the 3D animator where they overlap (`Action`). (L)
- [ ] **An event editor** (like Clickteam Fusion): a "conditions -> actions" table with no code (key pressed, collision, timer, variable; move, play a sound, create an object, change scene), compiled to Lua and to nodes; for people who do not write code. Related to the "gameplay node editor" item of phase 3. (L)
- [ ] **2D library packs**: a platformer, a top-down (Zelda-like), a shoot-em-up, a grid puzzle; ready enemies, pickups, checkpoints, a camera with bounds; a "2D game" project template in the hub. (M)
- [ ] **2D effects**: 2D lights and shadows, sprite normals, a pixel mode and palettes, parallax, 2D particles, looks for pixel art. (M) [P2]

## Phase 10. Interface polish: animations, customisation, settings

The owner's wishes (2026-11-24). The common line: the interface should feel alive and pleasant but stay controllable; everything that moves respects `reduce_motion` (it already exists in `olvin_ui`).

- [ ] **More animations and customisation of the engine**: animated transitions between panels, workspaces and dialogs, smooth show and hide, animated lists and drag and drop; wider customisation (themes, accents, interface density, scale, fonts, panel layouts, user themes as a file). (M) [P2]
- [ ] **A flexible config and detailed UI settings**: one config (RON or TOML) with layers "defaults -> user -> project"; every option has a description, a default and validation; the settings window covers every option, with search, reset per option and per group, profiles, import and export; whatever can be changed in the file can be changed in the interface, and the other way round. (M) [P1]
- [ ] **The editor right inside the IDE** (to clarify with the owner, two readings are possible): (a) the engine's own IDE as a full workplace: the scene view, Play, the inspector and the console sit as panels next to the code in one workspace, no window switching; (b) plugins for external IDEs (VS Code, RustRover, Zed) with a scene view and a game launcher. Start with (a), since the editor already has a Code workspace. (L) [P2]
- [ ] **Telegram-style micro-animations**: hovering or pressing a button or icon makes it come alive (animated icons: frame-based vector animation, a Lottie variant or our own format), spring transitions, click feedback; a set of ready animated icons for the editor and the hub; switched off by `reduce_motion`. (M) [P2]
- [ ] **A startup animation** (the editor and hub splash): Neat animation for the logo and module loading—not overdone, though still not as beautiful as Yulia herself♥) <written by DanyaDel, Nov 24, 2026> ; skippable, can be turned off in settings, does not lengthen startup (shown while loading really happens). (S) [P3]
- [ ] **Process animations**: compilation (a progress indicator from `cargo` output: "compiling N of M crates", then success or failure with an animation), saving, asset import, installing modules and library packs, building a game in the hub, search, decryption; one shared "process indicator" component with the states running, done and failed, in the IDE, the editor and the hub. (M) [P2]

## Order of work (what to do first)

1. **Phase 0** in full. Without CI and tests, everything after will break unnoticed.
2. **Vertical slice**: pick one small game and build it on the current engine. Write down what was missing. That list is the real set of priorities. — done: `apps/vertical_slice` (first person, physics, a trigger with a door, 3D sound and music, Lua, an animated glTF robot, pause, F5/F9 saves), built by `olvin-hub build --release`. Gaps in detail in [`apps/vertical_slice/GAPS.md`](apps/vertical_slice/GAPS.md) (Russian), by importance (all closed on 2026-11-21, leftovers are in the phase items above):
   - [x] On-screen text and game UI: fonts and drawing the `olvin_gui` `DrawList` (L) — phase 2, UI and 2D.
   - [x] Models and animated characters in the scene format, no hand-written code (M) — phase 2, skeletal animation.
   - [x] Project root as a resource: asset paths not relative to the working directory (audio, input, logs, hub template) (S).
   - [x] Saving engine state: `Transform`, physics, behaviour state (M) — phase 1, saves.
   - [x] Scene object id after spawning and game components in scenes (S) — next to prefabs.
   - [x] Lua: sound, resources, access to other objects (M) — phase 3, Lua.
   - [x] Hub: `game` and `lua` catalog modules, `olvin` re-exports `olvin_assets`/`olvin_anim` (S).
   - [x] Character controller pushes dynamic bodies, hit callback (S) — phase 3, physics.
   - [x] Pause stops audio (S).
   - [x] A headless plugin set without window and render, `AnimPlugin` without the renderer (S).
   - [x] Background scene loading sets up materials from `assets/` (S).
   - [x] Static colliders follow their `Transform` (S).
   - [x] Hub build: `dist/` outside the sources or in the template's `.gitignore` (S).
3. After the slice, close the blockers from phases 1 and 2 (skeletal animation, UI, particles, post-processing, prefabs).
4. Phase 4 (game build) right after the first slice, otherwise nobody can be shown the game. The P0 items of phase 8 are done together with it, before the first public build.
5. Phases 3, 5 and 6 in parallel, as the need appears.

## Definition of done

- The vertical slice builds with `olvin-hub build --release` and runs without the editor.
- `cargo test --workspace` and CI are green on Linux, Windows and macOS.
- Each subsystem in phases 1–3 has at least one test and one example.
- `ARCHITECTURE.md` and this file have no out-of-date items: the ticks are set together with the code.

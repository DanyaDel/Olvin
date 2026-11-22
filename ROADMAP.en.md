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

- [x] **CI on GitHub Actions**: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, build of the hub and the editor. (S). `.github/workflows/ci.yml`; everything is green locally, the first run on a GitHub runner has not happened yet.
- [ ] **Headless render tests** through llvmpipe (`__EGL_VENDOR_LIBRARY_FILENAMES=.../50_mesa.json WGPU_BACKEND=gl`), so that CI does not depend on a GPU. (M). Partly done: CI sets these variables and installs Mesa, and the 27 `olvin_render` tests pass locally on llvmpipe; still to do: confirm on a runner and add a window smoke test.
- [~] **Audit of `unwrap()` calls.** Of 609 calls in the code about 90 are outside tests; most sit in `#[cfg(test)]` and `tests.rs`. User paths and files now return `Result`/`ok_or` (hub and editor configs, shaders, `reparent`, the Duplicate/Delete menu, the confirm dialog), and `olvin_assets` and `olvin_scene::file` have no panics outside tests. What remains: invariants (`expect` on piped stdout, `MAX_LIGHTS > 0`, texture size, resource registration, the asset loader thread) and the hub project templates. (M)
- [ ] **Tests where there are none**: `olvin_physics` (colliders, raycast, trigger events), `olvin_ecs`, `olvin_transform` (hierarchy propagation), `olvin_scene::file` (save round-trip), `olvin_script` already has tests, `olvin_window`/`olvin_render` smoke tests. (M). Partly done: `olvin_ecs` (3 tests), `olvin_transform` (+3), `olvin_scene::file` (+3, file round-trip), `olvin_physics` (+3: raycast filters, fixed step, `CollisionEvents::of`), `olvin_core::logging` (3). No window smoke test (it needs a display).
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
- [ ] **Localisation**: string tables with keys and a fallback language. (S)

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
- [ ] **Timer systems**: timer events, coroutines or async tasks for delays in behaviours. (S) [P2]

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
4. Phase 4 (game build) right after the first slice, otherwise nobody can be shown the game.
5. Phases 3, 5 and 6 in parallel, as the need appears.

## Definition of done

- The vertical slice builds with `olvin-hub build --release` and runs without the editor.
- `cargo test --workspace` and CI are green on Linux, Windows and macOS.
- Each subsystem in phases 1–3 has at least one test and one example.
- `ARCHITECTURE.md` and this file have no out-of-date items: the ticks are set together with the code.

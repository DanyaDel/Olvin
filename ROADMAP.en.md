# Olvin: the plan towards a real engine

A document for people and AI agents. The current state is described in [ARCHITECTURE.md](ARCHITECTURE.en.md); this file lists only what still needs doing.

<p align="center"><a href="ROADMAP.md">Русский</a></p>

Priority labels: **P0** fix now, **P1** needed for the first game, **P2** for "Godot/Unity-lite" level, **P3** for completeness.
Size: **S** up to a week, **M** several weeks, **L** a month or more.

## The main principle

The engine should grow from a game, not from a list of features. First we need a **vertical slice**: one small game (for example, first-person with physics, triggers, sound, one character with an animation and saving) that builds and runs without manual code edits. Each item below is checked against whether this game needs it. Everything the vertical slice does not need goes to P2–P3.

Not to do now: rewriting the ECS (hecs is fine), supporting WebGPU and Android before the first stable desktop build, writing our own scripting language.

---

## Phase 0. Fix what already exists (P0)

Goal: what has been written is reliable, and the documentation does not lie.

- [x] **CI on GitHub Actions**: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, build of the hub and the editor. (S). `.github/workflows/ci.yml`; everything is green locally, the first run on a GitHub runner has not happened yet.
- [ ] **Headless render tests** through llvmpipe (`__EGL_VENDOR_LIBRARY_FILENAMES=.../50_mesa.json WGPU_BACKEND=gl`), so that CI does not depend on a GPU. (M). Partly done: CI sets these variables and installs Mesa, and the 27 `olvin_render` tests pass locally on llvmpipe; still to do: confirm on a runner and add a window smoke test.
- [ ] **Audit of the 344 `unwrap()` calls.** Keep those where a panic is justified (initialisation, invariants), replace the rest with `Result` or `anyhow` with context. Priority: `olvin_assets`, `olvin_scene::file`, `hub/*`, `editor/*` (user paths and files). (M)
- [ ] **Tests where there are none**: `olvin_physics` (colliders, raycast, trigger events), `olvin_ecs`, `olvin_transform` (hierarchy propagation), `olvin_scene::file` (save round-trip), `olvin_script` already has tests, `olvin_window`/`olvin_render` smoke tests. (M). Partly done: `olvin_ecs` (3 tests), `olvin_transform` (+3), `olvin_scene::file` (+3, file round-trip), `olvin_physics` (+3: raycast filters, fixed step, `CollisionEvents::of`), `olvin_core::logging` (3). No window smoke test (it needs a display).
- [ ] **Split large files**: `olvin_render/src/core.rs` (2251 lines) into subsystems (device, pipelines, shadows, materials, frame), `editor/olvin_editor/src/panels/viewport.rs` (1743), `olvin_editor_uv/src/panel.rs` (1262), `hub/olvin_hub_gui/src/main.rs` (1187). (M)
- [x] **Sync `ARCHITECTURE.md`**: the "Assets" item of the plan is partly done (glTF, textures, materials); `olvin_hub_gui` is listed on egui 0.35 but `Cargo.toml` has 0.36; check the line "Rust ≥ 1.95 (1.100 is used)". (S). egui 0.36 / egui_dock 0.21 corrected; 1.95 is the MSRV of eframe and egui_dock, and that is now written down; "Assets" in ARCHITECTURE is described as done.
- [x] **Document known limitations**: GL on proprietary NVIDIA under Wayland does not work (a wgpu limitation); written in the README next to the run examples, with the Vulkan workaround. (S)
- [x] **Log and panics**: a single logger with levels and a file, panic capture with a report saved next to the project. (S). `olvin_core::logging`, wired into `sandbox`, `script_demo`, the game template and the editor (`<project>/logs/editor.log`).
- [x] **Check for `TODO`/`todo!`** (there are none now, but make sure none appeared after the latest commits). (S). Checked: no `TODO`/`FIXME`/`todo!`/`unimplemented!` in the code.

---

## Phase 1. Core: what a game cannot be built without (P1)

- [ ] **Asset systems with handles and reference counting**: unloading unused assets, async loading, dependencies (mesh → material → texture), rebuilding on hot reload. `olvin_assets` with `.meta` and GUIDs already exists, but there is no runtime manager. (L)
- [ ] **Prefabs**: `olvin_scene` has no notion of a prefab yet (2 mentions). Needed: save an object as a prefab, instantiate it, override fields, update all instances. (L)
- [ ] **Versioned scene format**: a version field, migrations when the format changes, a separate test on old files. (M)
- [ ] **Scene and state management**: loading a scene in the background, transitions between scenes (menu, game, pause), states with enter and exit. (M)
- [ ] **Time**: fixed step for physics, time scale, pause, a deterministic step for tests. `Time` exists, but without a fixed step. (S)
- [ ] **Events between systems**: a typed event queue (currently, it seems, done manually through resources). (S)
- [ ] **Parallel scheduler** (item 4 of the old plan, rayon) for systems with no resource overlap. Do it once real systems exist and profiling shows the bottleneck. (M)
- [ ] **Profiler**: integration with tracy or puffin, a frame panel in the editor (system times, draw calls, triangles). (M)
- [ ] **Saving and loading the game** (a layer separate from scenes): component serialisation through a type registry, save slots. (M)
- [ ] **Localisation**: string tables with keys and a fallback language. (S)

---

## Phase 2. Rendering up to the level of a modern engine (P1–P2)

- [ ] **Skeletal animation**: glTF skins are currently ignored (`olvin_assets/model.rs`, "skin ignored (static pose)"). Needed: GPU skinning, animation import, clips, playback. (L) [P1]
- [ ] **Blending and animation graph** at runtime: `Animator` currently exists only in the editor. Transitions, layers, parameters, per-frame events. (L) [P2]
- [ ] **Morph targets** (blend shapes) for character faces. (M) [P2]
- [ ] **Shadows**: currently one shadow map (`shadow_map`). Cascades for directional light, PCF/PCSS, soft shadows for point and spot lights. (M) [P1]
- [ ] **Clustered lighting**: currently up to 16 sources. Grid clusters in the view volume, so that scenes with hundreds of lights do not fall over. (L) [P2]
- [ ] **Post-processing**: a separate stack of passes. Bloom, SSAO or GTAO, TAA or FXAA, colour grading through a LUT, height fog. ACES already exists. (M) [P1]
- [ ] **Transparency**: sorting and a separate pass for translucent objects, alpha test mask. (M) [P1]
- [ ] **LOD and instancing**: distance LOD, simplification through meshoptimizer (item 3 of the old plan). Compute culling and indirect draw for the Full tier. (L) [P2]
- [ ] **Reflections and environment**: IBL from HDR (HDR/EXR loading already exists, a convolution into a cubemap is needed), reflection probes. (L) [P2]
- [ ] **Particles**: CPU particles first, GPU particles later. Emitters with parameters that are edited in the editor. (L) [P1]
- [ ] **Decals** and **surfaces with several material layers**. (M) [P3]
- [ ] **Terrain**: a height map, texture layers, chunk LOD, brushes in the editor. (L) [P2]
- [ ] **2D pipeline**: sprites, atlases, tilemaps, text. Needed both for the in-game UI and for 2D games (item 7 of the old plan). (L) [P1 for UI]
- [ ] **In-game UI at runtime**: currently egui works only in the editor. Needed: buttons, text, layout, gamepad focus. Separate from egui, so that a game does not pull in editor dependencies. (L) [P1]
- [ ] **Shader graph**, or at least shader editing with hot reload (the IDE already has it; connect it to rendering). (L) [P3]

---

## Phase 3. Gameplay and systems (P1–P2)

- [ ] **Navigation (navmesh)**: baking from geometry, A* or funnel, agents that steer around obstacles. Nothing mentions it yet. (L) [P2]
- [ ] **Physics**: joints, collision layers and masks, sweep and shape-cast queries, continuous collision detection for fast bodies; the character already exists. (M) [P1]
- [ ] **Audio**: mixing with buses and effects (reverb, equaliser), occlusion through obstacles, streaming of long files, source priority. Currently only kira with 3D and music. (M) [P1]
- [ ] **Scripts in Lua 5.4 (`mlua`, a `lua` module in the hub)**: the decision is made (fast, familiar, easy to embed). Behaviours are written as Lua tables with the same callbacks as `Behaviour`: `start`, `update`, `fixed_update`, collisions and triggers, `on_destroy`. None of this is implemented yet. (L) [P1]
  - [ ] An optional `lua` feature in `olvin_script`, built with `vendored`, so that C code is not pulled in unless needed.
  - [ ] `ScriptCtx` bindings: input actions, `Transform`, `Rigidbody` (after the physics force API), `raycast`.
  - [ ] Loading `*.lua` from the project, and hot reload without restarting the editor.
  - [ ] Sandbox: a memory limit and an instruction-count limit per call. Script errors do not bring the frame down; they go to the console.
  - [ ] Editor parameters (`FromParams`) for Lua scripts.
  - [ ] Benchmark: 5 000 `update` calls per frame, Lua against Rust behaviours. Record the numbers here.
  - [ ] Web: decide what happens to scripts in the browser. Does not block the native version.
  - [ ] ADR (`docs/adr/`): why Lua 5.4 and not Rhai, Luau, JS, WASM, Python, C#.
  - [ ] The gameplay node editor compiles to Lua. Shader nodes compile to WGSL (a separate item).
  - [ ] Hub: the `lua` module appears in the catalogue only once the crate is ready.
- [ ] **Timeline and cutscenes** (sequencer): tracks for animation, sound, camera, events. (L) [P2]
- [ ] **Input**: rebinding keys at runtime, saving bindings, gamepad prompts, vibration. Action maps already exist. (S) [P1]
- [ ] **Networking**: transport (UDP through `quinn` or `renet`), component replication, client-side prediction, interpolation. A separate `olvin_net` crate. (XL) [P3]
- [ ] **Timer systems**: timer events, coroutines or async tasks for delays in behaviours. (S) [P2]

---

## Phase 4. Build and platforms (P1–P2)

- [ ] **Game build command**: `olvin-hub build --release` builds the game, copies the assets, packs them and produces a folder or an archive. There is no build command at all yet. (M) [P1]
- [ ] **Asset packing**: texture compression (KTX2 with BC7 or ASTC), mesh optimisation, one archive file with a table of contents. (L) [P2]
- [ ] **Desktop**: Windows and macOS in CI with build artefacts, not only Linux. (M) [P1]
- [ ] **Web**: wasm and WebGL2 (the architecture plans for it, but there is no build yet): `web-time` instead of `std::time`, asset loading over HTTP. (L) [P2]
- [ ] **Android**: recreating the surface on suspend and resume, touch input, a build through cargo-ndk. (XL) [P3]
- [ ] **Engine auto-update** and project version compatibility (migrations when Olvin is updated). (M) [P3]

---

## Phase 5. Editor (P1–P2)

- [ ] **Undo and redo for all operations**: currently present in part of the editor. Check that dragging, the inspector, Edit Mode, CSG and UV are covered. (M) [P1]
- [ ] **Play Mode that does not lose edits**: what happens to the scene when playback stops matters. A snapshot of the scene at start and a rollback on stop. (M) [P1]
- [ ] **Prefabs in the editor**: creating, overriding, applying back. Depends on phase 1. (M) [P1]
- [ ] **Material editor**: the `*.mat.ron` contracts exist (M1), a visual editor with a preview is needed. (M) [P2]
- [ ] **Profiler and statistics panel** (see phase 1). (S) [P2]
- [ ] **Animation editor**: connect Animator to the runtime after phase 2. (L) [P2]
- [ ] **Search in the scene and assets**, filters in the console. (S) [P2]
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
2. **Vertical slice**: pick one small game and build it on the current engine. Write down what was missing. That list is the real set of priorities.
3. After the slice, close the blockers from phases 1 and 2 (skeletal animation, UI, particles, post-processing, prefabs).
4. Phase 4 (game build) right after the first slice, otherwise nobody can be shown the game.
5. Phases 3, 5 and 6 in parallel, as the need appears.

## Definition of done

- The vertical slice builds with `olvin-hub build --release` and runs without the editor.
- `cargo test --workspace` and CI are green on Linux, Windows and macOS.
- Each subsystem in phases 1–3 has at least one test and one example.
- `ARCHITECTURE.md` and this file have no out-of-date items: the ticks are set together with the code.

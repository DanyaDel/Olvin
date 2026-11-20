# 0001. Scripting in Lua 5.4 (mlua)

<p align="center"><a href="0001-lua-scripting.md">Русский</a></p>

**Status:** Accepted — 2026-11-19 (commit `eea7d12`, "Script: Lua 5.4 behaviours with hot reload")

## Context

Behaviours in Olvin are written in Rust (`Behaviour` in `olvin_script`). That is fast, but every change needs a rebuild, and a modder needs the whole Rust toolchain. We need a second language: edits without restarting the editor, scripts loaded from the project folder, a script error does not take the game down.

What constrains the choice:

- **Performance and weak hardware.** The engine's minimum tier is WebGL2 / GLES 3.0. The scripting layer must not be the heaviest part of a game on an old laptop or phone.
- **Modularity.** Everything in Olvin is a separate crate and feature. A game without scripts must not pay for an interpreter in size or build time.
- **Web.** A wasm and WebGL2 build is planned (phase 4). The language must not close that road, even though we are not doing the web yet.
- **Sandbox.** Scripts from a project or from modders must not read files, start processes or hang the frame with an infinite loop.
- **Hot reload that keeps state.** Change the file, the behaviour updates, the object stays where it was.
- **Familiarity.** People who don't write Rust should be able to use it.
- [ROADMAP.en.md](../../ROADMAP.en.md) says it plainly: don't write our own scripting language.

## Decision

Lua 5.4 through the `mlua` 0.12 crate with the `lua54` and `vendored` features. The code is in `crates/olvin_script/src/lua.rs`, the tests in `lua_tests.rs`.

- **Optional.** The `lua` feature of `olvin_script` is off by default. In the `olvin` facade it is the `lua` feature (`olvin::script::lua`). Without it, Lua's C code is not built.
- **No system Lua.** `vendored` builds the interpreter from source when the crate builds. Only a C compiler is needed.
- **Same model as Rust behaviours.** A `*.lua` file returns a table with the callbacks `start`, `update`, `fixed_update`, `on_collision_enter/exit`, `on_trigger_enter/exit`, `on_destroy`. The file name is the script name in the scene. A Lua script registers in the same `ScriptRegistry` as a Rust behaviour; the scene can't tell them apart.
- **Editor parameters.** A `params` table describes the schema (`float`, `int`, `bool`, `vec3`, `string`, `entity`); values arrive typed in `self.params`.
- **Bindings.** `ctx` is userdata with methods for time, `Transform`, finding entities, physics (forces, velocity, `raycast`) and input. `ScriptCtx` is lent to Lua only for the duration of a callback. A `ctx` kept after the callback gives an error, not a dangling pointer.
- **Sandbox.** Only `table`, `string`, `math`, `utf8`, `coroutine` are loaded. No `io`, `os`, `debug`, `package`; `dofile`, `loadfile`, `require` are removed. `LuaLimits`: a memory limit for the whole Lua state (64 MB by default) and an instruction limit per callback (5,000,000 by default; the hook fires every 1000 instructions). A failed callback logs the error and disables the script until its file is reloaded. The frame goes on.
- **Hot reload.** `LuaPlugin` checks file modification times in `PreUpdate` every 250 ms and picks up new files. Callbacks are replaced inside the same prototype table, and every entity has its own `self` with `__index` pointing at the prototype. So state survives and `start` does not run again. A file with an error leaves the old version running.

## Considered options

### Lua 5.4 (chosen)

Pros:

- A small interpreter in portable C. It builds almost anywhere there is a C compiler, including old 32-bit ARM.
- Considered one of the faster interpreters among dynamic languages without a JIT. We don't need a JIT anyway: on some platforms (iOS, consoles, the browser) generating machine code is forbidden or awkward.
- The sandbox is built from standard pieces: choosing the standard libraries, a custom allocator with a limit, an instruction-count hook. `mlua` has all of it ready.
- Code is loaded from a string at runtime. Hot reload is just another `load` and swapping the functions in a table.
- 64-bit integers (since 5.3). A hecs entity id (`Entity::to_bits`, `u64`) goes into Lua as an integer without loss. In 5.1 and its descendants numbers are doubles only, and such an id would have to be packed.
- Modders and game designers know the language: Roblox, Garry's Mod, World of Warcraft, Defold, LÖVE use Lua or a dialect of it. There is a book and a reference manual for 5.4.
- `mlua` is a mature, actively maintained binding: userdata, hooks, memory limit, `vendored` builds.

Cons:

- C code in the build: a C compiler is needed for every target platform.
- No static types. A typo in a method name shows up only when it is called.
- Every Lua ↔ Rust call costs more than a direct Rust call. How much has not been measured yet (see "Consequences").
- 1-based indices, `nil` in tables, globals by default: the usual Lua pitfalls.
- LuaJIT was not seriously considered: it is stuck on 5.1 semantics, and its main advantage, the JIT, is unavailable exactly where JITs are not allowed at all.

### Rhai

A scripting language in pure Rust, made for embedding in Rust.

- Pros: no C at all, builds for `wasm32-unknown-unknown` with no fuss. Registering Rust types and functions is very simple. It has limits on operation count, call depth and string and array size. Scripts compile to an AST at runtime, so hot reload is possible.
- Cons: a tree-walking interpreter, no bytecode. Such interpreters are usually noticeably slower than bytecode ones like Lua; we did not compare them ourselves. The language is its own; outside the Rust ecosystem hardly anyone knows it, and there are few tutorials and little existing code.
- Why not: speed on weak hardware and familiarity for modders matter more than having no C code. This was the main competitor, and it has the advantage on the web.

### Luau

Roblox's Lua dialect: 5.1 base, gradual typing, its own fast interpreter, built around sandboxing.

- Pros: optional static types and an analyzer. The interpreter is more optimised than stock Lua. The sandbox was designed in from the start. `mlua` supports Luau (the `luau` feature), so the integration would be almost the same.
- Cons: the codebase is C++, larger and slower to build than Lua in C. The base is Lua 5.1: numbers are doubles only, no integers, so entity ids don't fit without packing. Luau has no "every N instructions" hook like 5.4; the limit would have to be built on its interrupt mechanism. Docs and community are mostly around Roblox.
- Why not: C++ in the build and losing integers outweighed the types. It stays the fallback: `mlua` bindings for Lua 5.4 and Luau are similar, so switching is cheaper than to any other language on this list.

### JavaScript (QuickJS or boa)

- Pros: the best-known language on the list, plus TypeScript for those who want types. QuickJS is compact for a full JS engine and supports a modern standard. boa is pure Rust. In the browser one could in theory use the page's own JS engine.
- Cons: QuickJS is C again, and the runtime is noticeably larger and more complex than Lua. boa is younger and slower than mature engines. The bindings (`rquickjs`, `boa_engine`) are less mature than `mlua`. JS's garbage collector and object model are heavier for old devices. A sandbox and limits are doable, but there are fewer ready tools. Calling "the page's JS" from a wasm engine is a separate bridge with its own cost, not a free gift.
- Why not: a bigger runtime and less mature Rust bindings, for a gain only in familiarity.

### WASM (wasmtime or wasmer)

Scripts as WebAssembly modules written in anything: Rust, AssemblyScript, C, Zig.

- Pros: the strongest sandbox of all options (isolated memory, "fuel" for instruction limits). With JIT compilation, near-native speed. Any language that compiles to wasm.
- Cons: wasmtime with Cranelift is a large dependency that noticeably adds to build time and binary size. A JIT is not needed on every platform and not allowed everywhere. Support for old and 32-bit devices is weaker. In the browser the runtime is different (the browser's own WebAssembly), so there would be two scripting backends. A modder needs a compiler and a build step: "save the file, see the change" turns into "rebuild the module". Keeping state across reloads is harder: it lives in the old module's linear memory. The host–module interface (component model, WIT) is more complex than userdata in Lua.
- Why not: heavy for weak hardware and awkward for quick iteration. It fits plugins, not gameplay scripts.

### Python

- Pros: one of the best-known languages, a huge ecosystem, `pyo3` is a good binding.
- Cons: CPython is large and has to ship with the game together with its standard library. It officially cannot be isolated: there is no reliable in-process sandbox. It is slow for a game loop. On the web only through Pyodide, which is a heavy download. RustPython is pure Rust, but slower than CPython and not fully compatible.
- Why not: size, no sandbox, speed.

### C# (.NET or Mono hosting)

- Pros: Unity's language, many in our target audience know it. Static types, good IDEs. Fast with a JIT.
- Cons: the .NET runtime is huge compared to the whole engine. Hosting from Rust is possible (through `hostfxr` or the Mono embedding API), but there are few mature crates, and we would write the glue ourselves. .NET Core has no sandbox (Code Access Security was removed). Hot reload needs assembly unloading (`AssemblyLoadContext`) and noticeable work. On iOS and in the browser there is no JIT, so AOT compilation or an interpreter is needed; on the web the runtime is heavy.
- Why not: runtime weight, weak Rust integration, no sandbox.

## Consequences

What we got:

- Scripts are edited without rebuilding and without restarting the game; object state survives a reload.
- An error or an infinite loop in a script doesn't break the frame: the callback is interrupted, the error goes to the log.
- Lua and Rust behaviours register in the same `ScriptRegistry` and attach to entities the same way. Heavy logic can be moved to Rust one behaviour at a time.
- Games without the `lua` feature don't carry the interpreter.

What it costs:

- **C code in the build.** `lua` needs a C compiler for the target platform. Cross-compiling is a bit harder than for pure Rust.
- **`unsafe` in the bindings.** `ScriptCtx` is lent to Lua through a raw pointer (`CtxSlot`), reset by a guard. This spot has to stay small and covered by tests.
- **No static types.** Mistakes in method and field names show up only at runtime. Annotations for an external analyzer or a move to Luau are not planned.
- **One thread.** The Lua state lives in an `Rc`; all Lua behaviours run on the main thread.
- **The instruction hook costs time.** It fires every 1000 instructions. Where the limit isn't needed, it can be turned off (`instructions: 0`).

Open questions (items in [ROADMAP.en.md](../../ROADMAP.en.md)):

- **The call cost is measured** (`benches/lua_vs_rust.rs`, i5-11400F): an empty Lua `update` costs ≈240 ns against 10 ns in Rust, a typical behaviour with input and movement ≈2.3 µs against 38 ns. 5,000 such scripts take 70% of a frame, so Lua suits hundreds of active scripts, not mass behaviours. Nearly all of the cost is vectors passed as tables across the Rust↔Lua boundary and the garbage they leave; the interpreter and the limit hook are a small part. That is fixed in the bindings, not by changing the language.
- **The web is not decided.** The C interpreter doesn't build for `wasm32-unknown-unknown` on its own: it needs libc stubs or a different wasm target. What to do with scripts in the browser is not chosen. This doesn't block the native build.
- **The editor and the hub don't use Lua yet.** The runtime is ready, but the editor doesn't load `*.lua`, Lua script parameters aren't visible in the inspector, and there is no `lua` module in the hub catalogue.
- **No `require` between files.** Shared code between scripts can't be factored out yet.

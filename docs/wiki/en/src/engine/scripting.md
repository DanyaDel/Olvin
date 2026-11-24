# Scripting

Two ways to write object logic: behaviours in Rust (`Behaviour`, the `script` module) and in Lua 5.4 (`lua`). Both attach to an object through the `Scripts` component (in a scene: `scripts: [(name, params)]`).

## Behaviours in Rust

```rust
struct Spinner { speed: f32 }
impl FromParams for Spinner {
    fn from_params(p: &Params) -> Self { Self { speed: p.f32("speed", 90.0) } }
    fn schema() -> Vec<ParamSpec> { vec![ParamSpec::float("speed", 90.0, "degrees per second")] }
}
impl Behaviour for Spinner {
    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        ctx.rotate_euler_deg(Vec3::new(0.0, self.speed * ctx.dt(), 0.0));
    }
}
// app.register_script::<Spinner>("Spinner");
```

`Behaviour` callbacks: `start`, `update`, `fixed_update`, `on_collision_enter/exit`, `on_trigger_enter/exit`, `on_destroy`; save hooks `save_state` / `load_state`. `ScriptCtx` gives the world and components, Transform helpers, time, lookup (`find_by_name`, `find_by_tag`), resources (`res`, `res_mut`), input, `raycast`, forces. Deferred `spawn/despawn/insert/remove` run at the end of the pass. A panic in a callback disables only that script. Ready-made: Rotator, Oscillator, Mover, FollowTarget, DestroyAfter, FreeFlyCamera, SimplePlayerController.

## Lua

A script is `assets/scripts/<name>.lua` returning a table of callbacks; the file name is the script's name in the scene.

```lua
return {
    params = { { name = "speed", kind = "float", default = 90, doc = "degrees per second" } },
    start  = function(self, ctx) self.t = 0 end,
    update = function(self, ctx)
        ctx:rotate_euler_deg_xyz(0, self.params.speed * ctx:dt(), 0)
    end,
}
```

`LuaPlugin { dir, hot_reload, limits }` is added after `DefaultPlugins`. Callbacks: `start`, `update`, `fixed_update`, `on_collision_enter/exit`, `on_trigger_enter/exit`, `on_destroy`, `on_message(self, ctx, name, value, from)`. `self` is per object, `self.params` is filled from the scene by the `params` types. The full list of `ctx:` functions and callbacks: [Lua API reference](../reference/lua-api.md).

**Hot reload:** a changed file is picked up, new callbacks replace the old ones, `self` survives, `start` does not run again; a file with an error leaves the working version in place.

**Sandbox:** only `table`, `string`, `math`, `utf8`, `coroutine` are loaded; no `io`, `os`, `debug`, `package`, `dofile`, `loadfile`, `require`. `load` compiles text only (binary chunks are refused: hand-edited bytecode corrupts the VM's memory), `string.dump` is removed, `collectgarbage` accepts only `"collect"` and `"count"`. A memory limit of 64 MB per state, 5,000,000 instructions per callback; messages (4096 queued), variables (1024), strings (16 KB) and sounds (1024) are capped. A failed callback disables the script until the file is reloaded, the frame does not crash.

**Performance:** `vec3` tables are convenient, but hot code has `*_xyz` twins without tables (the "walker" estimate is 0.42 µs, benchmark `cargo bench -p olvin_script --features lua --bench lua_vs_rust`). Why Lua: `docs/adr/0001-lua-scripting.md`.

## Messages and variables

`ctx:send(name, value[, target])` puts a `ScriptMessage` into `Events<ScriptMessage>`; the target gets `on_message`, the game reads the queue itself (this is how the slice shows the HUD). `ctx:var(name)` and `ctx:set_var(name, value)`: shared `ScriptVars` between scripts and the game (`vars.flag(name)`, `vars.set(..)`).

> **Status:** Lua does not run in the editor and its parameters are not visible in the inspector. Not done: a node editor, `require` between files, timers and delay coroutines ([#37](https://github.com/DanyaDel/Olvin/issues/37)), `self` state in saves, web. Hot reload of Rust code: [#70](https://github.com/DanyaDel/Olvin/issues/70).

## See also

- [Lua API reference](../reference/lua-api.md)
- [Tutorial: step 5](../tutorial/05-door.md)
- [Physics](physics.md)
- [States and saves](game-states-saves.md)

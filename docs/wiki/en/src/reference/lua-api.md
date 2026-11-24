# Lua API

Reference for Lua 5.4 scripts (`olvin_script`, feature `lua`). It is checked against the code by `docs/wiki/tools/check_lua_reference.py`: every registered `ctx:` function and every callback must be listed here. Introduction: [Scripting](../engine/scripting.md).

## A script

The file `assets/scripts/<name>.lua` returns a table. The file name is the script name in the scene (`scripts: [(name: "door", params: {..})]`).

```lua
return {
    params = { { name = "speed", kind = "float", default = 90, doc = "degrees per second" } },
    start = function(self, ctx) end,
    update = function(self, ctx) end,
}
```

`params`: a list of `{ name, kind, default, doc }`; `kind`: `float` (`number`), `int` (`integer`), `bool` (`boolean`), `vec3`, `string` (the default), `entity` (the name of another object); the type is `ParamKind`. Values from the scene (strings) are converted to the type and available as `self.params.<name>`. `self` is a table of its own for each object and survives hot reload.

## Callbacks

| Callback | Arguments | When |
|---|---|---|
| `start` | `(self, ctx)` | once before the first `update` |
| `update` | `(self, ctx)` | every frame |
| `fixed_update` | `(self, ctx)` | every fixed physics step (1/60 s) |
| `on_collision_enter` | `(self, ctx, other)` | contact begins; `other`: an object id |
| `on_collision_exit` | `(self, ctx, other)` | contact ends |
| `on_trigger_enter` | `(self, ctx, other)` | enters a trigger |
| `on_trigger_exit` | `(self, ctx, other)` | leaves a trigger |
| `on_destroy` | `(self, ctx)` | the object is being removed |
| `on_message` | `(self, ctx, name, value, from)` | a `ctx:send` message arrived |
| `on_net_message` | `(self, ctx, name, value, from)` | a network message arrived (feature `net`) |

Callbacks are read from the table on load: assigning `self.update = …` at runtime does not replace them. Objects in Lua are integer ids; `nil` means "no such thing".

## Vectors

`vec3(x, y, z)`: a table `{x, y, z}` with arithmetic (`a + b`, `a - b`, `-a`, `v * 2`, `a / 2`, componentwise `a * b`, `==`) and methods `v:dot(b)`, `v:length()`, `v:distance(b)`, `v:normalized()`, `v:cross(b)`, `v:lerp(b, t)`, `v:unpack()`. Wherever a function takes a vector, `vec3(..)`, `{x=, y=, z=}`, `{x, y, z}` or three numbers work; missing axes are 0.

**The `*_xyz` fast path:** every vector getter has a twin `name_xyz()` returning three numbers without a table; vector setters take three numbers (`set_position(x, y, z)`), and the `*_xyz` ones exactly three numbers. Use them in hot code. `axis2d_xy` returns two numbers.

## Time and the object

| Function | Result |
|---|---|
| `ctx:entity()` | the id of this object |
| `ctx:name()` | the object's name |
| `ctx:dt()` | frame step, simulation time (frozen on pause) |
| `ctx:fixed_dt()` | physics step |
| `ctx:elapsed()` | simulation time since the start |
| `ctx:frame()` | frame number |
| `ctx:unscaled_dt()` | frame step in real time |
| `ctx:time_scale()` | the time multiplier |
| `ctx:paused()` | `true` while paused |
| `ctx:log(msg)` | a message to the log (info) |
| `ctx:warn(msg)` | a warning to the log |

## This object's Transform

Angles are in degrees, rotation is Euler YXZ.

| Function | Fast path | What |
|---|---|---|
| `ctx:position()` | `ctx:position_xyz()` | local position |
| `ctx:world_position()` | `ctx:world_position_xyz()` | world position |
| `ctx:set_position(v)` / `(x, y, z)` | `ctx:set_position_xyz(x, y, z)` | set the position |
| `ctx:translate(v)` | `ctx:translate_xyz(x, y, z)` | move in the parent's axes |
| `ctx:translate_local(v)` | `ctx:translate_local_xyz(x, y, z)` | move in its own axes |
| `ctx:rotate_euler_deg(v)` | `ctx:rotate_euler_deg_xyz(x, y, z)` | rotate further |
| `ctx:rotation_euler_deg()` | `ctx:rotation_euler_deg_xyz()` | current angles |
| `ctx:set_rotation_euler_deg(v)` | `ctx:set_rotation_euler_deg_xyz(x, y, z)` | set the angles |
| `ctx:scale()` | `ctx:scale_xyz()` | scale |
| `ctx:set_scale(v)` | `ctx:set_scale_xyz(x, y, z)` | set the scale |
| `ctx:look_at(target[, up])` / `(x, y, z)` | `ctx:look_at_xyz(x, y, z[, ux, uy, uz])` | turn towards a point (up defaults to +Y) |
| `ctx:forward()` | `ctx:forward_xyz()` | forward direction |
| `ctx:right()` | `ctx:right_xyz()` | right |
| `ctx:up()` | `ctx:up_xyz()` | up |

## Other objects

Ids come from lookups, from the `other` of callbacks and from `ctx:raycast`.

| Function | Result |
|---|---|
| `ctx:find_by_name(name)` | the id of the first object with that name, or `nil` |
| `ctx:find_by_tag(tag)` | the id of the first object with the tag, or `nil` |
| `ctx:find_all_by_tag(tag)` | an array of ids |
| `ctx:find_child(name)` | the id of this object's direct child with that name, or `nil` |
| `ctx:alive(id)` | `true` if the object exists |
| `ctx:name_of(id)` | the name or `nil` |
| `ctx:has_tag(id, tag)` | `true` if the object has that tag |
| `ctx:position_of(id)` / `ctx:position_of_xyz(id)` | local position or `nil` |
| `ctx:world_position_of(id)` / `ctx:world_position_of_xyz(id)` | world position or `nil` |
| `ctx:rotation_euler_deg_of(id)` / `ctx:rotation_euler_deg_of_xyz(id)` | angles or `nil` |
| `ctx:scale_of(id)` / `ctx:scale_of_xyz(id)` | scale or `nil` |
| `ctx:set_position_of(id, v)` / `ctx:set_position_of_xyz(id, x, y, z)` | set the position; `true` if the object was found |
| `ctx:translate_of(id, v)` / `ctx:translate_of_xyz(id, x, y, z)` | move another object |
| `ctx:set_rotation_euler_deg_of(id, v)` / `ctx:set_rotation_euler_deg_of_xyz(id, x, y, z)` | set the angles |
| `ctx:set_scale_of(id, v)` / `ctx:set_scale_of_xyz(id, x, y, z)` | set the scale |
| `ctx:despawn(id)` | remove an object and its children (at the end of the pass) |
| `ctx:spawn_prefab(path, x, y, z[, yaw_deg])` | queue a prefab instance (`prefabs/...prefab.ron`, relative to `assets/`, no `..`); returns `false` when more than 16 were queued this frame or 256 spawned objects are alive. The game creates it after the pass and sends this object a `"spawned"` message whose value is the new root's id |
| `ctx:destroy_self()` | remove itself |

## Messages and variables

| Function | What |
|---|---|
| `ctx:send(name, value[, target])` | send a message: `value` is a number, string, bool, vector or `nil`; without `target` it is a broadcast: every object with a script gets `on_message` and the game reads `Events<ScriptMessage>`; with `target` it is addressed to one object; `from` is the sender's id |
| `ctx:var(name)` | a shared variable (`ScriptVars`) or `nil` |
| `ctx:set_var(name, value)` | write a shared variable (`nil` deletes) |

Names and strings are limited to 16 KB, the message queue to 4096, variables to 1024; exceeding that is a script error.

## Network

Feature `net` of `olvin_script` and `LuaNetPlugin` after `NetPlugin` (see [Networking](../engine/networking.md)). Without a network session `is_server()` is false and `is_client()` is true.

| Function | What |
|---|---|
| `ctx:is_server()` | this process runs the authoritative world (offline, host or dedicated) |
| `ctx:is_client()` | this process has a local player (offline, host or a remote client) |
| `ctx:local_client()` | our client id (0 for offline and host) or `nil` before the handshake |
| `ctx:owner()` | the client id that owns this object (`Owner`) or `nil` |
| `ctx:is_mine()` | this object is owned by the local player |
| `ctx:net_send(target, name, value)` | send a message: `target` is `"server"`, `"all"`, `"owner"` (this object's owner), a client id, or `{ except = id }`; a client can only send to `"server"`; `value` is a number, string, bool, vector or `nil`; name up to 64 characters, text up to 256 bytes, 64 sends per frame |
| `ctx:replicate_var(name)` | on the server: the shared variable `name` (`ctx:set_var`) is copied to every client when it changes, and to late joiners; clients read it with `ctx:var` |

The callback `on_net_message(self, ctx, name, value, from)` runs for messages that came over the network; `from` is the sender's client id, or `nil` when the server sent it. On a server, check what a client asks for here: the sender id is the one the server saw, never one the client wrote. Messages are rate- and size-limited per client like every other network message.

## Physics

Feature `physics`; for a Dynamic body only (otherwise the functions do nothing).

| Function | Fast path | What |
|---|---|---|
| `ctx:apply_force(v)` | `ctx:apply_force_xyz(x, y, z)` | a force for the next step (call every step) |
| `ctx:apply_impulse(v)` | `ctx:apply_impulse_xyz(x, y, z)` | an instant impulse (Δv = impulse / mass) |
| `ctx:set_velocity(v)` | `ctx:set_velocity_xyz(x, y, z)` | set the velocity |
| `ctx:velocity()` | `ctx:velocity_xyz()` | the velocity or `nil` |
| `ctx:motor_velocity()` | `ctx:motor_velocity_xyz()` | velocity of this object's Character Controller motor (the character moves by it), or `nil` |
| `ctx:set_motor_velocity(v)` | `ctx:set_motor_velocity_xyz(x, y, z)` | set it; `false` without a Character Controller |
| `ctx:grounded()` | | the character stands on the ground |
| `ctx:gravity()` | | the world's gravity, m/s², positive |
| `ctx:teleport(v)` / `ctx:teleport_of(id, v)` | `ctx:teleport_xyz(x, y, z)` / `ctx:teleport_of_xyz(id, x, y, z)` | put a body (any kind, a character too) at a position now and stop it; `false` when there is no such object. For respawns: a plain `set_position` also works but keeps the velocity |
| `ctx:raycast(origin, direction, max)` | `ctx:raycast_xyz(ox, oy, oz, dx, dy, dz, max)` | a ray: `nil` or `{ entity, point, normal, distance }`; the fast path returns `entity, px, py, pz, nx, ny, nz, distance` |

## Input

Feature `input`. `action` is an action name from `InputMap` ([Input](../engine/input.md)).

| Function | Result |
|---|---|
| `ctx:pressed(action)` | held |
| `ctx:set_cursor_locked(locked)` | grab (or free) the mouse cursor, for mouse look |
| `ctx:just_pressed(action)` | pressed this frame |
| `ctx:just_released(action)` | released this frame |
| `ctx:value(action)` | a number 0..1 (an axis) |
| `ctx:axis2d(action)` | a table `{ x, y }` |
| `ctx:axis2d_xy(action)` | two numbers `x, y` |

## Sound

Feature `audio`. Paths are relative to `assets/`, without `..` and absolute ones. Without an audio device the functions do nothing and `play_*` returns `nil`. At most 1024 sounds are tracked at once.

| Function | What |
|---|---|
| `ctx:play_sound(path[, opts])` | a 2D sound, returns a sound handle |
| `ctx:play_sound_at(path, where[, opts])` | a sound at a point: `where` is a vector or an object id |
| `ctx:play_sound_at_xyz(path, x, y, z[, opts])` | the same with numbers |
| `ctx:stop_sound(sound[, fade])` | stop (with a fade-out in seconds) |
| `ctx:sound_playing(sound)` | whether it is playing |
| `ctx:set_sound_volume(sound, volume)` | volume |
| `ctx:set_sound_pitch(sound, pitch)` | pitch (minimum 0.01) |
| `ctx:play_source([id])` | start the scene's `AudioSource` (own or of object `id`) |
| `ctx:stop_source([id])` | stop it |

`opts`: `{ volume, pitch, loop, bus, min_distance, max_distance, priority, stream, occludable }`; `bus` is a bus name (`"sfx"`, `"music"`, `"ambient"`, `"voice"`, or your own).

## Sandbox

Libraries: only `table`, `string`, `math`, `utf8`, `coroutine`. No `io`, `os`, `debug`, `package`, `dofile`, `loadfile`, `require`. Default limits (`LuaLimits`): memory 64 MB per state, 5,000,000 instructions per callback. A callback error disables the script until its file is reloaded and goes to the log, the frame does not crash. `ctx` is valid only inside a callback: do not store it.

## See also

- [Scripting](../engine/scripting.md)
- [Tutorial: step 5](../tutorial/05-door.md)
- [Physics](../engine/physics.md)
- [Audio](../engine/audio.md)

# Step 5. Plate and door in Lua

A pressure plate opens a door. The logic is in Lua: two short tables of callbacks. The `lua` module is needed.

## Objects in the scene

| Name | What | Position | Scale | Components |
|---|---|---|---|---|
| Door | cube | 0, 1.5, 0 | 2.4, 3, 0.3 | Box Collider, Rigidbody **Kinematic**, Scripts `door` |
| Plate | marker cube | 4, 0.03, 6 | 2, 0.06, 2 | Mesh Renderer only |
| Plate Trigger | cube (the mesh may be removed) | 4, 1, 6 | 2, 2, 2 | Box Collider with **Is Trigger**, Scripts `plate` |

The door is kinematic: its collider follows the `Transform` that the script moves. Script parameters are set in the Inspector, Scripts section (key and value as strings):

- `door`: `lift = 3.1`, `speed = 1.5`
- `plate`: `door = Door`, `message = Дверь открыта: зайди к роботу`

## door.lua

The file `assets/scripts/door.lua` (the file name is the script name in the scene). It lifts the door by `lift` while the shared variable `door_open` is true:

```lua
-- Sliding door: moves up by `lift` while the shared variable `door_open` is
-- true, back down when it is not. Simulation time, so it stops on pause.
-- The door is a kinematic body: its collider follows the Transform.
return {
    params = {
        { name = "lift", kind = "float", default = 3.1, doc = "how far up it slides, metres" },
        { name = "speed", kind = "float", default = 1.5, doc = "metres per second" },
    },

    start = function(self, ctx)
        local _, y = ctx:position_xyz()
        self.closed_y = y
    end,

    update = function(self, ctx)
        local x, y, z = ctx:position_xyz()
        local target = self.closed_y + (ctx:var("door_open") and self.params.lift or 0)
        local step = self.params.speed * ctx:dt()
        local dy = math.max(-step, math.min(step, target - y))
        if dy ~= 0 then
            ctx:set_position(x, y + dy, z)
        end
    end,
}
```

## plate.lua

The file `assets/scripts/plate.lua`. The `on_trigger_enter` callback fires when another object enters the trigger (`other` is its id):

```lua
-- Pressure plate (on its trigger volume): the player stepping in opens the
-- door named by `door` once, plays the door sound at the door and shows
-- `message` on the HUD.
--
-- The door's state is the shared variable `door_open` (ScriptVars): door.lua
-- slides the door by it, the game mirrors it into its saved `Progress`.
return {
    params = {
        { name = "door", kind = "string", default = "Door", doc = "name of the door entity" },
        { name = "message", kind = "string", default = "The door is open", doc = "shown when it opens" },
    },

    on_trigger_enter = function(self, ctx, other)
        if other ~= ctx:find_by_name("Player") or ctx:var("door_open") then
            return
        end
        local door = ctx:find_by_name(self.params.door)
        if not door then
            ctx:warn("no door named `" .. self.params.door .. "`")
            return
        end
        ctx:set_var("door_open", true)
        ctx:play_sound_at("sfx/door.wav", door, { volume = 0.9, max_distance = 30 })
        ctx:send("hud", self.params.message)
    end,
}
```

What happens here:

- `params` declares the parameters and their types (`float`, `string`); the values arrive as `self.params.*`.
- `ctx:position_xyz()` and `ctx:set_position(x, y, z)`: the fast path without tables.
- `ctx:var` / `ctx:set_var`: shared variables between scripts (`ScriptVars`). The door knows nothing about the plate; they are linked only by the name `door_open`.
- `ctx:find_by_name`, `ctx:play_sound_at`, `ctx:send`: find an object, sound ([step 6](06-sound.md)) and a message to the game ([step 7](07-ui.md)).
- Time: `ctx:dt()` is simulation time, so the door freezes on pause.

Scripts are read from `assets/scripts/` (`LuaPlugin { dir, .. }` from step 4). With `hot_reload` an edited file is picked up on the fly: each object's `self` survives and `start` does not run again. A failed callback disables the script until the file is reloaded, the frame does not crash, the error goes to the log.

## Check

`olvin-hub run`: step on the yellow plate and the door slides up. The message goes nowhere yet: the HUD comes in step 7. There is no sound either until step 6 (a `play_sound_at` call with a missing file logs an error, it does not crash).

## See also

- [Scripting](../engine/scripting.md)
- [Lua API reference](../reference/lua-api.md)
- [Physics: triggers](../engine/physics.md#triggers-and-events)
- [Next: step 6, sound](06-sound.md)

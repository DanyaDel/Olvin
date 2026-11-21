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

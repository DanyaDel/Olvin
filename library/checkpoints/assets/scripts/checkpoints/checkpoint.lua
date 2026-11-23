-- Checkpoint (on an entity with a trigger collider). The player entering it
-- makes it the active checkpoint: the shared variable `checkpoint` holds its
-- name and `checkpoint_pos` the place to respawn at (this entity's position
-- plus `spawn_offset`). Only the active checkpoint answers the "respawn"
-- message (value = the entity to move, else the player) by teleporting it
-- there; then it broadcasts "respawned". "checkpoint_reached" (value = name)
-- is broadcast on activation. Give every checkpoint its own entity name.
--
-- `initial`: this is the start of the level, active from the first frame.
-- The optional child `flag` rises when its checkpoint becomes active.
return {
    params = {
        { name = "initial", kind = "bool", default = false, doc = "active from the start (put one at the level's start)" },
        { name = "spawn_offset", kind = "vec3", default = vec3(0, 0.95, 0), doc = "from this entity's position to where the player's origin goes" },
        { name = "player", kind = "string", default = "Player", doc = "name of the player (an entity tagged Player counts too)" },
        { name = "flag", kind = "string", default = "Flag", doc = "name of the child that rises when active" },
        { name = "reach_sound", kind = "string", default = "", doc = "sound when reached (empty = none)" },
    },

    start = function(self, ctx)
        self.flag = ctx:find_child(self.params.flag)
        if self.flag then self.fx, self.fy, self.fz = ctx:position_of_xyz(self.flag) end
        self.shown = nil
        if self.params.initial then self:activate(ctx) end
    end,

    is_active = function(self, ctx)
        return ctx:var("checkpoint") == ctx:name()
    end,

    activate = function(self, ctx)
        if self:is_active(ctx) then return end
        local x, y, z = ctx:world_position_xyz()
        local o = self.params.spawn_offset
        ctx:set_var("checkpoint", ctx:name())
        ctx:set_var("checkpoint_pos", vec3(x + o.x, y + o.y, z + o.z))
        ctx:send("checkpoint_reached", ctx:name())
        if self.params.reach_sound ~= "" then
            ctx:play_sound_at(self.params.reach_sound, ctx:entity(), { max_distance = 25 })
        end
    end,

    on_trigger_enter = function(self, ctx, other)
        if ctx:name_of(other) == self.params.player or ctx:has_tag(other, "Player") then
            self:activate(ctx)
        end
    end,

    on_message = function(self, ctx, name, value)
        if name ~= "respawn" or not self:is_active(ctx) then return end
        local who = math.type(value) == "integer" and value or ctx:find_by_name(self.params.player)
        local pos = ctx:var("checkpoint_pos")
        if who and ctx:alive(who) and pos then
            ctx:teleport_of_xyz(who, pos.x, pos.y, pos.z)
            ctx:send("respawned", ctx:name())
        end
    end,

    update = function(self, ctx)
        if not self.flag then return end
        local active = self:is_active(ctx)
        if active ~= self.shown then
            self.shown = active
            ctx:set_position_of_xyz(self.flag, self.fx, self.fy + (active and 0.5 or 0), self.fz)
        end
    end,
}

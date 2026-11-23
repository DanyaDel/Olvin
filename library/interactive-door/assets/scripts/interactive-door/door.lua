-- Door that slides or swings open. Put it on the door's root: the entity with
-- the proximity trigger. It moves the child named by `hinge` (the pivot that
-- carries the solid panel): `slide` moves it, `hinge` mode turns it around Y.
--
-- It opens by proximity (the player enters the trigger), by a key press
-- (the `Interact` action, E by default, while inside the trigger) and/or by
-- messages: "open", "close", "toggle", "lock", "unlock". A message reaches the
-- door when it is sent to the door's entity, or broadcast with the door's `id`
-- as its value (a broadcast without a value reaches every door).
--
-- A locked door refuses to open until the message "unlock" arrives or the
-- shared variable `key_<key_id>` is true (the pickups pack sets it).
--
-- Sends (broadcast, value = the door's id or name): "door_opened",
-- "door_closed", "door_locked" (the player tried a locked door).
-- Sound hooks are paths under assets/; empty = silent.

local function smooth(t) return t * t * (3 - 2 * t) end

local function has(list, word)
    for w in string.gmatch(list, "[^,%s]+") do
        if w == word then return true end
    end
    return false
end

return {
    params = {
        { name = "id", kind = "string", default = "", doc = "name other scripts use to address this door (message value)" },
        { name = "mode", kind = "string", default = "slide", doc = "slide or hinge" },
        { name = "slide", kind = "vec3", default = vec3(2.2, 0, 0), doc = "slide mode: how far the hinge child moves when open (local axes, m)" },
        { name = "angle", kind = "float", default = 90, doc = "hinge mode: how far it swings around Y, degrees (negative: the other way)" },
        { name = "duration", kind = "float", default = 0.8, doc = "seconds from closed to open" },
        { name = "opens_by", kind = "string", default = "proximity", doc = "any of: proximity, key, message (comma separated)" },
        { name = "action", kind = "string", default = "Interact", doc = "input action of the `key` way" },
        { name = "auto_close", kind = "bool", default = true, doc = "proximity: close again after the player left" },
        { name = "close_delay", kind = "float", default = 1.0, doc = "seconds the door waits before closing" },
        { name = "locked", kind = "bool", default = false, doc = "start locked" },
        { name = "key_id", kind = "string", default = "", doc = "the key that unlocks it (variable key_<id>); empty = only the `unlock` message" },
        { name = "player", kind = "string", default = "Player", doc = "name of the player (an entity tagged Player counts too)" },
        { name = "hinge", kind = "string", default = "Hinge", doc = "name of the child that moves" },
        { name = "open_sound", kind = "string", default = "", doc = "sound played when it starts opening (empty = none)" },
        { name = "close_sound", kind = "string", default = "", doc = "sound played when it starts closing (empty = none)" },
        { name = "locked_sound", kind = "string", default = "", doc = "sound played when a locked door refuses (empty = none)" },
    },

    start = function(self, ctx)
        self.hinge = ctx:find_child(self.params.hinge)
        if not self.hinge then
            ctx:warn("no child named `" .. self.params.hinge .. "`: nothing to move")
        else
            self.x0, self.y0, self.z0 = ctx:position_of_xyz(self.hinge)
            _, self.yaw0 = ctx:rotation_euler_deg_of_xyz(self.hinge)
        end
        self.hinge_mode = self.params.mode == "hinge"
        self.open = 0 -- 0 closed .. 1 open
        self.want = false
        self.inside = false
        self.locked = self.params.locked
        self.close_at = nil
        self.moving = false
        self.refused_at = -10
        self.name_ = self.params.id ~= "" and self.params.id or ctx:name()
    end,

    is_player = function(self, ctx, other)
        return ctx:name_of(other) == self.params.player or ctx:has_tag(other, "Player")
    end,

    sound = function(self, ctx, path)
        if path ~= "" then
            ctx:play_sound_at(path, ctx:entity(), { max_distance = 25 })
        end
    end,

    set_want = function(self, ctx, want)
        if want == self.want then return end
        self.want = want
        self.close_at = nil
        self:sound(ctx, want and self.params.open_sound or self.params.close_sound)
    end,

    try_open = function(self, ctx)
        if self.locked then
            local key = self.params.key_id
            if key ~= "" and ctx:var("key_" .. key) then
                self.locked = false
            else
                if ctx:elapsed() - self.refused_at > 1 then
                    self.refused_at = ctx:elapsed()
                    ctx:send("door_locked", self.name_)
                    self:sound(ctx, self.params.locked_sound)
                end
                return
            end
        end
        self:set_want(ctx, true)
    end,

    on_trigger_enter = function(self, ctx, other)
        if not self:is_player(ctx, other) then return end
        self.inside = true
        self.close_at = nil
        if has(self.params.opens_by, "proximity") then
            self:try_open(ctx)
        end
    end,

    on_trigger_exit = function(self, ctx, other)
        if not self:is_player(ctx, other) then return end
        self.inside = false
        if self.want and self.params.auto_close and has(self.params.opens_by, "proximity") then
            self.close_at = ctx:elapsed() + self.params.close_delay
        end
    end,

    on_message = function(self, ctx, name, value)
        if value ~= nil and value ~= self.params.id then return end
        if name == "open" then
            self:try_open(ctx)
        elseif name == "close" then
            self:set_want(ctx, false)
        elseif name == "toggle" then
            if self.want then self:set_want(ctx, false) else self:try_open(ctx) end
        elseif name == "unlock" then
            self.locked = false
        elseif name == "lock" then
            self.locked = true
        end
    end,

    update = function(self, ctx)
        if self.inside and has(self.params.opens_by, "key") and ctx:just_pressed(self.params.action) then
            if self.want then self:set_want(ctx, false) else self:try_open(ctx) end
        end
        if self.close_at and ctx:elapsed() >= self.close_at then
            self:set_want(ctx, false)
        end

        local target = self.want and 1 or 0
        if self.open == target then return end
        local step = ctx:dt() / math.max(self.params.duration, 0.01)
        if self.open < target then
            self.open = math.min(target, self.open + step)
        else
            self.open = math.max(target, self.open - step)
        end
        if self.hinge then
            local k, p = smooth(self.open), self.params
            if self.hinge_mode then
                ctx:set_rotation_euler_deg_of_xyz(self.hinge, 0, self.yaw0 + p.angle * k, 0)
            else
                ctx:set_position_of_xyz(self.hinge,
                    self.x0 + p.slide.x * k, self.y0 + p.slide.y * k, self.z0 + p.slide.z * k)
            end
        end
        if self.open == target then
            ctx:send(target == 1 and "door_opened" or "door_closed", self.name_)
        end
    end,
}

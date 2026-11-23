-- Collectible: spins and bobs (its child `visual`), is collected when the
-- player enters its trigger. Put it on the root that carries the trigger.
--
-- kind "coin":   adds `amount` to the shared variable `score`
-- kind "health": sends "heal" (value `amount`) to the player (health-damage)
-- kind "key":    sets `key_<key_id>` to true and adds 1 to `keys` (doors read it)
-- kind "custom": adds `amount` to the variable named by `var`
-- Broadcasts "picked_up" (value = the kind). With `respawn` > 0 the pickup comes
-- back after that many seconds, otherwise it removes itself.
-- `pickup_sound` is an optional path under assets/ (empty = silent).
return {
    params = {
        { name = "kind", kind = "string", default = "coin", doc = "coin, health, key or custom" },
        { name = "amount", kind = "float", default = 1, doc = "score, health points or counter increment" },
        { name = "key_id", kind = "string", default = "gold", doc = "key: which key (variable key_<id>)" },
        { name = "var", kind = "string", default = "", doc = "custom: the variable to add `amount` to" },
        { name = "respawn", kind = "float", default = 0, doc = "seconds until it reappears; 0 = collected once" },
        { name = "spin", kind = "float", default = 120, doc = "spin, degrees per second" },
        { name = "bob", kind = "float", default = 0.12, doc = "bob height, metres" },
        { name = "bob_speed", kind = "float", default = 2.5, doc = "bobs per 2*pi seconds" },
        { name = "visual", kind = "string", default = "Visual", doc = "name of the child that spins and bobs" },
        { name = "player", kind = "string", default = "Player", doc = "name of the player (an entity tagged Player counts too)" },
        { name = "pickup_sound", kind = "string", default = "", doc = "sound when collected (empty = none)" },
    },

    start = function(self, ctx)
        self.visual = ctx:find_child(self.params.visual)
        self.t = (ctx:entity() % 97) * 0.37 -- neighbours do not bob in step
        self.collected = false
        self.back_at = 0
        if self.visual then
            self.vx, self.vy, self.vz = ctx:position_of_xyz(self.visual)
            self.sx, self.sy, self.sz = ctx:scale_of_xyz(self.visual)
        end
    end,

    update = function(self, ctx)
        if self.collected then
            if self.params.respawn > 0 and ctx:elapsed() >= self.back_at then
                self.collected = false
                if self.visual then ctx:set_scale_of_xyz(self.visual, self.sx, self.sy, self.sz) end
            end
            return
        end
        if not self.visual then return end
        local dt = ctx:dt()
        self.t = self.t + dt
        ctx:set_position_of_xyz(self.visual, self.vx,
            self.vy + math.sin(self.t * self.params.bob_speed) * self.params.bob, self.vz)
        local rx, yaw, rz = ctx:rotation_euler_deg_of_xyz(self.visual)
        ctx:set_rotation_euler_deg_of_xyz(self.visual, rx, yaw + self.params.spin * dt, rz)
    end,

    on_trigger_enter = function(self, ctx, other)
        if self.collected then return end
        if ctx:name_of(other) ~= self.params.player and not ctx:has_tag(other, "Player") then return end
        local p, kind = self.params, self.params.kind
        if kind == "coin" then
            ctx:set_var("score", (ctx:var("score") or 0) + p.amount)
        elseif kind == "health" then
            ctx:send("heal", p.amount, other)
        elseif kind == "key" then
            ctx:set_var("key_" .. p.key_id, true)
            ctx:set_var("keys", (ctx:var("keys") or 0) + 1)
        elseif kind == "custom" and p.var ~= "" then
            ctx:set_var(p.var, (ctx:var(p.var) or 0) + p.amount)
        end
        ctx:send("picked_up", kind)
        if p.pickup_sound ~= "" then
            ctx:play_sound_at(p.pickup_sound, ctx:entity(), { max_distance = 20 })
        end
        if p.respawn > 0 then
            self.collected = true
            self.back_at = ctx:elapsed() + p.respawn
            if self.visual then ctx:set_scale_of_xyz(self.visual, 0, 0, 0) end
        else
            ctx:destroy_self()
        end
    end,
}

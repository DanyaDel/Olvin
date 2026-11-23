-- Health for any entity (the player, an enemy, a crate).
--
-- Receives (send them to this entity: ctx:send(name, value, entity)):
--   "damage"  value = amount; the sender (`from`) is the source
--   "heal"    value = amount, up to max_health
--   "kill"    dies at once          "revive"  back to full health
-- Broadcasts: "damaged" (value amount), "healed" (value amount) and "died"
-- (value = this entity's name), all from this entity.
--
-- Mirrors the health into the shared variables `<var>` and `<var>_max`
-- (default "health" / "health_max"; an empty `var` switches it off) so the game
-- can draw it: see the notes of the pack.
--
-- After a hit it is invulnerable for `invulnerable_time` seconds (keep that
-- shorter than the interval of a hazard that damages over time). On death:
-- `destroy_on_death` removes the entity; `respawn_delay` >= 0 waits that long,
-- refills the health and broadcasts "respawn" (value = this entity's id), which
-- the checkpoints pack answers by teleporting the entity.
return {
    params = {
        { name = "max_health", kind = "float", default = 100, doc = "full health" },
        { name = "start_health", kind = "float", default = 0, doc = "health at the start; 0 = full" },
        { name = "invulnerable_time", kind = "float", default = 0.5, doc = "seconds without damage after a hit" },
        { name = "var", kind = "string", default = "health", doc = "shared variable mirroring the health (empty = none)" },
        { name = "destroy_on_death", kind = "bool", default = false, doc = "remove the entity when it dies" },
        { name = "respawn_delay", kind = "float", default = -1, doc = "seconds until it respawns after death; negative = never" },
        { name = "hurt_sound", kind = "string", default = "", doc = "sound when hurt (empty = none)" },
        { name = "death_sound", kind = "string", default = "", doc = "sound on death (empty = none)" },
    },

    start = function(self, ctx)
        local p = self.params
        self.health = p.start_health > 0 and math.min(p.start_health, p.max_health) or p.max_health
        self.dead = false
        self.last_hit = -1e9
        self.respawn_at = nil
        self:mirror(ctx)
    end,

    mirror = function(self, ctx)
        local v = self.params.var
        if v ~= "" then
            ctx:set_var(v, self.health)
            ctx:set_var(v .. "_max", self.params.max_health)
        end
    end,

    sound = function(self, ctx, path)
        if path ~= "" then ctx:play_sound_at(path, ctx:entity(), { max_distance = 30 }) end
    end,

    die = function(self, ctx)
        self.dead = true
        self.health = 0
        self:mirror(ctx)
        ctx:send("died", ctx:name())
        self:sound(ctx, self.params.death_sound)
        if self.params.destroy_on_death then
            ctx:destroy_self()
        elseif self.params.respawn_delay >= 0 then
            self.respawn_at = ctx:elapsed() + self.params.respawn_delay
        end
    end,

    revive = function(self, ctx)
        self.dead = false
        self.respawn_at = nil
        self.health = self.params.max_health
        self.last_hit = ctx:elapsed()
        self:mirror(ctx)
    end,

    on_message = function(self, ctx, name, value)
        local amount = type(value) == "number" and value or 0
        if name == "damage" then
            if self.dead or amount <= 0 then return end
            if ctx:elapsed() - self.last_hit < self.params.invulnerable_time then return end
            self.last_hit = ctx:elapsed()
            self.health = math.max(0, self.health - amount)
            self:mirror(ctx)
            ctx:send("damaged", amount)
            self:sound(ctx, self.params.hurt_sound)
            if self.health <= 0 then self:die(ctx) end
        elseif name == "heal" then
            if self.dead or amount <= 0 then return end
            self.health = math.min(self.params.max_health, self.health + amount)
            self:mirror(ctx)
            ctx:send("healed", amount)
        elseif name == "kill" then
            if not self.dead then self:die(ctx) end
        elseif name == "revive" then
            self:revive(ctx)
        end
    end,

    update = function(self, ctx)
        if self.respawn_at and ctx:elapsed() >= self.respawn_at then
            self:revive(ctx)
            ctx:send("respawn", ctx:entity())
        end
    end,
}

-- Spawner: makes instances of a prefab at random points around itself.
--
-- mode "interval": one instance every `interval` seconds
-- mode "message":  only on the message "spawn" (one instance per message)
-- mode "waves":    `wave_size` instances at once; the next wave comes
--                  `wave_pause` seconds after the last one of the previous wave
--                  is gone; `waves` waves in all (0 = endless), then it
--                  broadcasts "waves_done"
-- It never keeps more than `max_alive` of its instances alive. Messages: "spawn"
-- (one now), "start" / "stop" (the timer), "clear" (remove every instance).
-- The engine answers every spawn with a "spawned" message to the spawner (value
-- = the new instance's id); that is how the spawner counts what is alive.
-- Broadcasts "wave_started" (value = wave number) and "waves_done".
--
-- Uses ctx:spawn_prefab: the prefab path must lie under assets/; the engine
-- caps spawns per frame and in total (256 alive).
return {
    params = {
        { name = "prefab", kind = "string", default = "prefabs/spawners/Crate.prefab.ron", doc = "prefab to spawn (path under assets/)" },
        { name = "mode", kind = "string", default = "interval", doc = "interval, message or waves" },
        { name = "interval", kind = "float", default = 2.0, doc = "interval mode: seconds between spawns" },
        { name = "max_alive", kind = "int", default = 5, doc = "most instances alive at once" },
        { name = "radius", kind = "float", default = 3.0, doc = "spawn points are inside this circle (m) around the spawner" },
        { name = "y_offset", kind = "float", default = 0.5, doc = "height above the spawner, m" },
        { name = "random_yaw", kind = "bool", default = true, doc = "turn each instance by a random angle" },
        { name = "wave_size", kind = "int", default = 3, doc = "waves mode: instances per wave" },
        { name = "waves", kind = "int", default = 0, doc = "waves mode: number of waves, 0 = endless" },
        { name = "wave_pause", kind = "float", default = 3.0, doc = "waves mode: seconds between waves" },
        { name = "autostart", kind = "bool", default = true, doc = "run from the start (else wait for \"start\")" },
        { name = "seed", kind = "int", default = 0, doc = "random seed; 0 = from the entity id" },
    },

    start = function(self, ctx)
        local seed = self.params.seed
        self.rng = (seed ~= 0 and seed or ctx:entity()) % 2147483647
        if self.rng <= 0 then self.rng = 1 end
        self.alive = {}
        self.pending = 0
        self.running = self.params.autostart
        self.next_at = ctx:elapsed() + (self.params.mode == "interval" and self.params.interval or 0)
        self.wave = 0
        self.wave_open = false
    end,

    random = function(self)
        -- Park-Miller: small, deterministic, no state shared with other scripts.
        self.rng = (self.rng * 48271) % 2147483647
        return self.rng / 2147483647
    end,

    count = function(self, ctx)
        local kept = {}
        for _, id in ipairs(self.alive) do
            if ctx:alive(id) then kept[#kept + 1] = id end
        end
        self.alive = kept
        return #kept + self.pending
    end,

    spawn_one = function(self, ctx)
        local p = self.params
        if self:count(ctx) >= p.max_alive then return false end
        local cx, cy, cz = ctx:world_position_xyz()
        local angle = self:random() * 2 * math.pi
        local r = p.radius * math.sqrt(self:random())
        local yaw = p.random_yaw and self:random() * 360 or 0
        if ctx:spawn_prefab(p.prefab, cx + math.cos(angle) * r, cy + p.y_offset, cz + math.sin(angle) * r, yaw) then
            self.pending = self.pending + 1
            return true
        end
        return false
    end,

    on_message = function(self, ctx, name, value, from)
        if name == "spawned" then
            self.pending = math.max(0, self.pending - 1)
            if math.type(value) == "integer" then self.alive[#self.alive + 1] = value end
        elseif name == "spawn" then
            self:spawn_one(ctx)
        elseif name == "start" then
            self.running = true
        elseif name == "stop" then
            self.running = false
        elseif name == "clear" then
            for _, id in ipairs(self.alive) do ctx:despawn(id) end
            self.alive = {}
        end
    end,

    update = function(self, ctx)
        if not self.running then return end
        local p, now = self.params, ctx:elapsed()
        if p.mode == "interval" then
            if now >= self.next_at then
                self.next_at = now + p.interval
                self:spawn_one(ctx)
            end
        elseif p.mode == "waves" then
            if p.waves > 0 and self.wave >= p.waves and self:count(ctx) == 0 then
                if not self.done then
                    self.done = true
                    ctx:send("waves_done", ctx:name())
                end
                return
            end
            if self.wave_open then
                -- Between waves: wait until the field is clear, then the pause.
                if self:count(ctx) == 0 then
                    self.wave_open = false
                    self.next_at = now + p.wave_pause
                end
            elseif now >= self.next_at and (p.waves == 0 or self.wave < p.waves) then
                self.wave = self.wave + 1
                self.wave_open = true
                for _ = 1, p.wave_size do self:spawn_one(ctx) end
                ctx:send("wave_started", self.wave)
            end
        end
    end,
}

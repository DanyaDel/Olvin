-- Beacon: spins, bobs, and greets whoever walks into its trigger sphere with
-- a chirp (positional, at the beacon) and a HUD message.
return {
    params = {
        { name = "speed", kind = "float", default = 90, doc = "spin, degrees per second" },
        { name = "bob", kind = "float", default = 0.15, doc = "bob height, metres" },
    },

    start = function(self, ctx)
        self.base = ctx:position()
        self.t = 0
        self.visits = 0
        ctx:log("beacon online")
    end,

    update = function(self, ctx)
        local dt = ctx:dt()
        self.t = self.t + dt
        ctx:rotate_euler_deg_xyz(0, self.params.speed * dt, 0)
        local y = self.base.y + math.sin(self.t * 2) * self.params.bob
        ctx:set_position_xyz(self.base.x, y, self.base.z)
    end,

    on_trigger_enter = function(self, ctx, other)
        if other == ctx:find_by_name("Player") then
            self.visits = self.visits + 1
            ctx:play_sound_at("sfx/thud.wav", ctx:entity(), { pitch = 1.8, volume = 0.6, max_distance = 15 })
            ctx:send("hud", "Маяк: привет (" .. self.visits .. ")")
        end
    end,
}

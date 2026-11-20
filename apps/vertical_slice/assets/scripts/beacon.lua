-- Beacon: spins, bobs, and greets whoever walks into its trigger sphere.
-- Lua sees only its own entity's transform, lookups by name, physics and input:
-- no sound, no HUD, no game resources (see GAPS.md).
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
        self.t = self.t + ctx:dt()
        ctx:rotate_euler_deg(vec3(0, self.params.speed * ctx:dt(), 0))
        local y = self.base.y + math.sin(self.t * 2) * self.params.bob
        ctx:set_position(vec3(self.base.x, y, self.base.z))
    end,

    on_trigger_enter = function(self, ctx, other)
        if other == ctx:find_by_name("Player") then
            self.visits = self.visits + 1
            ctx:log("beacon: hello, player (visit " .. self.visits .. ")")
        end
    end,
}

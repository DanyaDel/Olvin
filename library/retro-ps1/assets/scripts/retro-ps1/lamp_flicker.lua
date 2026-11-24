-- Flickering lamp for the retro-ps1 demo. Put it on a Point Light entity.
--
-- Scripts can't change a light's intensity (yet), so the lamp blinks the old
-- way: while it is "off" the light entity is parked far below the level, out
-- of everyone's range, and then returns to where you put it. In vertex-lit
-- (Gouraud) mode this gives the hard on/off pops a PS1 flickering tube has.

return {
    params = {
        { name = "flicker_rate", kind = "float", default = 9.0, doc = "decisions per second: higher = faster flicker" },
        { name = "outage", kind = "float", default = 0.3, doc = "0..1: share of the time the lamp is off" },
    },

    start = function(self, ctx)
        self.home = ctx:position()
        self.off = false
    end,

    update = function(self, ctx)
        local p = self.params
        -- A hash of the current time slot: steady within a slot, jumpy between.
        local slot = math.floor(ctx:elapsed() * p.flicker_rate)
        local n = math.sin(slot * 12.9898) * 43758.5453
        n = n - math.floor(n)
        local off = n < p.outage
        if off ~= self.off then
            self.off = off
            if off then
                ctx:set_position(self.home.x, self.home.y - 500, self.home.z)
            else
                ctx:set_position(self.home.x, self.home.y, self.home.z)
            end
        end
    end,
}

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

-- Follows the entity named `target` with a delay: an exponential catch-up, so it
-- slows down smoothly when the target stops. Moves this entity's own position
-- (a camera rig, a drone, a pet); a parent may move but must not be rotated.
-- `offset` is added to the target's world position.
return {
    params = {
        { name = "target", kind = "string", default = "Player", doc = "name of the entity to follow" },
        { name = "offset", kind = "vec3", default = vec3(0, 2, 4), doc = "from the target to this entity, world axes, metres" },
        { name = "smoothing", kind = "float", default = 5.0, doc = "catch-up speed, 1/s (higher = tighter, 0 = jump at once)" },
        { name = "axes", kind = "string", default = "xyz", doc = "which axes follow, e.g. \"xz\" to keep the height" },
    },

    start = function(self, ctx)
        self.id = ctx:find_by_name(self.params.target)
        if not self.id then ctx:warn("no entity named `" .. self.params.target .. "`") end
    end,

    update = function(self, ctx)
        if not self.id or not ctx:alive(self.id) then
            self.id = ctx:find_by_name(self.params.target)
            if not self.id then return end
        end
        local p = self.params
        local tx, ty, tz = ctx:world_position_of_xyz(self.id)
        local gx, gy, gz = tx + p.offset.x, ty + p.offset.y, tz + p.offset.z
        local x, y, z = ctx:position_xyz()
        -- The goal is in world space: shift by the parent's position.
        local wx, wy, wz = ctx:world_position_xyz()
        gx, gy, gz = gx - (wx - x), gy - (wy - y), gz - (wz - z)
        local k = 1
        if p.smoothing > 0 then k = 1 - math.exp(-p.smoothing * ctx:dt()) end
        local a = p.axes
        if string.find(a, "x", 1, true) then x = x + (gx - x) * k end
        if string.find(a, "y", 1, true) then y = y + (gy - y) * k end
        if string.find(a, "z", 1, true) then z = z + (gz - z) * k end
        ctx:set_position_xyz(x, y, z)
    end,
}

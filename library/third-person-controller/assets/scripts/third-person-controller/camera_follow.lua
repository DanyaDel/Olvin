-- Orbit camera that follows a target. Reusable: put it on any camera.
--
-- Two ways to use it:
--   * target = ""        the camera is a CHILD of the thing it orbits (the
--                        third-person prefab does this): it keeps a position
--                        around its parent and never lags.
--   * target = "Name"    a free camera that follows the entity called Name
--                        and smooths its movement (`smoothing`).
-- The mouse / right stick (the `Look` action) orbits and tilts the camera.
-- There is no collision: the camera does not stop at walls.

local function clamp(v, lo, hi)
    if v < lo then return lo end
    if v > hi then return hi end
    return v
end

return {
    params = {
        { name = "target", kind = "string", default = "", doc = "entity to follow; empty = orbit the parent" },
        { name = "distance", kind = "float", default = 5.0, doc = "metres behind the pivot" },
        { name = "height", kind = "float", default = 0.6, doc = "pivot height above the target's origin, m" },
        { name = "sensitivity", kind = "float", default = 0.12, doc = "degrees per mouse pixel" },
        { name = "min_pitch", kind = "float", default = -55, doc = "lowest look angle, degrees (negative = looking down)" },
        { name = "max_pitch", kind = "float", default = 25, doc = "highest look angle, degrees" },
        { name = "start_pitch", kind = "float", default = -15, doc = "initial look angle, degrees" },
        { name = "smoothing", kind = "float", default = 12.0, doc = "follow speed (1/s) for a free camera; 0 = rigid" },
        { name = "invert_y", kind = "bool", default = false, doc = "invert the vertical mouse axis" },
        { name = "lock_cursor", kind = "bool", default = true, doc = "grab the mouse cursor while playing" },
    },

    start = function(self, ctx)
        local _, yaw = ctx:rotation_euler_deg_xyz()
        self.yaw = yaw
        self.pitch = self.params.start_pitch
        if self.params.target ~= "" then
            self.target = ctx:find_by_name(self.params.target)
            if not self.target then
                ctx:warn("no entity named `" .. self.params.target .. "` to follow")
            end
        end
        if self.params.lock_cursor then
            ctx:set_cursor_locked(true)
        end
    end,

    update = function(self, ctx)
        local p = self.params
        local lx, ly = ctx:axis2d_xy("Look")
        self.yaw = self.yaw - lx * p.sensitivity
        self.pitch = clamp(self.pitch + ly * p.sensitivity * (p.invert_y and -1 or 1), p.min_pitch, p.max_pitch)

        -- The camera sits on a sphere around the pivot and looks at it.
        local yaw, pitch = math.rad(self.yaw), math.rad(self.pitch)
        local cp = math.cos(pitch)
        local ox = p.distance * cp * math.sin(yaw)
        local oy = -p.distance * math.sin(pitch)
        local oz = p.distance * cp * math.cos(yaw)

        if p.target == "" then
            ctx:set_position_xyz(ox, p.height + oy, oz)
        elseif self.target and ctx:alive(self.target) then
            local tx, ty, tz = ctx:world_position_of_xyz(self.target)
            local wx, wy, wz = tx + ox, ty + p.height + oy, tz + oz
            if p.smoothing > 0 then
                local x, y, z = ctx:position_xyz()
                local k = 1 - math.exp(-p.smoothing * ctx:dt())
                wx, wy, wz = x + (wx - x) * k, y + (wy - y) * k, z + (wz - z) * k
            end
            ctx:set_position_xyz(wx, wy, wz)
        end
        ctx:set_rotation_euler_deg_xyz(self.pitch, self.yaw, 0)
    end,
}

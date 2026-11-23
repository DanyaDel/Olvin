-- Turns this entity to face the entity named `target` (a camera on a target, a
-- turret, a sign, an NPC's head). The aim point follows the target with
-- `smoothing`, so the turn is not instant. `yaw_only` keeps it upright and only
-- turns it around Y. The parent may move but must not be rotated or scaled.
return {
    params = {
        { name = "target", kind = "string", default = "Player", doc = "name of the entity to look at" },
        { name = "offset", kind = "vec3", default = vec3(0, 0, 0), doc = "aim this far from the target's position (world axes), e.g. (0, 1, 0) for the head" },
        { name = "smoothing", kind = "float", default = 8.0, doc = "how fast the aim point catches up, 1/s; 0 = turn at once" },
        { name = "yaw_only", kind = "bool", default = false, doc = "only turn around the Y axis" },
    },

    start = function(self, ctx)
        self.id = ctx:find_by_name(self.params.target)
        if not self.id then ctx:warn("no entity named `" .. self.params.target .. "`") end
        self.aim = nil
    end,

    update = function(self, ctx)
        if not self.id or not ctx:alive(self.id) then
            self.id = ctx:find_by_name(self.params.target)
            if not self.id then return end
        end
        local p = self.params
        local tx, ty, tz = ctx:world_position_of_xyz(self.id)
        tx, ty, tz = tx + p.offset.x, ty + p.offset.y, tz + p.offset.z
        local aim = self.aim
        if aim == nil or p.smoothing <= 0 then
            aim = { tx, ty, tz }
        else
            local k = 1 - math.exp(-p.smoothing * ctx:dt())
            aim = { aim[1] + (tx - aim[1]) * k, aim[2] + (ty - aim[2]) * k, aim[3] + (tz - aim[3]) * k }
        end
        self.aim = aim
        -- look_at wants the parent's space: shift by the parent's position.
        local wx, wy, wz = ctx:world_position_xyz()
        local lx, ly, lz = ctx:position_xyz()
        local ox, oy, oz = wx - lx, wy - ly, wz - lz
        if p.yaw_only then
            ctx:look_at_xyz(aim[1] - ox, ly, aim[3] - oz)
        else
            ctx:look_at_xyz(aim[1] - ox, aim[2] - oy, aim[3] - oz)
        end
    end,
}

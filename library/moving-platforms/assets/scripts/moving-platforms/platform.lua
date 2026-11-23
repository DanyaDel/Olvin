-- Moving platform. Put it on the root of a prefab with these children:
--   Body              the solid platform (collider + mesh, NO Rigidbody) that moves
--   Point1 .. PointN  empty markers it travels between, in this order
-- The markers do not move (they are the root's children, not the body's), so
-- you can place them in the editor and see the route. Keep the root unrotated
-- and unscaled.
--
-- `mode` pingpong goes 1 -> N -> 1; loop goes 1 -> N -> 1 -> N ... in a ring.
-- It waits `wait` seconds at every point. Messages: "start", "stop".
--
-- Why no Rigidbody: the physics engine teleports a collider that is moved through
-- its Transform, and a Character Controller walks on it normally. On a Kinematic
-- body it would stall (the controller's moving-platform friction gets stuck), and
-- it would not be carried either. Things that are not the player (crates) are not
-- carried. So, riders: a character is not carried by the platform itself, so
-- while the player stands on top of the body (feet within a hand's width of
-- its top, inside its footprint) the platform moves the player by the same
-- step it moves itself. `feet_offset` is the distance from the player's origin
-- to its feet (0.9 for a 1.8 m Character Controller). With `carry` off the
-- player is left behind.
return {
    params = {
        { name = "speed", kind = "float", default = 2.0, doc = "metres per second" },
        { name = "wait", kind = "float", default = 1.0, doc = "seconds to wait at each point" },
        { name = "mode", kind = "string", default = "pingpong", doc = "pingpong or loop" },
        { name = "autostart", kind = "bool", default = true, doc = "start moving at once (else wait for the message \"start\")" },
        { name = "carry", kind = "bool", default = true, doc = "carry the player standing on it" },
        { name = "body", kind = "string", default = "Body", doc = "name of the moving child" },
        { name = "player", kind = "string", default = "Player", doc = "name of the player (an entity tagged Player counts too)" },
        { name = "feet_offset", kind = "float", default = 0.9, doc = "distance from the player's origin down to its feet, m" },
    },

    start = function(self, ctx)
        self.body = ctx:find_child(self.params.body)
        self.points = {}
        local i = 1
        while true do
            local e = ctx:find_child("Point" .. i)
            if not e then break end
            local x, y, z = ctx:position_of_xyz(e)
            self.points[i] = { x, y, z }
            i = i + 1
        end
        if not self.body or #self.points < 2 then
            ctx:warn("needs a child `" .. self.params.body .. "` and at least Point1 and Point2")
            self.body = nil
            return
        end
        local p = self.points[1]
        ctx:set_position_of_xyz(self.body, p[1], p[2], p[3])
        self.at, self.dir, self.running = 1, 1, self.params.autostart
        self.wait_until = ctx:elapsed() + self.params.wait
    end,

    on_message = function(self, ctx, name)
        if name == "start" then self.running = true elseif name == "stop" then self.running = false end
    end,

    next_index = function(self)
        local n = #self.points
        if self.params.mode == "loop" then return self.at % n + 1 end
        if self.dir == 1 and self.at == n then self.dir = -1 end
        if self.dir == -1 and self.at == 1 then self.dir = 1 end
        return self.at + self.dir
    end,

    rider = function(self, ctx)
        local who = ctx:find_by_name(self.params.player) or ctx:find_by_tag("Player")
        if not who then return nil end
        local bx, by, bz = ctx:world_position_of_xyz(self.body)
        local sx, sy, sz = ctx:scale_of_xyz(self.body)
        local px, py, pz = ctx:world_position_of_xyz(who)
        local rise = (py - self.params.feet_offset) - (by + sy / 2)
        if math.abs(px - bx) <= sx / 2 + 0.1 and math.abs(pz - bz) <= sz / 2 + 0.1
            and rise > -0.25 and rise < 0.35 then
            return who
        end
        return nil
    end,

    update = function(self, ctx)
        if not self.body or not self.running or ctx:elapsed() < self.wait_until then return end
        local nxt = self:next_index()
        local target = self.points[nxt]
        local cx, cy, cz = ctx:position_of_xyz(self.body)
        local dx, dy, dz = target[1] - cx, target[2] - cy, target[3] - cz
        local dist = math.sqrt(dx * dx + dy * dy + dz * dz)
        local step = self.params.speed * ctx:dt()
        local mx, my, mz
        if step >= dist then
            mx, my, mz = dx, dy, dz
            self.at = nxt
            self.wait_until = ctx:elapsed() + self.params.wait
        else
            mx, my, mz = dx / dist * step, dy / dist * step, dz / dist * step
        end
        local who = self.params.carry and self:rider(ctx) or nil
        ctx:set_position_of_xyz(self.body, cx + mx, cy + my, cz + mz)
        if who then ctx:translate_of_xyz(who, mx, my, mz) end
    end,
}

-- Top-down / isometric character mover for a Character Controller entity.
--
-- WASD / left stick walks in 8 directions (any angle on a stick). Screen "up"
-- is world -Z turned by `camera_yaw`: 0 for a straight top-down view, 45 for
-- an isometric one (turn the camera the same way). The `Body` child turns
-- smoothly towards where you walk, or, with `aim_with_look`, towards where the
-- right stick points. Space jumps when `jump_height` is above 0, Shift sprints.
-- The root never rotates, so a camera parented to it follows along.
--
-- Actions used: Move, Look (optional aim), Jump, Sprint (the engine's default map).
-- Mouse aiming needs a screen-to-world ray, which the Lua API does not have yet.

local function wrap(deg)
    deg = deg % 360
    if deg > 180 then deg = deg - 360 end
    return deg
end

return {
    params = {
        { name = "speed", kind = "float", default = 5.0, doc = "walk speed, m/s" },
        { name = "sprint_speed", kind = "float", default = 8.0, doc = "speed while Sprint is held, m/s" },
        { name = "acceleration", kind = "float", default = 20.0, doc = "how fast speed changes (1/s); 0 = instant" },
        { name = "turn_smoothing", kind = "float", default = 18.0, doc = "how fast the body turns (1/s); 0 = instant" },
        { name = "camera_yaw", kind = "float", default = 0.0, doc = "which way is screen-up, degrees (45 = isometric)" },
        { name = "snap_to_8", kind = "bool", default = false, doc = "face only the 8 compass directions" },
        { name = "aim_with_look", kind = "bool", default = false, doc = "face the right stick (Look) while it is pushed" },
        { name = "jump_height", kind = "float", default = 0.0, doc = "metres; 0 = no jumping" },
        { name = "body", kind = "string", default = "Body", doc = "name of the child that turns" },
    },

    start = function(self, ctx)
        self.vx, self.vz = 0, 0
        self.facing = 0
        self.body = ctx:find_child(self.params.body)
        if not self.body then
            ctx:warn("no child named `" .. self.params.body .. "`: the character will not visibly turn")
        end
        if ctx:motor_velocity_xyz() == nil then
            ctx:warn("needs a Character Controller component on this entity")
        end
    end,

    update = function(self, ctx)
        local p, dt = self.params, ctx:dt()
        local yaw = math.rad(p.camera_yaw)
        local sin, cos = math.sin(yaw), math.cos(yaw)

        local mx, my = ctx:axis2d_xy("Move")
        local len = math.sqrt(mx * mx + my * my)
        if len > 1 then mx, my, len = mx / len, my / len, 1 end
        local dx = mx * cos - my * sin
        local dz = -mx * sin - my * cos

        local speed = ctx:pressed("Sprint") and p.sprint_speed or p.speed
        local _, vy = ctx:motor_velocity_xyz()
        vy = vy or 0
        local blend = 1
        if p.acceleration > 0 then blend = 1 - math.exp(-p.acceleration * dt) end
        self.vx = self.vx + (dx * speed - self.vx) * blend
        self.vz = self.vz + (dz * speed - self.vz) * blend

        if p.jump_height > 0 and ctx:grounded() and ctx:just_pressed("Jump") then
            vy = math.sqrt(2 * ctx:gravity() * p.jump_height)
        end
        ctx:set_motor_velocity_xyz(self.vx, vy, self.vz)

        -- Facing: the right stick if asked for, else the walking direction.
        if self.body then
            local fx, fz, active = dx, dz, len > 0.1
            if p.aim_with_look then
                local lx, ly = ctx:axis2d_xy("Look")
                if lx * lx + ly * ly > 9 then
                    -- Look is (right, up) like Move; mouse movement also lands here.
                    fx = lx * cos - ly * sin
                    fz = -lx * sin - ly * cos
                    active = true
                end
            end
            if active then
                local want = math.deg(math.atan(-fx, -fz))
                if p.snap_to_8 then want = math.floor(want / 45 + 0.5) * 45 end
                local diff = wrap(want - self.facing)
                local k = p.turn_smoothing > 0 and (1 - math.exp(-p.turn_smoothing * dt)) or 1
                self.facing = wrap(self.facing + diff * k)
                ctx:set_rotation_euler_deg_of_xyz(self.body, 0, self.facing, 0)
            end
        end
    end,
}

-- Third-person character controller for a Character Controller entity.
--
-- WASD / left stick moves relative to where the camera looks, the `Body`
-- child turns smoothly towards the walking direction, Space jumps, Shift
-- sprints. The camera is orbited by camera_follow.lua (a child named
-- `Camera` in the prefab; this script only reads which way it faces).
-- The root entity never rotates, so a camera parented to it keeps its place.
--
-- Actions used: Move, Jump, Sprint (the engine's default map).

local function wrap(deg)
    deg = deg % 360
    if deg > 180 then deg = deg - 360 end
    return deg
end

return {
    params = {
        { name = "speed", kind = "float", default = 4.5, doc = "walk speed, m/s" },
        { name = "sprint_speed", kind = "float", default = 7.5, doc = "speed while Sprint is held, m/s (same as speed = no sprint)" },
        { name = "jump_height", kind = "float", default = 1.3, doc = "metres; 0 turns jumping off" },
        { name = "acceleration", kind = "float", default = 14.0, doc = "how fast speed changes (1/s); 0 = instant" },
        { name = "air_control", kind = "float", default = 0.4, doc = "0..1: steering while in the air" },
        { name = "turn_smoothing", kind = "float", default = 14.0, doc = "how fast the body turns (1/s); 0 = instant" },
        { name = "camera", kind = "string", default = "Camera", doc = "name of the camera (child or anywhere in the scene)" },
        { name = "body", kind = "string", default = "Body", doc = "name of the child that turns to face the walking direction" },
    },

    start = function(self, ctx)
        self.vx, self.vz = 0, 0
        self.facing = 0
        self.camera = ctx:find_child(self.params.camera) or ctx:find_by_name(self.params.camera)
        self.body = ctx:find_child(self.params.body)
        if not self.camera then
            ctx:warn("no camera named `" .. self.params.camera .. "`: moving relative to the world axes")
        end
        if not self.body then
            ctx:warn("no child named `" .. self.params.body .. "`: the character will not visibly turn")
        end
        if ctx:motor_velocity_xyz() == nil then
            ctx:warn("needs a Character Controller component on this entity")
        end
    end,

    update = function(self, ctx)
        local p, dt = self.params, ctx:dt()

        -- Which way is "forward": where the camera looks, flattened.
        local cam_yaw = 0
        if self.camera then
            local _, y = ctx:rotation_euler_deg_of_xyz(self.camera)
            cam_yaw = y or 0
        end
        local yaw = math.rad(cam_yaw)
        local sin, cos = math.sin(yaw), math.cos(yaw)

        local mx, my = ctx:axis2d_xy("Move")
        local len = math.sqrt(mx * mx + my * my)
        if len > 1 then mx, my, len = mx / len, my / len, 1 end
        local dx = mx * cos - my * sin
        local dz = -mx * sin - my * cos

        local speed = ctx:pressed("Sprint") and p.sprint_speed or p.speed
        local _, vy = ctx:motor_velocity_xyz()
        vy = vy or 0
        local grounded = ctx:grounded()
        local blend = 1
        if p.acceleration > 0 then blend = 1 - math.exp(-p.acceleration * dt) end
        if not grounded then blend = blend * p.air_control end
        self.vx = self.vx + (dx * speed - self.vx) * blend
        self.vz = self.vz + (dz * speed - self.vz) * blend

        if grounded and p.jump_height > 0 and ctx:just_pressed("Jump") then
            vy = math.sqrt(2 * ctx:gravity() * p.jump_height)
        end
        ctx:set_motor_velocity_xyz(self.vx, vy, self.vz)

        -- Turn the body towards where we are going (its forward is -Z).
        if self.body and len > 0.1 then
            local want = math.deg(math.atan(-dx, -dz))
            local diff = wrap(want - self.facing)
            local k = p.turn_smoothing > 0 and (1 - math.exp(-p.turn_smoothing * dt)) or 1
            self.facing = wrap(self.facing + diff * k)
            ctx:set_rotation_euler_deg_of_xyz(self.body, 0, self.facing, 0)
        end
    end,
}

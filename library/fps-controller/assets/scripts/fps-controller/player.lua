-- First-person controller for a Character Controller entity.
--
-- Put it on the body (the entity with the Character Controller component);
-- the camera is a child of the body. Mouse look turns the body left/right
-- and tilts the camera, WASD / left stick walks, Shift sprints, Space jumps.
-- Optional: `Crouch` (add an action with that name to input.ron) lowers the
-- camera and slows you down, `head_bob` swings the camera while walking.
--
-- Actions used: Move, Look, Jump, Sprint (the engine's default map) and the
-- optional Crouch. Everything below `params` shows up in the Inspector.

local function clamp(v, lo, hi)
    if v < lo then return lo end
    if v > hi then return hi end
    return v
end

return {
    params = {
        { name = "speed", kind = "float", default = 4.0, doc = "walk speed, m/s" },
        { name = "sprint_speed", kind = "float", default = 7.0, doc = "speed while Sprint is held, m/s" },
        { name = "crouch_speed", kind = "float", default = 2.0, doc = "speed while Crouch is held, m/s" },
        { name = "jump_height", kind = "float", default = 1.2, doc = "metres" },
        { name = "sensitivity", kind = "float", default = 0.12, doc = "degrees per mouse pixel" },
        { name = "max_pitch", kind = "float", default = 85, doc = "how far up and down you can look, degrees" },
        { name = "acceleration", kind = "float", default = 12.0, doc = "how fast speed changes (1/s); 0 = instant" },
        { name = "air_control", kind = "float", default = 0.35, doc = "0..1: steering while in the air" },
        { name = "camera", kind = "string", default = "Camera", doc = "name of the child camera" },
        { name = "lock_cursor", kind = "bool", default = true, doc = "grab the mouse cursor for mouse look" },
        { name = "crouch_depth", kind = "float", default = 0.5, doc = "how far the camera drops when crouching, m" },
        { name = "head_bob", kind = "float", default = 0.0, doc = "camera bob height while walking, m (0 = off, try 0.04)" },
    },

    start = function(self, ctx)
        local _, yaw = ctx:rotation_euler_deg_xyz()
        self.yaw = yaw
        self.pitch = 0
        self.vx, self.vz = 0, 0
        self.crouch = 0
        self.bob = 0
        self.camera = ctx:find_child(self.params.camera)
        if self.camera then
            local _, y = ctx:position_of_xyz(self.camera)
            self.eye = y
        else
            ctx:warn("no child named `" .. self.params.camera .. "`: looking up and down is off")
        end
        if ctx:motor_velocity_xyz() == nil then
            ctx:warn("needs a Character Controller component on this entity")
        end
        if self.params.lock_cursor then
            ctx:set_cursor_locked(true)
        end
    end,

    update = function(self, ctx)
        local p, dt = self.params, ctx:dt()

        -- Look: the body turns (yaw), the camera tilts (pitch).
        local lx, ly = ctx:axis2d_xy("Look")
        self.yaw = self.yaw - lx * p.sensitivity
        self.pitch = clamp(self.pitch + ly * p.sensitivity, -p.max_pitch, p.max_pitch)
        ctx:set_rotation_euler_deg_xyz(0, self.yaw, 0)

        -- Walk: Move is (right, forward) in -1..1.
        local mx, my = ctx:axis2d_xy("Move")
        local len = math.sqrt(mx * mx + my * my)
        if len > 1 then mx, my = mx / len, my / len end
        local yaw = math.rad(self.yaw)
        local sin, cos = math.sin(yaw), math.cos(yaw)
        -- forward = (-sin, -cos), right = (cos, -sin) in x/z.
        local dx = mx * cos - my * sin
        local dz = -mx * sin - my * cos

        local crouching = ctx:pressed("Crouch")
        local speed = p.speed
        if crouching then
            speed = p.crouch_speed
        elseif ctx:pressed("Sprint") then
            speed = p.sprint_speed
        end

        local _, vy = ctx:motor_velocity_xyz()
        vy = vy or 0
        local grounded = ctx:grounded()
        local blend = 1
        if p.acceleration > 0 then
            blend = 1 - math.exp(-p.acceleration * dt)
        end
        if not grounded then
            blend = blend * p.air_control
        end
        self.vx = self.vx + (dx * speed - self.vx) * blend
        self.vz = self.vz + (dz * speed - self.vz) * blend

        if grounded and ctx:just_pressed("Jump") then
            vy = math.sqrt(2 * ctx:gravity() * p.jump_height)
        end
        ctx:set_motor_velocity_xyz(self.vx, vy, self.vz)

        -- Camera: pitch, crouch drop and an optional bob.
        if self.camera then
            local target = crouching and 1 or 0
            self.crouch = self.crouch + (target - self.crouch) * (1 - math.exp(-10 * dt))
            local y = self.eye - self.crouch * p.crouch_depth
            if p.head_bob > 0 and grounded then
                local moving = math.sqrt(self.vx * self.vx + self.vz * self.vz)
                self.bob = self.bob + moving * dt * 2.2
                y = y + math.sin(self.bob) * p.head_bob * clamp(moving / p.speed, 0, 1)
            end
            local px, _, pz = ctx:position_of_xyz(self.camera)
            ctx:set_position_of_xyz(self.camera, px, y, pz)
            ctx:set_rotation_euler_deg_of_xyz(self.camera, self.pitch, 0, 0)
        end
    end,
}

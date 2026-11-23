-- Camera shake. Put it on the camera. The message "shake" adds trauma:
--   value = a number 0..1 (how hard), or vec3(strength, seconds, 0) to also
--   choose how long it takes to die down (default `duration`).
-- Trauma decays linearly to zero; the offset grows with its square, so small
-- shakes are subtle. `listen` names a message that shakes it by `listen_strength`
-- by itself (for example "damaged" from health.lua).
--
-- It shakes the camera's local x and y by up to `max_offset` metres and puts
-- the exact original position back when it is done. It keeps track of what it
-- wrote: if another script moved the position since (fps-controller sets the
-- camera height every frame) that value becomes the new rest position. The
-- vertical part of the shake can be overwritten by such a script when it runs
-- after this one; the sideways part always shows.
return {
    params = {
        { name = "max_offset", kind = "float", default = 0.15, doc = "largest offset at full trauma, metres" },
        { name = "duration", kind = "float", default = 0.6, doc = "seconds a full trauma takes to die down" },
        { name = "frequency", kind = "float", default = 28, doc = "shake speed, Hz" },
        { name = "listen", kind = "string", default = "", doc = "also shake on this message (empty = only \"shake\")" },
        { name = "listen_strength", kind = "float", default = 0.4, doc = "trauma added by the `listen` message" },
    },

    start = function(self, ctx)
        self.trauma = 0
        self.rate = 1 / math.max(self.params.duration, 0.01)
        self.t = 0
        self.wrote = nil -- the position we last set: x, y, z
        self.base = nil
    end,

    on_message = function(self, ctx, name, value)
        if name == "shake" then
            local strength = 0
            if type(value) == "number" then
                strength = value
            elseif type(value) == "table" then
                strength = value.x
                if value.y > 0 then self.rate = 1 / value.y end
            end
            self.trauma = math.min(1, self.trauma + math.max(0, strength))
        elseif self.params.listen ~= "" and name == self.params.listen then
            self.trauma = math.min(1, self.trauma + self.params.listen_strength)
        end
    end,

    update = function(self, ctx)
        local x, y, z = ctx:position_xyz()
        local w = self.wrote
        if self.base == nil then
            self.base = { x, y, z }
        else
            -- A value that is not what we wrote was set by someone else: it is
            -- the new rest value of that axis.
            if w == nil or math.abs(x - w[1]) > 1e-4 then self.base[1] = x end
            if w == nil or math.abs(y - w[2]) > 1e-4 then self.base[2] = y end
            self.base[3] = z
        end
        local base = self.base
        local dt = ctx:dt()
        if self.trauma <= 0 then
            if w ~= nil then
                ctx:set_position_xyz(base[1], base[2], base[3])
                self.wrote = nil
            end
            return
        end
        self.t = self.t + dt
        self.trauma = math.max(0, self.trauma - dt * self.rate)
        local k = self.trauma * self.trauma * self.params.max_offset
        local f = self.t * self.params.frequency
        local ox = math.sin(f * 1.0 + 1.3) * math.cos(f * 0.37) * k
        local oy = math.sin(f * 1.7 + 4.1) * math.cos(f * 0.53 + 2.0) * k
        local nx, ny = base[1] + ox, base[2] + oy
        ctx:set_position_xyz(nx, ny, base[3])
        self.wrote = { nx, ny, base[3] }
        if self.trauma <= 0 then
            ctx:set_position_xyz(base[1], base[2], base[3])
            self.wrote = nil
        end
    end,
}

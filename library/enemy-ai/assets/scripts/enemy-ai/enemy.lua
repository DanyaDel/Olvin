-- A simple enemy for an entity with a Character Controller: patrol, spot the
-- player, chase, attack, give up and go back to patrolling.
--
-- Why the character controller: it gives gravity, steps, slopes and sliding
-- along walls for free; the script only sets the horizontal motor velocity.
--
-- patrol: where to walk, in order and back, `;` separated. An item is either
--   world coordinates "x,z" or the name of an entity ("PatrolA"). Empty =
--   stand still. It waits `patrol_wait` seconds at each point.
-- sight: the player counts as seen when closer than `sight_range`, inside the
--   `fov` cone (360 = all around) and not hidden behind something solid (a
--   ray from the enemy's eyes). A chased player is not lost before he is
--   farther than `lose_range` or unseen for `lose_time` seconds.
-- attack: within `attack_range` it sends "damage" (value `attack_damage`) to the
--   player every `attack_cooldown` seconds (the health-damage pack's health.lua
--   takes it). It stops while the player is dead ("died" / "revived").
-- Broadcasts "enemy_alert" (starts chasing) and "enemy_lost" (gave up), value =
-- its name.
local function split(s, sep)
    local out = {}
    for part in string.gmatch(s, "[^" .. sep .. "]+") do out[#out + 1] = part end
    return out
end

return {
    params = {
        { name = "patrol", kind = "string", default = "", doc = "points to walk between: \"x,z;x,z\" or entity names \"A;B\"" },
        { name = "patrol_speed", kind = "float", default = 2.0, doc = "m/s while patrolling" },
        { name = "patrol_wait", kind = "float", default = 1.0, doc = "seconds to wait at a point" },
        { name = "chase_speed", kind = "float", default = 4.5, doc = "m/s while chasing (the player walks at 4)" },
        { name = "sight_range", kind = "float", default = 12.0, doc = "how far it sees, m" },
        { name = "fov", kind = "float", default = 150.0, doc = "field of view, degrees; 360 = sees all around" },
        { name = "lose_range", kind = "float", default = 18.0, doc = "farther than this it gives up the chase, m" },
        { name = "lose_time", kind = "float", default = 4.0, doc = "seconds without seeing the player before it gives up" },
        { name = "attack_range", kind = "float", default = 1.8, doc = "distance at which it hits, m" },
        { name = "attack_damage", kind = "float", default = 10.0, doc = "damage per hit" },
        { name = "attack_cooldown", kind = "float", default = 1.2, doc = "seconds between hits" },
        { name = "eye_height", kind = "float", default = 0.6, doc = "height of its eyes above its origin, m" },
        { name = "player", kind = "string", default = "Player", doc = "name of the player (an entity tagged Player counts too)" },
        { name = "attack_sound", kind = "string", default = "", doc = "sound when it hits (empty = none)" },
    },

    start = function(self, ctx)
        self.points = {}
        for _, item in ipairs(split(self.params.patrol, ";")) do
            local a, b = string.match(item, "^%s*(-?[%d%.]+)%s*,%s*(-?[%d%.]+)%s*$")
            if a then
                self.points[#self.points + 1] = { tonumber(a), tonumber(b) }
            else
                local e = ctx:find_by_name((string.gsub(item, "^%s*(.-)%s*$", "%1")))
                if e then
                    local x, _, z = ctx:world_position_of_xyz(e)
                    self.points[#self.points + 1] = { x, z }
                else
                    ctx:warn("patrol point `" .. item .. "` not found")
                end
            end
        end
        self.state = "patrol"
        self.at, self.dir = 1, 1
        self.wait_until = 0
        self.next_attack = 0
        self.last_seen = -1e9
        self.player_dead = false
        if ctx:motor_velocity_xyz() == nil then
            ctx:warn("needs a Character Controller component on this entity")
        end
    end,

    on_message = function(self, ctx, name, value)
        if value == self.params.player then
            if name == "died" then self.player_dead = true
            elseif name == "revived" then self.player_dead = false end
        end
    end,

    -- Walks horizontally towards (x, z) at `speed`; faces the way it goes.
    walk = function(self, ctx, x, z, speed)
        local px, _, pz = ctx:world_position_xyz()
        local dx, dz = x - px, z - pz
        local len = math.sqrt(dx * dx + dz * dz)
        local _, vy = ctx:motor_velocity_xyz()
        if len < 0.05 then
            ctx:set_motor_velocity_xyz(0, vy or 0, 0)
            return len
        end
        dx, dz = dx / len, dz / len
        ctx:set_motor_velocity_xyz(dx * speed, vy or 0, dz * speed)
        ctx:set_rotation_euler_deg_xyz(0, math.deg(math.atan(-dx, -dz)), 0)
        return len
    end,

    stop = function(self, ctx)
        local _, vy = ctx:motor_velocity_xyz()
        ctx:set_motor_velocity_xyz(0, vy or 0, 0)
    end,

    sees = function(self, ctx, who)
        local p = self.params
        local ex, ey, ez = ctx:world_position_xyz()
        ey = ey + p.eye_height
        local tx, ty, tz = ctx:world_position_of_xyz(who)
        local dx, dy, dz = tx - ex, ty - ey, tz - ez
        local dist = math.sqrt(dx * dx + dy * dy + dz * dz)
        if dist > p.sight_range or dist < 1e-3 then return false, dist end
        if p.fov < 359 then
            local fx, _, fz = ctx:forward_xyz()
            local flat = math.sqrt(dx * dx + dz * dz)
            if flat > 0.01 and (fx * dx + fz * dz) / flat < math.cos(math.rad(p.fov / 2)) then
                return false, dist
            end
        end
        local hit, _, _, _, _, _, _, d = ctx:raycast_xyz(ex, ey, ez, dx / dist, dy / dist, dz / dist, dist)
        return hit == nil or hit == who or d >= dist - 0.6, dist
    end,

    update = function(self, ctx)
        local p = self.params
        local now = ctx:elapsed()
        local who = ctx:find_by_name(p.player) or ctx:find_by_tag("Player")
        local visible, dist = false, 1e9
        if who and not self.player_dead then
            visible, dist = self:sees(ctx, who)
        end

        if visible then
            self.last_seen = now
            if self.state ~= "chase" then
                self.state = "chase"
                ctx:send("enemy_alert", ctx:name())
            end
        elseif self.state == "chase" and (not who or self.player_dead or dist > p.lose_range
            or now - self.last_seen > p.lose_time) then
            self.state = "patrol"
            ctx:send("enemy_lost", ctx:name())
        end

        if self.state == "chase" then
            local tx, _, tz = ctx:world_position_of_xyz(who)
            local flat = self:walk(ctx, tx, tz, p.chase_speed)
            if flat <= p.attack_range * 0.8 then self:stop(ctx) end
            if dist <= p.attack_range + 0.9 and now >= self.next_attack then
                self.next_attack = now + p.attack_cooldown
                ctx:send("damage", p.attack_damage, who)
                if p.attack_sound ~= "" then
                    ctx:play_sound_at(p.attack_sound, ctx:entity(), { max_distance = 25 })
                end
            end
            return
        end

        local n = #self.points
        if n == 0 or now < self.wait_until then
            self:stop(ctx)
            return
        end
        local target = self.points[self.at]
        if self:walk(ctx, target[1], target[2], p.patrol_speed) < 0.3 then
            self.wait_until = now + p.patrol_wait
            if n > 1 then
                if self.at + self.dir > n or self.at + self.dir < 1 then self.dir = -self.dir end
                self.at = self.at + self.dir
            end
        end
    end,
}

-- Kill plane: when the target falls below height `y` it is respawned. Put the
-- script on any entity (an empty "KillPlane"). By default it broadcasts
-- "respawn" (value = the target's id), which the active checkpoint answers. With
-- `damage` > 0 it sends "damage" to the target instead, so the health-damage
-- script decides (a death with respawn_delay >= 0 then ends in "respawn").
return {
    params = {
        { name = "y", kind = "float", default = -20, doc = "world height below which the target is lost" },
        { name = "target", kind = "string", default = "Player", doc = "name of the entity watched" },
        { name = "damage", kind = "float", default = 0, doc = "> 0: send this much damage instead of respawning directly" },
    },

    start = function(self, ctx)
        self.last = -1e9
    end,

    update = function(self, ctx)
        local who = ctx:find_by_name(self.params.target) or ctx:find_by_tag(self.params.target)
        if not who then return end
        local _, y = ctx:world_position_of_xyz(who)
        if y < self.params.y then
            -- Again every second: a hit may have been refused (invulnerability).
            if ctx:elapsed() - self.last > 1 then
                self.last = ctx:elapsed()
                if self.params.damage > 0 then
                    ctx:send("damage", self.params.damage, who)
                else
                    ctx:send("respawn", who)
                end
            end
        end
    end,
}

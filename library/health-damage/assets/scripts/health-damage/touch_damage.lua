-- Damage on touch (spikes, a swinging blade, a bullet): on an entity with a
-- trigger or a solid collider. When something that matches `filter` touches it,
-- it sends "damage" (value `amount`) to it; the same target is hit again at
-- most every `cooldown` seconds. `destroy_on_hit` removes the owner (bullets).
return {
    params = {
        { name = "amount", kind = "float", default = 20, doc = "damage per hit" },
        { name = "cooldown", kind = "float", default = 1.0, doc = "seconds before the same target can be hit again" },
        { name = "filter", kind = "string", default = "", doc = "only entities with this name or tag; empty = anything with health" },
        { name = "destroy_on_hit", kind = "bool", default = false, doc = "remove this entity after a hit" },
    },

    start = function(self, ctx)
        self.hit_at = {}
    end,

    hit = function(self, ctx, other)
        local f = self.params.filter
        if f ~= "" and ctx:name_of(other) ~= f and not ctx:has_tag(other, f) then return end
        local last = self.hit_at[other]
        if last and ctx:elapsed() - last < self.params.cooldown then return end
        self.hit_at[other] = ctx:elapsed()
        ctx:send("damage", self.params.amount, other)
        if self.params.destroy_on_hit then ctx:destroy_self() end
    end,

    on_trigger_enter = function(self, ctx, other) self:hit(ctx, other) end,
    on_collision_enter = function(self, ctx, other) self:hit(ctx, other) end,
}

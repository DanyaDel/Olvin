-- Hazard zone (lava, acid, fire): put it on an entity with a trigger collider.
-- Every `interval` seconds it sends "damage" (value `amount`) to each entity
-- that stands in the zone and matches `filter`. Damage over time: keep the
-- target's invulnerable_time shorter than `interval`.
return {
    params = {
        { name = "amount", kind = "float", default = 10, doc = "damage per tick" },
        { name = "interval", kind = "float", default = 1.0, doc = "seconds between ticks" },
        { name = "filter", kind = "string", default = "", doc = "only entities with this name or tag; empty = everything inside" },
    },

    start = function(self, ctx)
        self.inside = {}
        self.next_tick = 0
    end,

    matches = function(self, ctx, other)
        local f = self.params.filter
        return f == "" or ctx:name_of(other) == f or ctx:has_tag(other, f)
    end,

    on_trigger_enter = function(self, ctx, other)
        if self:matches(ctx, other) then
            self.inside[other] = true
            -- The first tick comes right away, the rest every `interval`.
            self.next_tick = math.min(self.next_tick, ctx:elapsed())
        end
    end,

    on_trigger_exit = function(self, ctx, other)
        self.inside[other] = nil
    end,

    update = function(self, ctx)
        if next(self.inside) == nil or ctx:elapsed() < self.next_tick then return end
        self.next_tick = ctx:elapsed() + self.params.interval
        for id in pairs(self.inside) do
            if ctx:alive(id) then
                ctx:send("damage", self.params.amount, id)
            else
                self.inside[id] = nil
            end
        end
    end,
}

-- Glue between packs: a trigger volume that sends a message when something
-- enters or leaves. Put it on an entity with a trigger collider.
--
-- `filter` limits who counts (an entity name or tag; empty = anything).
-- `on_enter` / `on_exit` are message names (empty = send nothing). The value
-- is read from text: a number ("0.5"), "true"/"false", three numbers
-- ("0.5,1.2,0" becomes a vector, for "shake" that is strength,seconds,0) or a
-- plain string. `target` is the name of the entity that receives it; empty
-- broadcasts to everything.
-- Example: on_enter "shake", on_enter_value "0.8", target "Camera".
return {
    params = {
        { name = "filter", kind = "string", default = "Player", doc = "only this entity name or tag counts; empty = anything" },
        { name = "on_enter", kind = "string", default = "", doc = "message sent when it enters" },
        { name = "on_enter_value", kind = "string", default = "", doc = "its value (number, true/false, \"x,y,z\" or text)" },
        { name = "on_exit", kind = "string", default = "", doc = "message sent when it leaves" },
        { name = "on_exit_value", kind = "string", default = "", doc = "its value" },
        { name = "target", kind = "string", default = "", doc = "name of the receiving entity; empty = broadcast" },
        { name = "once", kind = "bool", default = false, doc = "fire only the first time" },
    },

    start = function(self, ctx)
        self.fired = false
    end,

    parse = function(self, text)
        if text == "" then return nil end
        local n = tonumber(text)
        if n then return n end
        if text == "true" then return true end
        if text == "false" then return false end
        local a, b, c = string.match(text, "^%s*(-?[%d%.]+)%s*,%s*(-?[%d%.]+)%s*,%s*(-?[%d%.]+)%s*$")
        if a then return vec3(tonumber(a), tonumber(b), tonumber(c)) end
        return text
    end,

    counts = function(self, ctx, other)
        local f = self.params.filter
        return f == "" or ctx:name_of(other) == f or ctx:has_tag(other, f)
    end,

    send = function(self, ctx, name, text)
        if name == "" then return end
        local target = nil
        if self.params.target ~= "" then
            target = ctx:find_by_name(self.params.target)
            if not target then
                ctx:warn("no entity named `" .. self.params.target .. "`")
                return
            end
        end
        ctx:send(name, self:parse(text), target)
    end,

    on_trigger_enter = function(self, ctx, other)
        if not self:counts(ctx, other) or (self.params.once and self.fired) then return end
        self.fired = true
        self:send(ctx, self.params.on_enter, self.params.on_enter_value)
    end,

    on_trigger_exit = function(self, ctx, other)
        if not self:counts(ctx, other) or (self.params.once and self.fired and self.params.on_exit == "") then return end
        self:send(ctx, self.params.on_exit, self.params.on_exit_value)
    end,
}

-- A plate or lever that sends a message to doors. Put it on an entity with a
-- trigger collider: the plate fires when the player steps in (`mode` = plate)
-- or when the player presses the `action` while standing in it (`mode` = press).
--
-- The message goes to every door whose `id` equals `target` (a broadcast with
-- that value); an empty `target` reaches all doors. Typical messages: "open",
-- "close", "toggle", "unlock". `exit_message` (optional) is sent when the
-- player leaves, which makes a pressure plate that holds a door open.
return {
    params = {
        { name = "target", kind = "string", default = "", doc = "id of the door(s); empty = all doors" },
        { name = "message", kind = "string", default = "toggle", doc = "open, close, toggle, unlock, lock" },
        { name = "exit_message", kind = "string", default = "", doc = "sent when the player steps off (empty = nothing)" },
        { name = "mode", kind = "string", default = "plate", doc = "plate (step in) or press (action while inside)" },
        { name = "action", kind = "string", default = "Interact", doc = "input action of the `press` mode" },
        { name = "once", kind = "bool", default = false, doc = "fire a single time" },
        { name = "player", kind = "string", default = "Player", doc = "name of the player (an entity tagged Player counts too)" },
    },

    start = function(self, ctx)
        self.inside = false
        self.fired = false
    end,

    fire = function(self, ctx, name)
        if name == "" or (self.params.once and self.fired) then return end
        self.fired = true
        local target = self.params.target
        ctx:send(name, target ~= "" and target or nil)
    end,

    on_trigger_enter = function(self, ctx, other)
        if ctx:name_of(other) ~= self.params.player and not ctx:has_tag(other, "Player") then return end
        self.inside = true
        if self.params.mode == "plate" then
            self:fire(ctx, self.params.message)
        end
    end,

    on_trigger_exit = function(self, ctx, other)
        if ctx:name_of(other) ~= self.params.player and not ctx:has_tag(other, "Player") then return end
        self.inside = false
        if self.params.mode == "plate" and not (self.params.once and self.fired and self.params.exit_message == "") then
            if self.params.exit_message ~= "" then
                local target = self.params.target
                ctx:send(self.params.exit_message, target ~= "" and target or nil)
            end
        end
    end,

    update = function(self, ctx)
        if self.inside and self.params.mode == "press" and ctx:just_pressed(self.params.action) then
            self:fire(ctx, self.params.message)
        end
    end,
}

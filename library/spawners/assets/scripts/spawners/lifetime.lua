-- Removes its entity `seconds` after it appeared. Put it on anything a spawner
-- makes, so the field does not fill up.
return {
    params = {
        { name = "seconds", kind = "float", default = 8.0, doc = "lifetime in seconds" },
    },

    start = function(self, ctx)
        self.until_ = ctx:elapsed() + self.params.seconds
    end,

    update = function(self, ctx)
        if ctx:elapsed() >= self.until_ then
            ctx:destroy_self()
        end
    end,
}

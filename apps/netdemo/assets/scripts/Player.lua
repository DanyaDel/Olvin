-- Runs on the server and on every client copy of a player.
-- The owner's client fires once a second; the server counts shots per player and
-- replicates the count, so every client can read it with ctx:var.
return {
  start = function(self, ctx)
    self.frame = 0
    if ctx:is_server() and ctx:owner() then
      ctx:replicate_var("shots_" .. ctx:owner())
    end
  end,
  update = function(self, ctx)
    self.frame = self.frame + 1
    if ctx:is_client() and not ctx:is_server() and ctx:is_mine() and self.frame % 60 == 0 then
      ctx:net_send("server", "fire", self.frame)
    end
  end,
  on_net_message = function(self, ctx, name, value, from)
    -- Every player script hears every message: only the sender's own counts.
    if ctx:is_server() and name == "fire" and from == ctx:owner() then
      local key = "shots_" .. from
      ctx:set_var(key, (ctx:var(key) or 0) + 1)
    end
  end,
}

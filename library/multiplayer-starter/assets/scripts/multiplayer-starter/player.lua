-- Multiplayer example for a replicated player.
--   * on a client, the owner "fires" once a second: a message to the server
--   * on the server, the script of the firing player counts the shot in the shared
--     variable shots_<client id>, which is replicated to every client
--   * every client can read ctx:var("shots_1") and so on (a HUD would show them)
-- Without a network session (offline in the editor) none of the net functions
-- exist, and the script does nothing.
return {
  params = { { name = "fire_every", kind = "int", default = 60 } },
  start = function(self, ctx)
    self.frame = 0
    if ctx.is_server and ctx:is_server() and ctx:owner() then
      ctx:replicate_var("shots_" .. ctx:owner())
    end
  end,
  update = function(self, ctx)
    if not ctx.is_server then return end
    self.frame = self.frame + 1
    if ctx:is_client() and not ctx:is_server() and ctx:is_mine()
       and self.frame % self.params.fire_every == 0 then
      ctx:net_send("server", "fire", self.frame)
    end
  end,
  on_net_message = function(self, ctx, name, value, from)
    -- Every player's script hears every message: only the sender's own one counts it.
    if ctx:is_server() and name == "fire" and from == ctx:owner() then
      local key = "shots_" .. from
      ctx:set_var(key, (ctx:var(key) or 0) + 1)
    end
  end,
}

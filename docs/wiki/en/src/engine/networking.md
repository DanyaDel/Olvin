# Networking

The netcode lives in the `olvin_net` crate (feature `net` of `olvin`, `net-quic` for real sockets). Decision and rationale: `docs/adr/0003-netcode.en.md` (ADR 0003). The goal is a netcode that is very flexible and still easy to write: it is built in layers, each usable alone, and the top layer is a few lines.

## Quick start

A host or a client from the command line, a replicated player prefab, one validated command, one Lua script. This is `apps/netdemo/examples/quickstart.rs`, and it compiles with the repository:

```rust
use olvin::core::{EventCursor, Events};
use olvin::net::*;
use olvin::prelude::*;
use olvin::script::lua::LuaPlugin;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Move(Vec2);

fn main() {
    let cfg = NetConfig::from_args("my-game", 1, std::env::args()).expect("--host | --join <code>");
    let mut app = App::new();
    app.add_plugin(HeadlessPlugins::default())
        .add_plugin(NetPlugin::new(cfg));
    app.add_plugin(LuaPlugin::new("assets/scripts"))
        .add_plugin(LuaNetPlugin);
    app.add_plugin(PlayerSpawner::new("prefabs/player.prefab.ron")); // a Replicated prefab per client
    app.replicate::<Transform>(); // positions travel and are interpolated
    app.net_command::<Move>("move", |_, m| {
        Verdict::check(m.0.length() <= 1.01, "too fast")
    });
    let mut seen = EventCursor::default();
    app.add_system(Stage::Update, move |res| {
        let moves = res.fetch::<Events<NetReceived<Move>>>();
        let mut world = res.fetch_mut::<World>();
        for m in moves.read(&mut seen) {
            for (o, t) in world.query_mut::<(&Owner, &mut Transform)>() {
                if Some(o.0) == m.from.client() {
                    t.translation += Vec3::new(m.msg.0.x, 0.0, m.msg.0.y) * 0.1;
                }
            }
        }
    });
    app.run();
}
```

Run `cargo run -p netdemo --example quickstart -- --host` and give the printed room code to `--join <code>` on another machine or terminal. Without arguments the game is offline: the same code, a server with one local player.

The player is `apps/netdemo/assets/prefabs/player.prefab.ron`: an ordinary prefab with one extra entry, `components: {"Replicated": "()"}`. In the editor you add the same entry to any object in the Inspector (Components) and it becomes a networked object, no code. The Lua script is `assets/scripts/Player.lua`:

```lua
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
```

`cargo run -p netdemo -- --selftest` starts a server and two bot clients in one process over QUIC on 127.0.0.1 and checks that they agree. `netdemo --host --frames 600` and `netdemo --join <code> --bot --frames 600` run them as separate processes.

## Layers

| Layer | What | Main API |
|---|---|---|
| L0 Transport | reliable and unreliable channels; `Loopback` in process with simulated lag, jitter, loss, duplicates, reordering; QUIC with TLS 1.3 | `Transport`, `Loopback`, `QuicServer`, `QuicClient` |
| L1 Session | `Offline`, `Host` (listen server, the host plays), `Dedicated`, `Client`; handshake with protocol, game and registry checks; join, leave, kick, heartbeat, timeouts, reconnect token; room codes; pluggable authentication | `NetConfig`, `NetPlugin`, `Net`, `NetEvent`, `Authenticator`, `RoomCode` |
| L2 Replication | the server owns the world; entities with `Replicated` appear on clients as the same prefab; delta snapshots with acks, interpolation, interest management, bandwidth budget | `Replicated`, `Owner`, `NetId`, `app.replicate::<T>()`, `ReplicateOptions` |
| L3 Messages | any serde type registered by name; commands the server validates; rate and size limits | `app.net_message::<T>()`, `app.net_command::<T>()`, `net.send(Target, &msg)`, `NetReceived<T>`, `Verdict` |
| L4 Prediction | the owner's character responds at once and is corrected by the server | `Prediction<S, I>`, `net.predict_character(..)`, `PoseHistory` |
| L5 Lua | `ctx:is_server()`, `ctx:net_send`, `on_net_message`, replicated variables | see the [Lua API](../reference/lua-api.md) |
| L6 Tools | `NetStats` resource (RTT, bandwidth, loss, tick lag), `Testbed` for tests, `OLVIN_NET_SIM` | `NetStats`, `testing::Testbed` |

Each layer works without the ones above it: a game can use only `Loopback` and `Net::send`, or replication without prediction, or Lua without Rust components.

## Configuration

One struct, `NetConfig`, built from a constructor and chained settings:

| Setting | Default | Meaning |
|---|---|---|
| `NetConfig::offline / host / dedicated / join / from_args` | | the mode; `from_args` reads `--host [addr]`, `--dedicated [addr]`, `--join <code or addr>`, `--name`, `--net-sim` |
| `tick_rate(hz)` | 30 | server ticks per second |
| `interp_delay(s)` | 0.1 | how far in the past remote entities are shown |
| `max_clients(n)` | 16 | |
| `bandwidth(bytes_per_s)` | 65536 | snapshot budget per client |
| `timeout(s)` | 10 | silence before a peer is dropped |
| `player_name`, `token` | | sent in the handshake; the server's `Authenticator` sees the token |
| `simulate(NetConditions)` | none | put "lag 80 ms, loss 2%" on the real transport (`OLVIN_NET_SIM="lag=80,loss=2,jitter=10"`) |
| `allow_unpinned(bool)` | false | join a bare `host:port` without a pinned certificate |
| `limits(NetLimits)` | | packet, message, name and entity limits |

Game id and version must match between server and client, and so must the registrations: components and messages are registered by name in the same order everywhere (the handshake compares a hash and refuses a mismatch with a message).

## Security

- **Server authority.** Only the server writes replicated components. A client has no message that sets state: it sends commands (`Move`, `Fire`) and the server validates them. `Owner(ClientId)` says who controls an entity; changing it is a server-side write.
- **Encryption and identity.** QUIC gives TLS 1.3. The host's certificate is self-signed, and the **room code** (`OLV-…`) carries the host's address and the first 16 bytes of the certificate's SHA-256: the client accepts only that certificate. A bare `host:port` is refused unless `allow_unpinned(true)`.
- **Session keys and replay.** After the handshake every packet carries a sequence number and a truncated HMAC-SHA256 tag keyed from both sides' nonces and the TLS exporter; duplicates and old packets are dropped. The handshake has a nonce and a timestamp window (60 s), so a recorded `Hello` cannot be replayed.
- **Untrusted input.** Packet sizes, message sizes, rates per client and per message type, names, entity counts and buffers are bounded; bad bytes never panic (tests feed garbage to every decoder); a client that keeps misbehaving is kicked. A client also refuses a server's prefab path that is not a relative `*.prefab.ron`.
- **No server secrets in the client.** The certificate key (`QuicIdentity`) and authenticators exist only in the server's code paths. Name server-only secrets `server_*`; a CI check for that is planned.

## Prediction

`net.predict_character(sample)` predicts the character the client owns on `move_character`; `Prediction<S, I>` does the same for any state and input. The client applies its input at once, sends the last inputs with sequence numbers, and the server applies exactly one input per tick. The server's state carries "the last input I applied"; when it differs from what the client predicted by more than a threshold, the client moves a fraction of the way to the server's state and replays the unacknowledged inputs (a big error snaps). The server clamps inputs (`sanitize`) and cannot be sped up by flooding. Spawn the player with `CharacterState::at(position)` and the same `Transform`.

## Testing a multiplayer game

`olvin_net::testing::Testbed` runs a server and clients as separate `App`s in one process over a `Loopback` with the conditions you choose; the test steps all of them with a fixed clock, so runs are fast and repeatable:

```rust
let mut tb = Testbed::new(NetConditions::lag_ms(80.0).with_loss_percent(2.0));
tb.add_server_with(NetConfig::dedicated("my-game", 1, "x"), register_game);
tb.add_client_with(NetConfig::join("my-game", 1, "1.1.1.1:1")?, register_game);
tb.run_until(5.0, |tb| tb.net(1).status() == NetStatus::Connected);
```

## Limits

Not included: matchmaking and lobbies on a server, NAT punching and relays, rollback netcode for fighting games, lockstep, voice, sharding for hundreds of players, anti-cheat beyond server authority and validation. Remote copies still carry the prefab's own components: give them no physics body of their own (query `Without<Remote>`). The editor has no Play-with-lag setting yet; use `OLVIN_NET_SIM` or `NetConfig::simulate`.

## See also

- [Time and events](time-events.md)
- [Scripting](scripting.md)
- [Prefabs](prefabs.md)
- [States and saves](game-states-saves.md)

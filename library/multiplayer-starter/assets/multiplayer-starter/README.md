# Multiplayer Starter

What you get: `prefabs/multiplayer-starter/Player.prefab.ron` (a player with a `Replicated`
entry), `scripts/multiplayer-starter/player.lua` and a demo arena.

Add the `net` feature of `olvin` (`net-quic` for real sockets) to your game and:

```rust
use olvin::net::*;
use olvin::prelude::*;
use olvin::script::lua::LuaPlugin;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Move(Vec2); // a client's wish; the server decides

fn main() {
    // `--host`, `--join <room code>` or nothing (offline).
    let cfg = NetConfig::from_args("my-game", 1, std::env::args()).expect("bad arguments");
    let mut app = App::new();
    app.add_plugin(DefaultPlugins::default()).add_plugin(NetPlugin::new(cfg));
    app.add_plugin(LuaPlugin::new("assets/scripts")).add_plugin(LuaNetPlugin);
    app.add_plugin(PlayerSpawner::new("prefabs/multiplayer-starter/Player.prefab.ron"));
    app.replicate::<Transform>();
    app.net_command::<Move>("move", |_, m| Verdict::check(m.0.length() <= 1.01, "too fast"));
    // ... a system that reads Events<NetReceived<Move>> and moves the sender's player
    app.run();
}
```

The host prints a room code (`Net::room_code`); players join with it. The code carries the
host's address and the fingerprint of its certificate, so the connection is encrypted and
pinned to the right server.

Rules worth knowing: only the server changes replicated state; clients send commands and the
server validates them. A networked object is any entity with `Replicated`: add the entry in the
Inspector (Components) instead of writing code. Mark the components that must travel with
`app.replicate::<T>()`, on the server and every client, in the same order.

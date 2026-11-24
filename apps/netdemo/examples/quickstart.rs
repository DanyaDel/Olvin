//! The wiki quick start, verbatim: `cargo run -p netdemo --example quickstart -- --host`
//! (then `--join <room code>` in another terminal). Players are the prefab in `assets/prefabs`,
//! the Lua script is `assets/scripts/Player.lua`.
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

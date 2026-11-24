//! The netdemo arena: players are a prefab with a `Replicated` marker, movement is
//! one validated command, and a Lua script counts shots. `build_app` is the whole
//! game; `main.rs` only picks a mode from the command line.

use olvin::net::{
    LuaNetPlugin, NetAppExt, NetConfig, NetPlugin, NetReceived, Owner, PlayerSpawner, Target,
    Verdict,
};
use olvin::prelude::*;
use olvin::render_assets::RenderAssets;
use olvin::scene::Transform;
use olvin::script::lua::LuaPlugin;
use serde::{Deserialize, Serialize};

/// A client's wish to move: a direction on the ground, at most length 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Move(pub Vec2);

const SPEED: f32 = 4.0;

pub fn assets_dir() -> std::path::PathBuf {
    std::env::var_os("NETDEMO_ASSETS")
        .map(Into::into)
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into())
}

/// The game for `cfg`. Same code for offline, host, dedicated and client.
pub fn build_app(cfg: NetConfig) -> App {
    let mut app = App::new();
    app.add_plugin(HeadlessPlugins::default())
        .add_plugin(NetPlugin::new(cfg))
        .add_plugin(LuaPlugin::new(assets_dir().join("scripts")))
        .add_plugin(LuaNetPlugin)
        .add_plugin(PlayerSpawner::new("prefabs/player.prefab.ron"));
    app.resources
        .fetch_mut::<RenderAssets>()
        .set_root(Some(assets_dir()));
    app.replicate::<Transform>();
    app.net_command::<Move>("move", |_, m| {
        Verdict::check(m.0.is_finite() && m.0.length() <= 1.01, "moving too fast")
    });
    app.add_system(Stage::Update, apply_moves);
    app
}

/// Server: accepted `Move` commands move the sender's player.
fn apply_moves(res: &mut olvin::core::Resources) {
    use olvin::core::{EventCursor, Events};
    thread_local! {
        static CURSOR: std::cell::RefCell<EventCursor> = std::cell::RefCell::new(EventCursor::default());
    }
    let dt = res.fetch::<Time>().delta();
    let moves: Vec<NetReceived<Move>> = CURSOR.with(|c| {
        res.fetch::<Events<NetReceived<Move>>>()
            .read(&mut c.borrow_mut())
            .cloned()
            .collect()
    });
    let mut world = res.fetch_mut::<World>();
    for NetReceived { from, msg } in moves {
        let Some(client) = from.client() else {
            continue;
        };
        for (o, t) in world.query_mut::<(&Owner, &mut Transform)>() {
            if o.0 == client {
                // One command is one frame of intent; the server decides the speed.
                t.translation += Vec3::new(msg.0.x, 0.0, msg.0.y) * SPEED * dt.max(1.0 / 60.0);
            }
        }
    }
}

/// A bot's brain: walks in a circle by sending `Move` every other frame.
pub fn bot_system(res: &mut olvin::core::Resources) {
    let t = res.fetch::<Time>().elapsed() as f32;
    let frame = res.fetch::<Time>().frame();
    if frame.is_multiple_of(2) {
        let dir = Vec2::new(t.cos(), t.sin());
        res.fetch_mut::<olvin::net::Net>()
            .send(Target::Server, &Move(dir));
    }
}
pub mod selftest;

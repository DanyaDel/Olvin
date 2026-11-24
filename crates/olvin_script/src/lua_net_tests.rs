//! A Lua-only game over `olvin_net`: one server, two clients, simulated lag.

use std::fs;
use std::path::PathBuf;

use olvin_core::App;
use olvin_ecs::World;
use olvin_net::testing::Testbed;
use olvin_net::{ClientId, NetConditions, NetConfig, NetPlugin, NetStatus, Owner, Replicated};
use olvin_scene::Transform;

use super::lua::{LuaNetPlugin, LuaPlugin};
use super::*;

fn scripts_dir(test: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olvin-luanet-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (name, src) in files {
        fs::write(dir.join(name), src).unwrap();
    }
    dir
}

const SERVER: &str = r#"
return {
  start = function(self, ctx)
    ctx:replicate_var("round")
    ctx:set_var("round", 1)
    self.got = 0
    ctx:set_var("server_role", ctx:is_server() and not ctx:is_client())
  end,
  on_net_message = function(self, ctx, name, value, from)
    if name == "fire" then
      self.got = self.got + 1
      ctx:set_var("round", 1 + self.got)
      ctx:set_var("last_from", from)
      ctx:net_send(from, "ack", value)
    end
  end,
}
"#;

const CLIENT: &str = r#"
return {
  start = function(self, ctx) self.sent = false end,
  update = function(self, ctx)
    if not self.sent and ctx:local_client() then
      ctx:net_send("server", "fire", 42)
      self.sent = true
      ctx:set_var("my_id", ctx:local_client())
    end
  end,
  on_net_message = function(self, ctx, name, value, from)
    if name == "ack" then
      ctx:set_var("acked", value)
      ctx:set_var("ack_from_server", from == nil)
    end
  end,
}
"#;

fn game(app: &mut App, dir: &std::path::Path, script: &str) {
    app.add_plugin(LuaPlugin {
        dir: dir.to_path_buf(),
        hot_reload: false,
        limits: Default::default(),
    });
    app.add_plugin(LuaNetPlugin);
    app.resources
        .fetch_mut::<World>()
        .spawn((Transform::IDENTITY, Scripts::new().with(script, [])));
}

fn var(tb: &Testbed, i: usize, name: &str) -> Option<ScriptValue> {
    tb.apps[i]
        .resources
        .fetch::<ScriptVars>()
        .get(name)
        .cloned()
}

fn rig(test: &str, conditions: NetConditions) -> Testbed {
    let dir = scripts_dir(test, &[("server.lua", SERVER), ("client.lua", CLIENT)]);
    let mut tb = Testbed::new(conditions);
    let d = dir.clone();
    tb.add_server_with(
        NetConfig::dedicated("lua", 1, "x").timeout(5.0),
        move |app| game(app, &d, "server"),
    );
    for n in ["ann", "bob"] {
        let d = dir.clone();
        tb.add_client_with(
            NetConfig::join("lua", 1, "1.1.1.1:1")
                .unwrap()
                .player_name(n)
                .timeout(5.0),
            move |app| game(app, &d, "client"),
        );
    }
    tb
}

#[test]
fn a_lua_only_game_talks_across_the_network() {
    let mut tb = rig("talk", NetConditions::lag_ms(40.0).with_loss_percent(5.0));
    assert!(tb.run_until(10.0, |tb| var(tb, 1, "acked").is_some()
        && var(tb, 2, "acked").is_some()));
    // Each client got its own ack from the server (nil sender), and the server saw real client ids.
    assert_eq!(var(&tb, 1, "acked"), Some(ScriptValue::Int(42)));
    assert_eq!(
        var(&tb, 1, "ack_from_server"),
        Some(ScriptValue::Bool(true))
    );
    assert_eq!(var(&tb, 0, "server_role"), Some(ScriptValue::Bool(true)));
    assert!(matches!(
        var(&tb, 0, "last_from"),
        Some(ScriptValue::Int(1 | 2))
    ));
    // The replicated variable reached both clients and follows the server.
    tb.run_for(2.0);
    for i in [1, 2] {
        assert_eq!(
            var(&tb, i, "round"),
            Some(ScriptValue::Int(3)),
            "client {i}"
        );
    }
    assert_eq!(var(&tb, 1, "my_id"), Some(ScriptValue::Int(1)));
    assert_eq!(var(&tb, 2, "my_id"), Some(ScriptValue::Int(2)));
}

#[test]
fn late_joiners_get_replicated_variables() {
    let mut tb = rig("late", NetConditions::lag_ms(10.0));
    tb.run_for(2.0);
    let dir = scripts_dir("late2", &[("client.lua", CLIENT)]);
    let c = tb.add_client_with(
        NetConfig::join("lua", 1, "1.1.1.1:1").unwrap().timeout(5.0),
        move |app| game(app, &dir, "client"),
    );
    assert!(tb.run_until(5.0, |tb| var(tb, c, "round") == Some(ScriptValue::Int(3))));
    assert_eq!(tb.net(c).status(), NetStatus::Connected);
}

const MISUSE: &str = r#"
return {
  update = function(self, ctx)
    if self.done then return end
    self.done = true
    local ok, err = pcall(function() ctx:net_send("all", "x", 1) end)
    ctx:set_var("all_err", ok and "none" or tostring(err))
    ok, err = pcall(function() ctx:net_send("server", "bad\nname", 1) end)
    ctx:set_var("name_err", ok and "none" or tostring(err))
    ok, err = pcall(function() ctx:net_send("server", "big", string.rep("x", 300)) end)
    ctx:set_var("big_err", ok and "none" or tostring(err))
    ok = pcall(function() for i = 1, 100 do ctx:net_send("server", "spam", i) end end)
    ctx:set_var("spam_ok", ok)
  end,
}
"#;

#[test]
fn clients_cannot_relay_or_send_bad_names_and_floods_are_capped() {
    let dir = scripts_dir("misuse", &[("server.lua", SERVER), ("misuse.lua", MISUSE)]);
    let mut tb = Testbed::new(NetConditions::NONE);
    let d = dir.clone();
    tb.add_server_with(NetConfig::dedicated("lua", 1, "x"), move |app| {
        game(app, &d, "server")
    });
    let c = tb.add_client_with(
        NetConfig::join("lua", 1, "1.1.1.1:1").unwrap(),
        move |app| game(app, &dir, "misuse"),
    );
    tb.run_for(2.0);
    let text = |n: &str| match var(&tb, c, n) {
        Some(ScriptValue::Str(s)) => s,
        other => format!("{other:?}"),
    };
    assert!(
        text("all_err").contains("only send to"),
        "{}",
        text("all_err")
    );
    assert!(
        text("name_err").contains("printable"),
        "{}",
        text("name_err")
    );
    assert!(text("big_err").contains("too long"), "{}", text("big_err"));
    assert_eq!(var(&tb, c, "spam_ok"), Some(ScriptValue::Bool(false)));
}

#[test]
fn offline_lua_runs_the_same_path_with_the_host_as_player() {
    let dir = scripts_dir("offline", &[("server.lua", SERVER), ("client.lua", CLIENT)]);
    let mut app = App::new();
    app.insert_resource(olvin_core::Time::with_fixed_delta(1.0 / 60.0));
    app.add_plugin(NetPlugin::new(NetConfig::offline("lua", 1)));
    game(&mut app, &dir, "server");
    game_script(&mut app, "client");
    for _ in 0..90 {
        app.update();
    }
    let vars = app.resources.fetch::<ScriptVars>();
    // Host is client 0: it fired once, the server answered it, and `from` was its id (0, not nil).
    assert_eq!(vars.get("acked"), Some(&ScriptValue::Int(42)));
    assert_eq!(vars.get("last_from"), Some(&ScriptValue::Int(0)));
    assert_eq!(vars.get("ack_from_server"), Some(&ScriptValue::Bool(true)));
    let _ = (ClientId::HOST, Owner(ClientId::HOST), Replicated::default());
}

fn game_script(app: &mut App, script: &str) {
    app.resources
        .fetch_mut::<World>()
        .spawn((Transform::IDENTITY, Scripts::new().with(script, [])));
}

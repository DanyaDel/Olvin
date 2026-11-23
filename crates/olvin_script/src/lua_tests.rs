use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use glam::Vec3;
use olvin_core::App;
use olvin_ecs::{Entity, World};
use olvin_scene::{Name, Transform};

use super::lua::{LuaLimits, LuaPlugin, LuaScripts};
use super::*;

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olvin-lua-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn spawn(app: &App, script: &str, params: &[(&str, &str)]) -> Entity {
    app.resources.fetch_mut::<World>().spawn((
        Transform::IDENTITY,
        Name("hero".into()),
        Scripts::new().with(script, params.iter().copied()),
    ))
}

fn step(app: &App) {
    run_update(&app.resources, Some(0.5));
}

fn x(app: &App, e: Entity) -> f32 {
    app.resources
        .fetch::<World>()
        .get::<&Transform>(e)
        .unwrap()
        .translation
        .x
}

/// A script with its own registry, no plugin: loaded from source.
fn app_with(name: &str, source: &str) -> (App, LuaScripts) {
    let mut app = App::new();
    app.add_plugin(ScriptPlugin::default());
    let mut lua = LuaScripts::new().unwrap();
    lua.load_source(
        &mut app.resources.fetch_mut::<ScriptRegistry>(),
        name,
        source,
    )
    .unwrap();
    (app, lua)
}

#[test]
fn callbacks_params_and_ctx_api() {
    let (app, _lua) = app_with(
        "Walker",
        r#"
        return {
            params = {
                { name = "speed", kind = "float", default = 2 },
                { name = "dir", kind = "vec3", default = vec3(1, 0, 0) },
                { name = "label", kind = "string", default = "hi" },
            },
            start = function(self, ctx)
                self.steps = 0
                ctx:set_scale(vec3(2, 2, 2))
                assert(ctx:name() == "hero")
            end,
            update = function(self, ctx)
                self.steps = self.steps + 1
                local d = self.params.dir
                ctx:translate(vec3(d.x * self.params.speed * ctx:dt(), d.y, d.z))
                ctx:set_position({ x = ctx:position().x, y = self.steps, z = 0 })
                assert(ctx:velocity() == nil)
                assert(ctx:find_by_name("hero") == ctx:entity())
                assert(self.params.label == "hi")
            end,
        }
        "#,
    );
    let e = spawn(&app, "Walker", &[("speed", "4")]);
    step(&app);
    step(&app);
    // 4 m/s * 0.5 s per step, twice.
    assert_eq!(x(&app, e), 4.0);
    {
        let w = app.resources.fetch::<World>();
        let t = w.get::<&Transform>(e).unwrap();
        assert_eq!((t.translation.y, t.scale), (2.0, Vec3::splat(2.0)));
    }
    let schema = app.resources.fetch::<ScriptRegistry>().export_schema();
    let spec = &schema.scripts["Walker"];
    assert_eq!(spec.len(), 3);
    assert_eq!(spec[1].kind, ParamKind::Vec3);
    assert_eq!(spec[1].default, "1,0,0");
}

#[test]
fn hot_reload_keeps_entity_state() {
    let dir = temp_dir("reload");
    let file = dir.join("Counter.lua");
    fs::write(
        &file,
        r#"return {
            start = function(self, ctx) self.n = 100 end,
            update = function(self, ctx) self.n = self.n + 1; ctx:set_position(vec3(self.n, 0, 0)) end,
        }"#,
    )
    .unwrap();
    let mut app = App::new();
    app.add_plugin(LuaPlugin {
        dir: dir.clone(),
        hot_reload: false,
        limits: LuaLimits::default(),
    });
    let e = spawn(&app, "Counter", &[]);
    step(&app);
    step(&app);
    assert_eq!(x(&app, e), 102.0);

    // New version: counts by 10 and no start(); state `n` must carry over.
    fs::write(
        &file,
        r#"return {
            start = function(self, ctx) self.n = -1 end,
            update = function(self, ctx) self.n = self.n + 10; ctx:set_position(vec3(self.n, 0, 0)) end,
        }"#,
    )
    .unwrap();
    bump_mtime(&file);
    let reloaded = {
        let res = &app.resources;
        let mut lua = res.fetch_mut::<LuaScripts>();
        let mut registry = res.fetch_mut::<ScriptRegistry>();
        lua.check_disk(&mut registry)
    };
    assert_eq!(reloaded, ["Counter"]);
    step(&app);
    assert_eq!(x(&app, e), 112.0, "start did not rerun, n survived");

    // Entities spawned after the reload use the new code.
    let f = spawn(&app, "Counter", &[]);
    step(&app);
    assert_eq!(x(&app, f), -1.0 + 10.0);
    fs::remove_dir_all(dir).unwrap();
}

fn bump_mtime(path: &std::path::Path) {
    let f = fs::File::options().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(5))
        .unwrap();
}

#[test]
fn broken_reload_keeps_the_old_version_and_errors_recover() {
    let dir = temp_dir("broken");
    let file = dir.join("S.lua");
    fs::write(
        &file,
        "return { update = function(self, ctx) ctx:translate(vec3(1,0,0)) end }",
    )
    .unwrap();
    let mut app = App::new();
    app.add_plugin(LuaPlugin {
        dir: dir.clone(),
        hot_reload: false,
        limits: LuaLimits::default(),
    });
    let e = spawn(&app, "S", &[]);
    step(&app);
    assert_eq!(x(&app, e), 1.0);

    let check = |app: &App| {
        let res = &app.resources;
        let mut lua = res.fetch_mut::<LuaScripts>();
        let mut registry = res.fetch_mut::<ScriptRegistry>();
        lua.check_disk(&mut registry)
    };
    // Syntax error: nothing reloads, the old code keeps running, reported once.
    fs::write(&file, "return { update = function( ").unwrap();
    bump_mtime(&file);
    assert!(check(&app).is_empty());
    assert!(check(&app).is_empty());
    step(&app);
    assert_eq!(x(&app, e), 2.0);

    // A runtime error disables the script (the frame survives) until reload.
    fs::write(
        &file,
        "return { update = function(self, ctx) ctx:translate(vec3(1,0,0)); error('boom') end }",
    )
    .unwrap();
    bump_mtime(&file);
    assert_eq!(check(&app), ["S"]);
    step(&app);
    step(&app);
    assert_eq!(x(&app, e), 3.0, "ran once, then skipped");
    fs::write(
        &file,
        "return { update = function(self, ctx) ctx:translate(vec3(10,0,0)) end }",
    )
    .unwrap();
    bump_mtime(&file);
    assert_eq!(check(&app), ["S"]);
    step(&app);
    assert_eq!(x(&app, e), 13.0, "fixed file runs again");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn poll_picks_up_new_files() {
    let dir = temp_dir("newfile");
    let mut app = App::new();
    app.add_plugin(LuaPlugin {
        dir: dir.clone(),
        hot_reload: false,
        limits: LuaLimits::default(),
    });
    assert!(!app.resources.fetch::<ScriptRegistry>().contains("Late"));
    fs::write(dir.join("Late.lua"), "return {}").unwrap();
    let res = &app.resources;
    let reloaded = res
        .fetch_mut::<LuaScripts>()
        .check_disk(&mut res.fetch_mut::<ScriptRegistry>());
    assert_eq!(reloaded, ["Late"]);
    assert!(app.resources.fetch::<ScriptRegistry>().contains("Late"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sandbox_limits_and_missing_libraries() {
    let (app, lua) = app_with(
        "Evil",
        r#"return {
            start = function(self, ctx)
                assert(io == nil and os == nil and debug == nil and package == nil)
                assert(dofile == nil and require == nil and loadfile == nil)
            end,
            update = function(self, ctx)
                ctx:translate(vec3(1, 0, 0))
                while true do end
            end,
        }"#,
    );
    let e = spawn(&app, "Evil", &[]);
    step(&app);
    step(&app);
    // Update ran once and was cut off by the instruction limit; start's asserts held.
    assert_eq!(x(&app, e), 1.0);

    let mut small = LuaScripts::with_limits(LuaLimits {
        memory: 4 << 20,
        instructions: 0,
    })
    .unwrap();
    let mut registry = ScriptRegistry::default();
    small
        .load_source(
            &mut registry,
            "Hog",
            "return { update = function() local t = {} for i = 1, 1e9 do t[i] = i end end }",
        )
        .unwrap();
    let app = {
        let mut app = App::new();
        app.add_plugin(ScriptPlugin::default());
        app.insert_resource(registry);
        app
    };
    let e = spawn(&app, "Hog", &[]);
    step(&app); // memory error is a Lua error, not an abort
    assert!(app.resources.fetch::<World>().contains(e));
    assert!(small.used_memory() < 8 << 20);
    drop(lua);
}

#[test]
fn ctx_is_unusable_outside_its_callback() {
    let (app, _lua) = app_with(
        "Leak",
        r#"
        local saved
        return {
            update = function(self, ctx)
                if saved then
                    local ok = pcall(function() return saved:dt() end)
                    ctx:set_position(vec3(ok and 1 or 2, 0, 0))
                else
                    saved = ctx
                end
            end,
        }"#,
    );
    let e = spawn(&app, "Leak", &[]);
    step(&app);
    step(&app);
    assert_eq!(
        x(&app, e),
        1.0,
        "a saved ctx handle is only valid inside its own callback"
    );
}

#[cfg(feature = "physics")]
#[test]
fn collision_callbacks_get_the_other_entity() {
    let (mut app, _lua) = app_with(
        "Hit",
        r#"return {
            on_collision_enter = function(self, ctx, other) ctx:set_position(vec3(other, 0, 0)) end,
            on_trigger_exit = function(self, ctx, other) ctx:set_position(vec3(-1, 0, 0)) end,
        }"#,
    );
    let a = spawn(&app, "Hit", &[]);
    let b = app
        .resources
        .fetch_mut::<World>()
        .spawn((Transform::IDENTITY,));
    app.insert_resource(olvin_physics::CollisionEvents {
        events: vec![olvin_physics::CollisionEvent {
            a,
            b,
            started: true,
            sensor: false,
        }],
    });
    step(&app);
    assert_eq!(x(&app, a), b.to_bits().get() as f32);
}

#[test]
fn vectors_fast_paths_and_overloads() {
    let (app, _lua) = app_with(
        "Vec",
        r#"
        return {
            update = function(self, ctx)
                local a = vec3(1, 2, 3) + vec3(1, 1, 1) * 2
                assert(a == vec3(3, 4, 5), tostring(a))
                assert(math.abs(vec3(3, 4, 0):length() - 5) < 1e-6)
                assert((-a).x == -3 and (a / 2).y == 2)
                assert(getmetatable(a) == "vec3")
                ctx:set_position(1, 2, 3)            -- three numbers
                ctx:translate({ 1, 0, 0 })            -- array table
                ctx:translate_xyz(0, 0, 1)            -- raw fast path
                local x, y, z = ctx:position_xyz()
                assert(x == 2 and y == 2 and z == 4)
                local p = ctx:position()
                assert(p == vec3(2, 2, 4) and p:distance(vec3(2, 2, 0)) == 4)
                ctx:set_rotation_euler_deg(0, 90, 0)
                local r = ctx:rotation_euler_deg()
                assert(math.abs(r.y - 90) < 1e-3)
                local mx, my = ctx:axis2d_xy("Move")
                assert(mx == 0 and my == 0 and ctx:axis2d("Move").x == 0)
                assert(not pcall(function() ctx:translate("up") end))
                assert(ctx:unscaled_dt() >= 0 and ctx:time_scale() == 1 and not ctx:paused())
            end,
        }
        "#,
    );
    let e = spawn(&app, "Vec", &[]);
    step(&app);
    assert_eq!(x(&app, e), 2.0, "asserts held");
}

#[test]
fn other_entities_transform_and_names() {
    let (app, _lua) = app_with(
        "Mover",
        r#"
        return {
            update = function(self, ctx)
                local door = ctx:find_by_name("Door")
                assert(ctx:name_of(door) == "Door" and ctx:has_tag(door, "gate"))
                ctx:translate_of(door, vec3(0, 1, 0))
                local p = ctx:position_of(door)
                ctx:set_scale_of(door, 2, 2, 2)
                ctx:set_position(p.y, 0, 0)
                assert(ctx:position_of(424242424242) == nil)
                assert(ctx:set_position_of(424242424242, 0, 0, 0) == false)
            end,
        }
        "#,
    );
    let e = spawn(&app, "Mover", &[]);
    let door = app.resources.fetch_mut::<World>().spawn((
        Transform::from_xyz(0.0, 5.0, 0.0),
        Name("Door".into()),
        olvin_scene::Tag("gate".into()),
    ));
    step(&app);
    assert_eq!(x(&app, e), 6.0);
    let w = app.resources.fetch::<World>();
    assert_eq!(w.get::<&Transform>(door).unwrap().scale, Vec3::splat(2.0));
}

#[test]
fn messages_and_vars_between_lua_and_rust() {
    use olvin_core::{EventCursor, Events};
    let (app, mut lua) = app_with(
        "Sender",
        r#"
        return {
            update = function(self, ctx)
                if not self.sent then
                    self.sent = true
                    ctx:send("open", vec3(1, 2, 3), ctx:find_by_name("Box"))
                    ctx:send("hud", "hello")
                    ctx:set_var("door_open", true)
                end
            end,
        }
        "#,
    );
    lua.load_source(
        &mut app.resources.fetch_mut::<ScriptRegistry>(),
        "Receiver",
        r#"
        return {
            on_message = function(self, ctx, name, value, from)
                if name == "open" then
                    assert(from == ctx:find_by_name("hero"))
                    ctx:set_position(value + vec3(0, 0, ctx:var("bonus") or 0))
                elseif name == "game" then
                    ctx:set_var("heard", value)
                end
            end,
        }
        "#,
    )
    .unwrap();
    spawn(&app, "Sender", &[]);
    let rx = app.resources.fetch_mut::<World>().spawn((
        Transform::IDENTITY,
        Name("Box".into()),
        Scripts::new().with("Receiver", []),
    ));
    app.resources.fetch_mut::<ScriptVars>().set("bonus", 10_i64);
    step(&app); // Sender sends
    step(&app); // Receiver gets it
    {
        let w = app.resources.fetch::<World>();
        assert_eq!(
            w.get::<&Transform>(rx).unwrap().translation,
            Vec3::new(1.0, 2.0, 13.0)
        );
    }
    assert!(app.resources.fetch::<ScriptVars>().flag("door_open"));
    // The game reads script messages and sends its own.
    let mut cursor = EventCursor::default();
    let names: Vec<String> = app
        .resources
        .fetch::<Events<ScriptMessage>>()
        .read(&mut cursor)
        .map(|m| m.name.clone())
        .collect();
    assert_eq!(names, ["open", "hud"]);
    app.resources
        .fetch_mut::<Events<ScriptMessage>>()
        .send(ScriptMessage::new("game", "hi"));
    step(&app);
    assert_eq!(
        app.resources.fetch::<ScriptVars>().get("heard"),
        Some(&ScriptValue::Str("hi".into()))
    );
}

#[test]
fn bad_values_are_lua_errors_not_crashes() {
    let (app, _lua) = app_with(
        "Bad",
        r#"
        return {
            update = function(self, ctx)
                assert(not pcall(function() ctx:send("x", function() end) end))
                assert(not pcall(function() ctx:set_var("x", {}) end))
                assert(not pcall(function() ctx:send(string.rep("a", 20000)) end))
                ctx:translate(1, 0, 0)
                ctx:send("x", {})  -- uncaught: the script is disabled, the frame goes on
            end,
        }
        "#,
    );
    let e = spawn(&app, "Bad", &[]);
    step(&app);
    step(&app);
    assert_eq!(x(&app, e), 1.0, "ran once, then skipped");
    let mut registry = ScriptRegistry::default();
    let err = LuaScripts::new()
        .unwrap()
        .load_source(&mut registry, "NotFn", "return { update = 5 }")
        .unwrap_err();
    assert!(err.contains("must be a function"), "{err}");
}

#[cfg(feature = "audio")]
#[test]
fn lua_plays_and_stops_sounds() {
    use olvin_audio::Audio;
    let (mut app, _lua) = app_with(
        "Noisy",
        r#"
        return {
            start = function(self, ctx)
                self.a = ctx:play_sound("sfx/a.wav", { volume = 0.5, bus = "ambient", loop = true })
                self.b = ctx:play_sound_at("sfx/b.wav", vec3(1, 2, 3), { max_distance = 10 })
                self.c = ctx:play_sound_at("sfx/c.wav", ctx:entity())
                assert(self.a and self.b and self.c)
                assert(not ctx:sound_playing(self.a)) -- no device in tests
                assert(not pcall(function() ctx:play_sound("../secret.wav") end))
                assert(not pcall(function() ctx:play_sound("/etc/passwd") end))
                assert(not pcall(function() ctx:play_sound("x.wav", { bus = "nope" }) end))
                ctx:set_sound_volume(self.a, 0.2)
            end,
            update = function(self, ctx)
                ctx:stop_sound(self.a, 0.1)
                ctx:set_position(1, 0, 0)
            end,
        }
        "#,
    );
    app.insert_resource(Audio::disabled("assets"));
    let e = spawn(&app, "Noisy", &[]);
    step(&app);
    assert_eq!(x(&app, e), 1.0, "start's asserts held");
    assert_eq!(app.resources.fetch::<Audio>().active_count(), 2);
}

#[test]
fn scripts_in_subfolders_are_named_by_their_path_and_reload() {
    let dir = temp_dir("nested");
    fs::create_dir_all(dir.join("pack/deep")).unwrap();
    fs::write(
        dir.join("Plain.lua"),
        "return { update = function(self, ctx) ctx:translate(vec3(1,0,0)) end }",
    )
    .unwrap();
    let nested = dir.join("pack/mover.lua");
    fs::write(
        &nested,
        "return { update = function(self, ctx) ctx:translate(vec3(2,0,0)) end }",
    )
    .unwrap();
    fs::write(
        dir.join("pack/deep/other.lua"),
        "return { update = function(self, ctx) ctx:translate(vec3(4,0,0)) end }",
    )
    .unwrap();
    let mut app = App::new();
    app.add_plugin(LuaPlugin {
        dir: dir.clone(),
        hot_reload: false,
        limits: LuaLimits::default(),
    });
    let names = app.resources.fetch::<LuaScripts>().names();
    assert!(
        ["Plain", "pack/mover", "pack/deep/other"]
            .iter()
            .all(|n| names.iter().any(|m| m == n)),
        "{names:?}"
    );
    let (a, b, c) = (
        spawn(&app, "Plain", &[]),
        spawn(&app, "pack/mover", &[]),
        spawn(&app, "pack/deep/other", &[]),
    );
    step(&app);
    assert_eq!((x(&app, a), x(&app, b), x(&app, c)), (1.0, 2.0, 4.0));

    // A changed nested file reloads under its path name; a new one is found.
    fs::write(
        &nested,
        "return { update = function(self, ctx) ctx:translate(vec3(8,0,0)) end }",
    )
    .unwrap();
    bump_mtime(&nested);
    fs::write(
        dir.join("pack/late.lua"),
        "return { update = function(self, ctx) end }",
    )
    .unwrap();
    let mut reloaded = {
        let res = &app.resources;
        let mut lua = res.fetch_mut::<LuaScripts>();
        let mut registry = res.fetch_mut::<ScriptRegistry>();
        lua.check_disk(&mut registry)
    };
    reloaded.sort();
    assert_eq!(reloaded, ["pack/late", "pack/mover"]);
    step(&app);
    assert_eq!(x(&app, b), 2.0 + 8.0);
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "physics")]
#[test]
fn character_api_motor_grounded_gravity_and_children() {
    let (mut app, _lua) = app_with(
        "Driver",
        r#"return {
            start = function(self, ctx)
                self.kid = ctx:find_child("Kid")
                ctx:log("kid " .. tostring(self.kid))
            end,
            update = function(self, ctx)
                local vx, vy, vz = ctx:motor_velocity_xyz()
                ctx:set_motor_velocity_xyz(3, vy, 0)
                -- Report what the API saw as the position of the child and the gravity.
                local g = ctx:gravity()
                local kx = ctx:position_of_xyz(self.kid)
                ctx:set_position_xyz(kx, g, ctx:grounded() and 1 or 0)
            end,
        }"#,
    );
    app.add_plugin(olvin_physics::PhysicsPlugin::default());
    let (hero, kid, stranger) = {
        let mut world = app.resources.fetch_mut::<World>();
        let hero = world.spawn((
            Transform::IDENTITY,
            Name("hero".into()),
            olvin_physics::CharacterMotor::default(),
            Scripts::new().with("Driver", []),
        ));
        let kid = world.spawn((
            Transform::from_translation(Vec3::new(7.0, 0.0, 0.0)),
            Name("Kid".into()),
            olvin_transform::Parent(hero),
        ));
        // Same name, not a child of the hero: find_child must not return it.
        let stranger = world.spawn((Transform::IDENTITY, Name("Kid".into())));
        (hero, kid, stranger)
    };
    let _ = (kid, stranger);
    app.startup();
    step(&app);
    // gravity() is the world's (9.81 down), the child was found (x = 7).
    let t = *app
        .resources
        .fetch::<World>()
        .get::<&Transform>(hero)
        .unwrap();
    assert_eq!(t.translation.x, 7.0, "find_child + position_of");
    assert!(
        (t.translation.y - 9.81).abs() < 0.01,
        "gravity {}",
        t.translation.y
    );
    // The motor velocity was set from Lua (read back through the component).
    let world = app.resources.fetch::<World>();
    let motor = world.get::<&olvin_physics::CharacterMotor>(hero).unwrap();
    assert_eq!(motor.velocity.x, 3.0);
}

#[test]
fn spawn_prefab_queues_requests_and_keeps_to_the_sandbox() {
    let (app, _lua) = app_with(
        "Spawner",
        r#"
        return {
            params = { { name = "n", kind = "int", default = 1 } },
            update = function(self, ctx)
                self.sent = 0
                for i = 1, self.params.n do
                    if ctx:spawn_prefab("prefabs/a/Crate.prefab.ron", i, 2, 3, 90) then
                        self.sent = self.sent + 1
                    end
                end
                ctx:set_var("sent", self.sent)
            end,
        }
        "#,
    );
    spawn(&app, "Spawner", &[("n", "40")]);
    step(&app);
    // Capped per frame; the game drains the queue between frames.
    assert_eq!(
        app.resources.fetch::<ScriptVars>().get("sent"),
        Some(&ScriptValue::Int(MAX_SPAWNS_PER_FRAME as i64))
    );
    let queue = app.resources.fetch_mut::<SpawnRequests>().take();
    assert_eq!(queue.len(), MAX_SPAWNS_PER_FRAME);
    assert_eq!(queue[1].transform.translation, Vec3::new(2.0, 2.0, 3.0));
    assert!(
        (queue[0].transform.rotation * Vec3::NEG_Z).x.abs() > 0.99,
        "yaw 90"
    );

    // Alive cap: marked roots count against it.
    {
        let mut w = app.resources.fetch_mut::<World>();
        for _ in 0..MAX_SPAWNED_ALIVE - 3 {
            w.spawn((ScriptSpawned,));
        }
    }
    step(&app);
    assert_eq!(
        app.resources.fetch::<ScriptVars>().get("sent"),
        Some(&ScriptValue::Int(3))
    );
}

#[test]
fn spawn_prefab_refuses_paths_outside_assets() {
    for path in ["../x.prefab.ron", "/abs/x.prefab.ron", "prefabs/x.lua"] {
        let (app, _lua) = app_with(
            "Bad",
            &format!(
                r#"return {{ update = function(self, ctx) ctx:spawn_prefab("{path}", 0, 0, 0) end }}"#
            ),
        );
        spawn(&app, "Bad", &[]);
        step(&app);
        assert!(app.resources.fetch::<SpawnRequests>().is_empty(), "{path}");
    }
}

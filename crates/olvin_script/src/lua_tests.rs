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

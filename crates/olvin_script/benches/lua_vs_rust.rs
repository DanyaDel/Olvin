//! 5 000 entities, one `update` behaviour each, Lua against Rust.
//!
//! Every case goes through the engine's real path: `ScriptPlugin` +
//! `run_update` (attach, per-entity `ScriptCtx`, `catch_unwind`), and for Lua
//! `LuaScripts::load_source` → `LuaBehaviour` → `ctx` userdata.
//!
//! Cases:
//! - `empty`: `update` does nothing (pure call overhead per behaviour);
//! - `walker`: reads the `Move` input axis and `dt`, moves the Transform;
//! - Lua twice: with the default instruction limit (a count hook every 1000
//!   instructions, what games get) and with the limit off;
//! - `walker fast`: the same walker on the multi-value API
//!   (`ctx:axis2d_xy`, `ctx:translate_xyz`), no tables;
//! - Lua-only parts of the walker (one `ctx` call each) to see where its
//!   time goes, old table API next to the number API.
//!
//! Run: `cargo bench -p olvin_script --features lua --bench lua_vs_rust`
//! (`OLVIN_BENCH_FRAMES=N` changes the number of measured frames).

use std::time::{Duration, Instant};

use glam::Vec3;
use olvin_core::App;
use olvin_ecs::{Entity, World};
use olvin_input::{ActionState, GamepadState, Input, InputMap, KeyCode};
use olvin_scene::Transform;
use olvin_script::lua::{LuaLimits, LuaScripts};
use olvin_script::{
    Behaviour, FromParams, ParamSpec, Params, ScriptCtx, ScriptPlugin, ScriptRegistry, Scripts,
    run_update,
};

const ENTITIES: usize = 5_000;
const WARMUP: usize = 30;
const DT: f32 = 1.0 / 60.0;

// --- Rust behaviours -----------------------------------------------------

struct Empty;

impl FromParams for Empty {
    fn from_params(_: &Params) -> Self {
        Self
    }
}

impl Behaviour for Empty {
    fn update(&mut self, _: &mut ScriptCtx<'_>) {}
}

struct Walker {
    speed: f32,
}

impl FromParams for Walker {
    fn from_params(p: &Params) -> Self {
        Self {
            speed: p.f32("speed", 3.0),
        }
    }

    fn schema() -> Vec<ParamSpec> {
        vec![ParamSpec::float("speed", 3.0, "metres per second")]
    }
}

impl Behaviour for Walker {
    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        let m = ctx.input().axis2d("Move");
        let s = self.speed * ctx.dt();
        ctx.translate(Vec3::new(m.x * s, 0.0, -m.y * s));
    }
}

// --- The same in Lua ------------------------------------------------------

const LUA_EMPTY: &str = r#"
return {
    update = function(self, ctx) end,
}
"#;

const LUA_WALKER: &str = r#"
return {
    params = { { name = "speed", kind = "float", default = 3 } },
    update = function(self, ctx)
        local m = ctx:axis2d("Move")
        local s = self.params.speed * ctx:dt()
        ctx:translate(vec3(m.x * s, 0, -m.y * s))
    end,
}
"#;

// The same walker on the multi-value fast path: no tables cross the boundary.
const LUA_WALKER_FAST: &str = r#"
return {
    params = { { name = "speed", kind = "float", default = 3 } },
    update = function(self, ctx)
        local mx, my = ctx:axis2d_xy("Move")
        local s = self.params.speed * ctx:dt()
        ctx:translate_xyz(mx * s, 0, -my * s)
    end,
}
"#;

// Lua-only breakdown of the walker: which part of the bridge costs what.
const LUA_PARTS: &[(&str, &str)] = &[
    (
        "PureLua",
        "return { update = function(self, ctx) self.x = (self.x or 0) + 3 * 0.016 end }",
    ),
    (
        "Dt",
        "return { update = function(self, ctx) local s = ctx:dt() end }",
    ),
    (
        "Axis",
        r#"return { update = function(self, ctx) local m = ctx:axis2d("Move") end }"#,
    ),
    (
        "Translate",
        "return { update = function(self, ctx) ctx:translate(vec3(0.01, 0, 0)) end }",
    ),
    (
        "AxisXy",
        r#"return { update = function(self, ctx) local x, y = ctx:axis2d_xy("Move") end }"#,
    ),
    (
        "TranslateXyz",
        "return { update = function(self, ctx) ctx:translate_xyz(0.01, 0, 0) end }",
    ),
    (
        "TranslateNums",
        "return { update = function(self, ctx) ctx:translate(0.01, 0, 0) end }",
    ),
    (
        "Position",
        "return { update = function(self, ctx) local p = ctx:position() end }",
    ),
    (
        "PositionXyz",
        "return { update = function(self, ctx) local x, y, z = ctx:position_xyz() end }",
    ),
];

// --- Harness ---------------------------------------------------------------

enum Impl {
    Rust,
    Lua(LuaLimits),
}

struct Case {
    label: &'static str,
    imp: Impl,
    script: &'static str,
}

struct Measured {
    median: Duration,
    min: Duration,
    p90: Duration,
    /// Position of the first entity afterwards (walker sanity check).
    moved: Vec3,
}

fn setup(case: &Case) -> (App, Option<LuaScripts>, Entity) {
    let mut app = App::new();
    app.add_plugin(ScriptPlugin { builtins: false });

    // Hold D and W: `Move` = (1, 1)-ish, like a player walking diagonally.
    let mut input = Input::default();
    input.set_key(KeyCode::KeyD, true);
    input.set_key(KeyCode::KeyW, true);
    let mut actions = ActionState::default();
    actions.update(&InputMap::default(), &input, &GamepadState::default());
    app.insert_resource(actions);

    let lua = {
        let mut registry = app.resources.fetch_mut::<ScriptRegistry>();
        match &case.imp {
            Impl::Rust => {
                registry.register::<Empty>("Empty");
                registry.register::<Walker>("Walker");
                registry.register::<Walker>("WalkerFast");
                None
            }
            Impl::Lua(limits) => {
                let mut lua = LuaScripts::with_limits(*limits).expect("lua state");
                lua.load_source(&mut registry, "Empty", LUA_EMPTY).unwrap();
                lua.load_source(&mut registry, "Walker", LUA_WALKER)
                    .unwrap();
                lua.load_source(&mut registry, "WalkerFast", LUA_WALKER_FAST)
                    .unwrap();
                for (name, source) in LUA_PARTS {
                    lua.load_source(&mut registry, name, source).unwrap();
                }
                Some(lua)
            }
        }
    };

    let first = {
        let mut world = app.resources.fetch_mut::<World>();
        let mut first = None;
        for i in 0..ENTITIES {
            let e = world.spawn((
                Transform::from_xyz(0.0, 0.0, i as f32),
                Scripts::new().with(case.script, [("speed", "3")]),
            ));
            first.get_or_insert(e);
        }
        first.unwrap()
    };
    (app, lua, first)
}

fn run(case: &Case, frames: usize) -> Measured {
    let (app, _lua, first) = setup(case);
    let start_pos = app
        .resources
        .fetch::<World>()
        .get::<&Transform>(first)
        .unwrap()
        .translation;
    // The first pass also attaches instances and runs `start`.
    for _ in 0..WARMUP {
        run_update(&app.resources, Some(DT));
    }
    let mut times: Vec<Duration> = (0..frames)
        .map(|_| {
            let t = Instant::now();
            run_update(&app.resources, Some(DT));
            t.elapsed()
        })
        .collect();
    times.sort();
    let end = app
        .resources
        .fetch::<World>()
        .get::<&Transform>(first)
        .unwrap()
        .translation;
    Measured {
        median: times[times.len() / 2],
        min: times[0],
        p90: times[times.len() * 9 / 10],
        moved: end - start_pos,
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn main() {
    let frames: usize = std::env::var("OLVIN_BENCH_FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300)
        .max(1);
    let no_limit = LuaLimits {
        instructions: 0,
        ..LuaLimits::default()
    };
    let cases = [
        Case {
            label: "rust  empty",
            imp: Impl::Rust,
            script: "Empty",
        },
        Case {
            label: "lua   empty",
            imp: Impl::Lua(LuaLimits::default()),
            script: "Empty",
        },
        Case {
            label: "lua   empty (no instr. limit)",
            imp: Impl::Lua(no_limit),
            script: "Empty",
        },
        Case {
            label: "rust  walker",
            imp: Impl::Rust,
            script: "Walker",
        },
        Case {
            label: "lua   walker",
            imp: Impl::Lua(LuaLimits::default()),
            script: "Walker",
        },
        Case {
            label: "lua   walker (no instr. limit)",
            imp: Impl::Lua(no_limit),
            script: "Walker",
        },
        Case {
            label: "lua   walker fast (*_xy/_xyz)",
            imp: Impl::Lua(LuaLimits::default()),
            script: "WalkerFast",
        },
        Case {
            label: "lua   walker fast (no limit)",
            imp: Impl::Lua(no_limit),
            script: "WalkerFast",
        },
        Case {
            label: "lua   part: pure Lua, no ctx",
            imp: Impl::Lua(LuaLimits::default()),
            script: "PureLua",
        },
        Case {
            label: "lua   part: ctx:dt()",
            imp: Impl::Lua(LuaLimits::default()),
            script: "Dt",
        },
        Case {
            label: "lua   part: ctx:axis2d(\"Move\")",
            imp: Impl::Lua(LuaLimits::default()),
            script: "Axis",
        },
        Case {
            label: "lua   part: ctx:translate(vec3())",
            imp: Impl::Lua(LuaLimits::default()),
            script: "Translate",
        },
        Case {
            label: "lua   part: ctx:axis2d_xy(\"Move\")",
            imp: Impl::Lua(LuaLimits::default()),
            script: "AxisXy",
        },
        Case {
            label: "lua   part: ctx:translate_xyz(x,y,z)",
            imp: Impl::Lua(LuaLimits::default()),
            script: "TranslateXyz",
        },
        Case {
            label: "lua   part: ctx:translate(x,y,z)",
            imp: Impl::Lua(LuaLimits::default()),
            script: "TranslateNums",
        },
        Case {
            label: "lua   part: ctx:position()",
            imp: Impl::Lua(LuaLimits::default()),
            script: "Position",
        },
        Case {
            label: "lua   part: ctx:position_xyz()",
            imp: Impl::Lua(LuaLimits::default()),
            script: "PositionXyz",
        },
    ];

    println!(
        "{ENTITIES} entities x 1 update behaviour, {WARMUP} warmup + {frames} measured frames, run_update per frame"
    );
    println!(
        "{:<38} {:>10} {:>10} {:>10} {:>12} {:>9}",
        "case", "median ms", "min ms", "p90 ms", "ns / update", "% 16.6ms"
    );
    let mut walked = Vec::new();
    for case in &cases {
        let r = run(case, frames);
        let per_call = r.median.as_secs_f64() * 1e9 / ENTITIES as f64;
        println!(
            "{:<38} {:>10.3} {:>10.3} {:>10.3} {:>12.1} {:>8.1}%",
            case.label,
            ms(r.median),
            ms(r.min),
            ms(r.p90),
            per_call,
            ms(r.median) / (1000.0 / 60.0) * 100.0
        );
        if case.script.starts_with("Walker") {
            walked.push((case.label, r.moved));
        }
    }
    // Both implementations must do the same work.
    let (_, reference) = walked[0];
    assert!(reference.length() > 0.1, "walker did not move: {reference}");
    for (label, moved) in &walked {
        assert!(
            moved.abs_diff_eq(reference, 1e-3),
            "`{label}` moved {moved}, rust moved {reference}"
        );
    }
    println!("walker check: every implementation moved {reference} per entity");
}

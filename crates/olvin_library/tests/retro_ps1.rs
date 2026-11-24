//! The retro-ps1 pack, imported into a temp project and played headless: the
//! demo scene loads, its Lua flicker script runs without a warning, and the
//! corridor lamp is parked out of range some of the time and back at others.

use std::sync::Mutex;

use olvin::prelude::*;
use olvin::script::lua::{LuaLimits, LuaPlugin};
use olvin_library::{ImportOptions, Library};

static PROBLEMS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            PROBLEMS
                .lock()
                .unwrap()
                .push(format!("[{}] {}", r.level(), r.args()));
        }
    }
    fn flush(&self) {}
}

#[test]
fn the_demo_scene_plays_and_the_lamp_flickers() {
    let _ = log::set_logger(&Capture);
    log::set_max_level(log::LevelFilter::Warn);
    let dir = tempfile::tempdir().unwrap();
    let report = Library::embedded()
        .import("retro-ps1", dir.path(), &ImportOptions::default())
        .unwrap();
    assert!(report.completed && report.warnings.is_empty(), "{report:?}");

    let mut app = App::new();
    app.insert_resource(ProjectPaths::new(dir.path()));
    app.insert_resource(Time::with_fixed_delta(1.0 / 60.0));
    app.add_plugin(HeadlessPlugins::default());
    app.add_plugin(LuaPlugin {
        dir: dir.path().join("assets/scripts"),
        hot_reload: false,
        limits: LuaLimits::default(),
    });
    let scene = dir.path().join("assets/scenes/retro-ps1/demo.scene.ron");
    let spawned = olvin::scene_file::load(&app.resources, &scene).unwrap();
    assert!(spawned > 30, "the demo scene is not empty: {spawned}");

    let lamp_y = |app: &App| {
        let world = app.resources.fetch::<World>();
        world
            .query::<(&Name, &Transform)>()
            .iter()
            .find(|(n, _)| n.0.starts_with("Lamp Corridor"))
            .map(|(_, t)| t.translation.y)
            .expect("the corridor lamp")
    };
    let (mut on, mut off) = (0, 0);
    for _ in 0..240 {
        app.update();
        if lamp_y(&app) > 0.0 {
            on += 1;
        } else {
            off += 1;
        }
    }
    assert!(on > 20 && off > 20, "flickers: {on} frames on, {off} off");
    let problems = PROBLEMS.lock().unwrap();
    assert!(
        problems.is_empty(),
        "logged problems:\n{}",
        problems.join("\n")
    );
}

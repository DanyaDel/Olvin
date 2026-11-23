//! The gameplay packs (doors, pickups, health, checkpoints, platforms, enemies,
//! spawners, camera effects), imported into a temp project and played headless:
//! `HeadlessPlugins` + the Lua plugin + the pack's demo scene, fixed 1/60 s
//! frames, keys and mouse fed into the `Input` resource like a window would.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use glam::EulerRot;
use olvin::core::Events;
use olvin::input::Input;
use olvin::prelude::*;
use olvin::script::lua::{LuaLimits, LuaPlugin};
use olvin::script::{ScriptMessage, ScriptSpawned, ScriptValue, ScriptVars};
use olvin::window::KeyCode;
use olvin_library::{ImportOptions, Library};

const DT: f32 = 1.0 / 60.0;

/// Warnings and errors from every test in this binary (Lua errors go to the log).
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
                .push(format!("[{}] {}: {}", r.level(), r.target(), r.args()));
        }
    }
    fn flush(&self) {}
}

fn capture_log() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = log::set_logger(&Capture);
        log::set_max_level(log::LevelFilter::Warn);
    });
}

/// No warning or error was logged so far (by any test of this binary).
fn assert_clean_log() {
    let problems = PROBLEMS.lock().unwrap();
    assert!(
        problems.is_empty(),
        "logged problems:\n{}",
        problems.join("\n")
    );
}

struct Rig {
    app: App,
    project: PathBuf,
    _dir: tempfile::TempDir,
    cursor: olvin::core::EventCursor,
    /// Every message sent so far: name, value, sender's name.
    log: Vec<(String, ScriptValue, Option<String>)>,
}

/// Imports `pack` into a fresh project (`edit` may rewrite the imported files
/// first, to change script parameters) and loads its demo scene.
fn rig(pack: &str, edit: impl FnOnce(&Path)) -> Rig {
    capture_log();
    let dir = tempfile::tempdir().unwrap();
    let library = Library::embedded();
    let report = library
        .import(pack, dir.path(), &ImportOptions::default())
        .unwrap();
    assert!(report.completed, "{report:?}");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    edit(&dir.path().join("assets"));

    let mut app = App::new();
    app.insert_resource(ProjectPaths::new(dir.path()));
    app.insert_resource(Time::with_fixed_delta(DT));
    app.add_plugin(HeadlessPlugins::default());
    let scripts = dir.path().join("assets/scripts");
    app.add_plugin(LuaPlugin {
        dir: scripts,
        hot_reload: false,
        limits: LuaLimits::default(),
    });
    let scene = dir
        .path()
        .join(format!("assets/scenes/{pack}/demo.scene.ron"));
    let spawned = olvin::scene_file::load(&app.resources, &scene).unwrap();
    assert!(spawned > 3, "the demo scene is not empty");
    let mut rig = Rig {
        app,
        project: dir.path().to_path_buf(),
        _dir: dir,
        cursor: Default::default(),
        log: Vec::new(),
    };
    // Bodies are created and the character settles on the floor.
    rig.frames(30);
    rig
}

/// Replaces `from` with `to` in a project file (relative to `assets/`).
fn replace_in(assets: &Path, file: &str, from: &str, to: &str) {
    let path = assets.join(file);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "`{from}` is not in {file}");
    std::fs::write(&path, text.replace(from, to)).unwrap();
}

impl Rig {
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.app.update();
            self.collect_messages();
            // Nothing else ends the frame for the input resource here.
            self.app.resources.fetch_mut::<Input>().mouse_delta = Vec2::ZERO;
        }
    }

    fn collect_messages(&mut self) {
        let new: Vec<ScriptMessage> = {
            let events = self.app.resources.fetch::<Events<ScriptMessage>>();
            events.read(&mut self.cursor).cloned().collect()
        };
        let world = self.app.resources.fetch::<World>();
        for m in new {
            let from = m
                .from
                .and_then(|e| world.get::<&Name>(e).ok().map(|n| n.0.clone()));
            self.log.push((m.name, m.value, from));
        }
    }

    /// How many messages called `name` were sent so far.
    fn sent(&self, name: &str) -> usize {
        self.log.iter().filter(|(n, _, _)| n == name).count()
    }

    fn key(&self, key: KeyCode, down: bool) {
        self.app.resources.fetch_mut::<Input>().set_key(key, down);
    }

    /// One frame with the mouse moved by `delta` pixels.
    fn mouse(&mut self, delta: Vec2) {
        self.app.resources.fetch_mut::<Input>().mouse_delta = delta;
        self.app.update();
        self.app.resources.fetch_mut::<Input>().mouse_delta = Vec2::ZERO;
    }

    /// Holds `keys` for `n` frames, then lets go.
    fn hold(&mut self, keys: &[KeyCode], n: usize) {
        for k in keys {
            self.key(*k, true);
        }
        self.frames(n);
        for k in keys {
            self.key(*k, false);
        }
    }

    fn entity(&self, name: &str) -> Entity {
        let world = self.app.resources.fetch::<World>();
        world
            .query::<(Entity, &Name)>()
            .iter()
            .find(|(_, n)| n.0 == name)
            .map(|(e, _)| e)
            .unwrap_or_else(|| panic!("no entity `{name}`"))
    }

    /// Local transform.
    fn transform(&self, name: &str) -> Transform {
        let e = self.entity(name);
        *self
            .app
            .resources
            .fetch::<World>()
            .get::<&Transform>(e)
            .unwrap()
    }

    fn pos(&self, name: &str) -> Vec3 {
        self.transform(name).translation
    }

    /// World position (through `GlobalTransform`, after the last frame).
    fn world_pos(&self, name: &str) -> Vec3 {
        let e = self.entity(name);
        self.app
            .resources
            .fetch::<World>()
            .get::<&GlobalTransform>(e)
            .unwrap()
            .0
            .w_axis
            .truncate()
    }

    /// Yaw in degrees of an entity's local rotation.
    fn yaw(&self, name: &str) -> f32 {
        self.transform(name)
            .rotation
            .to_euler(EulerRot::YXZ)
            .0
            .to_degrees()
    }

    fn pitch(&self, name: &str) -> f32 {
        self.transform(name)
            .rotation
            .to_euler(EulerRot::YXZ)
            .1
            .to_degrees()
    }

    fn grounded(&self, name: &str) -> bool {
        let e = self.entity(name);
        self.app
            .resources
            .fetch::<World>()
            .get::<&CharacterMotor>(e)
            .unwrap()
            .grounded
    }
}

fn horizontal(v: Vec3) -> f32 {
    Vec3::new(v.x, 0.0, v.z).length()
}

impl Rig {
    fn var(&self, name: &str) -> Option<ScriptValue> {
        self.app.resources.fetch::<ScriptVars>().get(name).cloned()
    }

    fn num(&self, name: &str) -> f64 {
        self.var(name).and_then(|v| v.as_f64()).unwrap_or(0.0)
    }

    fn exists(&self, name: &str) -> bool {
        let world = self.app.resources.fetch::<World>();
        world.query::<&Name>().iter().any(|n| n.0 == name)
    }

    /// Sends a message the way a game system would (delivered next frame).
    fn send(&self, name: &str, value: impl Into<ScriptValue>, target: Option<&str>) {
        let mut msg = ScriptMessage::new(name, value);
        if let Some(t) = target {
            msg = msg.to(self.entity(t));
        }
        self.app
            .resources
            .fetch_mut::<Events<ScriptMessage>>()
            .send(msg);
    }

    /// Moves an entity (local position) and resets its physics body.
    fn teleport(&mut self, name: &str, to: Vec3) {
        let e = self.entity(name);
        self.app
            .resources
            .fetch_mut::<World>()
            .get::<&mut Transform>(e)
            .unwrap()
            .translation = to;
    }

    /// World position of the moving platform's Body in the arena (the guard has a "Body" too).
    fn platform_body(&self) -> Vec3 {
        let root = self.entity("Platform");
        let world = self.app.resources.fetch::<World>();
        let found = world
            .query::<(&Name, &olvin::transform::Parent, &GlobalTransform)>()
            .iter()
            .find(|(n, par, _)| n.0 == "Body" && par.0 == root)
            .map(|(_, _, g)| g.0.w_axis.truncate());
        found.expect("the platform's Body")
    }

    /// Local transform of the child `child` of the entity `parent`.
    fn child(&self, parent: &str, child: &str) -> Transform {
        let p = self.entity(parent);
        let world = self.app.resources.fetch::<World>();
        let found = world
            .query::<(&Name, &olvin::transform::Parent, &Transform)>()
            .iter()
            .find(|(n, par, _)| n.0 == child && par.0 == p)
            .map(|(_, _, t)| *t);
        found.unwrap_or_else(|| panic!("no child `{child}` of `{parent}`"))
    }

    /// Frames until `done` is true (at most `max`); returns whether it was.
    fn until(&mut self, max: usize, mut done: impl FnMut(&Rig) -> bool) -> bool {
        for _ in 0..max {
            if done(self) {
                return true;
            }
            self.frames(1);
        }
        done(self)
    }
}

// --- Pack contents ------------------------------------------------------------

const PACKS: [&str; 4] = [
    "interactive-door",
    "pickups",
    "health-damage",
    "checkpoints",
];

#[test]
fn the_gameplay_packs_are_well_formed() {
    let library = Library::embedded();
    assert!(library.problems().is_empty(), "{:?}", library.problems());
    for id in PACKS.into_iter().chain(["gameplay-demo"]) {
        let pack = library.get(id).unwrap_or_else(|| panic!("no pack {id}"));
        assert_eq!(pack.manifest.category, olvin_library::Category::Gameplay);
        if id != "gameplay-demo" {
            assert!(pack.headline_prefab().is_some(), "{id}: a headline prefab");
        }
        assert!(pack.demo_scene().is_some(), "{id}: a demo scene");
        assert!(pack.lint().is_empty(), "{id}: {:?}", pack.lint());
        assert!(!pack.manifest.notes.as_deref().unwrap_or("").is_empty());
        for f in &pack.files {
            if f.path.ends_with(".scene.ron") || f.path.ends_with(".prefab.ron") {
                let text = std::str::from_utf8(&f.data).unwrap();
                olvin::scene::file::SceneFile::parse(text)
                    .unwrap_or_else(|e| panic!("{id}/{}: {e}", f.path));
            }
        }
    }
}

// --- Doors ----------------------------------------------------------------------

#[test]
fn door_opens_when_the_player_comes_near_and_closes_after_leaving() {
    let mut rig = rig("interactive-door", |_| {});
    let closed = rig.child("Door A (proximity)", "Hinge").translation;
    rig.frames(60);
    assert_eq!(
        rig.child("Door A (proximity)", "Hinge").translation,
        closed,
        "stays shut"
    );
    // Walk to the door (it is 10 m ahead); the trigger reaches 2 m in front of it.
    rig.key(KeyCode::KeyW, true);
    assert!(
        rig.until(200, |r| r.pos("Player").z < -3.0),
        "walked through the open door"
    );
    rig.key(KeyCode::KeyW, false);
    rig.frames(60);
    let open = rig.child("Door A (proximity)", "Hinge").translation;
    assert!(open.x - closed.x > 2.0, "slid open: {closed} -> {open}");
    assert!(
        rig.pos("Player").z < -3.0,
        "went through: {}",
        rig.pos("Player")
    );
    // Walk away behind the wall: the door closes after the delay.
    rig.hold(&[KeyCode::KeyW], 120);
    assert!(rig.until(
        240,
        |r| r.child("Door A (proximity)", "Hinge").translation.x < closed.x + 0.01
    ));
    assert_clean_log();
}

#[test]
fn a_closed_door_blocks_and_the_key_door_opens_on_interact() {
    let mut rig = rig("interactive-door", |_| {});
    rig.teleport("Player", Vec3::new(-8.0, 1.0, 0.0));
    rig.frames(10);
    rig.hold(&[KeyCode::KeyW], 150);
    let z = rig.pos("Player").z;
    assert!(z > -3.5, "the closed door blocks, player at z {z}");
    let hinge0 = rig.child("Door B (press E)", "Hinge").rotation;
    assert!(
        hinge0.to_euler(EulerRot::YXZ).0.abs() < 0.01,
        "still closed without E"
    );
    // Press E: the hinged door swings; then walk through.
    rig.hold(&[KeyCode::KeyE], 2);
    rig.frames(70);
    let yaw = rig
        .child("Door B (press E)", "Hinge")
        .rotation
        .to_euler(EulerRot::YXZ)
        .0
        .to_degrees();
    assert!((yaw - 100.0).abs() < 1.0, "swung {yaw} degrees");
    rig.hold(&[KeyCode::KeyW], 120);
    assert!(
        rig.pos("Player").z < -5.0,
        "walked through: {}",
        rig.pos("Player")
    );
    assert_clean_log();
}

#[test]
fn the_locked_door_waits_for_the_plate_and_announces_it() {
    let mut rig = rig("interactive-door", |_| {});
    let closed = rig.child("Door C (locked)", "Hinge").translation;
    // Standing in front of the locked door: it refuses.
    rig.teleport("Player", Vec3::new(8.0, 1.0, -2.5));
    rig.frames(60);
    assert_eq!(rig.child("Door C (locked)", "Hinge").translation, closed);
    // The plate sends "unlock" (value: the door's id); the player is still in the zone, so it needs a new approach.
    rig.teleport("Player", Vec3::new(8.0, 1.0, 3.0));
    rig.frames(20);
    rig.teleport("Player", Vec3::new(8.0, 1.0, -2.5));
    rig.frames(90);
    let open = rig.child("Door C (locked)", "Hinge").translation;
    assert!(open.x - closed.x > 2.0, "unlocked and opened: {open}");
    assert_clean_log();
}

// --- Pickups --------------------------------------------------------------------

#[test]
fn coins_are_collected_and_counted() {
    let mut rig = rig("pickups", |_| {});
    assert_eq!(rig.num("score"), 0.0);
    // Coin 3 is at (-1, 0, 2).
    rig.teleport("Player", Vec3::new(-1.0, 1.0, 4.5));
    rig.hold(&[KeyCode::KeyW], 60);
    assert_eq!(rig.num("score"), 1.0);
    assert!(!rig.exists("Coin 3"), "the collected coin is gone");
    assert!(rig.exists("Coin 2"), "the others stay");
    assert_eq!(rig.sent("picked_up"), 1);
    assert_eq!(rig.log.last().unwrap().1, ScriptValue::Str("coin".into()));
    // Walking over the same spot again adds nothing.
    rig.teleport("Player", Vec3::new(-1.0, 1.0, 4.5));
    rig.hold(&[KeyCode::KeyW], 60);
    assert_eq!(rig.num("score"), 1.0);
    assert_clean_log();
}

#[test]
fn pickups_spin_and_bob() {
    let mut rig = rig("pickups", |_| {});
    let a = rig.child("Coin 1", "Visual");
    rig.frames(25);
    let b = rig.child("Coin 1", "Visual");
    assert!((a.translation.y - b.translation.y).abs() > 0.01, "bobs");
    assert!(a.rotation.angle_between(b.rotation) > 0.3, "spins");
    assert_clean_log();
}

#[test]
fn the_key_sets_its_variables_and_a_respawning_coin_comes_back() {
    let mut rig = rig("pickups", |_| {});
    rig.teleport("Player", Vec3::new(4.0, 1.0, -1.0));
    rig.hold(&[KeyCode::KeyW], 45);
    assert_eq!(rig.var("key_red"), Some(ScriptValue::Bool(true)));
    assert_eq!(rig.num("keys"), 1.0);
    assert!(!rig.exists("Key"));

    // The respawning coin (10 points) at (0, 0, -2): hidden, then back after 3 s.
    rig.teleport("Player", Vec3::new(0.0, 1.0, 1.0));
    rig.hold(&[KeyCode::KeyW], 45);
    assert_eq!(rig.num("score"), 10.0);
    assert!(rig.exists("Coin Respawning"));
    assert_eq!(rig.child("Coin Respawning", "Visual").scale, Vec3::ZERO);
    rig.frames(200);
    assert!(
        rig.child("Coin Respawning", "Visual").scale.x > 0.4,
        "visible again"
    );
    // Back at its place and collectable once more.
    rig.teleport("Player", Vec3::new(0.0, 1.0, 1.0));
    rig.hold(&[KeyCode::KeyW], 45);
    assert_eq!(rig.num("score"), 20.0);
    assert_clean_log();
}

// --- Health and damage -----------------------------------------------------------

#[test]
fn health_follows_damage_heal_and_invulnerability() {
    let mut rig = rig("health-damage", |_| {});
    assert_eq!(rig.num("health"), 100.0);
    assert_eq!(rig.num("health_max"), 100.0);
    // Two hits in one frame: the second falls into the invulnerability window.
    rig.send("damage", 30.0, Some("Player"));
    rig.send("damage", 30.0, Some("Player"));
    rig.frames(3);
    assert_eq!(rig.num("health"), 70.0);
    assert_eq!(rig.sent("damaged"), 1);
    rig.frames(40);
    rig.send("damage", 10.0, Some("Player"));
    rig.send("heal", 5.0, Some("Player"));
    rig.frames(3);
    assert_eq!(rig.num("health"), 65.0);
    rig.send("heal", 500.0, Some("Player"));
    rig.frames(3);
    assert_eq!(rig.num("health"), 100.0, "capped at the maximum");
    assert_clean_log();
}

#[test]
fn death_broadcasts_died_and_the_player_returns_after_the_delay() {
    let mut rig = rig("health-damage", |_| {});
    rig.send("damage", 250.0, Some("Player"));
    rig.frames(3);
    assert_eq!(rig.num("health"), 0.0);
    assert_eq!(rig.sent("died"), 1);
    let died = rig.log.iter().find(|m| m.0 == "died").unwrap();
    assert_eq!(died.1, ScriptValue::Str("Player".into()));
    assert_eq!(died.2.as_deref(), Some("Player"));
    // A dead player takes no more damage or healing.
    rig.send("heal", 50.0, Some("Player"));
    rig.frames(3);
    assert_eq!(rig.num("health"), 0.0);
    // respawn_delay is 3 s in the demo: full health and a "respawn" message.
    rig.frames(190);
    assert_eq!(rig.num("health"), 100.0);
    assert_eq!(rig.sent("respawn"), 1);
    assert_clean_log();
}

#[test]
fn the_lava_hurts_over_time_and_the_spikes_per_touch() {
    let mut rig = rig("health-damage", |_| {});
    rig.key(KeyCode::KeyW, true);
    assert!(
        rig.until(200, |r| r.num("health") < 100.0),
        "the lava damages"
    );
    rig.key(KeyCode::KeyW, false);
    assert_eq!(rig.num("health"), 90.0);
    rig.frames(65);
    assert_eq!(rig.num("health"), 80.0, "one tick per second while inside");
    // Out of the lava no more damage.
    rig.teleport("Player", Vec3::new(0.0, 1.0, 6.0));
    rig.send("heal", 100.0, Some("Player"));
    rig.frames(90);
    assert_eq!(rig.num("health"), 100.0);
    // The spikes: 20 per touch.
    rig.teleport("Player", Vec3::new(6.0, 1.0, 0.0));
    rig.frames(20);
    assert_eq!(rig.num("health"), 80.0);
    assert_clean_log();
}

// --- Checkpoints -------------------------------------------------------------------

#[test]
fn the_start_checkpoint_is_active_and_falling_respawns_there() {
    let mut rig = rig("checkpoints", |_| {});
    assert_eq!(
        rig.var("checkpoint"),
        Some(ScriptValue::Str("Checkpoint Start".into()))
    );
    // The flag of the active checkpoint is up.
    assert!(rig.child("Checkpoint Start", "Flag").translation.y > 1.8);
    assert!(rig.child("Checkpoint Far", "Flag").translation.y < 1.6);
    // Walk off the slab into the pit.
    rig.key(KeyCode::KeyW, true);
    assert!(rig.until(300, |r| r.pos("Player").y < -3.0), "falls");
    rig.key(KeyCode::KeyW, false);
    assert!(rig.until(300, |r| r.sent("respawned") > 0), "respawned");
    rig.frames(60);
    let p = rig.pos("Player");
    assert!(
        p.distance(Vec3::new(0.0, 0.92, 6.0)) < 0.3,
        "back at the start: {p}"
    );
    assert!(rig.grounded("Player"));
    assert_clean_log();
}

#[test]
fn the_last_reached_checkpoint_is_the_respawn_point() {
    let mut rig = rig("checkpoints", |_| {});
    rig.teleport("Player", Vec3::new(0.0, 1.0, -18.0));
    rig.frames(30);
    assert_eq!(
        rig.var("checkpoint"),
        Some(ScriptValue::Str("Checkpoint Far".into()))
    );
    assert_eq!(rig.sent("checkpoint_reached"), 2);
    assert!(
        rig.child("Checkpoint Start", "Flag").translation.y < 1.6,
        "the old flag is down"
    );
    // Fall from the pit's middle: respawn at the far one.
    rig.teleport("Player", Vec3::new(0.0, 1.0, -9.0));
    assert!(rig.until(300, |r| r.sent("respawned") > 0));
    rig.frames(60);
    let p = rig.pos("Player");
    assert!(
        p.distance(Vec3::new(0.0, 0.92, -18.0)) < 0.3,
        "back at the far checkpoint: {p}"
    );
    assert_clean_log();
}

// --- Moving platforms ------------------------------------------------------------

/// Walks onto the platform waiting at the near edge and stops there.
fn board_platform(rig: &mut Rig) {
    rig.key(KeyCode::KeyW, true);
    assert!(
        rig.until(200, |r| r.pos("Player").z < -4.2),
        "on the platform"
    );
    rig.key(KeyCode::KeyW, false);
    rig.frames(20);
    assert!(
        rig.pos("Player").y > 0.8,
        "standing on it: {}",
        rig.pos("Player")
    );
}

#[test]
fn the_platform_carries_the_player_over_the_pit() {
    let mut rig = rig("moving-platforms", |_| {});
    let body0 = rig.world_pos("Body");
    board_platform(&mut rig);
    // It waits, then travels 6 m at 2 m/s towards -Z.
    assert!(
        rig.until(400, |r| r.world_pos("Body").z < body0.z - 5.9),
        "arrived"
    );
    let body = rig.world_pos("Body");
    let player = rig.pos("Player");
    assert!(
        (player.z - body.z).abs() < 1.8 && (player.x - body.x).abs() < 1.8,
        "still on it: player {player}, platform {body}"
    );
    assert!(
        player.y > 0.8 && player.z < -8.0,
        "carried across the pit: {player}"
    );
    // Walk off on the far side.
    rig.hold(&[KeyCode::KeyW], 80);
    assert!(
        rig.pos("Player").z < -13.5 && rig.pos("Player").y > 0.8,
        "on the far slab: {}",
        rig.pos("Player")
    );
    assert_clean_log();
}

#[test]
fn without_carry_the_player_is_left_behind() {
    let mut rig = rig("moving-platforms", |assets| {
        replace_in(
            assets,
            "scenes/moving-platforms/demo.scene.ron",
            "\"carry\":\"true\"",
            "\"carry\":\"false\"",
        );
    });
    board_platform(&mut rig);
    rig.frames(300);
    let (body, player) = (rig.world_pos("Body"), rig.pos("Player"));
    assert!(body.z < -10.5, "the platform left: {body}");
    assert!(
        player.z > -6.5 || player.y < -1.0,
        "the player did not ride: {player}"
    );
    assert_clean_log();
}

#[test]
fn the_platform_waits_and_comes_back() {
    let mut rig = rig("moving-platforms", |_| {});
    let z0 = rig.world_pos("Body").z;
    rig.frames(60 * 3 + 30);
    assert!(
        (rig.world_pos("Body").z - z0).abs() > 0.1,
        "left after the wait"
    );
    assert!(
        rig.until(60 * 7, |r| r.world_pos("Body").z < z0 - 5.9),
        "far point"
    );
    rig.frames(60 * 3);
    assert!(
        rig.until(60 * 7, |r| (r.world_pos("Body").z - z0).abs() < 0.05),
        "back at the start"
    );
    assert_clean_log();
}

#[test]
fn the_spinner_turns() {
    let mut rig = rig("moving-platforms", |_| {});
    let a = rig.transform("Spinner").rotation;
    rig.frames(60);
    let b = rig.transform("Spinner").rotation;
    let turned = a.angle_between(b).to_degrees();
    assert!((turned - 60.0).abs() < 3.0, "60 deg/s: {turned}");
    assert_clean_log();
}

// --- Enemy AI --------------------------------------------------------------------

#[test]
fn the_guard_patrols_between_its_points() {
    let mut rig = rig("enemy-ai", |_| {});
    let start = rig.pos("Guard");
    rig.frames(60 * 4);
    let moved = rig.pos("Guard");
    assert!(horizontal(moved - start) > 3.0, "walked {start} -> {moved}");
    assert!((moved.z + 8.0).abs() < 1.0, "stays on its line");
    assert!(
        rig.until(60 * 12, |r| r.pos("Guard").x > 5.0),
        "reaches the other point"
    );
    assert!(
        rig.until(60 * 12, |r| r.pos("Guard").x < -5.0),
        "and comes back"
    );
    assert_eq!(rig.sent("enemy_alert"), 0, "the player is far away");
    assert_clean_log();
}

#[test]
fn the_guard_chases_hits_and_gives_up() {
    let mut rig = rig("enemy-ai", |_| {});
    // In its view cone: it faces +X at first, so wait for it to turn towards -X, then stand in front.
    rig.teleport("Player", Vec3::new(0.0, 1.0, -3.0));
    assert!(rig.until(60 * 6, |r| r.sent("enemy_alert") > 0), "spotted");
    assert!(
        rig.until(60 * 6, |r| r.num("health") < 100.0),
        "hit the player"
    );
    assert!(
        horizontal(rig.pos("Guard") - rig.pos("Player")) < 2.6,
        "stands next to the player"
    );
    let damaged = rig.sent("damaged");
    rig.frames(60 * 3);
    assert!(
        rig.sent("damaged") > damaged,
        "hits again after the cooldown"
    );
    assert!(rig.num("health") <= 80.0);
    // Far away: gives up and patrols again.
    rig.teleport("Player", Vec3::new(0.0, 1.0, 16.0));
    assert!(rig.until(60 * 10, |r| r.sent("enemy_lost") > 0), "gave up");
    assert!(
        rig.until(60 * 10, |r| r.pos("Guard").z < -7.5),
        "back on its patrol line: {}",
        rig.pos("Guard")
    );
    assert_clean_log();
}

#[test]
fn a_wall_hides_the_player_and_the_enemy_can_be_killed() {
    let mut rig = rig("enemy-ai", |_| {});
    // Behind the wall (z = -14), 10 m from the guard: in range, out of sight.
    rig.teleport("Player", Vec3::new(0.0, 1.0, -17.0));
    rig.frames(60 * 4);
    assert_eq!(rig.sent("enemy_alert"), 0, "no line of sight");
    // Damage reaches the guard: 40 health, removed at 0.
    assert!(rig.exists("Guard"));
    rig.send("damage", 100.0, Some("Guard"));
    rig.frames(3);
    assert!(!rig.exists("Guard"));
    assert_eq!(rig.sent("died"), 1);
    assert_clean_log();
}

// --- Spawners ----------------------------------------------------------------------

impl Rig {
    /// Prefab instances scripts made that are alive now.
    fn spawned(&self) -> usize {
        let world = self.app.resources.fetch::<World>();
        world.query::<&ScriptSpawned>().iter().count()
    }
}

#[test]
fn the_interval_spawner_keeps_to_max_alive_and_refills() {
    let mut rig = rig("spawners", |_| {});
    let mut peak = 0;
    let mut seen_after_expiry = false;
    let mut first = None;
    for frame in 0..60 * 14 {
        rig.frames(1);
        let n = rig.spawned();
        peak = peak.max(n);
        assert!(n <= 4, "more than max_alive = 4: {n} at frame {frame}");
        if n == 4 && first.is_none() {
            first = Some(frame);
        }
        // Crates live 5 s, so by then the earliest are gone and replaced.
        if frame > 60 * 8 && n == 4 {
            seen_after_expiry = true;
        }
    }
    assert_eq!(peak, 4, "reaches the cap");
    assert!(first.unwrap() > 60 * 3, "one per second, not all at once");
    assert!(seen_after_expiry, "refills as crates expire");
    // They stand around the spawner (radius 3 at (0, 0, -6)).
    let world = rig.app.resources.fetch::<World>();
    for (_, t) in world.query::<(&ScriptSpawned, &GlobalTransform)>().iter() {
        let p = t.0.w_axis.truncate();
        assert!(Vec3::new(p.x, 0.0, p.z + 6.0).length() < 4.5, "{p}");
    }
    drop(world);
    assert_clean_log();
}

#[test]
fn the_message_spawner_spawns_per_message_up_to_the_cap() {
    let mut rig = rig("spawners", |_| {});
    rig.send("stop", ScriptValue::Nil, Some("Spawner (interval)"));
    rig.send("clear", ScriptValue::Nil, Some("Spawner (interval)"));
    rig.frames(5);
    let before = rig.spawned();
    assert_eq!(before, 0);
    for _ in 0..10 {
        rig.send("spawn", ScriptValue::Nil, Some("Spawner (message)"));
    }
    rig.frames(10);
    assert_eq!(rig.spawned(), 3, "max_alive = 3");
    // A free slot after the crates expire (5 s).
    rig.frames(60 * 6);
    assert_eq!(rig.spawned(), 0);
    rig.send("spawn", ScriptValue::Nil, Some("Spawner (message)"));
    rig.frames(5);
    assert_eq!(rig.spawned(), 1);
    assert_clean_log();
}

#[test]
fn the_wave_spawner_runs_its_waves_and_announces_the_end() {
    let mut rig = rig("spawners", |_| {});
    rig.send("stop", ScriptValue::Nil, Some("Spawner (interval)"));
    rig.send("clear", ScriptValue::Nil, Some("Spawner (interval)"));
    rig.frames(5);
    rig.send("start", ScriptValue::Nil, Some("Spawner (waves)"));
    rig.frames(10);
    assert_eq!(rig.spawned(), 3, "a wave of three at once");
    assert_eq!(rig.sent("wave_started"), 1);
    // Crates live 5 s; 2 s pause; the second wave.
    assert!(
        rig.until(60 * 12, |r| r.sent("wave_started") == 2),
        "second wave"
    );
    rig.frames(10);
    assert_eq!(rig.spawned(), 3);
    assert!(
        rig.until(60 * 12, |r| r.sent("waves_done") == 1),
        "done after the last crate is gone"
    );
    rig.frames(60 * 8);
    assert_eq!(rig.sent("wave_started"), 2, "no third wave");
    assert_eq!(rig.spawned(), 0);
    assert_clean_log();
}

// --- Camera effects ------------------------------------------------------------------

#[test]
fn camera_shake_moves_the_camera_and_decays_to_exactly_the_rest_position() {
    let mut rig = rig("camera-effects", |_| {});
    let rest = rig.pos("Camera");
    rig.frames(10);
    assert!((rig.pos("Camera") - rest).length() < 1e-4, "no shake yet");
    rig.send("shake", 1.0, Some("Camera"));
    let mut peak = 0.0f32;
    let mut late_peak = 0.0f32;
    for i in 0..60 {
        rig.frames(1);
        let off = (rig.pos("Camera") - rest).length();
        peak = peak.max(off);
        if i > 25 {
            late_peak = late_peak.max(off);
        }
    }
    assert!(
        peak > 0.02 && peak <= 0.22,
        "shakes by up to max_offset 0.15: {peak}"
    );
    assert!(late_peak < peak * 0.5, "dying down: {late_peak} vs {peak}");
    // 0.6 s at full trauma: well over by now.
    rig.frames(20);
    assert!(
        (rig.pos("Camera") - rest).length() < 1e-4,
        "back at rest: {}",
        rig.pos("Camera")
    );
    // The strength is clamped and a vector message sets the duration.
    rig.send("shake", Vec3::new(0.5, 2.0, 0.0), Some("Camera"));
    let mut slow = 0.0f32;
    for _ in 0..30 {
        rig.frames(1);
        slow = slow.max((rig.pos("Camera") - rest).length());
    }
    assert!(slow > 0.003, "a slower shake still shakes: {slow}");
    rig.frames(150);
    assert!((rig.pos("Camera") - rest).length() < 1e-4);
    assert_clean_log();
}

#[test]
fn the_trigger_zone_sends_its_message_to_the_camera() {
    let mut rig = rig("camera-effects", |_| {});
    let rest = rig.pos("Camera");
    assert_eq!(rig.sent("shake"), 0);
    rig.teleport("Player", Vec3::new(0.0, 1.0, -4.0));
    let mut moved = 0.0f32;
    for _ in 0..30 {
        rig.frames(1);
        moved = moved.max((rig.pos("Camera") - rest).length());
    }
    assert_eq!(rig.sent("shake"), 1);
    let shake = rig.log.iter().find(|m| m.0 == "shake").unwrap();
    assert_eq!(shake.1, ScriptValue::Number(0.8));
    assert_eq!(shake.2.as_deref(), Some("Shake Zone"));
    assert!(moved > 0.01, "the camera shook: {moved}");
    assert_clean_log();
}

#[test]
fn the_follower_trails_the_player_and_the_turret_faces_it() {
    let mut rig = rig("camera-effects", |_| {});
    rig.frames(240);
    let player = rig.world_pos("Player");
    let ball = rig.world_pos("Follower");
    assert!(
        (ball - (player + Vec3::new(2.0, 1.5, 2.0))).length() < 0.1,
        "settled beside the player: {ball} vs {player}"
    );
    // The player jumps far away: the ball lags, then arrives.
    rig.teleport("Player", Vec3::new(10.0, 1.0, -10.0));
    rig.frames(10);
    let lag =
        (rig.world_pos("Follower") - (rig.world_pos("Player") + Vec3::new(2.0, 1.5, 2.0))).length();
    assert!(lag > 5.0, "not instant: {lag}");
    rig.frames(300);
    let lag =
        (rig.world_pos("Follower") - (rig.world_pos("Player") + Vec3::new(2.0, 1.5, 2.0))).length();
    assert!(lag < 0.1, "arrived: {lag}");

    // The turret head looks at the player, wherever it is.
    for to in [Vec3::new(0.0, 1.0, 6.0), Vec3::new(10.0, 1.0, -10.0)] {
        rig.teleport("Player", to);
        rig.frames(120);
        let head = rig.child("Turret", "Turret Head");
        let at = rig.world_pos("Turret Head");
        let want = (rig.world_pos("Player") - at).normalize();
        let forward = head.rotation * Vec3::NEG_Z;
        assert!(forward.dot(want) > 0.99, "facing {forward}, want {want}");
    }
    assert_clean_log();
}

// --- The combined arena -----------------------------------------------------------------

#[test]
fn the_arena_starts_with_every_system_running() {
    let rig = rig("gameplay-demo", |_| {});
    assert_eq!(rig.num("health"), 100.0);
    assert_eq!(
        rig.var("checkpoint"),
        Some(ScriptValue::Str("Checkpoint Start".into()))
    );
    assert!(rig.exists("Guard") && rig.exists("Platform") && rig.exists("Door"));
    assert!(rig.grounded("Player"));
    assert_clean_log();
}

#[test]
fn the_arena_pickups_door_and_key_work_together() {
    let mut rig = rig("gameplay-demo", |_| {});
    // Coins add to the score; the health pack heals what the lava took.
    rig.teleport("Player", Vec3::new(-4.0, 1.0, 8.0));
    rig.frames(20);
    assert_eq!(rig.num("score"), 1.0);
    rig.send("damage", 50.0, Some("Player"));
    rig.frames(5);
    assert_eq!(rig.num("health"), 50.0);
    rig.teleport("Player", Vec3::new(-6.0, 1.0, 10.0));
    rig.frames(10);
    assert_eq!(rig.num("health"), 75.0);
    // The vault door stays shut without the key.
    let closed = rig.child("Door", "Hinge").translation;
    rig.teleport("Player", Vec3::new(0.0, 1.0, -34.0));
    rig.frames(90);
    assert_eq!(rig.child("Door", "Hinge").translation, closed);
    assert!(rig.sent("door_locked") >= 1);
    // Take the key, come back: it opens.
    rig.teleport("Player", Vec3::new(10.0, 1.0, -4.0));
    rig.frames(10);
    assert_eq!(rig.var("key_gold"), Some(ScriptValue::Bool(true)));
    rig.teleport("Player", Vec3::new(0.0, 1.0, 10.0));
    rig.frames(10);
    rig.teleport("Player", Vec3::new(0.0, 1.0, -34.0));
    rig.frames(90);
    assert!(
        rig.child("Door", "Hinge").translation.x - closed.x > 2.0,
        "open"
    );
    assert_clean_log();
}

#[test]
fn dying_sends_you_back_to_the_last_checkpoint() {
    let mut rig = rig("gameplay-demo", |_| {});
    rig.teleport("Player", Vec3::new(0.0, 1.0, -26.0));
    rig.frames(20);
    assert_eq!(
        rig.var("checkpoint"),
        Some(ScriptValue::Str("Checkpoint Far".into()))
    );
    rig.teleport("Player", Vec3::new(10.0, 1.0, -34.0));
    rig.frames(20);
    rig.send("damage", 500.0, Some("Player"));
    rig.frames(5);
    assert_eq!(rig.num("health"), 0.0);
    assert_eq!(rig.sent("died"), 1);
    // respawn_delay 2 s: full health and back at the far checkpoint.
    assert!(rig.until(60 * 4, |r| r.sent("respawned") > 0), "respawned");
    rig.frames(30);
    assert_eq!(rig.num("health"), 100.0);
    let p = rig.pos("Player");
    assert!(p.distance(Vec3::new(0.0, 0.92, -26.0)) < 0.4, "{p}");
    assert_clean_log();
}

#[test]
fn falling_into_the_pit_respawns_and_the_platform_gives_a_ride() {
    let mut rig = rig("gameplay-demo", |_| {});
    rig.teleport("Player", Vec3::new(8.0, 1.0, -14.0));
    assert!(
        rig.until(300, |r| r.sent("respawned") > 0),
        "the kill plane respawns"
    );
    rig.frames(60);
    assert!(rig.pos("Player").distance(Vec3::new(0.0, 0.92, 14.0)) < 0.4);
    // Step on the platform at its near end, ride it across.
    let body = rig.platform_body();
    rig.teleport("Player", Vec3::new(0.0, 1.0, body.z));
    rig.frames(20);
    assert!(
        rig.until(60 * 16, |r| r.platform_body().z < -19.9),
        "arrives"
    );
    let (p, b) = (rig.pos("Player"), rig.platform_body());
    assert!(
        p.y > 0.5 && (p.z - b.z).abs() < 1.8,
        "player {p} platform {b}"
    );
    assert_clean_log();
}

#[test]
fn the_guard_hurts_the_spawner_fills_and_damage_shakes_the_camera() {
    let mut rig = rig("gameplay-demo", |_| {});
    let rest = rig.pos("Camera");
    rig.teleport("Player", Vec3::new(0.0, 1.0, 2.0));
    assert!(
        rig.until(60 * 8, |r| r.num("health") < 100.0),
        "the guard hits"
    );
    assert!(rig.sent("enemy_alert") >= 1);
    let mut moved = 0.0f32;
    for _ in 0..30 {
        rig.frames(1);
        moved = moved.max((rig.pos("Camera") - rest).length());
    }
    assert!(moved > 0.005, "damage shook the camera: {moved}");
    // Far from the guard the spawner on the far slab fills up to its cap.
    rig.teleport("Player", Vec3::new(0.0, 1.0, 14.0));
    rig.frames(60 * 8);
    let crates = rig.spawned();
    assert!((1..=3).contains(&crates), "{crates} crates");
    assert_clean_log();
}

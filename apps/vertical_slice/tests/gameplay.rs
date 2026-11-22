//! The slice played headless: `HeadlessPlugins` + the real `GamePlugin` and
//! level, no window, no audio device, fixed 1/60 s frames. The level is found
//! through `ProjectPaths` (`cargo test` sets this crate's folder). Input is fed into the in-process
//! `Input` resource, the same way the window would.

use std::path::{Path, PathBuf};

use olvin::anim::Animator;
use olvin::game::{SaveSlots, State};
use olvin::input::Input;
use olvin::prelude::*;
use olvin::script::ScriptVars;
use vertical_slice::door::DOOR_OPEN;
use vertical_slice::hud::Hud;
use vertical_slice::player::{Player, PlayerLook};
use vertical_slice::robot::{Robot, current_state};
use vertical_slice::save::{Progress, load_slot, save_slot};
use vertical_slice::{GamePlugin, GameState, SliceConfig};

const DT: f32 = 1.0 / 60.0;

fn saves_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("vertical_slice-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The game, loaded and in `Playing`, bodies created.
fn game(tag: &str) -> App {
    let mut app = App::new();
    app.insert_resource(Time::with_fixed_delta(DT));
    app.add_plugin(HeadlessPlugins::default())
        .add_plugin(GamePlugin {
            config: SliceConfig {
                saves: Some(saves_dir(tag)),
                lua_hot_reload: false,
            },
        });
    for _ in 0..500 {
        app.update();
        if app
            .resources
            .fetch::<State<GameState>>()
            .is(GameState::Playing)
        {
            break;
        }
        // The scene is parsed on a worker thread.
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(
        app.resources
            .fetch::<State<GameState>>()
            .is(GameState::Playing),
        "level did not load"
    );
    frames(&mut app, 2);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

fn entity(app: &App, name: &str) -> Entity {
    let world = app.resources.fetch::<World>();
    world
        .query::<(Entity, &Name)>()
        .iter()
        .find(|(_, n)| n.0 == name)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("no entity `{name}`"))
}

fn pos(app: &App, e: Entity) -> Vec3 {
    app.resources
        .fetch::<World>()
        .get::<&Transform>(e)
        .unwrap()
        .translation
}

fn set_pos(app: &App, e: Entity, p: Vec3) {
    app.resources
        .fetch::<World>()
        .get::<&mut Transform>(e)
        .unwrap()
        .translation = p;
}

fn key(app: &App, key: KeyCode, down: bool) {
    app.resources.fetch_mut::<Input>().set_key(key, down);
}

/// Points the player at `target` (yaw only) and holds W for `n` frames.
fn walk_towards(app: &mut App, target: Vec3, n: usize) {
    let player = entity(app, "Player");
    let to = target - pos(app, player);
    {
        let world = app.resources.fetch::<World>();
        let mut look = world.get::<&mut PlayerLook>(player).unwrap();
        // forward = (-sin yaw, 0, -cos yaw)
        look.yaw = (-to.x).atan2(-to.z);
    }
    key(app, KeyCode::KeyW, true);
    frames(app, n);
    key(app, KeyCode::KeyW, false);
    frames(app, 1);
}

/// How far door.lua lifts the door (its `lift` parameter in the scene).
const DOOR_LIFT: f32 = 3.1;

/// Whether the door is open (the Lua scripts' shared variable) and its height.
fn door(app: &App) -> (bool, f32) {
    let e = entity(app, "Door");
    let open = app.resources.fetch::<ScriptVars>().flag(DOOR_OPEN);
    (open, pos(app, e).y)
}

fn set_door(app: &App, open: bool) {
    app.resources.fetch_mut::<ScriptVars>().set(DOOR_OPEN, open);
}

#[test]
fn level_loads_and_is_wired() {
    let app = game("wired");
    let world = app.resources.fetch::<World>();
    assert!(
        world.query::<&SceneId>().iter().count() >= 20,
        "scene ids on spawned objects"
    );
    assert_eq!(
        world.query::<(&Player, &CharacterMotor)>().iter().count(),
        1
    );
    assert_eq!(world.query::<(&Robot, &Animator)>().iter().count(), 1);
    assert_eq!(
        world.query::<&olvin::render::SkinnedMesh>().iter().count(),
        2,
        "body and visor primitives"
    );
    assert!(!app.resources.fetch::<Hud>().history.is_empty());
}

#[test]
fn the_crates_are_instances_of_one_prefab() {
    let app = game("prefab");
    let world = app.resources.fetch::<World>();
    let color = |name: &str| {
        let e = entity(&app, name);
        let prefab = world
            .get::<&olvin::scene::PrefabInstanceOf>(e)
            .unwrap_or_else(|_| panic!("{name} is not a prefab instance"));
        assert_eq!(prefab.0, "prefabs/crate.prefab.ron");
        // What the prefab brings along: body, collider, the Thud script.
        assert!(
            world.get::<&olvin::scene_file::RigidbodyDesc>(e).is_ok(),
            "{name}: rigidbody"
        );
        assert!(
            world.get::<&olvin::scene_file::ColliderDesc>(e).is_ok(),
            "{name}: collider"
        );
        assert!(
            world.get::<&olvin::scene_file::Scripts>(e).is_ok(),
            "{name}: script"
        );
        world
            .get::<&olvin::render::MeshRenderer>(e)
            .unwrap()
            .color
            .0
    };
    let plain = color("Crate 1");
    assert_eq!(color("Crate 2"), plain);
    assert_eq!(color("Crate 4"), plain);
    assert_ne!(color("Crate 3"), plain, "the override: a lighter wood");
    // The ball is no prefab.
    assert!(
        world
            .get::<&olvin::scene::PrefabInstanceOf>(entity(&app, "Ball"))
            .is_err()
    );
}

#[test]
fn walking_onto_the_plate_opens_the_door() {
    let mut app = game("plate");
    let (open, closed_y) = door(&app);
    assert!(!open);
    walk_towards(&mut app, Vec3::new(4.0, 1.0, 6.0), 90);
    let (open, _) = door(&app);
    assert!(open, "the plate trigger (plate.lua) did not fire");
    assert!(app.resources.fetch::<Progress>().door_opened);
    assert!(
        app.resources
            .fetch::<Hud>()
            .history
            .iter()
            .any(|m| m.contains("Дверь открыта"))
    );
    // The door slides up at 1.5 m/s.
    frames(&mut app, 180);
    let (_, y) = door(&app);
    assert!((y - (closed_y + DOOR_LIFT)).abs() < 0.01, "door at y = {y}");
}

#[test]
fn walking_into_a_crate_pushes_it() {
    let mut app = game("push");
    let crate_ = entity(&app, "Crate 4");
    let before = pos(&app, crate_);
    walk_towards(&mut app, before, 120);
    let after = pos(&app, crate_);
    let moved = Vec2::new(after.x - before.x, after.z - before.z).length();
    assert!(moved > 0.5, "crate moved only {moved} m");
}

#[test]
fn save_and_load_round_trip() {
    let mut app = game("save");
    frames(&mut app, 60); // let the crate stack settle
    let player = entity(&app, "Player");
    let crate_ = entity(&app, "Crate 3");
    set_door(&app, true);
    frames(&mut app, 30);
    let saved_player = pos(&app, player);
    let saved_crate = pos(&app, crate_);
    let (_, saved_door_y) = door(&app);
    save_slot(&app.resources, "test").unwrap();
    assert!(app.resources.fetch::<SaveSlots>().exists("test"));

    // Change everything: walk away, knock the crate over, close the door.
    set_pos(&app, player, Vec3::new(-6.0, 1.0, 9.0));
    {
        let world = app.resources.fetch::<World>();
        let mut physics = app.resources.fetch_mut::<Physics>();
        assert!(physics.set_velocity(&world, crate_, Vec3::new(6.0, 3.0, 0.0)));
    }
    set_door(&app, false);
    frames(&mut app, 90);
    assert!(pos(&app, crate_).distance(saved_crate) > 0.5);
    assert!(pos(&app, player).distance(saved_player) > 3.0);

    load_slot(&app.resources, "test").unwrap();
    frames(&mut app, 3);
    let crate_now = pos(&app, crate_);
    assert!(
        crate_now.distance(saved_crate) < 0.05,
        "crate {crate_now} vs saved {saved_crate}"
    );
    let player_now = pos(&app, player);
    assert!(
        player_now.distance(saved_player) < 0.1,
        "player {player_now} vs saved {saved_player}"
    );
    let (open, y) = door(&app);
    assert!(open, "the loaded progress reopens the door for the scripts");
    assert!((y - saved_door_y).abs() < 0.1, "door {y} vs {saved_door_y}");
    assert_eq!(app.resources.fetch::<Progress>().saves, 1);
}

#[test]
fn pause_freezes_the_world() {
    let mut app = game("pause");
    let beacon = entity(&app, "Beacon");
    let rotation = |app: &App| {
        app.resources
            .fetch::<World>()
            .get::<&Transform>(beacon)
            .unwrap()
            .rotation
    };
    let r0 = rotation(&app);
    frames(&mut app, 10);
    let r1 = rotation(&app);
    assert!(r0.angle_between(r1) > 0.01, "the Lua beacon should spin");

    key(&app, KeyCode::Escape, true);
    frames(&mut app, 1);
    key(&app, KeyCode::Escape, false);
    frames(&mut app, 1);
    assert!(
        app.resources
            .fetch::<State<GameState>>()
            .is(GameState::Paused)
    );
    assert!(
        app.resources.fetch::<Audio>().is_paused(),
        "sounds hold too"
    );
    let paused_at = rotation(&app);
    let ball = entity(&app, "Ball");
    let ball_at = pos(&app, ball);
    // Holding W while paused must not move the player either.
    let player = entity(&app, "Player");
    let player_at = pos(&app, player);
    key(&app, KeyCode::KeyW, true);
    frames(&mut app, 60);
    key(&app, KeyCode::KeyW, false);
    assert!(rotation(&app).angle_between(paused_at) < 1e-6);
    assert_eq!(pos(&app, ball), ball_at);
    assert_eq!(pos(&app, player), player_at);

    key(&app, KeyCode::Escape, true);
    frames(&mut app, 1);
    key(&app, KeyCode::Escape, false);
    frames(&mut app, 10);
    assert!(
        app.resources
            .fetch::<State<GameState>>()
            .is(GameState::Playing)
    );
    assert!(rotation(&app).angle_between(paused_at) > 0.01);
    assert!(!app.resources.fetch::<Audio>().is_paused());
}

#[test]
fn robot_waves_at_a_nearby_player() {
    let mut app = game("robot");
    let robot = {
        let world = app.resources.fetch::<World>();
        world.query::<(Entity, &Robot)>().iter().next().unwrap().0
    };
    let state = |app: &App| {
        let world = app.resources.fetch::<World>();
        let a = world.get::<&Animator>(robot).unwrap();
        (current_state(&a).unwrap().to_string(), a.version)
    };
    let (s, v0) = state(&app);
    assert_eq!(s, "Idle");
    frames(&mut app, 10);
    assert!(state(&app).1 > v0, "the animator should be advancing");

    // Teleport next to the robot (through the closed door: a test shortcut).
    let player = entity(&app, "Player");
    set_pos(&app, player, Vec3::new(0.5, 1.0, -3.5));
    frames(&mut app, 60);
    assert_eq!(state(&app).0, "Wave");

    set_pos(&app, player, Vec3::new(0.0, 1.0, 9.0));
    frames(&mut app, 60);
    assert_eq!(state(&app).0, "Idle");
}

/// The engine's character controller pushes dynamic bodies it walks into
/// and reports the hit (GAPS.md #8).
#[test]
fn engine_controller_pushes_and_reports_hits() {
    use olvin::physics::ControllerHit;
    use olvin::scene::file::{CharacterControllerDesc, ColliderDesc, ColliderShape, RigidbodyDesc};
    let mut app = App::new();
    app.insert_resource(Time::with_fixed_delta(DT));
    app.add_plugin(PhysicsPlugin::default());
    let (player, box_) = {
        let mut world = app.resources.fetch_mut::<World>();
        world.spawn((
            Transform::from_xyz(0.0, -0.5, 0.0).with_scale(Vec3::new(40.0, 1.0, 40.0)),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
        ));
        let player = world.spawn((
            Transform::from_xyz(0.0, 1.0, 0.0),
            CharacterControllerDesc::default(),
        ));
        let box_ = world.spawn((
            Transform::from_xyz(0.0, 0.5, -2.0),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
            RigidbodyDesc::default(),
        ));
        (player, box_)
    };
    frames(&mut app, 30);
    let before = pos(&app, box_);
    let mut hit = None;
    let mut cursor = olvin::core::EventCursor::default();
    for _ in 0..120 {
        if let Ok(mut m) = app
            .resources
            .fetch::<World>()
            .get::<&mut CharacterMotor>(player)
        {
            m.velocity.z = -4.0;
        }
        app.update();
        let events = app.resources.fetch::<olvin::core::Events<ControllerHit>>();
        hit = hit.or_else(|| events.read(&mut cursor).find(|h| h.other == box_).copied());
    }
    let moved = pos(&app, box_).distance(before);
    assert!(moved > 0.5, "the controller pushed the box only {moved} m");
    let hit = hit.expect("a controller hit on the box");
    assert_eq!(hit.character, player);
}

/// Bodies, scripts, sounds and particles of despawned objects must go away:
/// the level is torn down and respawned several times and the counts return
/// to what they were after the first load.
#[test]
fn respawning_the_level_does_not_leak_bodies_scripts_or_sounds() {
    use olvin::audio::Audio;
    use olvin::script::ScriptRuntime;

    fn counts(app: &App) -> (usize, usize, usize, usize, usize) {
        let physics = app.resources.fetch::<Physics>();
        (
            physics.world.bodies.len(),
            physics.world.colliders.len(),
            app.resources.fetch::<ScriptRuntime>().entity_count(),
            app.resources.fetch::<Audio>().active_count(),
            app.resources.fetch::<World>().len() as usize,
        )
    }
    fn despawn_all(app: &mut App) {
        let all: Vec<Entity> = app
            .resources
            .fetch::<World>()
            .iter()
            .map(|e| e.entity())
            .collect();
        {
            let mut world = app.resources.fetch_mut::<World>();
            for e in all {
                let _ = world.despawn(e);
            }
        }
        frames(app, 3);
    }

    let mut app = game("respawn");
    let first = counts(&app);
    assert!(
        first.0 > 0 && first.2 > 0,
        "the level has bodies and scripts: {first:?}"
    );

    let path = app
        .resources
        .fetch::<vertical_slice::Paths>()
        .assets
        .join(vertical_slice::SCENE);
    for round in 0..4 {
        despawn_all(&mut app);
        let empty = counts(&app);
        assert_eq!(
            (empty.0, empty.1, empty.2, empty.4),
            (0, 0, 0, 0),
            "round {round}: nothing left after despawning everything: {empty:?}"
        );
        olvin::scene_file::load(&app.resources, &path).unwrap();
        for _ in 0..200 {
            app.update();
            if olvin::models::pending_count(&app.resources) == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        frames(&mut app, 3);
        let again = counts(&app);
        // The scene alone (no `setup` extras like the robot marker or music):
        // bodies and colliders match the first load, scripts never pile up.
        assert_eq!(
            again.0, first.0,
            "round {round}: bodies {again:?} vs {first:?}"
        );
        assert_eq!(
            again.1, first.1,
            "round {round}: colliders {again:?} vs {first:?}"
        );
        assert!(
            again.2 <= first.2,
            "round {round}: scripts {again:?} vs {first:?}"
        );
        assert!(
            again.3 <= first.3 + 2,
            "round {round}: sounds {again:?} vs {first:?}"
        );
    }
}

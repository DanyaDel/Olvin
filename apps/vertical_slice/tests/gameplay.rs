//! The slice played headless: `HeadlessPlugins` + the real `GamePlugin` and
//! level, no window, no audio device, fixed 1/60 s frames. The level is found
//! through `ProjectPaths` (`cargo test` sets this crate's folder). Input is fed into the in-process
//! `Input` resource, the same way the window would.

use std::path::{Path, PathBuf};

use olvin::anim::Animator;
use olvin::game::{SaveSlots, State};
use olvin::input::Input;
use olvin::prelude::*;
use vertical_slice::door::DoorState;
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

fn door(app: &App) -> (DoorState, f32) {
    let e = entity(app, "Door");
    let world = app.resources.fetch::<World>();
    let d = *world.get::<&DoorState>(e).unwrap();
    let y = world.get::<&Transform>(e).unwrap().translation.y;
    (d, y)
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
fn walking_onto_the_plate_opens_the_door() {
    let mut app = game("plate");
    let (d, closed_y) = door(&app);
    assert!(!d.open);
    walk_towards(&mut app, Vec3::new(4.0, 1.0, 6.0), 90);
    let (d, _) = door(&app);
    assert!(d.open, "the plate trigger did not fire");
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
    let (d, y) = door(&app);
    assert!((y - (closed_y + d.lift)).abs() < 0.01, "door at y = {y}");
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
    let door_e = entity(&app, "Door");
    app.resources
        .fetch::<World>()
        .get::<&mut DoorState>(door_e)
        .unwrap()
        .open = true;
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
        world.get::<&mut DoorState>(door_e).unwrap().open = false;
    }
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
    let (d, y) = door(&app);
    assert!(d.open);
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

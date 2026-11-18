use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use glam::{Quat, Vec3};
use olvin_core::App;
use olvin_ecs::{Entity, World};
use olvin_scene::{Name, Transform};

use super::*;

#[derive(Default, Debug, Clone, PartialEq)]
struct Log {
    events: Vec<String>,
}

type Shared = Rc<RefCell<Log>>;

struct Probe {
    log: Shared,
    tag: String,
}

impl Probe {
    fn push(&self, what: impl Into<String>) {
        self.log
            .borrow_mut()
            .events
            .push(format!("{}:{}", self.tag, what.into()));
    }
}

impl Behaviour for Probe {
    fn start(&mut self, _: &mut ScriptCtx<'_>) {
        self.push("start");
    }
    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.push("update");
        if ctx.marked_to_die() {
            ctx.destroy_self();
        }
    }
    fn fixed_update(&mut self, _: &mut ScriptCtx<'_>) {
        self.push("fixed");
    }
    fn on_collision_enter(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.push(format!("collision_enter {}", ctx.name_of(other)));
    }
    fn on_collision_exit(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.push(format!("collision_exit {}", ctx.name_of(other)));
    }
    fn on_trigger_enter(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.push(format!("trigger_enter {}", ctx.name_of(other)));
    }
    fn on_trigger_exit(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.push(format!("trigger_exit {}", ctx.name_of(other)));
    }
    fn on_destroy(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.push(format!("destroy alive={}", ctx.alive()));
    }
}

/// Test-only: a script despawns itself when its entity has a `DieNow` marker.
#[derive(Clone, Copy)]
struct DieNow;

impl ScriptCtx<'_> {
    fn marked_to_die(&self) -> bool {
        self.component::<DieNow>().is_some()
    }
}

fn app_with_probe(log: &Shared) -> App {
    let mut app = App::new();
    app.add_plugin(ScriptPlugin::default());
    let l = log.clone();
    app.resources
        .fetch_mut::<ScriptRegistry>()
        .register_fn("Probe", vec![], move |p| {
            Box::new(Probe {
                log: l.clone(),
                tag: p.string("tag", "p"),
            })
        });
    app
}

fn count(log: &Shared, what: &str) -> usize {
    log.borrow()
        .events
        .iter()
        .filter(|e| e.as_str() == what)
        .count()
}

#[test]
fn lifecycle_start_once_updates_and_destroy() {
    let log = Shared::default();
    let mut app = app_with_probe(&log);
    let (a, b) = {
        let mut world = app.resources.fetch_mut::<World>();
        let a = world.spawn((
            Transform::IDENTITY,
            Scripts::new().with("Probe", [("tag", "a")]),
        ));
        let b = world.spawn((
            Transform::IDENTITY,
            Scripts::new().with("Probe", [("tag", "b")]),
        ));
        (a, b)
    };
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(count(&log, "a:start"), 1);
    assert_eq!(count(&log, "a:update"), 5);
    assert_eq!(
        log.borrow().events[0],
        "a:start",
        "start before the first update"
    );
    assert_eq!(log.borrow().events[2], "a:update");

    // Self-destroy through the context: on_destroy sees the entity alive.
    app.resources
        .fetch_mut::<World>()
        .insert_one(a, DieNow)
        .unwrap();
    app.update();
    assert!(!app.resources.fetch::<World>().contains(a));
    assert_eq!(count(&log, "a:destroy alive=true"), 1);
    // Despawned behind the scripts' back: on_destroy still runs, entity gone.
    app.resources.fetch_mut::<World>().despawn(b).unwrap();
    app.update();
    assert_eq!(count(&log, "b:destroy alive=false"), 1);
    assert_eq!(app.resources.fetch::<ScriptRuntime>().entity_count(), 0);
    let updates = count(&log, "a:update");
    app.update();
    assert_eq!(count(&log, "a:update"), updates, "no calls after destroy");
}

#[test]
fn despawn_takes_children_and_spawn_is_deferred() {
    struct Spawner {
        child: Option<Entity>,
    }
    impl Behaviour for Spawner {
        fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
            match self.child {
                None => {
                    let me = ctx.entity();
                    let child = ctx.spawn((
                        Transform::IDENTITY,
                        Name("Child".into()),
                        olvin_transform::Parent(me),
                    ));
                    assert!(!ctx.world().contains(child) || ctx.get::<Name>(child).is_none());
                    ctx.attach(
                        child,
                        builtin::Rotator {
                            speed_deg: Vec3::Y * 90.0,
                        },
                    );
                    self.child = Some(child);
                }
                Some(_) => ctx.destroy_self(),
            }
        }
    }
    let mut app = App::new();
    app.add_plugin(ScriptPlugin::default());
    let parent = app
        .resources
        .fetch_mut::<World>()
        .spawn((Transform::IDENTITY,));
    app.resources
        .fetch_mut::<ScriptRuntime>()
        .attach(parent, "Spawner", Spawner { child: None });
    app.update();
    let child = {
        let world = app.resources.fetch::<World>();
        world
            .query::<(Entity, &Name)>()
            .iter()
            .find(|(_, n)| n.0 == "Child")
            .map(|(e, _)| e)
            .expect("spawned after the pass")
    };
    assert!(
        app.resources
            .fetch::<ScriptRuntime>()
            .get::<builtin::Rotator>(child)
            .is_none()
    );
    app.update();
    // Second pass: parent destroys itself (and the child) at the end.
    let world = app.resources.fetch::<World>();
    assert!(!world.contains(parent) && !world.contains(child));
}

#[test]
fn params_are_typed() {
    let mut map = BTreeMap::new();
    for (k, v) in [
        ("speed", "2.5"),
        ("on", "Yes"),
        ("dir", "1, 2 ,3"),
        ("all", "4"),
        ("bad", "fast"),
        ("name", "Hero"),
        ("target", "Target"),
        ("ghost", "Nobody"),
    ] {
        map.insert(k.to_owned(), v.to_owned());
    }
    let mut world = World::new();
    let target = world.spawn((Name("Target".into()),));
    let p = Params::new(&map, Some(&world), "Test");
    assert_eq!(p.f32("speed", 0.0), 2.5);
    assert_eq!(p.f32("missing", 7.0), 7.0);
    assert_eq!(p.f32("bad", 1.0), 1.0, "malformed → default");
    assert!(p.bool("on", false));
    assert_eq!(p.vec3("dir", Vec3::ZERO), Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(p.vec3("all", Vec3::ZERO), Vec3::splat(4.0));
    assert_eq!(p.vec3("bad", Vec3::ONE), Vec3::ONE);
    assert_eq!(p.string("name", ""), "Hero");
    assert_eq!(p.entity("target"), Some(target));
    assert_eq!(p.entity("ghost"), None);
    assert_eq!(
        parse_vec3(&format_vec3(Vec3::new(0.5, -1.0, 2.0))),
        Some(Vec3::new(0.5, -1.0, 2.0))
    );

    // Built-ins read their parameters.
    let mut m = BTreeMap::new();
    m.insert("speed_deg".to_owned(), "0,180,0".to_owned());
    let r = builtin::Rotator::from_params(&Params::new(&m, None, "Rotator"));
    assert_eq!(r.speed_deg, Vec3::new(0.0, 180.0, 0.0));
}

#[test]
fn unknown_scripts_warn_once_and_are_skipped() {
    let mut app = App::new();
    app.add_plugin(ScriptPlugin::default());
    {
        let mut world = app.resources.fetch_mut::<World>();
        for _ in 0..3 {
            world.spawn((Scripts::new().with("NoSuchScript", []).with("Rotator", []),));
        }
    }
    app.update();
    app.update();
    let rt = app.resources.fetch::<ScriptRuntime>();
    assert_eq!(rt.warned_count(), 1, "one warning for three entities");
    assert_eq!(rt.entity_count(), 3, "known scripts still run");
}

#[test]
fn registry_macro_and_schema_round_trip() {
    #[derive(Default)]
    struct Door;
    impl Behaviour for Door {}
    impl FromParams for Door {
        fn from_params(_: &Params) -> Self {
            Door
        }
        fn schema() -> Vec<ParamSpec> {
            vec![ParamSpec::float("open_angle", 90.0, "degrees")]
        }
    }
    let mut r = ScriptRegistry::with_builtins();
    crate::register!(r, Door);
    crate::register!(r, "SlidingDoor" => Door);
    assert!(r.contains("Door") && r.contains("SlidingDoor") && r.contains("FollowTarget"));
    let schema = r.export_schema();
    assert_eq!(schema.scripts["Door"][0].name, "open_angle");
    assert_eq!(schema.scripts["Rotator"][0].kind, ParamKind::Vec3);
    let text = schema.to_ron();
    assert_eq!(ScriptSchema::from_ron(&text).unwrap(), schema);
}

#[test]
fn builtins_move_things() {
    let mut app = App::new();
    app.add_plugin(ScriptPlugin::default());
    let (rot, follower, target, doomed) = {
        let mut world = app.resources.fetch_mut::<World>();
        let target = world.spawn((Transform::from_xyz(10.0, 0.0, 0.0), Name("Target".into())));
        let rot = world.spawn((
            Transform::IDENTITY,
            Scripts::new().with("Rotator", [("speed_deg", "0,90,0")]),
        ));
        let follower = world.spawn((
            Transform::IDENTITY,
            Scripts::new().with(
                "FollowTarget",
                [
                    ("target", "Target"),
                    ("offset", "0,0,5"),
                    ("smoothing", "0"),
                ],
            ),
        ));
        let doomed = world.spawn((Scripts::new().with("DestroyAfter", [("seconds", "0")]),));
        (rot, follower, target, doomed)
    };
    // Wall-clock frame deltas are tiny; drive passes with a fixed dt instead.
    for _ in 0..60 {
        run_update(&app.resources, Some(1.0 / 60.0));
    }
    let world = app.resources.fetch::<World>();
    let r = world.get::<&Transform>(rot).unwrap().rotation;
    let expected = Quat::from_rotation_y(90f32.to_radians());
    assert!(
        r.angle_between(expected) < 0.01,
        "a quarter turn in one second"
    );
    let f = world.get::<&Transform>(follower).unwrap();
    assert!(f.translation.abs_diff_eq(Vec3::new(10.0, 0.0, 5.0), 1e-4));
    assert!(
        f.forward().abs_diff_eq(Vec3::NEG_Z, 1e-4),
        "looks at the target"
    );
    assert!(world.contains(target) && !world.contains(doomed));
}

#[test]
fn panicking_script_is_disabled_not_fatal() {
    struct Bomb(u32);
    impl Behaviour for Bomb {
        fn update(&mut self, _: &mut ScriptCtx<'_>) {
            self.0 += 1;
            panic!("boom");
        }
    }
    let mut app = App::new();
    app.add_plugin(ScriptPlugin::default());
    let e = app
        .resources
        .fetch_mut::<World>()
        .spawn((Transform::IDENTITY,));
    app.resources
        .fetch_mut::<ScriptRuntime>()
        .attach(e, "Bomb", Bomb(0));
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    app.update();
    app.update();
    std::panic::set_hook(hook);
    assert_eq!(
        app.resources
            .fetch::<ScriptRuntime>()
            .get::<Bomb>(e)
            .unwrap()
            .0,
        1
    );
}

#[cfg(feature = "physics")]
mod physics {
    use olvin_physics::{FIXED_DT, PhysicsPlugin};
    use olvin_scene::file::{ColliderDesc, ColliderShape, RigidbodyDesc};

    use super::*;

    /// One deterministic frame: physics (with fixed updates) then the script pass.
    fn frame(app: &App) {
        olvin_physics::step(&app.resources, FIXED_DT);
        run_update(&app.resources, Some(FIXED_DT));
    }

    #[test]
    fn collision_and_trigger_callbacks_and_fixed_update() {
        let log = Shared::default();
        let mut app = app_with_probe(&log);
        app.add_plugin(PhysicsPlugin::default());
        {
            let mut world = app.resources.fetch_mut::<World>();
            world.spawn((
                Name("Floor".into()),
                Transform::from_xyz(0.0, -0.5, 0.0).with_scale(Vec3::new(20.0, 1.0, 20.0)),
                ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
            ));
            let mut trigger = ColliderDesc::new(ColliderShape::Box {
                size: Vec3::new(4.0, 1.0, 4.0),
            });
            trigger.is_trigger = true;
            world.spawn((
                Name("Zone".into()),
                Transform::from_xyz(0.0, 2.5, 0.0),
                trigger,
            ));
            world.spawn((
                Name("Ball".into()),
                Transform::from_xyz(0.0, 4.0, 0.0),
                ColliderDesc::new(ColliderShape::Sphere { radius: 0.5 }),
                RigidbodyDesc::default(),
                Scripts::new().with("Probe", [("tag", "ball")]),
            ));
        }
        for _ in 0..120 {
            frame(&app);
        }
        assert_eq!(count(&log, "ball:trigger_enter Zone"), 1);
        assert_eq!(count(&log, "ball:trigger_exit Zone"), 1);
        assert_eq!(count(&log, "ball:collision_enter Floor"), 1);
        assert_eq!(count(&log, "ball:collision_exit Floor"), 0);
        assert_eq!(
            count(&log, "ball:fixed"),
            120,
            "one fixed update per physics step"
        );
        assert_eq!(count(&log, "ball:start"), 1);
        let events = &log.borrow().events;
        let enter = events
            .iter()
            .position(|e| e.contains("trigger_enter"))
            .unwrap();
        let exit = events
            .iter()
            .position(|e| e.contains("trigger_exit"))
            .unwrap();
        assert!(enter < exit);
    }

    #[cfg(feature = "input")]
    #[test]
    fn player_controller_walks_and_jumps_on_input() {
        use olvin_input::{ActionState, GamepadState, InputMap, KeyCode};
        use olvin_physics::CharacterMotor;
        use olvin_scene::file::CharacterControllerDesc;

        let mut app = App::new();
        app.add_plugin(PhysicsPlugin::default())
            .add_plugin(ScriptPlugin::default())
            .insert_resource(ActionState::default());
        let player = {
            let mut world = app.resources.fetch_mut::<World>();
            world.spawn((
                Transform::from_xyz(0.0, -0.5, 0.0).with_scale(Vec3::new(40.0, 1.0, 40.0)),
                ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
            ));
            world.spawn((
                Transform::from_xyz(0.0, 1.0, 0.0),
                CharacterControllerDesc::default(),
                Scripts::new().with("SimplePlayerController", [("speed", "4")]),
            ))
        };
        let map = InputMap::default();
        let mut input = olvin_input::Input::default();
        let pads = GamepadState::default();
        input.set_key(KeyCode::KeyW, true);
        let mut max_y: f32 = 0.0;
        for i in 0..90 {
            if i == 30 {
                input.set_key(KeyCode::Space, true);
            }
            if i == 31 {
                input.set_key(KeyCode::Space, false);
            }
            app.resources
                .fetch_mut::<ActionState>()
                .update(&map, &input, &pads);
            input.end_frame();
            frame(&app);
            max_y = max_y.max(
                app.resources
                    .fetch::<World>()
                    .get::<&Transform>(player)
                    .unwrap()
                    .translation
                    .y,
            );
        }
        let world = app.resources.fetch::<World>();
        let at = world.get::<&Transform>(player).unwrap().translation;
        assert!(at.z < -5.0, "walked forward (-Z) about 6 m: {at}");
        assert!(max_y > 1.8, "jumped ~1.2 m: {max_y}");
        assert!(world.get::<&CharacterMotor>(player).unwrap().grounded);
    }
}

#[cfg(feature = "input")]
#[test]
fn free_fly_camera_moves_with_actions() {
    use olvin_input::{ActionState, GamepadState, Input, InputMap, KeyCode};

    let mut app = App::new();
    app.add_plugin(ScriptPlugin::default())
        .insert_resource(ActionState::default());
    let cam = app.resources.fetch_mut::<World>().spawn((
        Transform::IDENTITY,
        Scripts::new().with("FreeFlyCamera", [("speed", "6"), ("lock_cursor", "true")]),
    ));
    let mut input = Input::default();
    input.set_key(KeyCode::KeyW, true);
    input.mouse_delta = glam::Vec2::new(-750.0, 0.0); // 90° left at 0.12°/px
    for _ in 0..30 {
        app.resources.fetch_mut::<ActionState>().update(
            &InputMap::default(),
            &input,
            &GamepadState::default(),
        );
        input.mouse_delta = glam::Vec2::ZERO;
        run_update(&app.resources, Some(1.0 / 30.0));
    }
    let t = *app
        .resources
        .fetch::<World>()
        .get::<&Transform>(cam)
        .unwrap();
    assert!(
        t.forward().abs_diff_eq(Vec3::NEG_X, 1e-3),
        "turned left: {}",
        t.forward()
    );
    assert!(t.translation.x < -5.0, "flew ~6 m: {}", t.translation);
    assert!(app.resources.fetch::<ActionState>().cursor_locked);
}

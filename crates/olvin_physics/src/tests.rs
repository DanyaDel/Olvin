use glam::{Quat, Vec3};
use olvin_core::{Events, Resources};
use olvin_ecs::{Entity, World};
use olvin_scene::file::{CharacterControllerDesc, ColliderDesc, ColliderShape, RigidbodyDesc};
use olvin_scene::{MeshGeometry, Transform};

use super::*;

fn res_with(physics: Physics) -> Resources {
    let mut res = Resources::default();
    res.insert(World::new());
    res.insert(physics);
    res
}

#[test]
fn box_falls_and_rests_on_static_ground() {
    let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    let falling = {
        let mut world = res.fetch_mut::<World>();
        world.spawn((
            Transform::from_xyz(0.0, -0.5, 0.0).with_scale(Vec3::new(20.0, 1.0, 20.0)),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
        ));
        world.spawn((
            Transform::from_xyz(0.0, 5.0, 0.0),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
            RigidbodyDesc::default(),
        ))
    };
    for _ in 0..240 {
        step(&res, FIXED_DT);
    }
    let y = res
        .fetch::<World>()
        .get::<&Transform>(falling)
        .unwrap()
        .translation
        .y;
    assert!(
        (y - 0.5).abs() < 0.05,
        "box should rest on the ground at y = 0.5, got {y}"
    );
}

#[test]
fn mesh_collider_catches_a_ball_and_triggers_dont() {
    let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    let (ball, ghost) = {
        let mut world = res.fetch_mut::<World>();
        // A 10x10 floor quad as a mesh collider.
        let floor = MeshGeometry {
            positions: vec![
                Vec3::new(-5.0, 0.0, -5.0),
                Vec3::new(5.0, 0.0, -5.0),
                Vec3::new(5.0, 0.0, 5.0),
                Vec3::new(-5.0, 0.0, 5.0),
            ],
            indices: vec![[0, 2, 1], [0, 3, 2]],
        };
        world.spawn((
            Transform::IDENTITY,
            ColliderDesc::new(ColliderShape::Mesh),
            floor,
        ));
        let ball = world.spawn((
            Transform::from_xyz(0.0, 3.0, 0.0),
            ColliderDesc::new(ColliderShape::Sphere { radius: 0.5 }),
            RigidbodyDesc::default(),
        ));
        let mut trigger = ColliderDesc::new(ColliderShape::Box {
            size: Vec3::splat(4.0),
        });
        trigger.is_trigger = true;
        world.spawn((Transform::from_xyz(0.0, 1.0, 3.0), trigger));
        let ghost = world.spawn((
            Transform::from_xyz(0.0, 3.0, 3.0),
            ColliderDesc::new(ColliderShape::Sphere { radius: 0.25 }),
            RigidbodyDesc {
                gravity_scale: 1.0,
                ..Default::default()
            },
        ));
        (ball, ghost)
    };
    for _ in 0..240 {
        step(&res, FIXED_DT);
    }
    let world = res.fetch::<World>();
    let ball_y = world.get::<&Transform>(ball).unwrap().translation.y;
    assert!(
        (ball_y - 0.5).abs() < 0.05,
        "ball rests on the mesh floor, got {ball_y}"
    );
    // The trigger box doesn't stop it; the floor does.
    let ghost_y = world.get::<&Transform>(ghost).unwrap().translation.y;
    assert!(
        (ghost_y - 0.25).abs() < 0.05,
        "trigger must not collide, got {ghost_y}"
    );
}

#[test]
fn dynamic_mesh_collider_uses_convex_hull() {
    let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    let cube = {
        let mut world = res.fetch_mut::<World>();
        world.spawn((
            Transform::from_xyz(0.0, -0.5, 0.0).with_scale(Vec3::new(20.0, 1.0, 20.0)),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
        ));
        // A unit cube given as triangles, scaled 2x: rests with its centre at y = 1.
        let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * 0.5;
        let positions = vec![
            c(-1., -1., -1.),
            c(1., -1., -1.),
            c(1., 1., -1.),
            c(-1., 1., -1.),
            c(-1., -1., 1.),
            c(1., -1., 1.),
            c(1., 1., 1.),
            c(-1., 1., 1.),
        ];
        let indices = vec![
            [0, 1, 2],
            [0, 2, 3],
            [4, 6, 5],
            [4, 7, 6],
            [0, 4, 5],
            [0, 5, 1],
            [3, 2, 6],
            [3, 6, 7],
            [0, 3, 7],
            [0, 7, 4],
            [1, 5, 6],
            [1, 6, 2],
        ];
        world.spawn((
            Transform::from_xyz(0.0, 4.0, 0.0).with_scale(Vec3::splat(2.0)),
            ColliderDesc::new(ColliderShape::Mesh),
            MeshGeometry { positions, indices },
            RigidbodyDesc::default(),
        ))
    };
    for _ in 0..240 {
        step(&res, FIXED_DT);
    }
    let y = res
        .fetch::<World>()
        .get::<&Transform>(cube)
        .unwrap()
        .translation
        .y;
    assert!(
        (y - 1.0).abs() < 0.05,
        "dynamic mesh body should fall and rest at y = 1, got {y}"
    );
}

fn ground(world: &mut World) -> Entity {
    world.spawn((
        Transform::from_xyz(0.0, -0.5, 0.0).with_scale(Vec3::new(40.0, 1.0, 40.0)),
        ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
    ))
}

fn with_events(res: &mut Resources) {
    res.insert(CollisionEvents::default());
}

#[test]
fn collision_and_trigger_events_name_entities() {
    let mut res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    with_events(&mut res);
    let (floor, falling, trigger) = {
        let mut world = res.fetch_mut::<World>();
        let floor = ground(&mut world);
        let falling = world.spawn((
            Transform::from_xyz(0.0, 4.0, 0.0),
            ColliderDesc::new(ColliderShape::Sphere { radius: 0.5 }),
            RigidbodyDesc::default(),
        ));
        let mut t = ColliderDesc::new(ColliderShape::Box {
            size: Vec3::new(4.0, 1.0, 4.0),
        });
        t.is_trigger = true;
        let trigger = world.spawn((Transform::from_xyz(0.0, 2.5, 0.0), t));
        (floor, falling, trigger)
    };
    let mut seen = Vec::new();
    for _ in 0..180 {
        step(&res, FIXED_DT);
        seen.extend(res.fetch::<CollisionEvents>().events.iter().copied());
    }
    let has = |a: Entity, b: Entity, started: bool, sensor: bool| {
        seen.iter().any(|e| {
            ((e.a == a && e.b == b) || (e.a == b && e.b == a))
                && e.started == started
                && e.sensor == sensor
        })
    };
    assert!(
        has(falling, trigger, true, true),
        "entered trigger: {seen:?}"
    );
    assert!(has(falling, trigger, false, true), "left trigger: {seen:?}");
    assert!(has(falling, floor, true, false), "hit the floor: {seen:?}");
    assert!(!has(falling, floor, false, false), "still resting on it");
}

#[test]
fn raycast_and_spherecast_hit_the_ground() {
    let res = res_with(Physics::new(Vec3::ZERO));
    let floor = ground(&mut res.fetch_mut::<World>());
    step(&res, FIXED_DT);
    let physics = res.fetch::<Physics>();
    let hit = physics
        .raycast(Vec3::new(1.0, 5.0, 2.0), Vec3::NEG_Y, 100.0)
        .expect("ray hits the floor");
    assert_eq!(hit.entity, floor);
    assert!((hit.distance - 5.0).abs() < 1e-3, "{hit:?}");
    assert!(hit.point.abs_diff_eq(Vec3::new(1.0, 0.0, 2.0), 1e-3));
    assert!(hit.normal.abs_diff_eq(Vec3::Y, 1e-3));
    assert!(
        physics
            .raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::Y, 100.0)
            .is_none()
    );
    assert!(
        physics
            .raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y, 2.0)
            .is_none()
    );

    let hit = physics
        .spherecast(Vec3::new(0.0, 5.0, 0.0), 0.5, Vec3::NEG_Y, 100.0)
        .unwrap();
    assert!((hit.distance - 4.5).abs() < 1e-3, "{hit:?}");
    assert_eq!(
        physics.overlap_sphere(Vec3::new(0.0, 0.2, 0.0), 0.5),
        vec![floor]
    );
}

#[test]
fn despawned_entities_lose_their_bodies() {
    let res = res_with(Physics::new(Vec3::ZERO));
    let floor = ground(&mut res.fetch_mut::<World>());
    step(&res, FIXED_DT);
    assert_eq!(res.fetch::<Physics>().world.bodies.len(), 1);
    res.fetch_mut::<World>().despawn(floor).unwrap();
    step(&res, FIXED_DT);
    assert_eq!(res.fetch::<Physics>().world.bodies.len(), 0);
}

fn character(world: &mut World, at: Vec3) -> Entity {
    world.spawn((
        Transform::from_translation(at),
        CharacterControllerDesc::default(),
    ))
}

fn run(res: &Resources, e: Entity, velocity: Vec3, steps: usize) -> (Vec3, CharacterMotor) {
    for _ in 0..steps {
        if let Ok(mut m) = res.fetch_mut::<World>().get::<&mut CharacterMotor>(e) {
            m.velocity.x = velocity.x;
            m.velocity.z = velocity.z;
        }
        step(res, FIXED_DT);
    }
    let world = res.fetch::<World>();
    let t = world.get::<&Transform>(e).unwrap().translation;
    (t, *world.get::<&CharacterMotor>(e).unwrap())
}

#[test]
fn character_falls_and_stands_grounded_on_the_floor() {
    let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    let c = {
        let mut world = res.fetch_mut::<World>();
        ground(&mut world);
        character(&mut world, Vec3::new(0.0, 3.0, 0.0))
    };
    let (at, motor) = run(&res, c, Vec3::ZERO, 120);
    assert!(motor.grounded, "{motor:?}");
    assert!(
        (at.y - 1.0).abs() < 0.05,
        "capsule centre 1 m above the floor: {at}"
    );
    assert!(
        motor.velocity.y.abs() < 1e-3,
        "landing zeroes the fall: {motor:?}"
    );
}

#[test]
fn character_stops_at_a_wall() {
    let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    let c = {
        let mut world = res.fetch_mut::<World>();
        ground(&mut world);
        world.spawn((
            Transform::from_xyz(3.0, 2.0, 0.0).with_scale(Vec3::new(1.0, 4.0, 10.0)),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
        ));
        character(&mut world, Vec3::new(0.0, 1.0, 0.0))
    };
    let (at, motor) = run(&res, c, Vec3::new(4.0, 0.0, 0.0), 120);
    assert!(
        at.x < 2.0 && at.x > 1.8,
        "stops at the wall face (x = 2): {at}"
    );
    assert!(motor.grounded);
}

#[test]
fn character_climbs_a_step_but_not_a_ledge() {
    let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    let (c, ledge_runner) = {
        let mut world = res.fetch_mut::<World>();
        ground(&mut world);
        // A 0.25 m step from x = 2 to 6, and a 0.8 m ledge far away on z = 10.
        world.spawn((
            Transform::from_xyz(4.0, 0.125, 0.0).with_scale(Vec3::new(4.0, 0.25, 4.0)),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
        ));
        world.spawn((
            Transform::from_xyz(4.0, 0.4, 10.0).with_scale(Vec3::new(4.0, 0.8, 4.0)),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
        ));
        (
            character(&mut world, Vec3::new(0.0, 1.0, 0.0)),
            character(&mut world, Vec3::new(0.0, 1.0, 10.0)),
        )
    };
    for _ in 0..90 {
        for e in [c, ledge_runner] {
            res.fetch_mut::<World>()
                .get::<&mut CharacterMotor>(e)
                .map(|mut m| m.velocity.x = 3.0)
                .ok();
        }
        step(&res, FIXED_DT);
    }
    let world = res.fetch::<World>();
    let at = world.get::<&Transform>(c).unwrap().translation;
    assert!(at.x > 3.0, "walked onto the step: {at}");
    assert!((at.y - 1.25).abs() < 0.06, "standing on top of it: {at}");
    let blocked = world.get::<&Transform>(ledge_runner).unwrap().translation;
    assert!(
        blocked.x < 1.6,
        "a ledge above the step offset blocks: {blocked}"
    );
}

#[test]
fn move_character_and_teleport() {
    let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    let c = {
        let mut world = res.fetch_mut::<World>();
        ground(&mut world);
        character(&mut world, Vec3::new(0.0, 1.05, 0.0))
    };
    run(&res, c, Vec3::ZERO, 30);
    {
        let mut physics = res.fetch_mut::<Physics>();
        let mut world = res.fetch_mut::<World>();
        let mv = move_character(&mut physics, &mut world, c, Vec3::new(0.5, -1.0, 0.0)).unwrap();
        assert!(mv.grounded);
        assert!((mv.translation.x - 0.5).abs() < 1e-3, "{mv:?}");
        assert!(mv.translation.y > -0.1, "the floor stops the downward part");
        // Teleport by writing the Transform.
        world.get::<&mut Transform>(c).unwrap().translation = Vec3::new(10.0, 1.0, 10.0);
    }
    let (at, _) = run(&res, c, Vec3::ZERO, 10);
    assert!(at.abs_diff_eq(Vec3::new(10.0, 1.0, 10.0), 0.05), "{at}");
}

#[test]
fn character_walks_far_without_stalling() {
    let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    let c = {
        let mut world = res.fetch_mut::<World>();
        ground(&mut world);
        character(&mut world, Vec3::new(0.0, 1.0, 0.0))
    };
    let mut last = 0.0;
    for i in 0..10 {
        let (at, _) = run(&res, c, Vec3::new(0.0, 0.0, -3.0), 60);
        assert!(last - at.z > 2.5, "second {i}: {at} (was z = {last})");
        last = at.z;
    }
}

#[test]
fn filtered_queries_skip_the_caster_and_see_triggers_on_request() {
    let res = res_with(Physics::new(Vec3::ZERO));
    let (floor, caster, zone) = {
        let mut world = res.fetch_mut::<World>();
        let floor = ground(&mut world);
        let caster = world.spawn((
            Transform::from_xyz(0.0, 3.0, 0.0),
            ColliderDesc::new(ColliderShape::Sphere { radius: 0.5 }),
        ));
        let mut t = ColliderDesc::new(ColliderShape::Box {
            size: Vec3::new(2.0, 0.5, 2.0),
        });
        t.is_trigger = true;
        let zone = world.spawn((Transform::from_xyz(0.0, 1.0, 0.0), t));
        (floor, caster, zone)
    };
    step(&res, FIXED_DT);
    let physics = res.fetch::<Physics>();
    let from = Vec3::new(0.0, 3.0, 0.0);

    // Луч из центра сферы сначала попадает в неё саму.
    let hit = physics.raycast(from, Vec3::NEG_Y, 10.0).unwrap();
    assert_eq!(hit.entity, caster);
    // Без себя и без триггеров — пол.
    let hit = physics
        .raycast_filtered(from, Vec3::NEG_Y, 10.0, Some(caster), false)
        .unwrap();
    assert_eq!(hit.entity, floor);
    assert!((hit.distance - 3.0).abs() < 1e-3, "{hit:?}");
    // С триггерами — зона над полом (верх на y = 1.25).
    let hit = physics
        .raycast_filtered(from, Vec3::NEG_Y, 10.0, Some(caster), true)
        .unwrap();
    assert_eq!(hit.entity, zone);
    assert!((hit.distance - 1.75).abs() < 1e-3, "{hit:?}");
    // Нулевое направление — не луч.
    assert!(physics.raycast(from, Vec3::ZERO, 10.0).is_none());

    let hit = physics
        .spherecast_filtered(from, 0.25, Vec3::NEG_Y, 10.0, Some(caster))
        .unwrap();
    assert_eq!(hit.entity, floor, "spherecast не видит триггеры");
    assert!((hit.distance - 2.75).abs() < 1e-3, "{hit:?}");

    let mut around = physics.overlap_sphere(Vec3::new(0.0, 1.0, 0.0), 0.3);
    around.sort();
    assert_eq!(around, vec![zone]);
}

#[test]
fn fixed_steps_accumulate_and_are_capped() {
    let mut physics = Physics::new(Vec3::new(0.0, -9.81, 0.0));
    assert_eq!(physics.gravity(), Vec3::new(0.0, -9.81, 0.0));
    // Пол-шага ещё не шаг, вторая половина добивает до одного.
    assert_eq!(physics.advance(FIXED_DT * 0.5), 0);
    assert_eq!(physics.advance(FIXED_DT * 0.5 + 1e-6), 1);
    assert_eq!(physics.advance(FIXED_DT * 3.0 + 1e-5), 3);
    // Долгий кадр (подвисание) не превращается в лавину шагов.
    assert_eq!(physics.advance(10.0), MAX_STEPS_PER_FRAME);
    assert_eq!(physics.advance(0.0), 0);
}

#[test]
fn static_and_dynamic_bodies_follow_a_moved_transform() {
    let res = res_with(Physics::new(Vec3::ZERO));
    let (wall, ball) = {
        let mut world = res.fetch_mut::<World>();
        let wall = world.spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
        ));
        let ball = world.spawn((
            Transform::from_xyz(0.0, 0.0, 10.0),
            ColliderDesc::new(ColliderShape::Sphere { radius: 0.5 }),
            RigidbodyDesc::default(),
        ));
        (wall, ball)
    };
    step(&res, FIXED_DT);
    {
        let world = res.fetch::<World>();
        let mut physics = res.fetch_mut::<Physics>();
        assert!(physics.set_velocity(&world, ball, Vec3::X));
        world.get::<&mut Transform>(wall).unwrap().translation = Vec3::new(5.0, 0.0, 0.0);
        world.get::<&mut Transform>(ball).unwrap().translation = Vec3::new(0.0, 0.0, -10.0);
    }
    step(&res, FIXED_DT);
    step(&res, FIXED_DT);
    let world = res.fetch::<World>();
    let physics = res.fetch::<Physics>();
    let hit = physics
        .raycast(Vec3::new(5.0, 5.0, 0.0), Vec3::NEG_Y, 10.0)
        .expect("the static box moved with its Transform");
    assert_eq!(hit.entity, wall);
    assert!(
        physics
            .raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y, 10.0)
            .is_none(),
        "nothing left at the old place"
    );
    let at = world.get::<&Transform>(ball).unwrap().translation;
    assert!(
        at.abs_diff_eq(Vec3::new(2.0 * FIXED_DT, 0.0, -10.0), 1e-3),
        "teleported, velocity kept: {at}"
    );
}

#[test]
fn teleport_stops_the_body() {
    let res = res_with(Physics::new(Vec3::ZERO));
    let ball = res.fetch_mut::<World>().spawn((
        Transform::IDENTITY,
        ColliderDesc::new(ColliderShape::Sphere { radius: 0.5 }),
        RigidbodyDesc::default(),
    ));
    step(&res, FIXED_DT);
    {
        let world = res.fetch::<World>();
        let mut physics = res.fetch_mut::<Physics>();
        physics.set_velocity(&world, ball, Vec3::new(3.0, 0.0, 0.0));
        physics.set_angular_velocity(&world, ball, Vec3::Y);
        world.get::<&mut Transform>(ball).unwrap().translation = Vec3::new(0.0, 7.0, 0.0);
        assert!(physics.teleport(&world, ball));
    }
    step(&res, FIXED_DT);
    let world = res.fetch::<World>();
    let physics = res.fetch::<Physics>();
    assert_eq!(physics.velocity(&world, ball), Some(Vec3::ZERO));
    assert_eq!(physics.angular_velocity(&world, ball), Some(Vec3::ZERO));
    let at = world.get::<&Transform>(ball).unwrap().translation;
    assert!(at.abs_diff_eq(Vec3::new(0.0, 7.0, 0.0), 1e-5), "{at}");
}

/// A character walking at a 2 kg box along -Z for two seconds; returns how
/// far the box moved and the hit events.
fn walk_into_box(push_mass: f32) -> (f32, Entity, Vec<ControllerHit>) {
    let mut res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    res.insert(Events::<ControllerHit>::default());
    let (c, box_) = {
        let mut world = res.fetch_mut::<World>();
        ground(&mut world);
        let c = world.spawn((
            Transform::from_xyz(0.0, 1.0, 0.0),
            CharacterControllerDesc {
                push_mass,
                ..Default::default()
            },
        ));
        let box_ = world.spawn((
            Transform::from_xyz(0.0, 0.5, -2.0),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
            RigidbodyDesc {
                mass: 2.0,
                ..Default::default()
            },
        ));
        (c, box_)
    };
    run(&res, c, Vec3::ZERO, 30);
    let before = res
        .fetch::<World>()
        .get::<&Transform>(box_)
        .unwrap()
        .translation;
    run(&res, c, Vec3::new(0.0, 0.0, -3.0), 120);
    let after = res
        .fetch::<World>()
        .get::<&Transform>(box_)
        .unwrap()
        .translation;
    let mut cursor = olvin_core::EventCursor::default();
    let hits = res
        .fetch::<Events<ControllerHit>>()
        .read(&mut cursor)
        .copied()
        .filter(|h| h.character == c)
        .collect();
    (before.z - after.z, box_, hits)
}

#[test]
fn character_pushes_dynamic_bodies_and_reports_hits() {
    let (moved, box_, hits) = walk_into_box(80.0);
    assert!(moved > 2.0, "pushed along: {moved} m");
    assert!(moved < 7.0, "not launched: {moved} m");
    let hit = hits
        .iter()
        .find(|h| h.other == box_)
        .expect("a hit on the box");
    assert!(hit.normal.abs_diff_eq(Vec3::Z, 0.1), "{hit:?}");
    assert!(hit.direction.z < -0.9, "{hit:?}");
    assert!(hit.point.z < -0.3, "on the box's near face: {hit:?}");
    assert!(
        hits.iter().any(|h| h.other != box_),
        "the floor is hit too (gravity)"
    );

    let (still, _, hits) = walk_into_box(0.0);
    assert!(still.abs() < 0.1, "push_mass 0 only blocks: {still} m");
    assert!(hits.iter().any(|h| h.other == box_));
}

#[test]
fn collision_events_of_puts_the_other_entity_first() {
    let mut world = World::new();
    let [a, b, c] = [(); 3].map(|_| world.spawn((Transform::IDENTITY,)));
    let events = CollisionEvents {
        events: vec![
            CollisionEvent {
                a,
                b,
                started: true,
                sensor: false,
            },
            CollisionEvent {
                a: c,
                b: a,
                started: false,
                sensor: true,
            },
        ],
    };
    let of_a: Vec<Entity> = events.of(a).map(|(other, _)| other).collect();
    assert_eq!(of_a, vec![b, c]);
    assert_eq!(events.of(b).count(), 1);
    let mut events = events;
    events.clear();
    assert_eq!(events.iter().count(), 0);
}

#[test]
fn zero_scale_keeps_the_transform_finite() {
    let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    let e = res.fetch_mut::<World>().spawn((
        Transform::from_xyz(0.0, 3.0, 0.0).with_scale(Vec3::ZERO),
        ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
        RigidbodyDesc::default(),
    ));
    for _ in 0..30 {
        step(&res, FIXED_DT);
    }
    let t = *res.fetch::<World>().get::<&Transform>(e).unwrap();
    assert!(t.translation.is_finite() && t.rotation.is_finite(), "{t:?}");
    assert!(t.translation.y < 3.0, "still falls: {t:?}");
}

#[test]
fn mesh_collider_with_stray_indices_is_skipped() {
    let res = res_with(Physics::new(Vec3::ZERO));
    res.fetch_mut::<World>().spawn((
        Transform::IDENTITY,
        ColliderDesc::new(ColliderShape::Mesh),
        MeshGeometry {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            indices: vec![[0, 1, 9]],
        },
    ));
    step(&res, FIXED_DT);
    assert_eq!(res.fetch::<Physics>().world.colliders.len(), 0);
}

#[test]
fn non_finite_frame_time_is_ignored() {
    let mut physics = Physics::new(Vec3::ZERO);
    assert_eq!(physics.consume(f32::NAN), 0);
    assert_eq!(physics.consume(-1.0), 0);
    assert_eq!(physics.consume(f32::INFINITY), 0);
    assert_eq!(physics.consume(FIXED_DT), 1);
}

/// A ground box and a falling ball, the ball's layer and mask set by the test.
fn layered_drop(ground_layer: u8, ground_mask: u32, ball_layer: u8, ball_mask: u32) -> f32 {
    let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    let ball = {
        let mut world = res.fetch_mut::<World>();
        let mut floor = ColliderDesc::new(ColliderShape::Box {
            size: Vec3::new(20.0, 1.0, 20.0),
        });
        (floor.layer, floor.mask) = (ground_layer, ground_mask);
        world.spawn((Transform::from_xyz(0.0, -0.5, 0.0), floor));
        let mut sphere = ColliderDesc::new(ColliderShape::Sphere { radius: 0.5 });
        (sphere.layer, sphere.mask) = (ball_layer, ball_mask);
        world.spawn((
            Transform::from_xyz(0.0, 3.0, 0.0),
            sphere,
            RigidbodyDesc::default(),
        ))
    };
    for _ in 0..180 {
        step(&res, FIXED_DT);
    }
    let world = res.fetch::<World>();
    let pose = *world.get::<&Transform>(ball).unwrap();
    pose.translation.y
}

#[test]
fn layers_and_masks_decide_who_collides() {
    // Everything on layer 0 and with the full mask: the ball rests on the floor.
    assert!((layered_drop(0, u32::MAX, 0, u32::MAX) - 0.5).abs() < 0.05);
    // Ball on layer 2, the floor's mask leaves layer 2 out: it falls through.
    assert!(layered_drop(0, !(1 << 2), 2, u32::MAX) < -5.0);
    // Both sides must agree: the ball's mask leaves the floor's layer out.
    assert!(layered_drop(1, u32::MAX, 2, !(1 << 1)) < -5.0);
    // Masks that include each other's layers collide.
    assert!((layered_drop(1, 1 << 2, 2, 1 << 1) - 0.5).abs() < 0.05);
}

#[test]
fn queries_filter_by_the_collider_layer_not_its_mask() {
    let res = res_with(Physics::new(Vec3::ZERO));
    let (wall, glass) = {
        let mut world = res.fetch_mut::<World>();
        let mut a = ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE });
        a.layer = 1;
        // A mask that includes nothing must not hide it from queries.
        a.mask = 0;
        let mut b = ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE });
        b.layer = 4;
        (
            world.spawn((Transform::from_xyz(5.0, 0.0, 0.0), a)),
            world.spawn((Transform::from_xyz(10.0, 0.0, 0.0), b)),
        )
    };
    step(&res, FIXED_DT);
    let physics = res.fetch::<Physics>();
    let ray = |opts: QueryOptions| physics.raycast_with(Vec3::ZERO, Vec3::X, 50.0, &opts);
    assert_eq!(ray(QueryOptions::default()).unwrap().entity, wall);
    assert_eq!(
        ray(QueryOptions::default().mask(1 << 1)).unwrap().entity,
        wall
    );
    // Mask without layer 1: the ray goes through the wall and hits the glass.
    let hit = ray(QueryOptions::default().mask(1 << 4)).unwrap();
    assert_eq!(hit.entity, glass);
    assert!((hit.distance - 9.5).abs() < 1e-3);
    assert!(ray(QueryOptions::default().mask(1 << 7)).is_none());
    assert_eq!(
        ray(QueryOptions::default().exclude(wall)).unwrap().entity,
        glass
    );
}

#[test]
fn shapecasts_sweep_boxes_and_capsules_and_overlap_finds_neighbours() {
    let res = res_with(Physics::new(Vec3::ZERO));
    let target = {
        let mut world = res.fetch_mut::<World>();
        world.spawn((
            Transform::from_xyz(6.0, 0.0, 0.0),
            ColliderDesc::new(ColliderShape::Box {
                size: Vec3::splat(2.0),
            }),
        ))
    };
    step(&res, FIXED_DT);
    let physics = res.fetch::<Physics>();
    let opts = QueryOptions::default();
    // The box's front starts at x = 1 and meets the target's face (x = 5): 4 m.
    let hit = physics
        .shapecast(
            CastShape::Box {
                half_extents: Vec3::ONE,
            },
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::X,
            20.0,
            &opts,
        )
        .unwrap();
    assert_eq!(hit.entity, target);
    assert!((hit.distance - 4.0).abs() < 1e-3, "{}", hit.distance);
    assert!(hit.normal.x < -0.9);
    // A capsule lying on its side (rotated) fits through a gap the box does not.
    let capsule = CastShape::Capsule {
        radius: 0.25,
        half_height: 0.5,
    };
    let up = physics.shapecast(
        capsule,
        Vec3::new(0.0, 5.0, 0.0),
        Quat::IDENTITY,
        Vec3::X,
        20.0,
        &opts,
    );
    assert!(up.is_none(), "passes above the target");
    // Overlap: a sphere around the target finds it, one far away does not.
    let near = physics.overlap(
        CastShape::Sphere { radius: 1.5 },
        Vec3::new(4.5, 0.0, 0.0),
        Quat::IDENTITY,
        &opts,
    );
    assert_eq!(near, [target]);
    assert!(
        physics
            .overlap(
                CastShape::Sphere { radius: 1.0 },
                Vec3::new(0.0, 9.0, 0.0),
                Quat::IDENTITY,
                &opts
            )
            .is_empty()
    );
    assert!(
        physics
            .overlap(
                CastShape::Sphere { radius: 1.5 },
                Vec3::new(4.5, 0.0, 0.0),
                Quat::IDENTITY,
                &opts.mask(1 << 5)
            )
            .is_empty()
    );
}

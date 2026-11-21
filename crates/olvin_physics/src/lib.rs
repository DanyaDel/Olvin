//! Physics on Rapier. Entities with a `ColliderDesc` (and optionally a
//! `RigidbodyDesc`) get bodies automatically; dynamic bodies write their pose
//! back into `Transform`, kinematic ones follow it. Fixed 60 Hz steps.
//!
//! Also: character controllers ([`CharacterMotor`], from
//! `CharacterControllerDesc`), collision/trigger events ([`CollisionEvents`]),
//! scene queries ([`Physics::raycast`], [`Physics::spherecast`]), pushing bodies
//! ([`Physics::apply_force`], [`Physics::apply_impulse`], [`Physics::set_velocity`]),
//! controller hits ([`ControllerHit`] through `Events`) and a hook for
//! code that must run before every fixed step ([`FixedSystems`]).
//!
//! Who owns the pose: kinematic bodies follow `Transform` every step; dynamic
//! and static (collider without `Rigidbody`) bodies are teleported when gameplay
//! writes their `Transform` (compared with the pose physics last saw, so an
//! untouched body costs one matrix compare per step). A teleport has no
//! velocity: a static collider that moves every frame should be Kinematic to
//! push things properly. Scale is baked into the shape at creation and is not
//! followed.

mod character;
mod events;
mod forces;
mod query;

pub use character::{CharacterMotor, CharacterMove, ControllerHit, move_character};
pub use events::{CollisionEvent, CollisionEvents};
pub use query::Hit;

use std::sync::mpsc;

use glam::{Mat4, Quat, Vec3};
use olvin_core::{App, Events, Plugin, Resources, Stage, Time};
use olvin_ecs::{EcsPlugin, Entity, World};
use olvin_scene::file::{
    BodyKind, CharacterControllerDesc, ColliderDesc, ColliderShape, RigidbodyDesc,
};
use olvin_scene::{MeshGeometry, Transform};
use olvin_transform::{Parent, world_matrix};
use rapier3d::prelude as rp;

pub use rapier3d;

pub const FIXED_DT: f32 = 1.0 / 60.0;
const MAX_STEPS_PER_FRAME: u32 = 5;

/// Marks an entity that already has a body in the simulation.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub handle: rp::RigidBodyHandle,
    kind: Kind,
    /// World matrix of the `Transform` the body was last synced with: a
    /// different one means gameplay moved a fixed or dynamic body.
    seen: Mat4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Dynamic,
    Kinematic,
    Fixed,
    /// Kinematic body moved by the character controller, not by `Transform`.
    Character,
}

pub struct Physics {
    pub world: rp::PhysicsWorld,
    accumulator: f32,
    /// Bodies with a force pending, cleared after the step.
    forced: Vec<rp::RigidBodyHandle>,
    /// Controller hits since the last step, moved into `Events<ControllerHit>`.
    pub(crate) hits: Vec<ControllerHit>,
    collector: rp::ChannelEventCollector,
    collisions: mpsc::Receiver<rp::CollisionEvent>,
    // The collector needs these channels; their events are not used.
    _forces: mpsc::Receiver<rp::ContactForceEvent>,
    _tears: mpsc::Receiver<rp::SoftBodyTearEvent>,
}

impl Physics {
    pub fn new(gravity: Vec3) -> Self {
        let mut world = rp::PhysicsWorld::new();
        world.gravity = v(gravity);
        world.integration_parameters.dt = FIXED_DT;
        let (cs, collisions) = mpsc::channel();
        let (fs, _forces) = mpsc::channel();
        let (ts, _tears) = mpsc::channel();
        Self {
            world,
            accumulator: 0.0,
            forced: Vec::new(),
            hits: Vec::new(),
            collector: rp::ChannelEventCollector::new(cs, fs, ts),
            collisions,
            _forces,
            _tears,
        }
    }

    pub fn gravity(&self) -> Vec3 {
        g(self.world.gravity)
    }

    /// Advances by `dt` seconds in fixed steps; returns how many were taken.
    /// Low level: no characters, events or [`FixedSystems`] (see [`step`]).
    pub fn advance(&mut self, dt: f32) -> u32 {
        let steps = self.consume(dt);
        for _ in 0..steps {
            self.world.step();
            self.clear_forces();
        }
        steps
    }

    /// Adds `dt` to the accumulator and returns how many fixed steps are due.
    fn consume(&mut self, dt: f32) -> u32 {
        // A NaN or negative frame time must not poison the accumulator
        // (`NaN.min(x)` is `x`: it would burst the step cap).
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        self.accumulator = (self.accumulator + dt).min(FIXED_DT * MAX_STEPS_PER_FRAME as f32);
        let mut steps = 0;
        while self.accumulator >= FIXED_DT {
            self.accumulator -= FIXED_DT;
            steps += 1;
        }
        steps
    }

    fn step_with_events(&mut self) {
        self.world.step_with_events(&(), &self.collector);
        self.clear_forces();
    }

    /// The entity owning a collider (every collider made by this crate knows it).
    pub fn collider_entity(&self, handle: rp::ColliderHandle) -> Option<Entity> {
        self.world
            .colliders
            .get(handle)
            .and_then(|c| entity_from(c.user_data))
    }
}

/// Code that must run right before every fixed physics step (Unity's
/// `FixedUpdate`), e.g. scripts. Systems get the resources with nothing borrowed.
#[derive(Default)]
pub struct FixedSystems {
    systems: Vec<FixedSystem>,
}

pub type FixedSystem = Box<dyn FnMut(&Resources)>;

impl FixedSystems {
    pub fn add(&mut self, system: impl FnMut(&Resources) + 'static) {
        self.systems.push(Box::new(system));
    }

    pub fn len(&self) -> usize {
        self.systems.len()
    }

    pub fn is_empty(&self) -> bool {
        self.systems.is_empty()
    }
}

pub struct PhysicsPlugin {
    pub gravity: Vec3,
}

impl Default for PhysicsPlugin {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(EcsPlugin)
            .insert_resource(Physics::new(self.gravity))
            .init_resource::<CollisionEvents>()
            .init_resource::<FixedSystems>()
            .add_event::<ControllerHit>()
            .add_system(Stage::PreUpdate, |res| {
                let dt = res.fetch::<Time>().delta();
                step(res, dt);
            });
    }
}

/// One frame of physics: create new bodies (and drop those of despawned
/// entities), then for every due fixed step: run [`FixedSystems`], push
/// teleported fixed/dynamic bodies and kinematic poses, move characters,
/// simulate, pull dynamic poses. [`CollisionEvents`] (if present) holds this
/// frame's events afterwards; controller hits go to `Events<ControllerHit>`.
pub fn step(res: &Resources, dt: f32) {
    let steps = {
        let mut physics = res.fetch_mut::<Physics>();
        let mut world = res.fetch_mut::<World>();
        if let Some(mut events) = res.get_mut::<CollisionEvents>() {
            events.clear();
        }
        remove_orphans(&world, &mut physics);
        // New colliders reach scene queries with the next step. (Rapier's
        // `detect_collisions` would do it now, but it consumes the bodies'
        // change flags and the next step then ignores them.)
        create_bodies(&mut world, &mut physics);
        physics.consume(dt)
    };
    for _ in 0..steps {
        run_fixed_systems(res);
        let mut physics = res.fetch_mut::<Physics>();
        let mut world = res.fetch_mut::<World>();
        create_bodies(&mut world, &mut physics);
        sync_moved(&world, &mut physics);
        push_kinematic(&world, &mut physics);
        character::move_characters(&mut world, &mut physics, FIXED_DT);
        physics.step_with_events();
        pull_poses(&mut world, &physics);
        let mut events = res.get_mut::<CollisionEvents>();
        events::drain(&physics, events.as_deref_mut());
        send_hits(res, &mut physics);
    }
}

/// Moves the step's controller hits (and those of [`move_character`] calls
/// since the last step) into `Events<ControllerHit>`, or drops them.
fn send_hits(res: &Resources, physics: &mut Physics) {
    match res.get_mut::<Events<ControllerHit>>() {
        Some(mut events) => events.send_batch(physics.hits.drain(..)),
        None => physics.hits.clear(),
    }
}

/// The matrix [`sync_moved`] compares: the local one for roots (cheap), the
/// whole parent chain for children.
fn current_matrix(world: &World, e: Entity, t: &Transform, parented: bool) -> Option<Mat4> {
    if parented {
        world_matrix(world, e)
    } else {
        Some(t.matrix())
    }
}

/// Teleports fixed and dynamic bodies whose `Transform` gameplay changed
/// since physics last saw it. Velocity is kept, like setting
/// `transform.position` of a Unity rigidbody.
fn sync_moved(world: &World, physics: &mut Physics) {
    let moved: Vec<(Entity, rp::RigidBodyHandle, Mat4)> = world
        .query::<(Entity, &Body, &Transform, Option<&Parent>)>()
        .iter()
        .filter(|(_, b, _, _)| matches!(b.kind, Kind::Fixed | Kind::Dynamic))
        .filter_map(|(e, b, t, parent)| {
            let m = current_matrix(world, e, t, parent.is_some())?;
            (m != b.seen).then_some((e, b.handle, m))
        })
        .collect();
    for (e, handle, m) in moved {
        if let Some(rb) = physics.world.bodies.get_mut(handle) {
            rb.set_position(pose(&decompose(m)), true);
        }
        if let Ok(mut b) = world.get::<&mut Body>(e) {
            b.seen = m;
        }
    }
}

fn run_fixed_systems(res: &Resources) {
    let Some(mut systems) = res
        .get_mut::<FixedSystems>()
        .map(|mut f| std::mem::take(&mut *f))
    else {
        return;
    };
    for system in &mut systems.systems {
        system(res);
    }
    // Keep systems a fixed system may have added meanwhile.
    let mut slot = res.fetch_mut::<FixedSystems>();
    systems.systems.append(&mut slot.systems);
    *slot = systems;
}

fn push_kinematic(world: &World, physics: &mut Physics) {
    for (e, body) in world.query::<(Entity, &Body)>().iter() {
        if body.kind != Kind::Kinematic {
            continue;
        }
        if let Some(m) = world_matrix(world, e)
            && let Some(rb) = physics.world.bodies.get_mut(body.handle)
        {
            rb.set_next_kinematic_position(pose(&decompose(m)));
        }
    }
}

/// Dynamic bodies and characters own their world pose; store it relative to the parent.
fn pull_poses(world: &mut World, physics: &Physics) {
    let moved: Vec<(Entity, Option<Entity>, Vec3, Option<Quat>)> = world
        .query::<(Entity, &Body, Option<&Parent>)>()
        .iter()
        .filter(|(_, b, _)| matches!(b.kind, Kind::Dynamic | Kind::Character))
        .filter_map(|(e, b, parent)| {
            let p = physics.world.bodies.get(b.handle)?.position();
            // A character's rotation belongs to gameplay (only its position is simulated).
            let rotation =
                (b.kind == Kind::Dynamic).then(|| Quat::from_array(p.rotation.to_array()));
            Some((e, parent.map(|p| p.0), g(p.translation), rotation))
        })
        .collect();
    for (e, parent, translation, rotation) in moved {
        let parent_matrix = parent.and_then(|p| world_matrix(world, p));
        if let Ok(mut t) = world.get::<&mut Transform>(e) {
            match (parent_matrix, rotation) {
                (Some(pm), Some(rotation)) => {
                    let local =
                        pm.inverse() * glam::Mat4::from_rotation_translation(rotation, translation);
                    let (_, r, tr) = split(local);
                    if tr.is_finite() {
                        t.translation = tr;
                        t.rotation = r;
                    }
                }
                (Some(pm), None) => {
                    let local = pm.inverse().transform_point3(translation);
                    if local.is_finite() {
                        t.translation = local;
                    }
                }
                (None, rotation) => {
                    t.translation = translation;
                    if let Some(r) = rotation {
                        t.rotation = r;
                    }
                }
            }
        }
        if let Ok(mut c) = world.get::<&mut character::Character>(e) {
            c.last = translation;
        }
        if let Some(m) = world_matrix(world, e)
            && let Ok(mut b) = world.get::<&mut Body>(e)
        {
            b.seen = m;
        }
    }
}

/// Removes the bodies of entities that were despawned.
fn remove_orphans(world: &World, physics: &mut Physics) {
    let orphans: Vec<rp::RigidBodyHandle> = physics
        .world
        .bodies
        .iter()
        .filter(|(_, rb)| entity_from(rb.user_data).is_some_and(|e| !world.contains(e)))
        .map(|(h, _)| h)
        .collect();
    for h in orphans {
        physics.world.remove_body(h);
    }
}

/// `to_scale_rotation_translation` with a finite rotation: a zero (or
/// non-finite) scale makes glam return NaN, which would poison the body and
/// then the `Transform` physics writes back.
fn split(m: Mat4) -> (Vec3, Quat, Vec3) {
    let (scale, rotation, translation) = m.to_scale_rotation_translation();
    let rotation = if rotation.is_finite() {
        rotation
    } else {
        Quat::IDENTITY
    };
    (scale, rotation, translation)
}

fn decompose(m: glam::Mat4) -> Transform {
    let (scale, rotation, translation) = split(m);
    Transform {
        translation,
        rotation,
        scale,
    }
}

/// An entity waiting for its body: everything copied out of the ECS first.
struct Pending {
    entity: Entity,
    transform: Transform,
    collider: Option<ColliderDesc>,
    rigidbody: Option<RigidbodyDesc>,
    mesh: Option<MeshGeometry>,
    character: Option<CharacterControllerDesc>,
}

fn create_bodies(world: &mut World, physics: &mut Physics) {
    let pending: Vec<Pending> = world
        .query::<(
            Entity,
            &Transform,
            Option<&ColliderDesc>,
            Option<&RigidbodyDesc>,
            Option<&MeshGeometry>,
            Option<&CharacterControllerDesc>,
        )>()
        .without::<&Body>()
        .iter()
        .filter(|(_, _, c, r, _, cc)| c.is_some() || r.is_some() || cc.is_some())
        .map(|(entity, t, c, r, m, cc)| Pending {
            entity,
            transform: *t,
            collider: c.cloned(),
            rigidbody: r.cloned(),
            mesh: m.cloned(),
            character: cc.cloned(),
        })
        .collect();
    for mut p in pending {
        // Bodies live in world space: use the world pose of parented objects.
        let seen = world_matrix(world, p.entity).unwrap_or_else(|| p.transform.matrix());
        p.transform = decompose(seen);
        let data = entity_bits(p.entity);
        if let Some(desc) = &p.character {
            if p.rigidbody.is_some() {
                log::warn!("character controller ignores the entity's Rigidbody");
            }
            let (body, character) = character::create(physics, desc, &p.transform, data);
            let _ = world.insert(
                p.entity,
                (
                    Body {
                        handle: body,
                        kind: Kind::Character,
                        seen,
                    },
                    character,
                ),
            );
            if world.get::<&CharacterMotor>(p.entity).is_err() {
                let motor = CharacterMotor {
                    push_mass: desc.push_mass,
                    ..Default::default()
                };
                let _ = world.insert_one(p.entity, motor);
            }
            continue;
        }

        let kind = match &p.rigidbody {
            Some(r) if r.kind == BodyKind::Dynamic => Kind::Dynamic,
            Some(_) => Kind::Kinematic,
            None => Kind::Fixed,
        };
        let mut builder = match kind {
            Kind::Dynamic => rp::RigidBodyBuilder::dynamic(),
            Kind::Kinematic | Kind::Character => rp::RigidBodyBuilder::kinematic_position_based(),
            Kind::Fixed => rp::RigidBodyBuilder::fixed(),
        }
        .pose(pose(&p.transform))
        .user_data(data);
        if let Some(r) = &p.rigidbody {
            let free = r.freeze_rotation.map(|f| !f);
            builder = builder
                .linear_damping(r.drag)
                .angular_damping(r.angular_drag)
                .gravity_scale(r.gravity_scale)
                .ccd_enabled(r.ccd)
                .enabled_rotations(free[0], free[1], free[2]);
        }
        let handle = physics.world.insert_body(builder);

        let shape = p
            .collider
            .clone()
            .unwrap_or_else(|| ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }));
        match collider_builder(
            &shape,
            p.transform.scale,
            p.mesh.as_ref(),
            kind == Kind::Dynamic,
        ) {
            Some(mut c) => {
                if let Some(r) = &p.rigidbody {
                    c = c.mass(r.mass);
                }
                physics
                    .world
                    .insert_collider(c.user_data(data), Some(handle));
            }
            None => log::warn!("mesh collider without geometry: body has no collider"),
        }
        let _ = world.insert_one(p.entity, Body { handle, kind, seen });
    }
}

/// Rapier has no scale: bake the object's scale into the shape.
/// Moving bodies get the convex hull of mesh colliders: Rapier (like PhysX)
/// doesn't simulate concave triangle meshes as dynamic bodies.
fn collider_builder(
    desc: &ColliderDesc,
    scale: Vec3,
    mesh: Option<&MeshGeometry>,
    dynamic: bool,
) -> Option<rp::ColliderBuilder> {
    let s = scale.abs();
    let builder = match &desc.shape {
        ColliderShape::Box { size } => {
            let h = (*size * s * 0.5).max(Vec3::splat(1e-4));
            rp::ColliderBuilder::cuboid(h.x, h.y, h.z)
        }
        ColliderShape::Sphere { radius } => {
            rp::ColliderBuilder::ball((radius * s.max_element()).max(1e-4))
        }
        ColliderShape::Capsule { radius, height } => {
            let r = (radius * s.x.max(s.z)).max(1e-4);
            let half = (height * s.y * 0.5 - r).max(0.0);
            rp::ColliderBuilder::capsule_y(half, r)
        }
        ColliderShape::Mesh => {
            let mesh = mesh?;
            // Parry indexes the vertices unchecked and panics on a stray index.
            let n = mesh.positions.len() as u32;
            if mesh.indices.iter().flatten().any(|&i| i >= n) {
                log::warn!("mesh collider has indices past its vertices: ignored");
                return None;
            }
            let vertices: Vec<rp::Vector> = mesh.positions.iter().map(|p| v(*p * scale)).collect();
            if dynamic {
                rp::ColliderBuilder::convex_hull(&vertices)?
            } else {
                rp::ColliderBuilder::trimesh(vertices, mesh.indices.clone()).ok()?
            }
        }
    };
    let mut builder = builder
        .translation(v(desc.center * scale))
        .sensor(desc.is_trigger)
        .friction(desc.friction)
        .restitution(desc.restitution)
        .active_events(rp::ActiveEvents::COLLISION_EVENTS);
    if desc.is_trigger {
        // Triggers also fire for kinematic bodies and characters, like in
        // Unity, but not static-vs-static (a trigger touching the floor).
        builder = builder.active_collision_types(
            rp::ActiveCollisionTypes::default()
                | rp::ActiveCollisionTypes::KINEMATIC_FIXED
                | rp::ActiveCollisionTypes::KINEMATIC_KINEMATIC,
        );
    }
    Some(builder)
}

fn entity_bits(e: Entity) -> u128 {
    e.to_bits().get() as u128
}

fn entity_from(data: u128) -> Option<Entity> {
    Entity::from_bits(u64::try_from(data).ok()?)
}

fn v(p: Vec3) -> rp::Vector {
    rp::Vector::from_array(p.to_array())
}

fn g(p: rp::Vector) -> Vec3 {
    Vec3::from_array(p.to_array())
}

fn pose(t: &Transform) -> rp::Pose {
    rp::Pose::from_parts(
        v(t.translation),
        rp::Rotation::from_array(t.rotation.normalize().to_array()),
    )
}

#[cfg(test)]
mod tests {
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
            let mv =
                move_character(&mut physics, &mut world, c, Vec3::new(0.5, -1.0, 0.0)).unwrap();
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
}

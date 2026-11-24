//! The per-frame glue between the ECS and rapier: the `step` system, pushing
//! teleports and kinematic poses in, pulling dynamic poses out.

use glam::{Mat4, Quat, Vec3};
use olvin_core::{Events, Resources};
use olvin_ecs::{Entity, World};
use olvin_scene::Transform;
use olvin_transform::{Parent, world_matrix};
use rapier3d::prelude as rp;

use super::*;

/// One frame of physics: create new bodies (and drop those of despawned
/// entities), then for every due fixed step: run [`FixedSystems`], push
/// teleported fixed/dynamic bodies and kinematic poses, move characters,
/// simulate, pull dynamic poses. [`CollisionEvents`] (if present) holds this
/// frame's events afterwards; controller hits go to `Events<ControllerHit>`.
pub fn step(res: &Resources, dt: f32) {
    olvin_core::profile_scope!("physics step");
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
        crate::joints::remove_joints(&mut world, &mut physics);
        crate::joints::create_joints(&mut world, &mut physics);
        physics.consume(dt)
    };
    for _ in 0..steps {
        {
            olvin_core::profile_scope!("physics fixed systems");
            run_fixed_systems(res);
        }
        olvin_core::profile_scope!("physics fixed step");
        let mut physics = res.fetch_mut::<Physics>();
        let mut world = res.fetch_mut::<World>();
        create_bodies(&mut world, &mut physics);
        crate::joints::create_joints(&mut world, &mut physics);
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
pub(crate) fn send_hits(res: &Resources, physics: &mut Physics) {
    match res.get_mut::<Events<ControllerHit>>() {
        Some(mut events) => events.send_batch(physics.hits.drain(..)),
        None => physics.hits.clear(),
    }
}

/// The matrix [`sync_moved`] compares: the local one for roots (cheap), the
/// whole parent chain for children.
pub(crate) fn current_matrix(
    world: &World,
    e: Entity,
    t: &Transform,
    parented: bool,
) -> Option<Mat4> {
    if parented {
        world_matrix(world, e)
    } else {
        Some(t.matrix())
    }
}

/// Teleports fixed and dynamic bodies whose `Transform` gameplay changed
/// since physics last saw it. Velocity is kept, like setting
/// `transform.position` of a Unity rigidbody.
pub(crate) fn sync_moved(world: &World, physics: &mut Physics) {
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

pub(crate) fn run_fixed_systems(res: &Resources) {
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

pub(crate) fn push_kinematic(world: &World, physics: &mut Physics) {
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
pub(crate) fn pull_poses(world: &mut World, physics: &Physics) {
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
pub(crate) fn remove_orphans(world: &World, physics: &mut Physics) {
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
pub(crate) fn split(m: Mat4) -> (Vec3, Quat, Vec3) {
    let (scale, rotation, translation) = m.to_scale_rotation_translation();
    let rotation = if rotation.is_finite() {
        rotation
    } else {
        Quat::IDENTITY
    };
    (scale, rotation, translation)
}

pub(crate) fn decompose(m: glam::Mat4) -> Transform {
    let (scale, rotation, translation) = split(m);
    Transform {
        translation,
        rotation,
        scale,
    }
}

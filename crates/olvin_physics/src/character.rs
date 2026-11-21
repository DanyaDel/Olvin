//! Character controller (Unity's `CharacterController`) on Rapier's
//! `KinematicCharacterController`: a kinematic capsule that slides along
//! walls, climbs steps and slopes up to a limit and snaps down to the ground.
//! It pushes dynamic bodies it walks into (`CharacterMotor::push_mass`) and
//! reports every collider it runs into as a [`ControllerHit`] event.

use glam::Vec3;
use olvin_ecs::{Entity, World};
use olvin_scene::Transform;
use olvin_scene::file::CharacterControllerDesc;
use olvin_transform::{Parent, world_matrix};
use rapier3d::control::{
    CharacterAutostep, CharacterCollision, CharacterLength, EffectiveCharacterMovement,
    KinematicCharacterController,
};
use rapier3d::prelude as rp;
use serde::{Deserialize, Serialize};

use crate::{Body, FIXED_DT, Physics, g, v};

/// Drives an entity with a `CharacterControllerDesc`; added automatically
/// with the controller. Gameplay sets `velocity` (world space, m/s): usually
/// x/z from input every frame and y once for a jump. Every fixed step gravity
/// is added to `velocity.y`, the capsule is moved by `velocity * dt` with
/// collisions, and `grounded` is updated. Landing or hitting a ceiling zeroes
/// the vertical speed. Serializable, so saves keep a jump in mid-air.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterMotor {
    pub velocity: Vec3,
    /// Standing on something walkable after the last step.
    pub grounded: bool,
    /// Sliding down a slope steeper than the limit.
    pub sliding: bool,
    /// 0 turns gravity off (flying, ladders, or moving only with [`move_character`]).
    pub gravity_scale: f32,
    /// Mass (kg) dynamic bodies are pushed with; 0 = the character only
    /// stops at them. Starts at `CharacterControllerDesc::push_mass`.
    pub push_mass: f32,
}

impl Default for CharacterMotor {
    fn default() -> Self {
        Self {
            velocity: Vec3::ZERO,
            grounded: false,
            sliding: false,
            gravity_scale: 1.0,
            push_mass: CharacterControllerDesc::default().push_mass,
        }
    }
}

/// A character ran into a collider while moving (Unity's
/// `OnControllerColliderHit`), sent through `Events<ControllerHit>`: once per
/// collider and fixed step, also for [`move_character`]. Triggers are not hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerHit {
    pub character: Entity,
    pub other: Entity,
    /// Contact point on `other`, world space.
    pub point: Vec3,
    /// Surface normal of `other` at the point (towards the character).
    pub normal: Vec3,
    /// Unit direction the character was moving in.
    pub direction: Vec3,
}

/// Result of [`move_character`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterMove {
    /// The movement actually applied (after collisions).
    pub translation: Vec3,
    pub grounded: bool,
}

/// Internal runtime state of a character.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Character {
    collider: rp::ColliderHandle,
    controller: KinematicCharacterController,
    /// World position the simulation last wrote: a different `Transform` means a teleport.
    pub(crate) last: Vec3,
}

pub(crate) fn create(
    physics: &mut Physics,
    desc: &CharacterControllerDesc,
    transform: &Transform,
    data: u128,
) -> (rp::RigidBodyHandle, Character) {
    let s = transform.scale.abs();
    let radius = (desc.radius * s.x.max(s.z)).max(0.01);
    let half = (desc.height * s.y * 0.5 - radius).max(0.0);
    let body = physics.world.insert_body(
        rp::RigidBodyBuilder::kinematic_position_based()
            .translation(v(transform.translation))
            .user_data(data),
    );
    let collider = physics.world.insert_collider(
        rp::ColliderBuilder::capsule_y(half, radius)
            .user_data(data)
            .active_events(rp::ActiveEvents::COLLISION_EVENTS),
        Some(body),
    );
    let slope = desc.slope_limit_deg.clamp(0.0, 89.9).to_radians();
    let step = desc.step_offset * s.y;
    let controller = KinematicCharacterController {
        up: rp::Vector::Y,
        offset: CharacterLength::Absolute(0.02),
        slide: true,
        autostep: (step > 0.0).then_some(CharacterAutostep {
            max_height: CharacterLength::Absolute(step),
            min_width: CharacterLength::Absolute(radius * 0.5),
            include_dynamic_bodies: false,
        }),
        max_slope_climb_angle: slope,
        min_slope_slide_angle: slope,
        snap_to_ground: Some(CharacterLength::Absolute(step.max(0.05))),
        ..Default::default()
    };
    (
        body,
        Character {
            collider,
            controller,
            last: transform.translation,
        },
    )
}

/// Computes a collision-aware move from the body's latest (pending) pose;
/// the colliders it ran into are appended to `hits`.
fn compute(
    physics: &Physics,
    body: rp::RigidBodyHandle,
    character: &Character,
    desired: Vec3,
    dt: f32,
    hits: &mut Vec<CharacterCollision>,
) -> Option<(Vec3, EffectiveCharacterMovement)> {
    let rb = physics.world.bodies.get(body)?;
    let pos = *rb.next_position();
    let shape = physics.world.colliders.get(character.collider)?.shape();
    let filter = rp::QueryFilter::default()
        .exclude_rigid_body(body)
        .exclude_sensors();
    let queries = physics.world.query_pipeline_with_filter(filter);
    let movement = character
        .controller
        .move_shape(dt, &queries, shape, &pos, v(desired), |c| hits.push(c));
    Some((g(pos.translation), movement))
}

/// Rapier only pushes a character out of overlaps on a zero-length move, and a
/// moving character that starts inside the floor (e.g. placed exactly on it)
/// would stay stuck. A zero move first resolves that.
fn depenetrate(physics: &mut Physics, body: rp::RigidBodyHandle, character: &Character, dt: f32) {
    let Some((from, mv)) = compute(physics, body, character, Vec3::ZERO, dt, &mut Vec::new())
    else {
        return;
    };
    let push = g(mv.translation);
    if push.length_squared() > 1e-12
        && let Some(rb) = physics.world.bodies.get_mut(body)
    {
        rb.set_next_kinematic_translation(v(from + push));
    }
}

/// [`compute`], but a move blocked from the very start (the character
/// overlaps something, e.g. it was placed exactly on the floor) is retried
/// after pushing the character out.
fn move_unstuck(
    physics: &mut Physics,
    body: rp::RigidBodyHandle,
    character: &Character,
    desired: Vec3,
    dt: f32,
    hits: &mut Vec<CharacterCollision>,
) -> Option<(Vec3, EffectiveCharacterMovement)> {
    let first = compute(physics, body, character, desired, dt, hits)?;
    let wanted = Vec3::new(desired.x, 0.0, desired.z).length();
    let got = Vec3::new(first.1.translation.x, 0.0, first.1.translation.z).length();
    if wanted < 1e-5 || got > wanted * 0.05 {
        return Some(first);
    }
    depenetrate(physics, body, character, dt);
    hits.clear();
    compute(physics, body, character, desired, dt, hits)
}

/// After a move: queues a [`ControllerHit`] per collider hit and pushes the
/// dynamic ones with Rapier's impulse solver (it transfers the part of the
/// move the body blocked, scaled by the character/body mass ratio).
#[allow(clippy::too_many_arguments)]
fn after_move(
    physics: &mut Physics,
    entity: Entity,
    body: rp::RigidBodyHandle,
    character: &Character,
    hits: &[CharacterCollision],
    desired: Vec3,
    push_mass: f32,
    dt: f32,
) {
    if hits.is_empty() {
        return;
    }
    let direction = desired.normalize_or_zero();
    for (i, hit) in hits.iter().enumerate() {
        // The solver may slide along the same collider several times per move.
        if hits[..i].iter().any(|h| h.handle == hit.handle) {
            continue;
        }
        if let Some(other) = physics.collider_entity(hit.handle) {
            physics.hits.push(ControllerHit {
                character: entity,
                other,
                point: g(hit.hit.witness1),
                normal: g(hit.hit.normal1),
                direction,
            });
        }
    }
    if push_mass <= 0.0 {
        return;
    }
    let Some(shape) = physics
        .world
        .colliders
        .get(character.collider)
        .map(|c| c.shared_shape().clone())
    else {
        return;
    };
    let filter = rp::QueryFilter::default()
        .exclude_rigid_body(body)
        .exclude_sensors();
    let w = &mut physics.world;
    let mut queries = w.broad_phase.as_query_pipeline_mut(
        w.narrow_phase.query_dispatcher(),
        &mut w.bodies,
        &mut w.colliders,
        filter,
    );
    character.controller.solve_character_collision_impulses(
        dt,
        &mut queries,
        &*shape,
        push_mass,
        hits,
    );
}

/// Applies a teleport: `Transform` was moved by gameplay since the last write.
fn sync_teleport(world: &World, physics: &mut Physics, e: Entity, body: rp::RigidBodyHandle) {
    let Some(at) = world_matrix(world, e).map(|m| m.w_axis.truncate()) else {
        return;
    };
    let Ok(mut c) = world.get::<&mut Character>(e) else {
        return;
    };
    if at.distance_squared(c.last) > 1e-8
        && let Some(rb) = physics.world.bodies.get_mut(body)
    {
        rb.set_translation(v(at), true);
        c.last = at;
    }
}

/// One fixed step for every character with a [`CharacterMotor`].
pub(crate) fn move_characters(world: &mut World, physics: &mut Physics, dt: f32) {
    let gravity = physics.gravity();
    let characters: Vec<(Entity, rp::RigidBodyHandle, Character, CharacterMotor)> = world
        .query::<(Entity, &Body, &Character, &CharacterMotor)>()
        .iter()
        .map(|(e, b, c, m)| (e, b.handle, *c, *m))
        .collect();
    for (e, body, _, _) in &characters {
        sync_teleport(world, physics, *e, *body);
    }
    let mut hits = Vec::new();
    for (e, body, character, mut motor) in characters {
        motor.velocity += gravity * motor.gravity_scale * dt;
        let desired = motor.velocity * dt;
        hits.clear();
        let Some((from, mv)) = move_unstuck(physics, body, &character, desired, dt, &mut hits)
        else {
            continue;
        };
        after_move(
            physics,
            e,
            body,
            &character,
            &hits,
            desired,
            motor.push_mass,
            dt,
        );
        let moved = g(mv.translation);
        if let Some(rb) = physics.world.bodies.get_mut(body) {
            rb.set_next_kinematic_translation(v(from + moved));
        }
        motor.grounded = mv.grounded;
        motor.sliding = mv.is_sliding_down_slope;
        if mv.grounded && motor.velocity.y < 0.0 {
            motor.velocity.y = 0.0;
        }
        // Bumped a ceiling.
        if desired.y > 1e-5 && moved.y < desired.y * 0.5 {
            motor.velocity.y = 0.0;
        }
        if let Ok(mut m) = world.get::<&mut CharacterMotor>(e) {
            *m = motor;
        }
    }
}

/// Moves a character right now by `desired` (world space, metres) with
/// collisions, like Unity's `CharacterController.Move`. Updates `Transform`
/// and `CharacterMotor::grounded`, pushes bodies and queues hits like a
/// motor step. `None` if the entity has no controller yet (it gets one at
/// the next physics step).
///
/// Mixing this with `CharacterMotor::velocity` adds both; set the motor's
/// `gravity_scale` to 0 to drive the character only from here.
pub fn move_character(
    physics: &mut Physics,
    world: &mut World,
    entity: Entity,
    desired: Vec3,
) -> Option<CharacterMove> {
    let body = world.get::<&Body>(entity).ok()?.handle;
    let character = *world.get::<&Character>(entity).ok()?;
    sync_teleport(world, physics, entity, body);
    let mut hits = Vec::new();
    let (from, mv) = move_unstuck(physics, body, &character, desired, FIXED_DT, &mut hits)?;
    let push_mass = world
        .get::<&CharacterMotor>(entity)
        .map_or(0.0, |m| m.push_mass);
    after_move(
        physics, entity, body, &character, &hits, desired, push_mass, FIXED_DT,
    );
    let moved = g(mv.translation);
    let to = from + moved;
    physics
        .world
        .bodies
        .get_mut(body)?
        .set_next_kinematic_translation(v(to));
    let parent = world
        .get::<&Parent>(entity)
        .ok()
        .and_then(|p| world_matrix(world, p.0));
    if let Ok(mut t) = world.get::<&mut Transform>(entity) {
        t.translation = parent.map_or(to, |pm| pm.inverse().transform_point3(to));
    }
    if let Ok(mut c) = world.get::<&mut Character>(entity) {
        c.last = to;
    }
    if let Ok(mut m) = world.get::<&mut CharacterMotor>(entity) {
        m.grounded = mv.grounded;
        m.sliding = mv.is_sliding_down_slope;
    }
    Some(CharacterMove {
        translation: moved,
        grounded: mv.grounded,
    })
}

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

mod bodies;
mod character;
mod events;
mod forces;
mod query;
mod sync;

pub use character::{CharacterMotor, CharacterMove, ControllerHit, move_character};
pub use events::{CollisionEvent, CollisionEvents};
pub use query::{CastShape, Hit, QueryOptions};

use bodies::*;
pub use sync::step;
use sync::*;

use std::sync::mpsc;

use glam::{Mat4, Vec3};
use olvin_core::{App, Plugin, Resources, Stage, Time};
use olvin_ecs::{EcsPlugin, Entity};
use olvin_scene::Transform;
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
mod tests;

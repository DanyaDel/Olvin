//! Prediction of a `CharacterMotor` character on `olvin_physics::move_character`
//! (feature `physics`). The same [`character_step`] runs on the server and on the
//! owning client, so with equal colliders they agree to the last bit.

use glam::{Vec2, Vec3};
use olvin_core::Resources;
use olvin_ecs::{Entity, World};
use olvin_physics::{Physics, move_character};
use olvin_scene::Transform;
use serde::{Deserialize, Serialize};

use crate::net::Net;
use crate::predict::Prediction;

/// What a character is, for prediction: position, velocity, ground contact.
/// Replicated to everybody; the owner uses it for reconciliation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CharacterState {
    pub position: Vec3,
    pub velocity: Vec3,
    pub grounded: bool,
}

impl CharacterState {
    /// Standing at `position`. Spawn the player entity with this (and the same
    /// `Transform`): the owner is only predicted once it has a state.
    pub fn at(position: Vec3) -> Self {
        Self {
            position,
            ..Default::default()
        }
    }
}

/// One tick of player intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CharacterInput {
    /// World-space direction on the ground (x, z), length at most 1 (the server clamps it).
    pub dir: Vec2,
    pub jump: bool,
}

/// Movement constants (a resource; defaults are the usual shooter feel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterParams {
    pub speed: f32,
    pub jump_speed: f32,
    pub gravity: f32,
}

impl Default for CharacterParams {
    fn default() -> Self {
        Self {
            speed: 5.0,
            jump_speed: 6.0,
            gravity: 9.81,
        }
    }
}

/// Advances a character by one tick of `input` (the shared simulation).
pub fn character_step(res: &Resources, e: Entity, input: &CharacterInput, dt: f32) {
    let params = res
        .get::<CharacterParams>()
        .map_or_else(CharacterParams::default, |p| *p);
    let mut world = res.fetch_mut::<World>();
    // The prediction moves the character; physics must not also integrate it.
    if let Ok(mut m) = world.get::<&mut olvin_physics::CharacterMotor>(e) {
        m.gravity_scale = 0.0;
        m.velocity = Vec3::ZERO;
    }
    let mut state = world.get::<&CharacterState>(e).map_or_else(
        |_| CharacterState {
            position: world
                .get::<&Transform>(e)
                .map_or(Vec3::ZERO, |t| t.translation),
            ..Default::default()
        },
        |s| *s,
    );
    let dir = input.dir.clamp_length_max(1.0);
    state.velocity.x = dir.x * params.speed;
    state.velocity.z = dir.y * params.speed;
    if input.jump && state.grounded {
        state.velocity.y = params.jump_speed;
    }
    state.velocity.y -= params.gravity * dt;
    let desired = state.velocity * dt;
    let mut physics = res.fetch_mut::<Physics>();
    if let Some(mv) = move_character(&mut physics, &mut world, e, desired) {
        state.grounded = mv.grounded;
        if mv.grounded && state.velocity.y < 0.0 {
            state.velocity.y = 0.0;
        }
        if desired.y > 1e-5 && mv.translation.y < desired.y * 0.5 {
            state.velocity.y = 0.0;
        }
    }
    state.position = world
        .get::<&Transform>(e)
        .map_or(state.position, |t| t.translation);
    let _ = world.insert_one(e, state);
}

fn on_correct(res: &Resources, e: Entity) {
    let world = res.fetch_mut::<World>();
    let Ok(state) = world.get::<&CharacterState>(e).map(|s| *s) else {
        return;
    };
    if let Ok(mut t) = world.get::<&mut Transform>(e) {
        t.translation = state.position;
    }
    res.fetch_mut::<Physics>().teleport(&world, e);
}

/// The ready-made [`Prediction`] for characters. `sample` reads the player's input.
pub fn character_prediction(
    sample: fn(&Resources) -> CharacterInput,
) -> Prediction<CharacterState, CharacterInput> {
    Prediction::new(
        sample,
        character_step,
        |a: &CharacterState, b: &CharacterState| a.position.distance(b.position),
        |a: &CharacterState, b: &CharacterState, t: f32| CharacterState {
            position: a.position.lerp(b.position, t),
            velocity: a.velocity.lerp(b.velocity, t),
            grounded: b.grounded,
        },
    )
    .sanitize(|i| {
        i.dir = if i.dir.is_finite() {
            i.dir.clamp_length_max(1.0)
        } else {
            Vec2::ZERO
        }
    })
    .on_correct(on_correct)
}

impl Net {
    /// Predicts character movement for the entity each client owns: registers
    /// `CharacterState` and `CharacterInput`, and stops snapshots from moving
    /// the owner's `Transform` (the prediction does).
    pub fn predict_character(&mut self, sample: fn(&Resources) -> CharacterInput) -> &mut Self {
        self.predict(character_prediction(sample));
        self.owner_simulated::<Transform>();
        self
    }
}

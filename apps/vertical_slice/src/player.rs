//! First-person controller: mouse look, WASD, sprint, jump, and pushing
//! dynamic bodies (the character controller itself does not push them).

use olvin::prelude::*;
use serde::{Deserialize, Serialize};

/// Marks the player's body (the entity with the character controller).
#[derive(Clone, Copy, Debug, Default)]
pub struct Player;

/// Marks the camera child of the player.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerCamera;

/// Look angles in radians. Saved, so a loaded game looks the same way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerLook {
    pub yaw: f32,
    pub pitch: f32,
}

pub const WALK_SPEED: f32 = 4.0;
pub const SPRINT_SPEED: f32 = 7.0;
pub const JUMP_SPEED: f32 = 5.0;
/// Radians per pixel of mouse movement.
pub const MOUSE_SENSITIVITY: f32 = 0.0025;
/// Capsule radius + how far ahead of it bodies get pushed.
const PUSH_REACH: f32 = 0.4 + 0.35;
/// Pushed bodies move at this fraction of the walking speed.
const PUSH_FACTOR: f32 = 0.8;

/// The player's step: look, move, jump, push. Runs in `Playing`.
pub fn control(res: &mut Resources) {
    let (look, mv, sprint, jump) = {
        let a = res.fetch::<ActionState>();
        (
            a.axis2d("Look"),
            a.axis2d("Move"),
            a.pressed("Sprint"),
            a.just_pressed("Jump"),
        )
    };
    let mut push: Option<(Vec3, Vec3)> = None;
    let mut pitch = None;
    {
        let mut world = res.fetch_mut::<World>();
        for (t, view, motor) in world
            .query_mut::<(&mut Transform, &mut PlayerLook, &mut CharacterMotor)>()
            .with::<&Player>()
        {
            view.yaw -= look.x * MOUSE_SENSITIVITY;
            view.pitch = (view.pitch + look.y * MOUSE_SENSITIVITY).clamp(-1.5, 1.5);
            t.rotation = Quat::from_rotation_y(view.yaw);
            let forward = t.rotation * Vec3::NEG_Z;
            let right = t.rotation * Vec3::X;
            let dir = (forward * mv.y + right * mv.x).clamp_length_max(1.0);
            let speed = if sprint { SPRINT_SPEED } else { WALK_SPEED };
            motor.velocity.x = dir.x * speed;
            motor.velocity.z = dir.z * speed;
            if jump && motor.grounded {
                motor.velocity.y = JUMP_SPEED;
            }
            pitch = Some(view.pitch);
            if dir.length_squared() > 0.01 {
                push = Some((t.translation, dir * speed));
            }
        }
        if let Some(pitch) = pitch {
            for (t, _) in world.query_mut::<(&mut Transform, &PlayerCamera)>() {
                t.rotation = Quat::from_rotation_x(pitch);
            }
        }
    }
    if let Some((at, velocity)) = push {
        push_bodies(res, at, velocity);
    }
}

/// Rapier's kinematic character controller stops at dynamic bodies instead of
/// shoving them, and Olvin has no "controller hit" callback, so the game looks
/// for bodies right in front of the capsule and drags them along.
pub fn push_bodies(res: &Resources, at: Vec3, velocity: Vec3) {
    let dir = Vec3::new(velocity.x, 0.0, velocity.z).normalize_or_zero();
    if dir == Vec3::ZERO {
        return;
    }
    let world = res.fetch::<World>();
    let mut physics = res.fetch_mut::<Physics>();
    let probe = at + dir * PUSH_REACH - Vec3::Y * 0.3;
    for e in physics.overlap_sphere(probe, 0.35) {
        if world.get::<&Player>(e).is_ok() {
            continue;
        }
        let Some(current) = physics.velocity(&world, e) else {
            continue; // not a dynamic body
        };
        let target = dir * velocity.length() * PUSH_FACTOR;
        if current.dot(dir) < target.length() {
            physics.set_velocity(&world, e, Vec3::new(target.x, current.y, target.z));
        }
    }
}

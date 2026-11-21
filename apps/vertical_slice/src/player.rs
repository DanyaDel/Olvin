//! First-person controller: mouse look, WASD, sprint, jump. Pushing crates is
//! the engine's character controller (`CharacterMotor::push_mass`).

use olvin::prelude::*;
use serde::{Deserialize, Serialize};

/// Marks the player's body (the entity with the character controller).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Player;

/// Marks the camera child of the player.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
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

/// The player's step: look, move, jump. Runs in `Playing`.
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
        }
        if let Some(pitch) = pitch {
            for (t, _) in world.query_mut::<(&mut Transform, &PlayerCamera)>() {
                t.rotation = Quat::from_rotation_x(pitch);
            }
        }
    }
}

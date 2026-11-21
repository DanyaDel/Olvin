//! The animated character: a skinned glTF robot that idles, turns to the
//! player and waves when the player comes close.
//!
//! The model, its Idle ↔ Wave graph and its capsule are in the level scene
//! (`EntityDesc::model`); the engine builds the meshes, skeleton and
//! `Animator`. The game only drives the graph's parameter and the facing.

use olvin::anim::Animator;
use olvin::prelude::*;

use crate::player::Player;

/// The robot's object in the scene.
pub const NAME: &str = "Robot";
/// The robot waves while the player is closer than this.
pub const WAVE_DISTANCE: f32 = 3.5;
/// Animator parameter driving Idle ↔ Wave (declared in the scene's graph).
pub const NEAR_PARAM: &str = "near";

/// Marks the robot's root (it carries the `Animator`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Robot;

/// Faces the player and waves when they are near. Simulation time, so it
/// freezes on pause like the animation itself.
pub fn update(res: &mut Resources) {
    let dt = res.fetch::<Time>().delta();
    let mut world = res.fetch_mut::<World>();
    let Some(player) = world
        .query_mut::<(&Transform, &Player)>()
        .into_iter()
        .map(|(t, _)| t.translation)
        .next()
    else {
        return;
    };
    for (t, animator, _) in world.query_mut::<(&mut Transform, &mut Animator, &Robot)>() {
        let to = player - t.translation;
        let flat = Vec2::new(to.x, to.z);
        let near = flat.length() < WAVE_DISTANCE;
        animator.params.set_bool(NEAR_PARAM, near);
        if near && flat.length() > 0.1 {
            // The model faces +Z.
            let want = Quat::from_rotation_y(flat.x.atan2(flat.y));
            t.rotation = t.rotation.slerp(want, (dt * 6.0).min(1.0));
        }
    }
}

/// Name of the state the robot's base layer is in (tests, debugging).
pub fn current_state(animator: &Animator) -> Option<&str> {
    let layer = animator.layers.first()?;
    layer
        .graph
        .states
        .get(layer.state.current)
        .map(|s| s.name.as_str())
}

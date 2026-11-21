//! Runtime playback of a scene's object animation ([`olvin_scene::action`]):
//! an [`ActionPlayer`] resource drives the `Transform` of every entity that
//! has F-curves, found by its [`SceneId`]. `AnimPlugin` advances it.

use std::collections::HashMap;

use glam::{EulerRot, Quat, Vec3};
use olvin_core::{Resources, Time};
use olvin_ecs::World;
use olvin_scene::action::{Action, Prop};
use olvin_scene::{SceneId, Transform};

/// An object's authored transform: location, Euler rotation (degrees),
/// scale. Channels without a curve keep these values.
pub type Base = [Vec3; 3];

/// Plays an [`Action`] in game time.
///
/// The loop period is `frame_end - frame_start` frames, so an animation whose
/// last key repeats its first one loops without a stutter.
#[derive(Clone, Debug)]
pub struct ActionPlayer {
    pub action: Action,
    /// Current frame (fractional while playing).
    pub frame: f32,
    pub playing: bool,
    pub looping: bool,
    /// 1.0 = the action's fps.
    pub speed: f32,
    bases: HashMap<u64, Base>,
}

impl Default for ActionPlayer {
    /// Nothing to play (the resource `AnimPlugin` starts with).
    fn default() -> Self {
        Self::new(Action::default(), HashMap::new())
    }
}

impl ActionPlayer {
    pub fn new(action: Action, bases: HashMap<u64, Base>) -> Self {
        Self {
            frame: action.frame_start as f32,
            playing: action.autoplay,
            looping: action.looping,
            speed: 1.0,
            action,
            bases,
        }
    }

    /// Moves time on by `dt` seconds.
    pub fn advance(&mut self, dt: f32) {
        if !self.playing || !dt.is_finite() {
            return;
        }
        let (start, end) = (self.action.frame_start as f32, self.action.frame_end as f32);
        self.frame += dt * self.action.fps * self.speed;
        let period = end - start;
        if period <= 0.0 {
            self.frame = start;
        } else if self.frame > end {
            if self.looping {
                self.frame = start + (self.frame - start) % period;
            } else {
                self.frame = end;
                self.playing = false;
            }
        }
    }

    /// The local transform of `object` at the current frame, `None` if it
    /// is not animated.
    pub fn transform(&self, object: u64) -> Option<Transform> {
        if !self.action.has_object(object) {
            return None;
        }
        let mut v = self
            .bases
            .get(&object)
            .copied()
            .unwrap_or([Vec3::ZERO, Vec3::ZERO, Vec3::ONE]);
        for c in self.action.curves.iter().filter(|c| c.object == object) {
            let slot = match c.prop {
                Prop::Location => &mut v[0],
                Prop::Rotation => &mut v[1],
                Prop::Scale => &mut v[2],
            };
            if let (Some(value), Some(s)) =
                (c.eval(self.frame), slot.as_mut().get_mut(c.index as usize))
            {
                *s = value;
            }
        }
        Some(Transform {
            translation: v[0],
            rotation: Quat::from_euler(
                EulerRot::YXZ,
                v[1].y.to_radians(),
                v[1].x.to_radians(),
                v[1].z.to_radians(),
            ),
            scale: v[2],
        })
    }

    /// Writes the animated transforms into the world.
    pub fn apply(&self, world: &mut World) {
        for (id, t) in world.query_mut::<(&SceneId, &mut Transform)>() {
            if let Some(new) = self.transform(id.0) {
                *t = new;
            }
        }
    }
}

/// `AnimPlugin`'s system: advances the [`ActionPlayer`] resource, if any.
pub(crate) fn update(res: &mut Resources) {
    let dt = res.get::<Time>().map_or(0.0, |t| t.delta());
    let Some(mut player) = res.get_mut::<ActionPlayer>() else {
        return;
    };
    if player.action.curves.is_empty() {
        return;
    }
    player.advance(dt);
    player.apply(&mut res.fetch_mut::<World>());
}

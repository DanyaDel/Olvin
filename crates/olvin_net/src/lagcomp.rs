//! Lag compensation (server): where was this entity when the shooter fired?
//!
//! [`LagCompensation`] is the interface a game's hit test uses; [`PoseHistory`]
//! is the simple implementation: the last second of positions of every
//! replicated entity, recorded once per server tick. A hitscan command carries
//! the tick the shooter saw (`Net::tick` at the client minus interpolation delay)
//! and the server tests the ray against `position_at` instead of the present.

use std::collections::{HashMap, VecDeque};

use glam::Vec3;
use olvin_ecs::{Entity, World};
use olvin_scene::Transform;

use crate::replicate::NetId;

/// Answers "where was `entity` at server tick `tick`?".
pub trait LagCompensation {
    fn position_at(&self, entity: Entity, tick: u32) -> Option<Vec3>;
}

/// Ring of recent positions, a resource you insert yourself. Call
/// [`PoseHistory::record`] once per server tick (`Net::tick` changes), e.g. from a
/// system in `Stage::PostUpdate`.
pub struct PoseHistory {
    max_ticks: usize,
    frames: VecDeque<(u32, HashMap<Entity, Vec3>)>,
}

impl PoseHistory {
    /// Keeps the last `max_ticks` ticks (30 is one second at the default rate).
    pub fn new(max_ticks: usize) -> Self {
        Self {
            max_ticks: max_ticks.max(2),
            frames: VecDeque::new(),
        }
    }

    pub fn record(&mut self, tick: u32, world: &World) {
        let frame = world
            .query::<(Entity, &Transform)>()
            .with::<&NetId>()
            .iter()
            .map(|(e, t)| (e, t.translation))
            .collect();
        self.frames.push_back((tick, frame));
        while self.frames.len() > self.max_ticks {
            self.frames.pop_front();
        }
    }
}

impl LagCompensation for PoseHistory {
    /// Interpolates between the two recorded ticks around `tick`; clamps to the
    /// oldest or newest frame outside the history.
    fn position_at(&self, entity: Entity, tick: u32) -> Option<Vec3> {
        let at = |i: usize| {
            self.frames
                .get(i)
                .and_then(|(t, f)| Some((*t, *f.get(&entity)?)))
        };
        let i = self.frames.iter().position(|(t, _)| *t >= tick)?;
        let (t1, p1) = at(i)?;
        if i == 0 || t1 == tick {
            return Some(p1);
        }
        let (t0, p0) = at(i - 1)?;
        let f = (tick - t0) as f32 / (t1 - t0).max(1) as f32;
        Some(p0.lerp(p1, f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewinds_and_interpolates() {
        let mut world = World::new();
        let e = world.spawn((NetId(1), Transform::from_xyz(0.0, 0.0, 0.0)));
        let mut h = PoseHistory::new(10);
        for tick in 1..=20u32 {
            world.get::<&mut Transform>(e).unwrap().translation.x = tick as f32;
            h.record(tick, &world);
        }
        assert_eq!(h.position_at(e, 15).unwrap().x, 15.0);
        assert_eq!(h.position_at(e, 20).unwrap().x, 20.0);
        // Older than the history: the oldest frame we have.
        assert_eq!(h.position_at(e, 2).unwrap().x, 11.0);
        // Newer than anything: unknown.
        assert!(h.position_at(e, 99).is_none());
    }
}

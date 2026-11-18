//! Collision and trigger events, mapped from Rapier colliders to entities.

use olvin_ecs::Entity;
use rapier3d::prelude as rp;

use crate::Physics;

/// Two colliders started or stopped touching during a fixed step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionEvent {
    pub a: Entity,
    pub b: Entity,
    /// `true` on contact begin (Unity's `OnCollisionEnter`), `false` on end.
    pub started: bool,
    /// At least one of the two is a trigger (`OnTrigger*` instead of `OnCollision*`).
    pub sensor: bool,
}

/// Events of the current frame (all its fixed steps). Cleared at the start of
/// the next physics frame. Pairs whose collider was removed are dropped.
#[derive(Debug, Default)]
pub struct CollisionEvents {
    pub events: Vec<CollisionEvent>,
}

impl CollisionEvents {
    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = &CollisionEvent> {
        self.events.iter()
    }

    /// Events involving `entity`, with the other entity first.
    pub fn of(&self, entity: Entity) -> impl Iterator<Item = (Entity, &CollisionEvent)> {
        self.events.iter().filter_map(move |e| {
            if e.a == entity {
                Some((e.b, e))
            } else if e.b == entity {
                Some((e.a, e))
            } else {
                None
            }
        })
    }
}

/// Moves pending Rapier events into `out` (or drops them when there is no resource).
pub(crate) fn drain(physics: &Physics, mut out: Option<&mut CollisionEvents>) {
    while let Ok(event) = physics.collisions.try_recv() {
        let Some(out) = out.as_deref_mut() else {
            continue;
        };
        let (h1, h2, flags, started) = match event {
            rp::CollisionEvent::Started(a, b, f) => (a, b, f, true),
            rp::CollisionEvent::Stopped(a, b, f) => (a, b, f, false),
        };
        let (Some(a), Some(b)) = (physics.collider_entity(h1), physics.collider_entity(h2)) else {
            continue;
        };
        out.events.push(CollisionEvent {
            a,
            b,
            started,
            sensor: flags.contains(rp::CollisionEventFlags::SENSOR),
        });
    }
}

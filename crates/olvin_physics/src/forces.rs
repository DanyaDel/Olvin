//! Pushing dynamic bodies from code: forces, impulses, velocity; teleports.

use glam::Vec3;
use olvin_ecs::{Entity, World};
use olvin_transform::world_matrix;
use rapier3d::prelude as rp;

use crate::character::Character;
use crate::{Body, CharacterMotor, Kind, Physics, g, v};

impl Physics {
    /// Pushes the entity's body with a force in newtons (`ForceMode.Force`).
    /// It acts for the next fixed step only, so call it every step you want
    /// the push (e.g. from `fixed_update`). Returns false when the entity has
    /// no dynamic body (yet): kinematic, fixed and character bodies ignore it.
    pub fn apply_force(&mut self, world: &World, entity: Entity, force: Vec3) -> bool {
        let Some(handle) = dynamic_body(world, entity) else {
            return false;
        };
        let Some(rb) = self.world.bodies.get_mut(handle) else {
            return false;
        };
        rb.add_force(v(force), true);
        if !self.forced.contains(&handle) {
            self.forced.push(handle);
        }
        true
    }

    /// Instant change of momentum in newton-seconds (`ForceMode.Impulse`):
    /// velocity changes by `impulse / mass`. Same rules as [`Physics::apply_force`].
    pub fn apply_impulse(&mut self, world: &World, entity: Entity, impulse: Vec3) -> bool {
        let Some(rb) = dynamic_body(world, entity).and_then(|h| self.world.bodies.get_mut(h))
        else {
            return false;
        };
        rb.apply_impulse(v(impulse), true);
        true
    }

    /// Sets the linear velocity in m/s (`Rigidbody.velocity`). Same rules as
    /// [`Physics::apply_force`]; characters use `CharacterMotor::velocity`.
    pub fn set_velocity(&mut self, world: &World, entity: Entity, velocity: Vec3) -> bool {
        let Some(rb) = dynamic_body(world, entity).and_then(|h| self.world.bodies.get_mut(h))
        else {
            return false;
        };
        rb.set_linvel(v(velocity), true);
        true
    }

    /// Linear velocity of the entity's dynamic body.
    pub fn velocity(&self, world: &World, entity: Entity) -> Option<Vec3> {
        let handle = dynamic_body(world, entity)?;
        Some(g(self.world.bodies.get(handle)?.linvel()))
    }

    /// Angular velocity in rad/s around world axes (`Rigidbody.angularVelocity`).
    pub fn angular_velocity(&self, world: &World, entity: Entity) -> Option<Vec3> {
        let handle = dynamic_body(world, entity)?;
        Some(g(self.world.bodies.get(handle)?.angvel()))
    }

    /// Sets the angular velocity in rad/s. Same rules as [`Physics::apply_force`].
    pub fn set_angular_velocity(&mut self, world: &World, entity: Entity, w: Vec3) -> bool {
        let Some(rb) = dynamic_body(world, entity).and_then(|h| self.world.bodies.get_mut(h))
        else {
            return false;
        };
        rb.set_angvel(v(w), true);
        true
    }

    /// Puts the entity's body (any kind) at the world pose of its `Transform`
    /// right now and stops it: velocities and pending forces are cleared and
    /// the body is woken, so its contacts are rebuilt from the new pose at the
    /// next step. A character's `CharacterMotor::velocity` is zeroed too. For
    /// respawns; a plain `Transform` write is followed at the next step anyway
    /// (keeping the velocity). False when the entity has no body yet.
    pub fn teleport(&mut self, world: &World, entity: Entity) -> bool {
        let Ok(mut body) = world.get::<&mut Body>(entity) else {
            return false;
        };
        let Some(m) = world_matrix(world, entity) else {
            return false;
        };
        let Some(rb) = self.world.bodies.get_mut(body.handle) else {
            return false;
        };
        let (_, rotation, translation) = m.to_scale_rotation_translation();
        if body.kind == Kind::Character {
            // Only the position is simulated for characters.
            rb.set_translation(v(translation), true);
            if let Ok(mut c) = world.get::<&mut Character>(entity) {
                c.last = translation;
            }
            if let Ok(mut motor) = world.get::<&mut CharacterMotor>(entity) {
                motor.velocity = Vec3::ZERO;
            }
        } else {
            let pose = rp::Pose::from_parts(
                v(translation),
                rp::Rotation::from_array(rotation.normalize().to_array()),
            );
            rb.set_position(pose, true);
            if body.kind == Kind::Kinematic {
                rb.set_next_kinematic_position(pose);
            }
        }
        rb.set_linvel(rp::Vector::ZERO, true);
        rb.set_angvel(rp::Vector::ZERO, true);
        rb.reset_forces(true);
        body.seen = m;
        true
    }

    /// Forces are one-shot here (Rapier keeps them until cleared).
    pub(crate) fn clear_forces(&mut self) {
        for handle in self.forced.drain(..) {
            if let Some(rb) = self.world.bodies.get_mut(handle) {
                rb.reset_forces(false);
            }
        }
    }
}

fn dynamic_body(world: &World, entity: Entity) -> Option<rp::RigidBodyHandle> {
    let body = world.get::<&Body>(entity).ok()?;
    (body.kind == Kind::Dynamic).then_some(body.handle)
}

#[cfg(test)]
mod tests {
    use olvin_core::Resources;
    use olvin_scene::Transform;
    use olvin_scene::file::{BodyKind, ColliderDesc, ColliderShape, RigidbodyDesc};

    use crate::{FIXED_DT, step};

    use super::*;

    /// A zero-gravity world with a dynamic ball and a kinematic box (the ball's
    /// mass is read back, not assumed).
    fn setup() -> (Resources, Entity, Entity) {
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Physics::new(Vec3::ZERO));
        let (ball, wall) = {
            let mut world = res.fetch_mut::<World>();
            let ball = world.spawn((
                Transform::IDENTITY,
                ColliderDesc::new(ColliderShape::Sphere { radius: 0.5 }),
                RigidbodyDesc::default(),
            ));
            let kin = RigidbodyDesc {
                kind: BodyKind::Kinematic,
                ..Default::default()
            };
            let wall = world.spawn((
                Transform::from_xyz(10.0, 0.0, 0.0),
                ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
                kin,
            ));
            (ball, wall)
        };
        step(&res, FIXED_DT); // creates the bodies
        (res, ball, wall)
    }

    fn mass(res: &Resources, e: Entity) -> f32 {
        let world = res.fetch::<World>();
        let h = world.get::<&Body>(e).unwrap().handle;
        res.fetch::<Physics>().world.bodies.get(h).unwrap().mass()
    }

    #[test]
    fn impulse_changes_velocity_by_impulse_over_mass() {
        let (res, ball, _) = setup();
        let m = mass(&res, ball);
        let world = res.fetch::<World>();
        let mut physics = res.fetch_mut::<Physics>();
        assert!(physics.apply_impulse(&world, ball, Vec3::new(2.0 * m, 0.0, 0.0)));
        let vel = physics.velocity(&world, ball).unwrap();
        assert!((vel - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-4, "{vel}");
    }

    #[test]
    fn set_velocity_moves_the_body() {
        let (res, ball, _) = setup();
        {
            let world = res.fetch::<World>();
            let mut physics = res.fetch_mut::<Physics>();
            assert!(physics.set_velocity(&world, ball, Vec3::new(0.0, 3.0, 0.0)));
        }
        for _ in 0..60 {
            step(&res, FIXED_DT);
        }
        let y = res
            .fetch::<World>()
            .get::<&Transform>(ball)
            .unwrap()
            .translation
            .y;
        assert!(
            (y - 3.0).abs() < 0.1,
            "moved about 3 m in a second, got {y}"
        );
    }

    #[test]
    fn force_acts_for_one_step_only() {
        let (res, ball, _) = setup();
        let m = mass(&res, ball);
        {
            let world = res.fetch::<World>();
            let mut physics = res.fetch_mut::<Physics>();
            assert!(physics.apply_force(&world, ball, Vec3::new(m * 60.0, 0.0, 0.0)));
        }
        step(&res, FIXED_DT);
        let world = res.fetch::<World>();
        let after_one = res.fetch::<Physics>().velocity(&world, ball).unwrap();
        // a = F/m = 60 m/s^2 over 1/60 s -> 1 m/s.
        assert!((after_one.x - 1.0).abs() < 1e-3, "{after_one}");
        drop(world);
        for _ in 0..10 {
            step(&res, FIXED_DT);
        }
        let world = res.fetch::<World>();
        let later = res.fetch::<Physics>().velocity(&world, ball).unwrap();
        assert!(
            (later.x - 1.0).abs() < 1e-3,
            "force must not repeat: {later}"
        );
    }

    #[test]
    fn non_dynamic_or_unknown_entities_are_refused() {
        let (res, _, wall) = setup();
        let world = res.fetch::<World>();
        let mut physics = res.fetch_mut::<Physics>();
        assert!(!physics.apply_force(&world, wall, Vec3::X));
        assert!(!physics.apply_impulse(&world, wall, Vec3::X));
        assert!(!physics.set_velocity(&world, wall, Vec3::X));
        assert_eq!(physics.velocity(&world, wall), None);
        let bare = {
            drop(world);
            res.fetch_mut::<World>().spawn((Transform::IDENTITY,))
        };
        let world = res.fetch::<World>();
        assert!(!physics.apply_impulse(&world, bare, Vec3::X));
    }
}

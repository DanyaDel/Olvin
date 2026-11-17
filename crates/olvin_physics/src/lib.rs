//! Physics on Rapier. Entities with a `ColliderDesc` (and optionally a
//! `RigidbodyDesc`) get bodies automatically; dynamic bodies write their pose
//! back into `Transform`, kinematic ones follow it. Fixed 60 Hz steps.

use glam::{Quat, Vec3};
use olvin_core::{App, Plugin, Resources, Stage, Time};
use olvin_ecs::{EcsPlugin, Entity, World};
use olvin_scene::file::{BodyKind, ColliderDesc, ColliderShape, RigidbodyDesc};
use olvin_scene::{MeshGeometry, Transform};
use olvin_transform::{Parent, world_matrix};
use rapier3d::prelude as rp;

pub const FIXED_DT: f32 = 1.0 / 60.0;
const MAX_STEPS_PER_FRAME: u32 = 5;

/// Marks an entity that already has a body in the simulation.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub handle: rp::RigidBodyHandle,
    kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Dynamic,
    Kinematic,
    Fixed,
}

pub struct Physics {
    pub world: rp::PhysicsWorld,
    accumulator: f32,
}

impl Physics {
    pub fn new(gravity: Vec3) -> Self {
        let mut world = rp::PhysicsWorld::new();
        world.gravity = v(gravity);
        world.integration_parameters.dt = FIXED_DT;
        Self { world, accumulator: 0.0 }
    }

    /// Advances by `dt` seconds in fixed steps; returns how many were taken.
    pub fn advance(&mut self, dt: f32) -> u32 {
        self.accumulator = (self.accumulator + dt).min(FIXED_DT * MAX_STEPS_PER_FRAME as f32);
        let mut steps = 0;
        while self.accumulator >= FIXED_DT {
            self.world.step();
            self.accumulator -= FIXED_DT;
            steps += 1;
        }
        steps
    }
}

pub struct PhysicsPlugin {
    pub gravity: Vec3,
}

impl Default for PhysicsPlugin {
    fn default() -> Self {
        Self { gravity: Vec3::new(0.0, -9.81, 0.0) }
    }
}

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(EcsPlugin).insert_resource(Physics::new(self.gravity)).add_system(Stage::PreUpdate, |res| {
            let dt = res.fetch::<Time>().delta();
            step(res, dt);
        });
    }
}

/// One frame of physics: create new bodies, push kinematic poses, simulate, pull dynamic poses.
pub fn step(res: &Resources, dt: f32) {
    let mut physics = res.fetch_mut::<Physics>();
    let mut world = res.fetch_mut::<World>();
    create_bodies(&mut world, &mut physics);
    let kinematic: Vec<(Entity, Body)> =
        world.query::<(Entity, &Body)>().iter().filter(|(_, b)| b.kind == Kind::Kinematic).map(|(e, b)| (e, *b)).collect();
    for (e, body) in kinematic {
        if let Some(m) = world_matrix(&world, e)
            && let Some(rb) = physics.world.bodies.get_mut(body.handle)
        {
            rb.set_next_kinematic_position(pose(&decompose(m)));
        }
    }
    if physics.advance(dt) == 0 {
        return;
    }
    // Dynamic bodies own their world pose; store it relative to the parent.
    let moved: Vec<(Entity, Option<Entity>, Vec3, Quat)> = world
        .query::<(Entity, &Body, Option<&Parent>)>()
        .iter()
        .filter(|(_, b, _)| b.kind == Kind::Dynamic)
        .filter_map(|(e, b, parent)| {
            let p = physics.world.bodies.get(b.handle)?.position();
            Some((e, parent.map(|p| p.0), Vec3::from_array(p.translation.to_array()), Quat::from_array(p.rotation.to_array())))
        })
        .collect();
    for (e, parent, translation, rotation) in moved {
        let (translation, rotation) = match parent.and_then(|p| world_matrix(&world, p)) {
            Some(pm) => {
                let local = pm.inverse() * glam::Mat4::from_rotation_translation(rotation, translation);
                let (_, r, t) = local.to_scale_rotation_translation();
                (t, r)
            }
            None => (translation, rotation),
        };
        if let Ok(mut t) = world.get::<&mut Transform>(e) {
            t.translation = translation;
            t.rotation = rotation;
        }
    }
}

fn decompose(m: glam::Mat4) -> Transform {
    let (scale, rotation, translation) = m.to_scale_rotation_translation();
    Transform { translation, rotation, scale }
}

/// An entity waiting for its body: everything copied out of the ECS first.
type Pending = (Entity, Transform, Option<ColliderDesc>, Option<RigidbodyDesc>, Option<MeshGeometry>);

fn create_bodies(world: &mut World, physics: &mut Physics) {
    let pending: Vec<Pending> = world
        .query::<(Entity, &Transform, Option<&ColliderDesc>, Option<&RigidbodyDesc>, Option<&MeshGeometry>)>()
        .without::<&Body>()
        .iter()
        .filter(|(_, _, c, r, _)| c.is_some() || r.is_some())
        .map(|(e, t, c, r, m)| (e, *t, c.cloned(), r.cloned(), m.cloned()))
        .collect();
    // Bodies live in world space: use the world pose of parented objects.
    let pending: Vec<Pending> = pending
        .into_iter()
        .map(|(e, t, c, r, m)| (e, world_matrix(world, e).map_or(t, decompose), c, r, m))
        .collect();

    for (entity, transform, collider, rigidbody, mesh) in pending {
        let kind = match &rigidbody {
            Some(r) if r.kind == BodyKind::Dynamic => Kind::Dynamic,
            Some(_) => Kind::Kinematic,
            None => Kind::Fixed,
        };
        let mut builder = match kind {
            Kind::Dynamic => rp::RigidBodyBuilder::dynamic(),
            Kind::Kinematic => rp::RigidBodyBuilder::kinematic_position_based(),
            Kind::Fixed => rp::RigidBodyBuilder::fixed(),
        }
        .pose(pose(&transform));
        if let Some(r) = &rigidbody {
            let free = r.freeze_rotation.map(|f| !f);
            builder = builder
                .linear_damping(r.drag)
                .angular_damping(r.angular_drag)
                .gravity_scale(r.gravity_scale)
                .ccd_enabled(r.ccd)
                .enabled_rotations(free[0], free[1], free[2]);
        }
        let handle = physics.world.insert_body(builder);

        let shape = collider.clone().unwrap_or_else(|| ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }));
        match collider_builder(&shape, transform.scale, mesh.as_ref()) {
            Some(mut c) => {
                if let Some(r) = &rigidbody {
                    c = c.mass(r.mass);
                }
                physics.world.insert_collider(c, Some(handle));
            }
            None => log::warn!("mesh collider without geometry: body has no collider"),
        }
        let _ = world.insert_one(entity, Body { handle, kind });
    }
}

/// Rapier has no scale: bake the object's scale into the shape.
fn collider_builder(desc: &ColliderDesc, scale: Vec3, mesh: Option<&MeshGeometry>) -> Option<rp::ColliderBuilder> {
    let s = scale.abs();
    let builder = match &desc.shape {
        ColliderShape::Box { size } => {
            let h = (*size * s * 0.5).max(Vec3::splat(1e-4));
            rp::ColliderBuilder::cuboid(h.x, h.y, h.z)
        }
        ColliderShape::Sphere { radius } => rp::ColliderBuilder::ball((radius * s.max_element()).max(1e-4)),
        ColliderShape::Capsule { radius, height } => {
            let r = (radius * s.x.max(s.z)).max(1e-4);
            let half = (height * s.y * 0.5 - r).max(0.0);
            rp::ColliderBuilder::capsule_y(half, r)
        }
        ColliderShape::Mesh => {
            let mesh = mesh?;
            let vertices = mesh.positions.iter().map(|p| v(*p * scale)).collect();
            rp::ColliderBuilder::trimesh(vertices, mesh.indices.clone()).ok()?
        }
    };
    Some(
        builder
            .translation(v(desc.center * scale))
            .sensor(desc.is_trigger)
            .friction(desc.friction)
            .restitution(desc.restitution),
    )
}

fn v(p: Vec3) -> rp::Vector {
    rp::Vector::from_array(p.to_array())
}

fn pose(t: &Transform) -> rp::Pose {
    rp::Pose::from_parts(v(t.translation), rp::Rotation::from_array(t.rotation.normalize().to_array()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res_with(physics: Physics) -> Resources {
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(physics);
        res
    }

    #[test]
    fn box_falls_and_rests_on_static_ground() {
        let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
        let falling = {
            let mut world = res.fetch_mut::<World>();
            world.spawn((
                Transform::from_xyz(0.0, -0.5, 0.0).with_scale(Vec3::new(20.0, 1.0, 20.0)),
                ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
            ));
            world.spawn((
                Transform::from_xyz(0.0, 5.0, 0.0),
                ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
                RigidbodyDesc::default(),
            ))
        };
        for _ in 0..240 {
            step(&res, FIXED_DT);
        }
        let y = res.fetch::<World>().get::<&Transform>(falling).unwrap().translation.y;
        assert!((y - 0.5).abs() < 0.05, "box should rest on the ground at y = 0.5, got {y}");
    }

    #[test]
    fn mesh_collider_catches_a_ball_and_triggers_dont() {
        let res = res_with(Physics::new(Vec3::new(0.0, -9.81, 0.0)));
        let (ball, ghost) = {
            let mut world = res.fetch_mut::<World>();
            // A 10x10 floor quad as a mesh collider.
            let floor = MeshGeometry {
                positions: vec![
                    Vec3::new(-5.0, 0.0, -5.0),
                    Vec3::new(5.0, 0.0, -5.0),
                    Vec3::new(5.0, 0.0, 5.0),
                    Vec3::new(-5.0, 0.0, 5.0),
                ],
                indices: vec![[0, 2, 1], [0, 3, 2]],
            };
            world.spawn((Transform::IDENTITY, ColliderDesc::new(ColliderShape::Mesh), floor));
            let ball = world.spawn((
                Transform::from_xyz(0.0, 3.0, 0.0),
                ColliderDesc::new(ColliderShape::Sphere { radius: 0.5 }),
                RigidbodyDesc::default(),
            ));
            let mut trigger = ColliderDesc::new(ColliderShape::Box { size: Vec3::splat(4.0) });
            trigger.is_trigger = true;
            world.spawn((Transform::from_xyz(0.0, 1.0, 3.0), trigger));
            let ghost = world.spawn((
                Transform::from_xyz(0.0, 3.0, 3.0),
                ColliderDesc::new(ColliderShape::Sphere { radius: 0.25 }),
                RigidbodyDesc { gravity_scale: 1.0, ..Default::default() },
            ));
            (ball, ghost)
        };
        for _ in 0..240 {
            step(&res, FIXED_DT);
        }
        let world = res.fetch::<World>();
        let ball_y = world.get::<&Transform>(ball).unwrap().translation.y;
        assert!((ball_y - 0.5).abs() < 0.05, "ball rests on the mesh floor, got {ball_y}");
        // The trigger box doesn't stop it; the floor does.
        let ghost_y = world.get::<&Transform>(ghost).unwrap().translation.y;
        assert!((ghost_y - 0.25).abs() < 0.05, "trigger must not collide, got {ghost_y}");
    }
}

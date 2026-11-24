//! Creating rapier bodies and colliders from `RigidBodyDesc` / `ColliderDesc`.

use glam::Vec3;
use olvin_ecs::{Entity, World};
use olvin_scene::file::{
    BodyKind, CharacterControllerDesc, ColliderDesc, ColliderShape, RigidbodyDesc,
};
use olvin_scene::{MeshGeometry, Transform};
use olvin_transform::world_matrix;
use rapier3d::prelude as rp;

use super::*;

/// An entity waiting for its body: everything copied out of the ECS first.
pub(crate) struct Pending {
    entity: Entity,
    transform: Transform,
    collider: Option<ColliderDesc>,
    rigidbody: Option<RigidbodyDesc>,
    mesh: Option<MeshGeometry>,
    character: Option<CharacterControllerDesc>,
}

pub(crate) fn create_bodies(world: &mut World, physics: &mut Physics) {
    let pending: Vec<Pending> = world
        .query::<(
            Entity,
            &Transform,
            Option<&ColliderDesc>,
            Option<&RigidbodyDesc>,
            Option<&MeshGeometry>,
            Option<&CharacterControllerDesc>,
        )>()
        .without::<&Body>()
        .iter()
        .filter(|(_, _, c, r, _, cc)| c.is_some() || r.is_some() || cc.is_some())
        .map(|(entity, t, c, r, m, cc)| Pending {
            entity,
            transform: *t,
            collider: c.cloned(),
            rigidbody: r.cloned(),
            mesh: m.cloned(),
            character: cc.cloned(),
        })
        .collect();
    for mut p in pending {
        // Bodies live in world space: use the world pose of parented objects.
        let seen = world_matrix(world, p.entity).unwrap_or_else(|| p.transform.matrix());
        p.transform = decompose(seen);
        let data = entity_bits(p.entity);
        if let Some(desc) = &p.character {
            if p.rigidbody.is_some() {
                log::warn!("character controller ignores the entity's Rigidbody");
            }
            let (body, character) = character::create(physics, desc, &p.transform, data);
            let _ = world.insert(
                p.entity,
                (
                    Body {
                        handle: body,
                        kind: Kind::Character,
                        seen,
                    },
                    character,
                ),
            );
            if world.get::<&CharacterMotor>(p.entity).is_err() {
                let motor = CharacterMotor {
                    push_mass: desc.push_mass,
                    ..Default::default()
                };
                let _ = world.insert_one(p.entity, motor);
            }
            continue;
        }

        let kind = match &p.rigidbody {
            Some(r) if r.kind == BodyKind::Dynamic => Kind::Dynamic,
            Some(_) => Kind::Kinematic,
            None => Kind::Fixed,
        };
        let mut builder = match kind {
            Kind::Dynamic => rp::RigidBodyBuilder::dynamic(),
            Kind::Kinematic | Kind::Character => rp::RigidBodyBuilder::kinematic_position_based(),
            Kind::Fixed => rp::RigidBodyBuilder::fixed(),
        }
        .pose(pose(&p.transform))
        .user_data(data);
        if let Some(r) = &p.rigidbody {
            let free = r.freeze_rotation.map(|f| !f);
            builder = builder
                .linear_damping(r.drag)
                .angular_damping(r.angular_drag)
                .gravity_scale(r.gravity_scale)
                .ccd_enabled(r.ccd)
                .enabled_rotations(free[0], free[1], free[2]);
        }
        let handle = physics.world.insert_body(builder);

        let shape = p
            .collider
            .clone()
            .unwrap_or_else(|| ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }));
        match collider_builder(
            &shape,
            p.transform.scale,
            p.mesh.as_ref(),
            kind == Kind::Dynamic,
        ) {
            Some(mut c) => {
                if let Some(r) = &p.rigidbody {
                    c = c.mass(r.mass);
                }
                physics
                    .world
                    .insert_collider(c.user_data(data), Some(handle));
            }
            None => log::warn!("mesh collider without geometry: body has no collider"),
        }
        let _ = world.insert_one(p.entity, Body { handle, kind, seen });
    }
}

/// Rapier has no scale: bake the object's scale into the shape.
/// Moving bodies get the convex hull of mesh colliders: Rapier (like PhysX)
/// doesn't simulate concave triangle meshes as dynamic bodies.
pub(crate) fn collider_builder(
    desc: &ColliderDesc,
    scale: Vec3,
    mesh: Option<&MeshGeometry>,
    dynamic: bool,
) -> Option<rp::ColliderBuilder> {
    let s = scale.abs();
    let builder = match &desc.shape {
        ColliderShape::Box { size } => {
            let h = (*size * s * 0.5).max(Vec3::splat(1e-4));
            rp::ColliderBuilder::cuboid(h.x, h.y, h.z)
        }
        ColliderShape::Sphere { radius } => {
            rp::ColliderBuilder::ball((radius * s.max_element()).max(1e-4))
        }
        ColliderShape::Capsule { radius, height } => {
            let r = (radius * s.x.max(s.z)).max(1e-4);
            let half = (height * s.y * 0.5 - r).max(0.0);
            rp::ColliderBuilder::capsule_y(half, r)
        }
        ColliderShape::Mesh => {
            let mesh = mesh?;
            // Parry indexes the vertices unchecked and panics on a stray index.
            let n = mesh.positions.len() as u32;
            if mesh.indices.iter().flatten().any(|&i| i >= n) {
                log::warn!("mesh collider has indices past its vertices: ignored");
                return None;
            }
            let vertices: Vec<rp::Vector> = mesh.positions.iter().map(|p| v(*p * scale)).collect();
            if dynamic {
                rp::ColliderBuilder::convex_hull(&vertices)?
            } else {
                rp::ColliderBuilder::trimesh(vertices, mesh.indices.clone()).ok()?
            }
        }
    };
    let mut builder = builder
        .translation(v(desc.center * scale))
        .sensor(desc.is_trigger)
        .friction(desc.friction)
        .restitution(desc.restitution)
        .collision_groups(rp::InteractionGroups::new(
            rp::Group::from_bits_truncate(desc.layer_bit()),
            rp::Group::from_bits_truncate(desc.mask),
            rp::InteractionTestMode::And,
        ))
        .active_events(rp::ActiveEvents::COLLISION_EVENTS);
    if desc.is_trigger {
        // Triggers also fire for kinematic bodies and characters, like in
        // Unity, but not static-vs-static (a trigger touching the floor).
        builder = builder.active_collision_types(
            rp::ActiveCollisionTypes::default()
                | rp::ActiveCollisionTypes::KINEMATIC_FIXED
                | rp::ActiveCollisionTypes::KINEMATIC_KINEMATIC,
        );
    }
    Some(builder)
}

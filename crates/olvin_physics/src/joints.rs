//! Joints: `JointDesc` components become Rapier impulse joints.
//!
//! The joint is made when both bodies exist, in the pose they have then:
//! the frames are set so that the two bodies stay as they were placed (a hinge
//! door hangs where it is), and the limits are relative to that placement.
//! A joint to the world fixes a point in space where the anchor is now.

use glam::{Quat, Vec3};
use olvin_ecs::{Entity, World};
use olvin_scene::SceneId;
use olvin_scene::file::{JointDesc, JointKind};
use olvin_transform::world_matrix;
use rapier3d::prelude as rp;

use crate::{Body, Physics, g, v};

/// Marks a `JointDesc` that was handled. `handle` is `None` when the joint
/// could not be made (the other object is missing), so it is not retried every step.
#[derive(Clone, Copy, Debug)]
pub struct JointLink {
    pub handle: Option<rp::ImpulseJointHandle>,
}

fn quat(r: rp::Rotation) -> Quat {
    Quat::from_array(r.to_array())
}

fn rotation(q: Quat) -> rp::Rotation {
    rp::Rotation::from_array(q.normalize().to_array())
}

/// Position, rotation and scale a body is in right now.
struct Placement {
    body: rp::RigidBodyHandle,
    position: Vec3,
    rotation: Quat,
    scale: Vec3,
}

fn placement(world: &World, physics: &Physics, e: Entity) -> Option<Placement> {
    let body = world.get::<&Body>(e).ok()?.handle;
    let pose = physics.world.bodies.get(body)?.position();
    let scale = world_matrix(world, e)
        .map(|m| m.to_scale_rotation_translation().0)
        .unwrap_or(Vec3::ONE);
    Some(Placement {
        body,
        position: g(pose.translation),
        rotation: quat(pose.rotation),
        scale,
    })
}

fn builder(desc: &JointDesc, a1: Vec3, a2: Vec3, kind_axis: Vec3) -> rp::GenericJoint {
    let (a1, a2) = (v(a1), v(a2));
    let contacts = desc.collide_connected;
    let axis = v(kind_axis);
    match desc.kind {
        JointKind::Fixed => rp::FixedJointBuilder::new()
            .local_anchor1(a1)
            .local_anchor2(a2)
            .contacts_enabled(contacts)
            .into(),
        JointKind::Hinge { limits, .. } => {
            let mut b = rp::RevoluteJointBuilder::new(axis)
                .local_anchor1(a1)
                .local_anchor2(a2)
                .contacts_enabled(contacts);
            if let Some([lo, hi]) = limits {
                // Rapier measures the second body against the first: the sign is
                // the other way round from this object's own turn.
                b = b.limits([(-hi).to_radians(), (-lo).to_radians()]);
            }
            b.into()
        }
        JointKind::Ball => rp::SphericalJointBuilder::new()
            .local_anchor1(a1)
            .local_anchor2(a2)
            .contacts_enabled(contacts)
            .into(),
        JointKind::Slider { limits, .. } => {
            let mut b = rp::PrismaticJointBuilder::new(axis)
                .local_anchor1(a1)
                .local_anchor2(a2)
                .contacts_enabled(contacts);
            if let Some([lo, hi]) = limits {
                b = b.limits([-hi, -lo]);
            }
            b.into()
        }
        JointKind::Spring {
            rest_length,
            stiffness,
            damping,
        } => {
            rp::SpringJointBuilder::new(rest_length.max(0.0), stiffness.max(0.0), damping.max(0.0))
                .local_anchor1(a1)
                .local_anchor2(a2)
                .contacts_enabled(contacts)
                .into()
        }
    }
}

/// Makes the joints of entities that have a `JointDesc` and a body but no `JointLink`.
pub(crate) fn create_joints(world: &mut World, physics: &mut Physics) {
    let pending: Vec<(Entity, JointDesc)> = world
        .query::<(Entity, &JointDesc, &Body)>()
        .without::<&JointLink>()
        .iter()
        .map(|(e, d, _)| (e, d.clone()))
        .collect();
    for (e, desc) in pending {
        let handle = make(world, physics, e, &desc);
        let _ = world.insert_one(e, JointLink { handle });
    }
}

fn make(
    world: &World,
    physics: &mut Physics,
    e: Entity,
    desc: &JointDesc,
) -> Option<rp::ImpulseJointHandle> {
    let p1 = placement(world, physics, e)?;
    let anchor1 = desc.anchor * p1.scale;
    let (body2, p2_position, p2_rotation, anchor2) = match desc.connected {
        Some(id) => {
            let other = world
                .query::<(Entity, &SceneId)>()
                .iter()
                .find(|(_, s)| s.0 == id)
                .map(|(o, _)| o);
            let Some(other) = other.filter(|o| *o != e) else {
                log::warn!("joint of {e:?}: no other object with id {id}");
                return None;
            };
            let Some(p2) = placement(world, physics, other) else {
                log::warn!("joint of {e:?}: object {id} has no physics body");
                return None;
            };
            (
                p2.body,
                p2.position,
                p2.rotation,
                desc.connected_anchor * p2.scale,
            )
        }
        None => {
            // Fixed to the world at the anchor's present place.
            let body = physics.world_anchor();
            (
                body,
                Vec3::ZERO,
                Quat::IDENTITY,
                p1.position + p1.rotation * anchor1,
            )
        }
    };
    let _ = p2_position;
    let axis = match desc.kind {
        JointKind::Hinge { axis, .. } | JointKind::Slider { axis, .. } => {
            axis.try_normalize().unwrap_or(Vec3::Y)
        }
        _ => Vec3::Y,
    };
    let mut joint = builder(desc, anchor1, anchor2, axis);
    // Make the frames coincide in the world where the bodies are now: the
    // second one is turned by the bodies' relative rotation.
    let f1 = quat(joint.local_frame1.rotation);
    joint.local_frame2.rotation = rotation(p2_rotation.inverse() * p1.rotation * f1);
    Some(
        physics
            .world
            .impulse_joints
            .insert(p1.body, body2, joint, true),
    )
}

/// Takes the joint out of the simulation when its `JointDesc` was removed.
pub(crate) fn remove_joints(world: &mut World, physics: &mut Physics) {
    let gone: Vec<(Entity, Option<rp::ImpulseJointHandle>)> = world
        .query::<(Entity, &JointLink)>()
        .without::<&JointDesc>()
        .iter()
        .map(|(e, l)| (e, l.handle))
        .collect();
    for (e, handle) in gone {
        if let Some(h) = handle {
            physics.world.impulse_joints.remove(h, true);
        }
        let _ = world.remove_one::<JointLink>(e);
    }
}

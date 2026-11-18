//! Scene queries: ray and sphere casts against every collider.

use glam::Vec3;
use olvin_ecs::Entity;
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude as rp;

use crate::{Physics, g, v};

/// What a cast hit. `distance` is along the (normalised) direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub entity: Entity,
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

impl Physics {
    /// First solid collider along the ray (triggers are ignored), like
    /// `Physics.Raycast`. Sees bodies as of the last physics step.
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<Hit> {
        self.raycast_filtered(origin, direction, max_distance, None, false)
    }

    /// [`Physics::raycast`] that can skip one entity (e.g. the caster) and hit triggers.
    pub fn raycast_filtered(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        exclude: Option<Entity>,
        hit_triggers: bool,
    ) -> Option<Hit> {
        let dir = direction.try_normalize()?;
        let ray = rp::Ray::new(v(origin), v(dir));
        let filter = self.filter(exclude, hit_triggers);
        let (handle, hit) = self
            .world
            .cast_ray_and_get_normal(&ray, max_distance, true, filter)?;
        Some(Hit {
            entity: self.collider_entity(handle)?,
            point: origin + dir * hit.time_of_impact,
            normal: g(hit.normal),
            distance: hit.time_of_impact,
        })
    }

    /// Sweeps a sphere along the ray (`Physics.SphereCast`); triggers are ignored.
    pub fn spherecast(
        &self,
        origin: Vec3,
        radius: f32,
        direction: Vec3,
        max_distance: f32,
    ) -> Option<Hit> {
        self.spherecast_filtered(origin, radius, direction, max_distance, None)
    }

    pub fn spherecast_filtered(
        &self,
        origin: Vec3,
        radius: f32,
        direction: Vec3,
        max_distance: f32,
        exclude: Option<Entity>,
    ) -> Option<Hit> {
        let dir = direction.try_normalize()?;
        let ball = rp::Ball::new(radius.max(1e-4));
        let (handle, hit) = self.world.cast_shape(
            &rp::Pose::from_translation(v(origin)),
            v(dir),
            &ball,
            ShapeCastOptions::with_max_time_of_impact(max_distance),
            self.filter(exclude, false),
        )?;
        Some(Hit {
            entity: self.collider_entity(handle)?,
            point: g(hit.witness1),
            normal: g(hit.normal1),
            distance: hit.time_of_impact,
        })
    }

    /// Every entity whose collider overlaps the sphere (`Physics.OverlapSphere`), triggers included.
    pub fn overlap_sphere(&self, center: Vec3, radius: f32) -> Vec<Entity> {
        let ball = rp::Ball::new(radius.max(1e-4));
        let mut out: Vec<Entity> = self
            .world
            .intersect_shape(
                rp::Pose::from_translation(v(center)),
                &ball,
                rp::QueryFilter::default(),
            )
            .filter_map(|(h, _)| self.collider_entity(h))
            .collect();
        out.dedup();
        out
    }

    fn filter(&self, exclude: Option<Entity>, hit_triggers: bool) -> rp::QueryFilter<'_> {
        let mut filter = rp::QueryFilter::default();
        if !hit_triggers {
            filter = filter.exclude_sensors();
        }
        if let Some(e) = exclude
            && let Some((h, _)) = self
                .world
                .bodies
                .iter()
                .find(|(_, b)| crate::entity_from(b.user_data) == Some(e))
        {
            filter = filter.exclude_rigid_body(h);
        }
        filter
    }
}

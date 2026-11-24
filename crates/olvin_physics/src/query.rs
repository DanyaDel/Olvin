//! Scene queries: ray and sphere casts against every collider.

use glam::{Quat, Vec3};
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

/// What a query may hit. `QueryOptions::default()` is the plain query: every
/// layer, solid colliders only, nothing excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryOptions {
    /// Layers to hit: bit `n` is layer `n` (see `ColliderDesc::layer`). Only
    /// the layer of the collider counts, not its own mask (like Unity).
    pub mask: u32,
    /// Skip this entity (the caster).
    pub exclude: Option<Entity>,
    pub hit_triggers: bool,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            mask: u32::MAX,
            exclude: None,
            hit_triggers: false,
        }
    }
}

impl QueryOptions {
    pub fn mask(mut self, mask: u32) -> Self {
        self.mask = mask;
        self
    }

    pub fn exclude(mut self, entity: Entity) -> Self {
        self.exclude = Some(entity);
        self
    }

    pub fn triggers(mut self) -> Self {
        self.hit_triggers = true;
        self
    }
}

/// A shape to sweep or overlap, centred at the query's position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CastShape {
    Sphere {
        radius: f32,
    },
    /// Half the size along each of the box's own axes.
    Box {
        half_extents: Vec3,
    },
    /// Along the shape's own Y; `half_height` is the straight part's half length.
    Capsule {
        radius: f32,
        half_height: f32,
    },
}

impl CastShape {
    fn shared(&self) -> rp::SharedShape {
        let tiny = 1e-4;
        match *self {
            Self::Sphere { radius } => rp::SharedShape::ball(radius.max(tiny)),
            Self::Box { half_extents } => {
                let h = half_extents.abs().max(Vec3::splat(tiny));
                rp::SharedShape::cuboid(h.x, h.y, h.z)
            }
            Self::Capsule {
                radius,
                half_height,
            } => rp::SharedShape::capsule_y(half_height.max(0.0), radius.max(tiny)),
        }
    }
}

impl Physics {
    fn filter_with<'a>(
        &'a self,
        options: &QueryOptions,
        layers: &'a dyn Fn(rp::ColliderHandle, &rp::Collider) -> bool,
    ) -> rp::QueryFilter<'a> {
        let mut filter = self.filter(options.exclude, options.hit_triggers);
        if options.mask != u32::MAX {
            filter.predicate = Some(layers);
        }
        filter
    }

    /// [`Physics::raycast`] with options.
    pub fn raycast_with(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        options: &QueryOptions,
    ) -> Option<Hit> {
        let dir = direction.try_normalize()?;
        let ray = rp::Ray::new(v(origin), v(dir));
        let mask = options.mask;
        let layers = move |_: rp::ColliderHandle, c: &rp::Collider| {
            c.collision_groups().memberships.bits() & mask != 0
        };
        let (handle, hit) = self.world.cast_ray_and_get_normal(
            &ray,
            max_distance,
            true,
            self.filter_with(options, &layers),
        )?;
        Some(Hit {
            entity: self.collider_entity(handle)?,
            point: origin + dir * hit.time_of_impact,
            normal: g(hit.normal),
            distance: hit.time_of_impact,
        })
    }

    /// Sweeps `shape` from `position` (rotated by `rotation`) along `direction`
    /// and returns the first thing it touches: a sphere, box or capsule cast.
    /// A shape that starts inside something reports distance 0.
    pub fn shapecast(
        &self,
        shape: CastShape,
        position: Vec3,
        rotation: Quat,
        direction: Vec3,
        max_distance: f32,
        options: &QueryOptions,
    ) -> Option<Hit> {
        let dir = direction.try_normalize()?;
        let shape = shape.shared();
        let pose = rp::Pose::from_parts(
            v(position),
            rp::Rotation::from_array(rotation.normalize().to_array()),
        );
        let mask = options.mask;
        let layers = move |_: rp::ColliderHandle, c: &rp::Collider| {
            c.collision_groups().memberships.bits() & mask != 0
        };
        let (handle, hit) = self.world.cast_shape(
            &pose,
            v(dir),
            &*shape,
            ShapeCastOptions::with_max_time_of_impact(max_distance),
            self.filter_with(options, &layers),
        )?;
        Some(Hit {
            entity: self.collider_entity(handle)?,
            point: g(hit.witness1),
            normal: g(hit.normal1),
            distance: hit.time_of_impact,
        })
    }

    /// Every entity whose collider overlaps `shape` at `position` / `rotation`.
    pub fn overlap(
        &self,
        shape: CastShape,
        position: Vec3,
        rotation: Quat,
        options: &QueryOptions,
    ) -> Vec<Entity> {
        let shape = shape.shared();
        let pose = rp::Pose::from_parts(
            v(position),
            rp::Rotation::from_array(rotation.normalize().to_array()),
        );
        let mask = options.mask;
        let layers = move |_: rp::ColliderHandle, c: &rp::Collider| {
            c.collision_groups().memberships.bits() & mask != 0
        };
        let mut out: Vec<Entity> = self
            .world
            .intersect_shape(pose, &*shape, self.filter_with(options, &layers))
            .filter_map(|(h, _)| self.collider_entity(h))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

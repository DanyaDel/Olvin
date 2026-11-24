//! The engine side: bake from a scene's colliders, move agents along paths.

use glam::{Mat4, Vec3};
use olvin_core::{App, Plugin, Resources, Stage, Time};
use olvin_ecs::{Entity, World};
use olvin_scene::file::{ColliderDesc, ColliderShape, RigidbodyDesc};
use olvin_scene::{MeshGeometry, Transform};
use olvin_transform::world_matrix;

use crate::{BakeError, NavConfig, NavGrid, Triangle};

/// Most paths one frame may plan; the other agents wait for the next frame.
const PATHS_PER_FRAME: usize = 8;
/// A moving target is followed by re-planning at most this often, seconds.
const REPLAN_EVERY: f32 = 0.5;
/// How far from walkable ground an agent or a target may be and still be snapped to it.
const SNAP: f32 = 1.5;

/// Resource: the baked grid of the level.
#[derive(Default)]
pub struct Navigation {
    pub grid: Option<NavGrid>,
}

impl Navigation {
    /// Bakes the static colliders of `world` (see [`collect_triangles`]).
    pub fn bake(&mut self, world: &World, config: NavConfig) -> Result<(), BakeError> {
        self.grid = Some(NavGrid::bake(&collect_triangles(world), config)?);
        Ok(())
    }
}

/// The triangles of every static, solid collider: boxes and mesh colliders
/// without a rigidbody. Triggers, spheres and capsules are not floors or walls.
pub fn collect_triangles(world: &World) -> Vec<Triangle> {
    let mut out = Vec::new();
    for (entity, collider, rigidbody) in world
        .query::<(Entity, &ColliderDesc, Option<&RigidbodyDesc>)>()
        .iter()
    {
        if collider.is_trigger || rigidbody.is_some() {
            continue;
        }
        let Some(matrix) = world_matrix(world, entity) else {
            continue;
        };
        match &collider.shape {
            ColliderShape::Box { size } => {
                let m = matrix * Mat4::from_translation(collider.center);
                box_triangles(m, *size, &mut out);
            }
            ColliderShape::Mesh => {
                let Ok(mesh) = world.get::<&MeshGeometry>(entity) else {
                    continue;
                };
                for tri in &mesh.indices {
                    let at = |i: u32| {
                        mesh.positions
                            .get(i as usize)
                            .map(|p| matrix.transform_point3(*p))
                    };
                    if let (Some(a), Some(b), Some(c)) = (at(tri[0]), at(tri[1]), at(tri[2])) {
                        out.push([a, b, c]);
                    }
                }
            }
            ColliderShape::Sphere { .. } | ColliderShape::Capsule { .. } => {}
        }
    }
    out
}

fn box_triangles(m: Mat4, size: Vec3, out: &mut Vec<Triangle>) {
    let h = size * 0.5;
    let corner = |i: usize| {
        m.transform_point3(Vec3::new(
            if i & 1 == 0 { -h.x } else { h.x },
            if i & 2 == 0 { -h.y } else { h.y },
            if i & 4 == 0 { -h.z } else { h.z },
        ))
    };
    // Each face as four corner indices in a ring.
    const FACES: [[usize; 4]; 6] = [
        [0, 1, 3, 2],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 3, 7, 5],
    ];
    for f in FACES {
        out.push([corner(f[0]), corner(f[1]), corner(f[2])]);
        out.push([corner(f[0]), corner(f[2]), corner(f[3])]);
    }
}

/// Where an agent is in its errand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NavStatus {
    /// No target.
    #[default]
    Idle,
    /// Following a path.
    Moving,
    /// Within `stop_distance` of the target.
    Arrived,
    /// The target cannot be reached (or there is no grid yet).
    NoPath,
}

/// Component: walks its entity to `target` along baked ground.
#[derive(Clone, Debug)]
pub struct NavAgent {
    /// Metres per second.
    pub speed: f32,
    /// Counts as arrived this close (flat distance) to the target.
    pub stop_distance: f32,
    /// Set it to send the agent somewhere; `None` stops it.
    pub target: Option<Vec3>,
    /// Move the entity's `Transform` directly. Off: only `desired_velocity`
    /// is written, for a character controller or a rigidbody to follow.
    pub drive: bool,
    pub status: NavStatus,
    /// Where the agent wants to go this frame (metres per second).
    pub desired_velocity: Vec3,
    path: Vec<Vec3>,
    next: usize,
    planned_for: Option<Vec3>,
    replan_in: f32,
}

impl Default for NavAgent {
    fn default() -> Self {
        Self {
            speed: 3.0,
            stop_distance: 0.2,
            target: None,
            drive: true,
            status: NavStatus::Idle,
            desired_velocity: Vec3::ZERO,
            path: Vec::new(),
            next: 0,
            planned_for: None,
            replan_in: 0.0,
        }
    }
}

impl NavAgent {
    pub fn new(speed: f32) -> Self {
        Self {
            speed,
            ..Self::default()
        }
    }

    pub fn go_to(&mut self, target: Vec3) {
        self.target = Some(target);
    }

    pub fn stop(&mut self) {
        self.target = None;
    }

    /// The points still ahead, for drawing the route.
    pub fn path(&self) -> &[Vec3] {
        &self.path[self.next.min(self.path.len())..]
    }
}

fn steer(res: &mut Resources) {
    let dt = res.fetch::<Time>().delta();
    let nav = res.fetch::<Navigation>();
    let mut world = res.fetch_mut::<World>();
    let mut planned = 0;
    for (agent, transform) in world.query_mut::<(&mut NavAgent, &mut Transform)>() {
        agent.desired_velocity = Vec3::ZERO;
        let Some(target) = agent.target else {
            agent.status = NavStatus::Idle;
            agent.path.clear();
            agent.planned_for = None;
            continue;
        };
        let pos = transform.translation;
        let flat = |a: Vec3, b: Vec3| Vec3::new(a.x - b.x, 0.0, a.z - b.z).length();
        if flat(pos, target) <= agent.stop_distance {
            agent.status = NavStatus::Arrived;
            agent.path.clear();
            agent.planned_for = None;
            continue;
        }
        agent.replan_in -= dt;
        let stale = agent.planned_for != Some(target)
            || (agent.replan_in <= 0.0 && agent.status == NavStatus::Moving);
        if stale && (planned < PATHS_PER_FRAME || agent.path.is_empty()) {
            planned += 1;
            agent.replan_in = REPLAN_EVERY;
            agent.planned_for = Some(target);
            match nav
                .grid
                .as_ref()
                .and_then(|g| g.find_path(pos, target, SNAP))
            {
                Some(path) => {
                    agent.path = path;
                    agent.next = 1.min(agent.path.len() - 1);
                    agent.status = NavStatus::Moving;
                }
                None => {
                    agent.path.clear();
                    agent.status = NavStatus::NoPath;
                }
            }
        }
        if agent.status != NavStatus::Moving {
            continue;
        }
        while agent.next + 1 < agent.path.len() && flat(pos, agent.path[agent.next]) < 0.15 {
            agent.next += 1;
        }
        let Some(&aim) = agent.path.get(agent.next) else {
            continue;
        };
        let to = aim - pos;
        let dir = if agent.drive {
            to
        } else {
            Vec3::new(to.x, 0.0, to.z)
        };
        let velocity = dir.normalize_or_zero() * agent.speed;
        agent.desired_velocity = velocity;
        if agent.drive {
            // Never step past the point being aimed at.
            let step = (velocity * dt).clamp_length_max(to.length());
            transform.translation += step;
        }
    }
}

/// Adds [`Navigation`] and moves every [`NavAgent`] in `Stage::Update`.
pub struct NavPlugin {
    /// Bake the grid from the scene's colliders on the first frame that has any.
    pub bake: Option<NavConfig>,
}

impl NavPlugin {
    /// No automatic bake: call [`Navigation::bake`] when the level is loaded.
    pub fn new() -> Self {
        Self { bake: None }
    }

    /// Bake from the colliders as soon as the scene has some.
    pub fn auto_bake(mut self, config: NavConfig) -> Self {
        self.bake = Some(config);
        self
    }
}

impl Default for NavPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for NavPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Navigation>();
        app.init_resource::<World>();
        if let Some(config) = self.bake {
            let mut done = false;
            app.add_system(Stage::PreUpdate, move |res| {
                if done {
                    return;
                }
                let world = res.fetch::<World>();
                if world.query::<&ColliderDesc>().iter().next().is_none() {
                    return;
                }
                match res.fetch_mut::<Navigation>().bake(&world, config) {
                    Ok(()) => log::info!("navigation baked"),
                    Err(e) => log::warn!("navigation not baked: {e}"),
                }
                done = true;
            });
        }
        app.add_system(Stage::Update, steer);
    }
}

#[cfg(test)]
mod tests {
    use olvin_scene::file::ColliderShape;

    use super::*;

    fn box_at(world: &mut World, centre: Vec3, size: Vec3) {
        world.spawn((
            Transform::from_translation(centre),
            ColliderDesc::new(ColliderShape::Box { size }),
        ));
    }

    fn level() -> App {
        let mut app = App::new();
        app.insert_resource(Time::with_fixed_delta(0.1));
        app.add_plugin(NavPlugin::new().auto_bake(NavConfig {
            agent_radius: 0.3,
            ..NavConfig::default()
        }));
        let mut world = app.resources.fetch_mut::<World>();
        // A 20 x 20 floor (top at y = 0) and a wall across it with a gap at x > 6.
        box_at(
            &mut world,
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(20.0, 1.0, 20.0),
        );
        box_at(
            &mut world,
            Vec3::new(-2.0, 1.0, 0.0),
            Vec3::new(16.0, 2.0, 0.5),
        );
        drop(world);
        app
    }

    fn spawn_agent(app: &App, from: Vec3, to: Vec3) -> Entity {
        let mut agent = NavAgent::new(4.0);
        agent.go_to(to);
        app.resources
            .fetch_mut::<World>()
            .spawn((Transform::from_translation(from), agent))
    }

    #[test]
    fn colliders_are_baked_and_an_agent_walks_around_the_wall() {
        let mut app = level();
        let e = spawn_agent(&app, Vec3::new(-6.0, 0.0, -5.0), Vec3::new(-6.0, 0.0, 5.0));
        for _ in 0..300 {
            app.update();
        }
        let world = app.resources.fetch::<World>();
        let agent = world.get::<&NavAgent>(e).unwrap();
        let at = world.get::<&Transform>(e).unwrap().translation;
        assert_eq!(agent.status, NavStatus::Arrived, "stopped at {at}");
        assert!(at.distance(Vec3::new(-6.0, 0.0, 5.0)) < 0.5, "{at}");
    }

    #[test]
    fn an_unreachable_target_reports_no_path_and_the_agent_stays() {
        let mut app = level();
        let from = Vec3::new(-6.0, 0.0, -5.0);
        let e = spawn_agent(&app, from, Vec3::new(500.0, 0.0, 500.0));
        for _ in 0..5 {
            app.update();
        }
        let world = app.resources.fetch::<World>();
        assert_eq!(world.get::<&NavAgent>(e).unwrap().status, NavStatus::NoPath);
        assert_eq!(world.get::<&Transform>(e).unwrap().translation, from);
    }

    #[test]
    fn without_drive_only_the_desired_velocity_is_written() {
        let mut app = level();
        let from = Vec3::new(-6.0, 0.0, -5.0);
        let e = spawn_agent(&app, from, Vec3::new(-6.0, 0.0, -2.0));
        app.resources
            .fetch_mut::<World>()
            .get::<&mut NavAgent>(e)
            .unwrap()
            .drive = false;
        for _ in 0..3 {
            app.update();
        }
        let world = app.resources.fetch::<World>();
        let v = world.get::<&NavAgent>(e).unwrap().desired_velocity;
        assert!(v.z > 3.9 && v.y == 0.0, "{v}");
        assert_eq!(world.get::<&Transform>(e).unwrap().translation, from);
    }
}

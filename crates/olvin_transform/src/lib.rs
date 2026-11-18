//! Transform hierarchy. `Transform` is local to the `Parent`; `GlobalTransform`
//! is the world matrix, recomputed every frame in `Stage::PostUpdate` so
//! rendering (and next frame's physics) see what gameplay did this frame.

use std::collections::HashMap;

use glam::Mat4;
use olvin_core::{App, Plugin, Stage};
use olvin_ecs::{EcsPlugin, Entity, World};
use olvin_scene::Transform;

/// The entity this one's `Transform` is relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parent(pub Entity);

/// World matrix. Read it; write `Transform`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlobalTransform(pub Mat4);

impl Default for GlobalTransform {
    fn default() -> Self {
        Self(Mat4::IDENTITY)
    }
}

impl GlobalTransform {
    /// World pose without scale (for cameras, physics).
    pub fn to_transform(&self) -> Transform {
        let (scale, rotation, translation) = self.0.to_scale_rotation_translation();
        Transform {
            translation,
            rotation,
            scale,
        }
    }
}

pub struct TransformPlugin;

impl Plugin for TransformPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(EcsPlugin)
            .add_system(Stage::PostUpdate, |res| {
                propagate(&mut res.fetch_mut::<World>())
            });
    }
}

/// World matrix of an entity right now, whether or not propagation ran yet.
pub fn world_matrix(world: &World, entity: Entity) -> Option<Mat4> {
    let mut m = world.get::<&Transform>(entity).ok()?.matrix();
    let mut current = world.get::<&Parent>(entity).ok().map(|p| p.0);
    for _ in 0..256 {
        let Some(p) = current else { break };
        m = world
            .get::<&Transform>(p)
            .map(|t| t.matrix())
            .unwrap_or(Mat4::IDENTITY)
            * m;
        current = world.get::<&Parent>(p).ok().map(|p| p.0);
    }
    Some(m)
}

/// Recomputes every `GlobalTransform`, adding missing ones.
pub fn propagate(world: &mut World) {
    let missing: Vec<Entity> = world
        .query::<(Entity, &Transform)>()
        .without::<&GlobalTransform>()
        .iter()
        .map(|(e, _)| e)
        .collect();
    for e in missing {
        let _ = world.insert_one(e, GlobalTransform::default());
    }

    // Roots: a straight copy, the common case for large flat scenes.
    for (t, g) in world
        .query_mut::<(&Transform, &mut GlobalTransform)>()
        .without::<&Parent>()
    {
        g.0 = t.matrix();
    }

    let children: Vec<(Entity, Entity, Mat4)> = world
        .query::<(Entity, &Parent, &Transform)>()
        .iter()
        .map(|(e, p, t)| (e, p.0, t.matrix()))
        .collect();
    if children.is_empty() {
        return;
    }
    let local: HashMap<Entity, (Entity, Mat4)> =
        children.iter().map(|(e, p, m)| (*e, (*p, *m))).collect();
    let mut done: HashMap<Entity, Mat4> = HashMap::with_capacity(children.len());
    for (e, _, _) in &children {
        resolve(*e, world, &local, &mut done, 0);
    }
    for (e, m) in done {
        if let Ok(mut g) = world.get::<&mut GlobalTransform>(e) {
            g.0 = m;
        }
    }
}

fn resolve(
    e: Entity,
    world: &World,
    local: &HashMap<Entity, (Entity, Mat4)>,
    done: &mut HashMap<Entity, Mat4>,
    depth: u32,
) -> Mat4 {
    if let Some(m) = done.get(&e) {
        return *m;
    }
    let Some(&(parent, m)) = local.get(&e) else {
        // A root (already copied) or a dangling parent.
        return world
            .get::<&GlobalTransform>(e)
            .map(|g| g.0)
            .unwrap_or(Mat4::IDENTITY);
    };
    // A parent cycle is a bug in game code; cut it rather than overflow.
    let parent_world = if depth > 256 {
        Mat4::IDENTITY
    } else {
        resolve(parent, world, local, done, depth + 1)
    };
    let global = parent_world * m;
    done.insert(e, global);
    global
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    #[test]
    fn children_follow_parents_through_levels() {
        let mut world = World::new();
        let root = world.spawn((Transform::from_xyz(10.0, 0.0, 0.0)
            .with_rotation(Quat::from_rotation_y(90f32.to_radians())),));
        let child = world.spawn((Transform::from_xyz(0.0, 0.0, -2.0), Parent(root)));
        let grandchild = world.spawn((
            Transform::from_xyz(0.0, 1.0, 0.0).with_scale(Vec3::splat(2.0)),
            Parent(child),
        ));
        propagate(&mut world);

        let at = |w: &World, e| {
            w.get::<&GlobalTransform>(e)
                .unwrap()
                .0
                .transform_point3(Vec3::ZERO)
        };
        // Root turned 90° left: its -Z points to -X.
        assert!(
            at(&world, child).abs_diff_eq(Vec3::new(8.0, 0.0, 0.0), 1e-4),
            "{}",
            at(&world, child)
        );
        assert!(at(&world, grandchild).abs_diff_eq(Vec3::new(8.0, 1.0, 0.0), 1e-4));
        assert!(
            world_matrix(&world, grandchild)
                .unwrap()
                .abs_diff_eq(world.get::<&GlobalTransform>(grandchild).unwrap().0, 1e-5)
        );

        // Moving the root moves the whole branch next frame.
        world.get::<&mut Transform>(root).unwrap().translation.x = 0.0;
        propagate(&mut world);
        assert!(at(&world, grandchild).abs_diff_eq(Vec3::new(-2.0, 1.0, 0.0), 1e-4));
    }

    #[test]
    fn cycles_do_not_hang() {
        let mut world = World::new();
        let a = world.spawn((Transform::IDENTITY,));
        let b = world.spawn((Transform::from_xyz(1.0, 0.0, 0.0), Parent(a)));
        world.insert_one(a, Parent(b)).unwrap();
        propagate(&mut world);
    }

    #[test]
    fn plugin_propagates_in_post_update() {
        let mut app = App::new();
        app.add_plugin(TransformPlugin);
        let (root, child) = {
            let mut world = app.resources.fetch_mut::<World>();
            let root = world.spawn((Transform::from_xyz(1.0, 2.0, 3.0),));
            let child = world.spawn((Transform::from_xyz(0.0, 1.0, 0.0), Parent(root)));
            (root, child)
        };
        // Игровая система в Update двигает корень — рендер в этом же кадре видит новое.
        app.add_system(Stage::Update, move |res| {
            res.fetch_mut::<World>()
                .get::<&mut Transform>(root)
                .unwrap()
                .translation
                .x += 1.0;
        });
        app.startup();
        app.update();
        let world = app.resources.fetch::<World>();
        let pos = world
            .get::<&GlobalTransform>(child)
            .expect("GlobalTransform добавлен сам")
            .0
            .transform_point3(Vec3::ZERO);
        assert!(pos.abs_diff_eq(Vec3::new(2.0, 3.0, 3.0), 1e-5), "{pos}");
    }

    #[test]
    fn scale_and_rotation_compose_and_unparenting_makes_a_root() {
        let mut world = World::new();
        let root = world.spawn((Transform::IDENTITY.with_scale(Vec3::splat(2.0)),));
        let child = world.spawn((
            Transform::from_xyz(1.0, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_z(90f32.to_radians())),
            Parent(root),
        ));
        propagate(&mut world);
        let g = world.get::<&GlobalTransform>(child).unwrap().to_transform();
        // Смещение ребёнка растягивается масштабом родителя, масштаб наследуется.
        assert!(g.translation.abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-5));
        assert!(g.scale.abs_diff_eq(Vec3::splat(2.0), 1e-5));
        assert!((g.rotation * Vec3::X).abs_diff_eq(Vec3::Y, 1e-5));

        // Без Parent объект становится корнем: глобальная матрица = локальная.
        world.remove_one::<Parent>(child).unwrap();
        propagate(&mut world);
        let g = world.get::<&GlobalTransform>(child).unwrap().0;
        assert!(g.abs_diff_eq(world.get::<&Transform>(child).unwrap().matrix(), 1e-6));
    }

    #[test]
    fn despawned_parent_leaves_child_at_its_local_pose() {
        let mut world = World::new();
        let root = world.spawn((Transform::from_xyz(5.0, 0.0, 0.0),));
        let child = world.spawn((Transform::from_xyz(0.0, 1.0, 0.0), Parent(root)));
        world.despawn(root).unwrap();
        propagate(&mut world);
        let pos = world
            .get::<&GlobalTransform>(child)
            .unwrap()
            .0
            .transform_point3(Vec3::ZERO);
        assert!(pos.abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-5), "{pos}");
        // world_matrix ведёт себя так же: висячий родитель = единичная матрица.
        assert!(
            world_matrix(&world, child)
                .unwrap()
                .transform_point3(Vec3::ZERO)
                .abs_diff_eq(pos, 1e-5)
        );
        assert!(world_matrix(&world, root).is_none());
    }
}

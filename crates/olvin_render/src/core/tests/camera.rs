//! Which camera is drawn.

use olvin_ecs::Entity;
use olvin_scene::ActiveCamera;

use super::*;

#[test]
fn the_active_camera_wins_over_the_first_one() {
    let mut world = World::new();
    let first = world.spawn((Transform::from_xyz(0.0, 0.0, 5.0), Camera::default()));
    let second = world.spawn((Transform::from_xyz(3.0, 0.0, 5.0), Camera::default()));
    let first_vp = camera_view_proj(&world, 1.0).unwrap();
    // Nothing marked: the first camera.
    let of = |e: Entity| {
        let t = *world.get::<&Transform>(e).unwrap();
        let c = *world.get::<&Camera>(e).unwrap();
        c.projection(1.0) * Camera::view(&t)
    };
    let (of_first, of_second) = (of(first), of(second));
    assert_eq!(first_vp, of_first);
    world.insert_one(second, ActiveCamera).unwrap();
    assert_eq!(camera_view_proj(&world, 1.0).unwrap(), of_second);
    world.remove_one::<ActiveCamera>(second).unwrap();
    assert_eq!(camera_view_proj(&world, 1.0).unwrap(), first_vp);
    assert!(camera_view_proj(&World::new(), 1.0).is_none());
}

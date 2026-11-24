use glam::Vec3;

use super::*;

/// A horizontal rectangle at height `y`.
fn floor(y: f32, x0: f32, z0: f32, x1: f32, z1: f32) -> Vec<Triangle> {
    let (a, b, c, d) = (
        Vec3::new(x0, y, z0),
        Vec3::new(x1, y, z0),
        Vec3::new(x1, y, z1),
        Vec3::new(x0, y, z1),
    );
    vec![[a, b, c], [a, c, d]]
}

/// A vertical wall along X at depth `z`, between `x0` and `x1`, `h` tall.
fn wall_x(z: f32, x0: f32, x1: f32, base: f32, h: f32) -> Vec<Triangle> {
    let (a, b, c, d) = (
        Vec3::new(x0, base, z),
        Vec3::new(x1, base, z),
        Vec3::new(x1, base + h, z),
        Vec3::new(x0, base + h, z),
    );
    vec![[a, b, c], [a, c, d]]
}

fn cfg() -> NavConfig {
    NavConfig {
        cell: 0.25,
        agent_radius: 0.3,
        ..NavConfig::default()
    }
}

fn length(path: &[Vec3]) -> f32 {
    path.windows(2).map(|w| w[0].distance(w[1])).sum()
}

#[test]
fn open_floor_gives_a_straight_line() {
    let grid = NavGrid::bake(&floor(0.0, 0.0, 0.0, 10.0, 10.0), cfg()).unwrap();
    let path = grid
        .find_path(Vec3::new(1.0, 0.0, 1.0), Vec3::new(9.0, 0.0, 8.0), 0.5)
        .unwrap();
    assert_eq!(path.len(), 2, "{path:?}");
    let straight = Vec3::new(1.0, 0.0, 1.0).distance(Vec3::new(9.0, 0.0, 8.0));
    assert!((length(&path) - straight).abs() < 0.01);
}

#[test]
fn a_wall_with_a_gap_is_walked_around() {
    let mut tris = floor(0.0, 0.0, 0.0, 10.0, 10.0);
    tris.extend(wall_x(5.0, 0.0, 7.0, 0.0, 3.0)); // open between x = 7 and 10
    let grid = NavGrid::bake(&tris, cfg()).unwrap();
    let (from, to) = (Vec3::new(2.0, 0.0, 2.0), Vec3::new(2.0, 0.0, 8.0));
    let path = grid.find_path(from, to, 0.5).unwrap();
    assert!(path.len() >= 3, "must bend around the wall: {path:?}");
    assert!(
        path.iter().any(|p| p.x > 6.9),
        "goes through the gap: {path:?}"
    );
    // No point of the walk is closer to the wall than the agent's radius.
    for w in path.windows(2) {
        for i in 0..=20 {
            let p = w[0].lerp(w[1], i as f32 / 20.0);
            if p.x < 7.0 {
                assert!((p.z - 5.0).abs() >= 0.25, "{p} touches the wall");
            }
        }
    }
}

#[test]
fn a_closed_wall_means_no_path() {
    let mut tris = floor(0.0, 0.0, 0.0, 10.0, 10.0);
    tris.extend(wall_x(5.0, -1.0, 11.0, 0.0, 3.0));
    let grid = NavGrid::bake(&tris, cfg()).unwrap();
    assert!(!grid.is_reachable(Vec3::new(2.0, 0.0, 2.0), Vec3::new(2.0, 0.0, 8.0), 0.5));
    assert!(grid.is_reachable(Vec3::new(2.0, 0.0, 2.0), Vec3::new(8.0, 0.0, 2.0), 0.5));
}

#[test]
fn steps_up_to_max_step_are_climbed_higher_ones_are_not() {
    let low = 0.3;
    let mut tris = floor(0.0, 0.0, 0.0, 5.0, 6.0);
    tris.extend(floor(low, 5.0, 0.0, 10.0, 6.0));
    tris.extend(wall_x(0.0, 5.0, 5.0, 0.0, 0.0)); // nothing: keeps the helper used
    let grid = NavGrid::bake(&tris, cfg()).unwrap();
    assert!(grid.is_reachable(Vec3::new(1.0, 0.0, 3.0), Vec3::new(9.0, low, 3.0), 0.5));

    let mut tris = floor(0.0, 0.0, 0.0, 5.0, 6.0);
    tris.extend(floor(1.0, 5.0, 0.0, 10.0, 6.0));
    let grid = NavGrid::bake(&tris, cfg()).unwrap();
    assert!(!grid.is_reachable(Vec3::new(1.0, 0.0, 3.0), Vec3::new(9.0, 1.0, 3.0), 0.5));
}

#[test]
fn slopes_follow_the_limit() {
    // A ramp from y = 0 at x = 0 to y = 2 at x = 4 (about 27 degrees), then flat.
    let ramp = |rise: f32| {
        let mut t = vec![
            [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(4.0, rise, 0.0),
                Vec3::new(4.0, rise, 6.0),
            ],
            [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(4.0, rise, 6.0),
                Vec3::new(0.0, 0.0, 6.0),
            ],
        ];
        t.extend(floor(rise, 4.0, 0.0, 8.0, 6.0));
        t
    };
    let gentle = NavGrid::bake(&ramp(2.0), cfg()).unwrap();
    assert!(gentle.is_reachable(Vec3::new(0.7, 0.35, 3.0), Vec3::new(7.0, 2.0, 3.0), 0.5));
    let steep = NavGrid::bake(&ramp(6.0), cfg()).unwrap(); // about 56 degrees
    assert!(!steep.is_reachable(Vec3::new(0.7, 0.5, 3.0), Vec3::new(7.0, 6.0, 3.0), 0.5));
}

#[test]
fn a_low_ceiling_blocks_and_a_high_one_does_not() {
    let region = |ceiling: f32| {
        let mut t = floor(0.0, 0.0, 0.0, 10.0, 6.0);
        t.extend(floor(ceiling, 4.0, -1.0, 6.0, 7.0)); // a slab over x = 4..6
        t
    };
    let low = NavGrid::bake(&region(1.2), cfg()).unwrap();
    // Walking under it is impossible: the way along z is closed, x = 4..6 is gone.
    assert!(!low.is_reachable(Vec3::new(1.0, 0.0, 3.0), Vec3::new(9.0, 0.0, 3.0), 0.5));
    let high = NavGrid::bake(&region(2.5), cfg()).unwrap();
    assert!(high.is_reachable(Vec3::new(1.0, 0.0, 3.0), Vec3::new(9.0, 0.0, 3.0), 0.5));
}

#[test]
fn floors_above_each_other_are_separate_layers() {
    let mut tris = floor(0.0, 0.0, 0.0, 10.0, 10.0);
    tris.extend(floor(3.0, 0.0, 0.0, 10.0, 10.0));
    let grid = NavGrid::bake(&tris, cfg()).unwrap();
    // Not connected: nothing joins the floors.
    assert!(!grid.is_reachable(Vec3::new(2.0, 0.0, 2.0), Vec3::new(2.0, 3.0, 2.0), 0.5));
    assert!(grid.is_reachable(Vec3::new(2.0, 3.0, 2.0), Vec3::new(8.0, 3.0, 8.0), 0.5));
    let p = grid
        .find_path(Vec3::new(2.0, 3.0, 2.0), Vec3::new(8.0, 3.0, 8.0), 0.5)
        .unwrap();
    assert!(
        p.iter().all(|p| (p.y - 3.0).abs() < 0.01),
        "stays on the upper floor"
    );
}

#[test]
fn blockers_reroute_and_removing_them_restores_the_way() {
    let mut tris = floor(0.0, 0.0, 0.0, 10.0, 10.0);
    tris.extend(wall_x(5.0, 0.0, 7.0, 0.0, 3.0));
    let mut grid = NavGrid::bake(&tris, cfg()).unwrap();
    let (from, to) = (Vec3::new(2.0, 0.0, 2.0), Vec3::new(2.0, 0.0, 8.0));
    assert!(grid.is_reachable(from, to, 0.5));
    let door = grid.add_blocker(Vec3::new(8.5, 0.0, 5.0), 2.0, 2.0);
    assert!(!grid.is_reachable(from, to, 0.5), "the gap is shut");
    assert!(grid.remove_blocker(door));
    assert!(!grid.remove_blocker(door), "a blocker is removed once");
    assert!(grid.is_reachable(from, to, 0.5));
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    assert_eq!(
        NavGrid::bake(&[], cfg()).unwrap_err(),
        BakeError::NoGeometry
    );
    let nan = [[Vec3::NAN, Vec3::ZERO, Vec3::X]];
    assert_eq!(
        NavGrid::bake(&nan, cfg()).unwrap_err(),
        BakeError::NoGeometry
    );
    let huge = floor(0.0, 0.0, 0.0, 100_000.0, 100_000.0);
    assert!(matches!(
        NavGrid::bake(&huge, cfg()),
        Err(BakeError::TooLarge { .. })
    ));
    let flat = floor(0.0, 0.0, 0.0, 1.0, 1.0);
    for bad in [
        NavConfig { cell: 0.0, ..cfg() },
        NavConfig {
            agent_radius: f32::NAN,
            ..cfg()
        },
        NavConfig {
            max_slope_deg: 90.0,
            ..cfg()
        },
    ] {
        assert!(matches!(
            NavGrid::bake(&flat, bad),
            Err(BakeError::BadConfig(_))
        ));
    }
}

#[test]
fn ends_off_the_ground_have_no_path() {
    let grid = NavGrid::bake(&floor(0.0, 0.0, 0.0, 10.0, 10.0), cfg()).unwrap();
    assert!(
        grid.find_path(Vec3::new(50.0, 0.0, 50.0), Vec3::new(5.0, 0.0, 5.0), 0.5)
            .is_none()
    );
    assert!(
        grid.find_path(Vec3::new(5.0, 9.0, 5.0), Vec3::new(5.0, 0.0, 5.0), 0.5)
            .is_none()
    );
}

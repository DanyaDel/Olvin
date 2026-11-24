//! More lights than the uniform buffer holds: the clustered path.

use super::*;

/// A 6 x 6 grid of red lamps over a white floor, seen from above: every lamp
/// lights the floor under it, the 36 of them, not just the best 16.
fn lamps(world: &mut World, count_per_side: usize) -> Vec<(f32, f32)> {
    let mut spots = Vec::new();
    for i in 0..count_per_side {
        for j in 0..count_per_side {
            let x = -5.5 + i as f32 * 2.2;
            let z = -5.5 + j as f32 * 2.2;
            world.spawn((
                Transform::from_xyz(x, 0.6, z),
                Light::point([1.0, 0.0, 0.0], 8.0, 1.0),
            ));
            spots.push((x, z));
        }
    }
    spots
}

fn lit_red(c: [u8; 3]) -> bool {
    c[0] > c[1] + 40 && c[0] > c[2] + 40
}

#[test]
fn lights_past_the_sixteenth_still_light_the_floor() {
    for editor_like in [false, true] {
        let Some(mut h) = Harness::new(editor_like) else {
            return;
        };
        let plane = h.meshes.add(MeshData::plane(20.0));
        let mut world = World::new();
        world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
        world.spawn(top_down_camera(12.0));
        let spots = lamps(&mut world, 6);
        assert!(spots.len() > crate::MAX_LIGHTS);
        let settings = RenderSettings {
            light_color: Color::BLACK,
            ambient: Color::BLACK,
            ground_ambient: Color::BLACK,
            tonemapping: Tonemapping::None,
            ..Default::default()
        };
        let (_, at) = h.render(&world, &settings);
        let dark: Vec<_> = spots
            .iter()
            .filter(|&&(x, z)| !lit_red(at(column(x, 12.0), column(z, 12.0))))
            .collect();
        assert!(
            dark.is_empty(),
            "editor_like={editor_like}: {} of {} lamps lit nothing: {dark:?}",
            dark.len(),
            spots.len()
        );
    }
}

#[test]
fn a_hundred_lights_cost_the_same_frame_and_keep_working() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let plane = h.meshes.add(MeshData::plane(20.0));
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
    world.spawn(top_down_camera(8.0));
    // 10 x 10 lamps closer together, so most overlap several clusters.
    for i in 0..10 {
        for j in 0..10 {
            world.spawn((
                Transform::from_xyz(-4.0 + i as f32 * 0.9, 0.5, -4.0 + j as f32 * 0.9),
                Light::point([0.0, 1.0, 0.0], 3.0, 1.0),
            ));
        }
    }
    let settings = RenderSettings {
        light_color: Color::BLACK,
        ..Default::default()
    };
    let (stats, at) = h.render(&world, &settings);
    assert_eq!(stats.instances, 1);
    let centre = at(SIZE / 2, SIZE / 2);
    assert!(
        centre[1] > centre[0] + 30,
        "green lamps light the floor: {centre:?}"
    );
}

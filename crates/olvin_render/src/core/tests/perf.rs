//! Frame cost of the default (non-retro) path: `cargo test -p olvin_render
//! perf -- --ignored --nocapture`. Prints ms per frame (prepare + draw +
//! readback) of a lit scene; compare numbers across commits on one machine.

use super::*;

#[test]
#[ignore = "benchmark"]
fn default_path_frame_cost() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    h.size = 256;
    let cube = h.meshes.add(MeshData::cube(1.0));
    let plane = h.meshes.add(MeshData::plane(60.0));
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
    for i in 0..400 {
        let (x, z) = ((i % 20) as f32 * 2.0 - 20.0, (i / 20) as f32 * 2.0 - 40.0);
        world.spawn((
            Transform::from_xyz(x, 0.5, z),
            MeshRenderer::new(cube, Color::rgb(0.8, 0.6, 0.4)),
        ));
    }
    for i in 0..8 {
        world.spawn((
            Transform::from_xyz(i as f32 * 4.0 - 14.0, 2.0, -10.0 - i as f32 * 3.0),
            Light::point([1.0, 0.9, 0.7], 3.0, 8.0),
        ));
    }
    world.spawn((
        Transform::from_xyz(0.0, 3.0, 8.0).looking_at(Vec3::new(0.0, 0.0, -20.0), Vec3::Y),
        Camera::default(),
    ));
    let settings = RenderSettings::default();
    for _ in 0..5 {
        let _ = h.render(&world, &settings);
    }
    let mut samples = Vec::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        for _ in 0..20 {
            let _ = h.render(&world, &settings);
        }
        samples.push(t.elapsed().as_secs_f64() * 1000.0 / 20.0);
    }
    samples.sort_by(f64::total_cmp);
    eprintln!(
        "default path: median {:.2} ms/frame, runs {samples:.2?}",
        samples[2]
    );
}

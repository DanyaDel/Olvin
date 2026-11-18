//! Stress demo: a field of spinning instanced cubes and a fly camera.
//! WASD/Space/Ctrl move, hold RMB to look, Shift to go fast, Esc to quit.
//! `OLVIN_CUBES=250000 cargo run -p sandbox --release` for a heavier field.

use olvin::prelude::*;

struct Spin(f32);

struct FlyCamera {
    yaw: f32,
    pitch: f32,
    speed: f32,
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"),
    )
    .init();

    let mut app = App::new();
    app.add_plugin(DefaultPlugins {
        window: WindowPlugin {
            title: "Olvin Sandbox".into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .add_system(Stage::Startup, setup)
    .add_system(Stage::Update, spin)
    .add_system(Stage::Update, fly_camera)
    .add_system(Stage::Update, |res| {
        if res.fetch::<Input>().just_pressed(KeyCode::Escape) {
            res.fetch_mut::<AppExit>().requested = true;
        }
    })
    .add_system(Stage::Last, stats_logger());
    app.run();
}

fn setup(res: &mut Resources) {
    let count: usize = std::env::var("OLVIN_CUBES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40_000);
    let side = (count as f32).sqrt().ceil() as usize;
    let spacing = 2.5;
    let half = side as f32 * spacing * 0.5;

    let mut meshes = res.fetch_mut::<Meshes>();
    let cube = meshes.add(MeshData::cube(1.0));
    let ground = meshes.add(MeshData::plane(1.0));
    let mut world = res.fetch_mut::<World>();

    world.spawn((
        Transform::from_xyz(0.0, -3.5, 0.0).with_scale(Vec3::splat(half * 2.0 + 10.0)),
        MeshRenderer {
            mesh: ground,
            color: Color::rgb(0.12, 0.13, 0.15),
        },
    ));

    world.spawn_batch((0..count).map(|i| {
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        let (u, v) = (x / side as f32, z / side as f32);
        let wave = ((x * 0.3).sin() + (z * 0.3).cos()) * 1.5;
        (
            Transform::from_xyz(x * spacing - half, wave, z * spacing - half)
                .with_rotation(Quat::from_rotation_y(i as f32)),
            MeshRenderer {
                mesh: cube,
                color: Color::rgb(0.25 + 0.75 * u, 0.4, 0.25 + 0.75 * v),
            },
            Spin(0.5 + (i % 7) as f32 * 0.25),
        )
    }));

    let mut camera = Transform::from_xyz(0.0, 25.0, half + 20.0);
    camera.look_to(Vec3::new(0.0, -0.45, -1.0), Vec3::Y);
    world.spawn((
        camera,
        Camera {
            far: 2000.0,
            ..Default::default()
        },
        FlyCamera {
            yaw: 0.0,
            pitch: -0.42,
            speed: 25.0,
        },
    ));
    log::info!("spawned {count} cubes");
}

fn spin(res: &mut Resources) {
    let dt = res.fetch::<Time>().delta();
    let mut world = res.fetch_mut::<World>();
    for (transform, spin) in world.query_mut::<(&mut Transform, &Spin)>() {
        transform.rotation *= Quat::from_rotation_y(spin.0 * dt);
    }
}

fn fly_camera(res: &mut Resources) {
    let dt = res.fetch::<Time>().delta();
    let input = res.fetch::<Input>();
    let mut world = res.fetch_mut::<World>();
    for (transform, fly) in world.query_mut::<(&mut Transform, &mut FlyCamera)>() {
        if input.mouse_pressed(MouseButton::Right) {
            fly.yaw -= input.mouse_delta.x * 0.003;
            fly.pitch = (fly.pitch - input.mouse_delta.y * 0.003).clamp(-1.5, 1.5);
            transform.rotation = Quat::from_rotation_y(fly.yaw) * Quat::from_rotation_x(fly.pitch);
        }
        let axis = |pos, neg| input.pressed(pos) as i32 as f32 - input.pressed(neg) as i32 as f32;
        let dir = transform.forward() * axis(KeyCode::KeyW, KeyCode::KeyS)
            + transform.right() * axis(KeyCode::KeyD, KeyCode::KeyA)
            + Vec3::Y * axis(KeyCode::Space, KeyCode::ControlLeft);
        let boost = if input.pressed(KeyCode::ShiftLeft) {
            4.0
        } else {
            1.0
        };
        transform.translation += dir.normalize_or_zero() * fly.speed * boost * dt;
    }
}

fn stats_logger() -> impl FnMut(&mut Resources) {
    let (mut acc, mut frames) = (0.0f32, 0u32);
    move |res| {
        acc += res.fetch::<Time>().delta();
        frames += 1;
        if acc >= 1.0 {
            let s = *res.fetch::<RenderStats>();
            log::info!(
                "{:.0} fps ({:.2} ms) | draws {} | visible {} | culled {}",
                frames as f32 / acc,
                acc * 1000.0 / frames as f32,
                s.draw_calls,
                s.instances,
                s.culled
            );
            (acc, frames) = (0.0, 0);
        }
    }
}

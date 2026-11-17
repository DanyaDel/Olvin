//! Files of a fresh game project. The `olvin` dependency and module
//! features are filled in by [`crate::Project::create`].

pub fn cargo_toml(name: &str) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
log = "0.4"
env_logger = "0.11"

# Engine and libraries are always optimized, the game itself compiles fast.
[profile.dev.package."*"]
opt-level = 3

[profile.release]
lto = "thin"
"#
    )
}

pub fn files(name: &str) -> [(&'static str, String); 3] {
    [
        ("src/main.rs", MAIN_RS.replace("{name}", name)),
        (".gitignore", "/target\n".into()),
        (
            "README.md",
            format!(
                "# {name}\n\nИгра на Olvin. Модули ставятся через `olvin-hub add <модуль>` \
                 (список: `olvin-hub list`).\n\n```sh\ncargo run --release\n```\n"
            ),
        ),
    ]
}

const MAIN_RS: &str = r#"use olvin::prelude::*;

struct Spin(f32);

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"),
    )
    .init();

    let mut app = App::new();
    app.add_plugin(DefaultPlugins {
        window: WindowPlugin { title: "{name}".into(), ..Default::default() },
        ..Default::default()
    })
    .add_system(Stage::Startup, setup)
    .add_system(Stage::Update, spin)
    .add_system(Stage::Update, |res| {
        if res.fetch::<Input>().just_pressed(KeyCode::Escape) {
            res.fetch_mut::<AppExit>().requested = true;
        }
    });
    app.run();
}

fn setup(res: &mut Resources) {
    let cube = res.fetch_mut::<Meshes>().add(MeshData::cube(1.0));
    let mut world = res.fetch_mut::<World>();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer { mesh: cube, color: Color::rgb(0.9, 0.5, 0.2) },
        Spin(1.0),
    ));
    world.spawn((
        Transform::from_xyz(0.0, 1.5, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        Camera::default(),
    ));
}

fn spin(res: &mut Resources) {
    let dt = res.fetch::<Time>().delta();
    for (transform, spin) in res.fetch_mut::<World>().query_mut::<(&mut Transform, &Spin)>() {
        transform.rotation *= Quat::from_rotation_y(spin.0 * dt);
    }
}
"#;

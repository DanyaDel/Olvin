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

# Engine and libraries are always optimized, the game itself compiles fast.
[profile.dev.package."*"]
opt-level = 3

[profile.release]
lto = "thin"

# Standalone: never part of an enclosing workspace.
[workspace]
"#
    )
}

/// `graphical`: window + render + scenes; otherwise a headless app.
pub fn files(name: &str, graphical: bool) -> [(&'static str, String); 3] {
    let main = if graphical { MAIN_RS } else { HEADLESS_RS };
    [
        ("src/main.rs", main.replace("{name}", name)),
        // `olvin-hub build` puts the game into dist/ (see `Project::dist_dir`).
        (".gitignore", GITIGNORE.into()),
        (
            "README.md",
            format!(
                "# {name}\n\nA game made with Olvin. Add modules with `olvin-hub add <module>` \
                 (list: `olvin-hub list`), edit the scene with `olvin-hub editor open`.\n\n```sh\ncargo run --release\n```\n"
            ),
        ),
    ]
}

/// Build output, logs and save slots are not sources.
pub const GITIGNORE: &str = "/target\n/dist\n/logs\n/saves\n";

const MAIN_RS: &str = r#"use olvin::prelude::*;

struct Spin(f32);

fn main() {
    // Log to stderr and logs/game.log; panic reports go to logs/crash-*.txt.
    // Paths are relative to the project folder (`ProjectPaths`: the exe's
    // folder after `olvin-hub build`, the crate under cargo), not the working directory.
    olvin::core::Logging::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")
        .file("logs/game.log")
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
    // The scene saved in olvin-editor, if there is one.
    let scene = res.fetch::<ProjectPaths>().asset("scenes/main.scene.ron");
    match olvin::scene_file::load(res, scene) {
        Ok(count) => log::info!("scene loaded: {count} objects"),
        Err(err) => {
            log::info!("no editor scene ({err}), showing a cube");
            let cube = res.fetch_mut::<Meshes>().add(MeshData::cube(1.0));
            let mut world = res.fetch_mut::<World>();
            world.spawn((
                Transform::IDENTITY,
                MeshRenderer { mesh: cube, color: Color::rgb(0.9, 0.5, 0.2), material: None },
                Spin(1.0),
            ));
            world.spawn((
                Transform::from_xyz(0.0, 1.5, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
                Camera::default(),
            ));
        }
    }
}

fn spin(res: &mut Resources) {
    let dt = res.fetch::<Time>().delta();
    for (transform, spin) in res.fetch_mut::<World>().query_mut::<(&mut Transform, &Spin)>() {
        transform.rotation *= Quat::from_rotation_y(spin.0 * dt);
    }
}
"#;

const HEADLESS_RS: &str = r#"//! Headless Olvin app: every enabled module without a window or GPU.
//! Add modules with olvin-hub.

use olvin::prelude::*;

fn main() {
    // logs/ inside the project folder, wherever the game is started from.
    olvin::core::Logging::new("info").file("logs/game.log").init();
    let mut app = App::new();
    app.add_plugin(HeadlessPlugins::default()).add_system(Stage::Update, |res| {
        if res.fetch::<Time>().frame() >= 3 {
            println!("{name}: headless app is running");
            res.fetch_mut::<AppExit>().requested = true;
        }
    });
    app.run();
}
"#;

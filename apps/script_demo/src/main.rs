//! Headless gameplay demo: no window, no audio device (pass `--audio` to
//! open one). A character walks through a trigger with a pickup, a crate
//! falls on the floor, a cube spins and a platform bobs. Prints positions.
//!
//! `cargo run -p script_demo` (RUST_LOG=info for script logs)

use std::time::Duration;

use olvin::audio::AudioSourceDesc;
use olvin::input::KeyCode;
use olvin::prelude::*;
use olvin::scene::file::{CharacterControllerDesc, ColliderDesc, ColliderShape, RigidbodyDesc};

/// A game script: walks the character forward, jumps once, reports hits.
struct AutoPilot {
    speed: f32,
    jumped: bool,
}

impl FromParams for AutoPilot {
    fn from_params(p: &Params) -> Self {
        Self {
            speed: p.f32("speed", 3.0),
            jumped: false,
        }
    }

    fn schema() -> Vec<ParamSpec> {
        vec![ParamSpec::float("speed", 3.0, "walk speed, m/s")]
    }
}

impl Behaviour for AutoPilot {
    fn start(&mut self, ctx: &mut ScriptCtx) {
        println!("[{}] start at {}", ctx.name(), ctx.position());
    }

    fn update(&mut self, ctx: &mut ScriptCtx) {
        // `Move` from input when held, otherwise walk forward on our own.
        let input = ctx.input().axis2d("Move");
        let dir = if input.length() > 0.1 {
            Vec3::new(input.x, 0.0, -input.y)
        } else {
            Vec3::NEG_Z
        };
        let elapsed = ctx.elapsed();
        let speed = self.speed;
        let mut jump = false;
        if let Some(mut motor) = ctx.motor() {
            motor.velocity.x = dir.x * speed;
            motor.velocity.z = dir.z * speed;
            if !self.jumped && elapsed > 2.0 && motor.grounded {
                motor.velocity.y = 5.0;
                jump = true;
            }
        }
        if jump {
            self.jumped = true;
            println!("[{}] jump!", ctx.name());
        }
    }

    fn on_trigger_enter(&mut self, ctx: &mut ScriptCtx, other: Entity) {
        println!("[{}] entered trigger {}", ctx.name(), ctx.name_of(other));
    }
}

/// Collected when the player walks into it.
struct Pickup;

impl FromParams for Pickup {
    fn from_params(_: &Params) -> Self {
        Pickup
    }
}

impl Behaviour for Pickup {
    fn on_trigger_enter(&mut self, ctx: &mut ScriptCtx, other: Entity) {
        println!("[{}] picked up by {}", ctx.name(), ctx.name_of(other));
        if let Some(mut audio) = ctx.res_mut::<olvin::audio::Audio>() {
            audio.play_at("sfx/pickup.ogg", ctx.world_position());
        }
        ctx.destroy_self();
    }

    fn on_destroy(&mut self, ctx: &mut ScriptCtx) {
        println!("[{}] destroyed", ctx.name());
    }
}

/// Logs what the crate lands on.
struct Thud;

impl FromParams for Thud {
    fn from_params(_: &Params) -> Self {
        Thud
    }
}

impl Behaviour for Thud {
    fn on_collision_enter(&mut self, ctx: &mut ScriptCtx, other: Entity) {
        println!(
            "[{}] hit {} at y = {:.2}",
            ctx.name(),
            ctx.name_of(other),
            ctx.position().y
        );
    }
}

fn main() {
    olvin::core::Logging::new("warn")
        .file("logs/script_demo.log")
        .init();
    let audio = std::env::args().any(|a| a == "--audio");

    let mut app = App::new();
    app.add_plugin(DefaultPlugins {
        audio: AudioPlugin {
            enabled: audio,
            ..Default::default()
        },
        input: InputPlugin {
            map: Some(InputMap::default()),
        },
        ..Default::default()
    });
    olvin::script::register!(
        app.resources.fetch_mut::<ScriptRegistry>(),
        AutoPilot,
        Pickup,
        Thud
    );

    let (player, crate_, spinner, platform) = {
        let mut world = app.resources.fetch_mut::<World>();
        world.spawn((
            Name("Floor".into()),
            Transform::from_xyz(0.0, -0.5, 0.0).with_scale(Vec3::new(40.0, 1.0, 40.0)),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
        ));
        let player = world.spawn((
            Name("Player".into()),
            Transform::from_xyz(0.0, 1.0, 0.0),
            CharacterControllerDesc::default(),
            Scripts::new().with("AutoPilot", [("speed", "3")]),
            AudioListener,
        ));
        let mut zone = ColliderDesc::new(ColliderShape::Box {
            size: Vec3::new(3.0, 2.0, 1.0),
        });
        zone.is_trigger = true;
        world.spawn((
            Name("Coin".into()),
            Transform::from_xyz(3.0, 1.0, -8.0),
            zone,
            Scripts::new().with("Pickup", []),
        ));
        let crate_ = world.spawn((
            Name("Crate".into()),
            Transform::from_xyz(3.0, 4.0, -2.0),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
            RigidbodyDesc::default(),
            Scripts::new().with("Thud", []),
        ));
        let spinner = world.spawn((
            Name("Spinner".into()),
            Transform::from_xyz(-3.0, 1.0, 0.0),
            Scripts::new().with("Rotator", [("speed_deg", "0,90,0")]),
            AudioSourceDesc {
                clip: "sfx/hum.ogg".into(),
                looping: true,
                ..Default::default()
            },
        ));
        let platform = world.spawn((
            Name("Platform".into()),
            Transform::from_xyz(-3.0, 0.5, -4.0),
            Scripts::new().with("Oscillator", [("amplitude", "0,1,0"), ("frequency", "0.5")]),
        ));
        (player, crate_, spinner, platform)
    };

    // Synthetic input, in-process only: hold D for the first second (strafe right).
    app.resources
        .fetch_mut::<olvin::input::Input>()
        .set_key(KeyCode::KeyD, true);

    for frame in 0..300 {
        app.update();
        if frame == 60 {
            app.resources
                .fetch_mut::<olvin::input::Input>()
                .set_key(KeyCode::KeyD, false);
        }
        if frame % 30 == 0 {
            let world = app.resources.fetch::<World>();
            let pos = |e| {
                world
                    .get::<&Transform>(e)
                    .map(|t| t.translation)
                    .unwrap_or(Vec3::NAN)
            };
            let yaw = world
                .get::<&Transform>(spinner)
                .map(|t| {
                    t.rotation
                        .to_euler(olvin::math::EulerRot::YXZ)
                        .0
                        .to_degrees()
                })
                .unwrap_or(0.0);
            let grounded = world
                .get::<&CharacterMotor>(player)
                .map(|m| m.grounded)
                .unwrap_or(false);
            println!(
                "frame {frame:3}: player {:.2} (grounded {grounded}), crate {:.2}, spinner yaw {yaw:6.1}°, platform y {:.2}",
                pos(player),
                pos(crate_),
                pos(platform).y,
            );
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    let audio = app.resources.fetch::<Audio>();
    println!(
        "audio: device {}, {} sounds tracked",
        if audio.is_available() {
            "open"
        } else {
            "off (no-op)"
        },
        audio.active_count()
    );
}

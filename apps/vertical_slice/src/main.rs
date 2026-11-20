//! Olvin vertical slice: first person, physics, a trigger that opens a door,
//! positional sound and music, an animated glTF robot, pause, save slots.
//!
//! `cargo run -p vertical_slice --release`
//! Options: `--frames N` (or `OLVIN_SLICE_AUTOQUIT=N`) quits after N frames,
//! `--no-audio` opens no sound device.
//!
//! Controls: WASD move, mouse look, Space jump, Shift sprint, Esc pause
//! (Q quits while paused), F5 quick save, F9 quick load.

use olvin::prelude::*;
use vertical_slice::{GamePlugin, SliceConfig, input_map};

fn main() {
    let mut config = SliceConfig::detect();
    let args: Vec<String> = std::env::args().collect();
    config.audio = !args.iter().any(|a| a == "--no-audio");
    let frames: Option<u64> = args
        .iter()
        .position(|a| a == "--frames")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .or_else(|| std::env::var("OLVIN_SLICE_AUTOQUIT").ok())
        .and_then(|n| n.parse().ok());

    olvin::core::Logging::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")
        .file(config.root().join("logs/vertical_slice.log"))
        .init();
    log::info!("assets: {}", config.assets.display());

    let mut app = App::new();
    app.add_plugin(DefaultPlugins {
        window: WindowPlugin {
            title: "Olvin — vertical slice".into(),
            ..Default::default()
        },
        input: InputPlugin {
            map: Some(input_map()),
        },
        audio: AudioPlugin {
            assets_dir: config.assets.clone(),
            enabled: config.audio,
        },
        ..Default::default()
    })
    .add_plugin(GamePlugin { config })
    .add_system(Stage::Last, frame_counter(frames));
    app.run();
}

/// Logs fps once a second; with a limit, requests exit after that many frames.
fn frame_counter(limit: Option<u64>) -> impl FnMut(&mut Resources) {
    let (mut acc, mut frames, mut total) = (0.0f32, 0u32, 0u64);
    move |res| {
        total += 1;
        acc += res.fetch::<Time>().unscaled_delta();
        frames += 1;
        if acc >= 1.0 {
            let s = *res.fetch::<RenderStats>();
            log::info!(
                "{:.0} fps | draws {} | visible {} | culled {}",
                frames as f32 / acc,
                s.draw_calls,
                s.instances,
                s.culled
            );
            (acc, frames) = (0.0, 0);
        }
        if limit.is_some_and(|n| total >= n) {
            log::info!("auto-quit after {total} frames");
            res.fetch_mut::<AppExit>().requested = true;
        }
    }
}

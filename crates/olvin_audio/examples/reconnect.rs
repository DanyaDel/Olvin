//! Manual check of device recovery: plays a soft looping tone and reports
//! the output state every second. While it runs, restart the audio server
//! (`systemctl --user restart pipewire pipewire-pulse wireplumber`) or switch
//! the output device; the tone must come back on its own within a few seconds.
//!
//!     cargo run -p olvin_audio --example reconnect [seconds]

use std::time::{Duration, Instant};

use olvin_audio::{Audio, PlayParams};
use olvin_ecs::World;

fn main() {
    olvin_core::Logging::new("info").init();
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let dir = std::env::temp_dir().join(format!("olvin-reconnect-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("tone.wav"), tone(1.0)).expect("tone.wav");

    let mut audio = Audio::open(&dir);
    audio.play_with(
        "tone.wav",
        PlayParams {
            looping: true,
            volume: 0.15,
            ..Default::default()
        },
    );
    let mut world = World::new();
    let start = Instant::now();
    let mut report = start;
    while start.elapsed() < Duration::from_secs(seconds) {
        audio.update(&mut world);
        if report.elapsed() >= Duration::from_secs(1) {
            report = Instant::now();
            println!(
                "{:>3} s  output: {:<5}  reconnects: {}",
                start.elapsed().as_secs(),
                if audio.is_available() { "on" } else { "LOST" },
                audio.reconnect_count()
            );
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// One second of a soft 220 Hz tone, 16-bit mono WAV.
fn tone(seconds: f32) -> Vec<u8> {
    let rate = 44_100u32;
    let n = (rate as f32 * seconds) as u32;
    let mut out = Vec::with_capacity(44 + n as usize * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + n * 2).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(n * 2).to_le_bytes());
    for i in 0..n {
        let t = i as f32 / rate as f32;
        let s = (t * 220.0 * std::f32::consts::TAU).sin() * 0.5;
        out.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

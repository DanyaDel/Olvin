use std::time::Instant;

use crate::Resources;

/// Frame timing, updated at the start of every frame (`Stage::First`).
#[derive(Debug, Default)]
pub struct Time {
    delta: f32,
    elapsed: f64,
    frame: u64,
    last: Option<Instant>,
}

impl Time {
    /// Seconds since the previous frame (0 on the first frame).
    pub fn delta(&self) -> f32 {
        self.delta
    }

    /// Seconds since the first frame.
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub(crate) fn tick(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last {
            // Clamp so a debugger pause or a window drag doesn't teleport the simulation.
            self.delta = (now - last).as_secs_f32().min(0.25);
            self.elapsed += self.delta as f64;
            self.frame += 1;
        }
        self.last = Some(now);
    }
}

pub(crate) fn update_time(res: &mut Resources) {
    res.fetch_mut::<Time>().tick();
}

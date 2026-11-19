use std::time::Instant;

use crate::Resources;

/// Longest real frame the simulation accepts, so a debugger pause or a window
/// drag doesn't teleport it.
const MAX_FRAME: f32 = 0.25;

/// Frame timing, updated at the start of every frame (`Stage::First`).
///
/// `delta`/`elapsed` are simulation time: scaled by [`Time::time_scale`] and
/// frozen while [`paused`](Time::paused). The `unscaled_*` pair ignores both,
/// for UI and menus. For deterministic tests use [`Time::set_fixed_delta`] (every
/// frame lasts exactly that long, no wall clock) or [`Time::advance`].
#[derive(Debug)]
pub struct Time {
    delta: f32,
    elapsed: f64,
    unscaled_delta: f32,
    unscaled_elapsed: f64,
    frame: u64,
    scale: f32,
    paused: bool,
    step: bool,
    fixed_delta: Option<f32>,
    last: Option<Instant>,
}

impl Default for Time {
    fn default() -> Self {
        Self {
            delta: 0.0,
            elapsed: 0.0,
            unscaled_delta: 0.0,
            unscaled_elapsed: 0.0,
            frame: 0,
            scale: 1.0,
            paused: false,
            step: false,
            fixed_delta: None,
            last: None,
        }
    }
}

impl Time {
    /// A clock for tests: every frame lasts exactly `delta` seconds, no wall clock.
    pub fn with_fixed_delta(delta: f32) -> Self {
        let mut time = Self::default();
        time.set_fixed_delta(Some(delta));
        time
    }

    /// Simulation seconds since the previous frame (0 on the first frame and while paused).
    pub fn delta(&self) -> f32 {
        self.delta
    }

    /// Simulation seconds since the first frame.
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// Real seconds since the previous frame, ignoring scale and pause.
    pub fn unscaled_delta(&self) -> f32 {
        self.unscaled_delta
    }

    /// Real seconds since the first frame, ignoring scale and pause.
    pub fn unscaled_elapsed(&self) -> f64 {
        self.unscaled_elapsed
    }

    /// Frame counter. Counts paused frames too.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn time_scale(&self) -> f32 {
        self.scale
    }

    /// Multiplier for simulation time (1 = real time, 0.5 = slow motion).
    /// Negative and NaN values are treated as 0.
    pub fn set_time_scale(&mut self, scale: f32) {
        self.scale = if scale > 0.0 { scale } else { 0.0 };
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.paused = false;
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// While paused, lets the next frame run with a normal delta and pauses
    /// again after it (frame stepping). Has no effect when not paused.
    pub fn step(&mut self) {
        self.step = true;
    }

    pub fn fixed_delta(&self) -> Option<f32> {
        self.fixed_delta
    }

    /// `Some(dt)`: every frame lasts exactly `dt` real seconds (deterministic
    /// runs and tests, also advances the very first frame). `None`: wall clock.
    pub fn set_fixed_delta(&mut self, delta: Option<f32>) {
        self.fixed_delta = delta.map(|d| d.max(0.0));
    }

    /// Advances the clock by one frame of `real_delta` seconds without touching the
    /// wall clock. Scale, pause and stepping apply as usual. Not clamped: the caller
    /// owns the step size.
    pub fn advance(&mut self, real_delta: f32) {
        let real = real_delta.max(0.0);
        let running = !self.paused || self.step;
        self.step = false;
        self.unscaled_delta = real;
        self.unscaled_elapsed += real as f64;
        self.delta = if running { real * self.scale } else { 0.0 };
        self.elapsed += self.delta as f64;
        self.frame += 1;
    }

    pub(crate) fn tick(&mut self) {
        if let Some(dt) = self.fixed_delta {
            self.advance(dt);
            return;
        }
        let now = Instant::now();
        if let Some(last) = self.last {
            self.advance((now - last).as_secs_f32().min(MAX_FRAME));
        }
        self.last = Some(now);
    }
}

pub(crate) fn update_time(res: &mut Resources) {
    res.fetch_mut::<Time>().tick();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, Stage};

    #[test]
    fn scale_and_pause_affect_sim_time_only() {
        let mut t = Time::default();
        t.set_time_scale(0.5);
        t.advance(0.1);
        assert_eq!((t.delta(), t.unscaled_delta()), (0.05, 0.1));
        t.pause();
        t.advance(0.1);
        assert_eq!(t.delta(), 0.0);
        assert_eq!(t.unscaled_delta(), 0.1);
        assert!((t.elapsed() - 0.05).abs() < 1e-9);
        assert!((t.unscaled_elapsed() - 0.2).abs() < 1e-6);
        assert_eq!(t.frame(), 2);
        t.resume();
        t.advance(0.1);
        assert_eq!(t.delta(), 0.05);
    }

    #[test]
    fn step_runs_one_frame_while_paused() {
        let mut t = Time::default();
        t.pause();
        t.step();
        t.advance(0.1);
        assert_eq!(t.delta(), 0.1);
        t.advance(0.1);
        assert_eq!(t.delta(), 0.0);
        assert!(t.paused());
        // Stepping when not paused doesn't leave a pending step behind.
        t.resume();
        t.step();
        t.advance(0.1);
        t.pause();
        t.advance(0.1);
        assert_eq!(t.delta(), 0.0);
    }

    #[test]
    fn bad_values_are_sanitised() {
        let mut t = Time::default();
        t.set_time_scale(-2.0);
        assert_eq!(t.time_scale(), 0.0);
        t.set_time_scale(f32::NAN);
        assert_eq!(t.time_scale(), 0.0);
        t.set_time_scale(1.0);
        t.advance(-1.0);
        assert_eq!(t.delta(), 0.0);
    }

    #[test]
    fn fixed_delta_is_deterministic_through_app() {
        let mut app = App::new();
        app.insert_resource(Time::with_fixed_delta(0.5));
        app.add_system(Stage::Update, |res| {
            let t = res.fetch::<Time>();
            assert_eq!(t.delta(), 0.5);
        });
        for _ in 0..4 {
            app.update();
        }
        let t = app.resources.fetch::<Time>();
        assert_eq!((t.frame(), t.elapsed()), (4, 2.0));
    }

    #[test]
    fn wall_clock_first_frame_is_zero() {
        let mut t = Time::default();
        t.tick();
        assert_eq!((t.delta(), t.frame()), (0.0, 0));
    }
}

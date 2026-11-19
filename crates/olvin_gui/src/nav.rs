//! Key-repeat for held directions, so a stick or d-pad steps through a menu
//! instead of flying across it.

use glam::Vec2;

use crate::geom::Dir;

/// Turns a held direction vector into discrete steps: one at once, then after
/// `delay` seconds one every `interval`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavRepeat {
    pub delay: f32,
    pub interval: f32,
    /// Vector length that counts as "held" (stick dead zone for UI).
    pub threshold: f32,
    held: Option<Dir>,
    timer: f32,
}

impl Default for NavRepeat {
    fn default() -> Self {
        Self {
            delay: 0.4,
            interval: 0.12,
            threshold: 0.5,
            held: None,
            timer: 0.0,
        }
    }
}

impl NavRepeat {
    /// `axis` is a +Y-up vector (e.g. `ActionState::axis2d`), `dt` the frame time
    /// (use real time: menus work while the game is paused). Returns a step to take this frame.
    pub fn update(&mut self, axis: Vec2, dt: f32) -> Option<Dir> {
        let dir = Dir::from_axis(axis, self.threshold);
        if dir != self.held {
            self.held = dir;
            self.timer = self.delay;
            return dir;
        }
        let dir = dir?;
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.timer += self.interval;
            return Some(dir);
        }
        None
    }
}

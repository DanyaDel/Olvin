//! Delays and repeats without counting frames by hand: `after(2.0, "respawn")`
//! fires one [`TimerFired`] event two seconds later, `every(0.5, "spawn")` fires
//! every half second until cancelled.
//!
//! Timers run on simulation time, so they stop while the game is paused and
//! follow `Time::time_scale`; [`Timers::after_unscaled`] and
//! [`Timers::every_unscaled`] ignore both (menus, UI animations).

use crate::{App, Events, Plugin, Resources, Stage, Time};

/// A repeating timer that fell behind (a long frame) fires at most this many
/// times in one frame; the rest of the backlog is dropped.
const MAX_CATCH_UP: u32 = 8;

/// Handle of a timer, for [`Timers::cancel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(u64);

/// Event: a timer ran out. Read it with an `EventCursor` like any other event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimerFired {
    pub id: TimerId,
    /// The name given to `after` / `every`.
    pub tag: String,
}

#[derive(Debug)]
struct Entry {
    id: TimerId,
    tag: String,
    remaining: f64,
    /// `Some` for a repeating timer.
    interval: Option<f64>,
    unscaled: bool,
}

/// Resource: the running timers. Added by [`TimerPlugin`].
#[derive(Debug, Default)]
pub struct Timers {
    entries: Vec<Entry>,
    next_id: u64,
}

impl Timers {
    /// Fires once, `seconds` of game time from now.
    pub fn after(&mut self, seconds: f32, tag: impl Into<String>) -> TimerId {
        self.add(seconds, None, tag.into(), false)
    }

    /// Fires every `seconds` of game time (the first time after `seconds`).
    /// An interval that is not positive is raised to one millisecond.
    pub fn every(&mut self, seconds: f32, tag: impl Into<String>) -> TimerId {
        self.add(seconds, Some(seconds), tag.into(), false)
    }

    /// Like [`Timers::after`], on real time: ignores pause and time scale.
    pub fn after_unscaled(&mut self, seconds: f32, tag: impl Into<String>) -> TimerId {
        self.add(seconds, None, tag.into(), true)
    }

    /// Like [`Timers::every`], on real time.
    pub fn every_unscaled(&mut self, seconds: f32, tag: impl Into<String>) -> TimerId {
        self.add(seconds, Some(seconds), tag.into(), true)
    }

    fn add(&mut self, seconds: f32, interval: Option<f32>, tag: String, unscaled: bool) -> TimerId {
        let id = TimerId(self.next_id);
        self.next_id += 1;
        self.entries.push(Entry {
            id,
            tag,
            remaining: f64::from(seconds.max(0.0)),
            interval: interval.map(|i| f64::from(i.max(0.001))),
            unscaled,
        });
        id
    }

    /// Stops a timer. False if it already fired (one-shot) or was cancelled.
    pub fn cancel(&mut self, id: TimerId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != before
    }

    /// Stops every timer with this tag; returns how many.
    pub fn cancel_tag(&mut self, tag: &str) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| e.tag != tag);
        before - self.entries.len()
    }

    /// How many timers have a tag starting with `prefix`.
    pub fn count_with_prefix(&self, prefix: &str) -> usize {
        self.entries
            .iter()
            .filter(|e| e.tag.starts_with(prefix))
            .count()
    }

    /// Stops every timer whose tag starts with `prefix`; returns how many.
    pub fn cancel_prefix(&mut self, prefix: &str) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| !e.tag.starts_with(prefix));
        before - self.entries.len()
    }

    pub fn is_active(&self, id: TimerId) -> bool {
        self.entries.iter().any(|e| e.id == id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Moves every timer by this frame's time and returns what fired, oldest
    /// first. Called by [`TimerPlugin`]; public for tests and custom loops.
    pub fn advance(&mut self, delta: f32, unscaled_delta: f32) -> Vec<TimerFired> {
        let mut fired = Vec::new();
        self.entries.retain_mut(|e| {
            e.remaining -= f64::from(if e.unscaled { unscaled_delta } else { delta });
            let mut times = 0;
            while e.remaining <= 0.0 {
                fired.push(TimerFired {
                    id: e.id,
                    tag: e.tag.clone(),
                });
                match e.interval {
                    Some(interval) => {
                        e.remaining += interval;
                        times += 1;
                        if times == MAX_CATCH_UP {
                            e.remaining = e.remaining.max(interval);
                            break;
                        }
                    }
                    None => return false,
                }
            }
            true
        });
        fired
    }
}

fn run(res: &mut Resources) {
    let (delta, unscaled) = {
        let time = res.fetch::<Time>();
        (time.delta(), time.unscaled_delta())
    };
    let fired = res.fetch_mut::<Timers>().advance(delta, unscaled);
    if !fired.is_empty() {
        res.fetch_mut::<Events<TimerFired>>().send_batch(fired);
    }
}

/// Adds [`Timers`] and the [`TimerFired`] event; timers advance in `Stage::PreUpdate`.
pub struct TimerPlugin;

impl Plugin for TimerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Timers>();
        app.add_event::<TimerFired>();
        app.add_system(Stage::PreUpdate, run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventCursor;

    fn tags(fired: &[TimerFired]) -> Vec<&str> {
        fired.iter().map(|f| f.tag.as_str()).collect()
    }

    #[test]
    fn one_shot_fires_once_at_its_time() {
        let mut t = Timers::default();
        let id = t.after(1.0, "boom");
        assert!(t.advance(0.5, 0.5).is_empty());
        assert!(t.is_active(id));
        assert_eq!(tags(&t.advance(0.5, 0.5)), ["boom"]);
        assert!(!t.is_active(id));
        assert!(t.advance(5.0, 5.0).is_empty());
    }

    #[test]
    fn repeating_timer_keeps_its_rhythm_without_drift() {
        let mut t = Timers::default();
        t.every(0.3, "tick");
        let total: usize = (0..10).map(|_| t.advance(0.1, 0.1).len()).sum();
        // 1.0 s at a 0.3 s interval: fires at 0.3, 0.6, 0.9.
        assert_eq!(total, 3);
    }

    #[test]
    fn a_long_frame_catches_up_but_not_forever() {
        let mut t = Timers::default();
        t.every(0.1, "tick");
        assert_eq!(t.advance(0.35, 0.35).len(), 3);
        let mut t = Timers::default();
        t.every(0.001, "tick");
        assert_eq!(t.advance(100.0, 100.0).len(), MAX_CATCH_UP as usize);
        assert_eq!(t.len(), 1, "still running afterwards");
    }

    #[test]
    fn cancel_by_id_and_by_tag() {
        let mut t = Timers::default();
        let a = t.after(1.0, "a");
        t.after(1.0, "b");
        t.every(1.0, "b");
        assert!(t.cancel(a));
        assert!(!t.cancel(a));
        assert_eq!(t.cancel_tag("b"), 2);
        assert!(t.is_empty());
    }

    #[test]
    fn scaled_timers_stop_in_a_pause_unscaled_ones_do_not() {
        let mut t = Timers::default();
        t.after(1.0, "game");
        t.after_unscaled(1.0, "menu");
        // Paused: simulation delta 0, real time keeps running.
        assert_eq!(tags(&t.advance(0.0, 1.0)), ["menu"]);
        assert_eq!(tags(&t.advance(1.0, 1.0)), ["game"]);
    }

    #[test]
    fn order_follows_creation_and_zero_delay_fires_next_frame() {
        let mut t = Timers::default();
        t.after(0.0, "first");
        t.after(-3.0, "second");
        assert_eq!(tags(&t.advance(0.016, 0.016)), ["first", "second"]);
    }

    #[test]
    fn the_plugin_turns_time_into_events() {
        let mut app = App::new();
        app.insert_resource(Time::with_fixed_delta(0.5));
        app.add_plugin(TimerPlugin);
        app.resources.fetch_mut::<Timers>().after(1.0, "go");
        let mut cursor = EventCursor::default();
        let mut seen = Vec::new();
        for _ in 0..4 {
            app.update();
            let events = app.resources.fetch::<Events<TimerFired>>();
            seen.extend(events.read(&mut cursor).map(|f| f.tag.clone()));
        }
        assert_eq!(seen, ["go"]);
    }
}

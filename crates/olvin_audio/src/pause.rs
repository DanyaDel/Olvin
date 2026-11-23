//! Pausing: single sounds and the `Time` pause.

use super::*;

impl Audio {
    /// Holds one sound where it is (until [`resume_sound`](Self::resume_sound)),
    /// whatever the game pause does.
    pub fn pause_sound(&mut self, id: SoundId) {
        if let Some(p) = self.playing.get_mut(&id) {
            p.user_paused = true;
            self.apply_holds();
        }
    }

    pub fn resume_sound(&mut self, id: SoundId) {
        if let Some(p) = self.playing.get_mut(&id) {
            p.user_paused = false;
            self.apply_holds();
        }
    }

    /// Whether the sound is held, by [`pause_sound`](Self::pause_sound) or the game pause.
    pub fn is_sound_paused(&self, id: SoundId) -> bool {
        self.playing.get(&id).is_some_and(|p| p.held)
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Game pause: holds every sound on a pausable bus, `false` continues
    /// them. Sounds started while paused are not held (menu sounds). The
    /// plugin calls this when `Time` is paused or resumed, unless
    /// [`set_follow_time_pause(false)`](Self::set_follow_time_pause).
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        if !paused {
            for p in self.playing.values_mut() {
                p.pause_exempt = false;
            }
        }
        self.apply_holds();
    }

    pub fn follows_time_pause(&self) -> bool {
        self.follow_time
    }

    /// Whether the plugin pauses audio together with `Time` (on by default).
    pub fn set_follow_time_pause(&mut self, follow: bool) {
        self.follow_time = follow;
    }

    pub fn is_bus_pausable(&self, bus: BusId) -> bool {
        self.buses.get(bus.0).is_some_and(|b| b.pausable)
    }

    /// Whether the game pause holds the bus's sounds. On for every bus but
    /// `music`, so the soundtrack keeps playing under a pause menu.
    pub fn set_bus_pausable(&mut self, bus: BusId, pausable: bool) {
        if let Some(b) = self.buses.get_mut(bus.0) {
            b.pausable = pausable;
            self.apply_holds();
        }
    }

    /// Pauses or resumes voices whose held state changed.
    pub(super) fn apply_holds(&mut self) {
        let last = self.buses.len() - 1;
        for p in self.playing.values_mut() {
            let bus = &self.buses[p.params.bus.0.min(last)];
            let hold = p.user_paused || (self.paused && bus.pausable && !p.pause_exempt);
            if hold != p.held {
                p.held = hold;
                if let Some(h) = &mut p.handle {
                    if hold {
                        h.pause(SMOOTH);
                    } else {
                        h.resume(SMOOTH);
                    }
                }
            }
        }
    }
}

//! Keeping the output alive. kira restarts its stream when the default device
//! changes, but not when the audio server (PipeWire, PulseAudio) is down at
//! that moment, and not when no device existed at start: then sound stays
//! dead until the game restarts. So the engine watches for itself.
//!
//! A heartbeat clock lives inside the mixer: it only ticks while the stream
//! really renders. When it stands still for [`STALL`], or there was no device
//! to begin with, the output counts as lost: voices drop (loops and music wait
//! virtualized, one-shots are gone) and a new mixer is tried every [`RETRY`].
//! When one opens, buses, reverb and master volume come back and the waiting
//! sounds restart from the beginning, like voices won back from virtualization.

use std::time::{Duration, Instant};

use kira::clock::{ClockHandle, ClockSpeed};
use kira::{AudioManager, AudioManagerSettings, Capacities};

use super::{Audio, Backend, Reverb, SMOOTH, amplitude_to_db, backend_settings};

/// Heartbeat resolution: fine enough to notice a stall quickly, cheap to run.
const PROBE_TICKS_PER_SECOND: f64 = 100.0;
/// The heartbeat may stand still this long before the output counts as lost
/// (a busy machine can hold the audio thread for a moment).
pub(crate) const STALL: Duration = Duration::from_secs(2);
/// How often a lost output is tried again.
pub(crate) const RETRY: Duration = Duration::from_secs(2);

pub(crate) struct Probe {
    clock: ClockHandle,
    pub ticks: u64,
    changed: Instant,
}

/// Whether the engine looks after a device at all, and how that goes.
#[derive(Default)]
pub(crate) struct Device {
    /// `Audio::open` wants sound; `Audio::disabled` never touches a device.
    pub wanted: bool,
    pub probe: Option<Probe>,
    pub retry_at: Option<Instant>,
    /// When the output was lost (for the log when it comes back).
    pub lost_at: Option<Instant>,
    pub reconnects: u32,
}

impl Audio {
    /// Opens a mixer on the default device with every bus, the reverb and
    /// the master volume of this `Audio`.
    pub(crate) fn connect(&mut self, now: Instant) -> Result<(), String> {
        let settings = AudioManagerSettings::<Backend> {
            capacities: Capacities {
                sub_track_capacity: 1024,
                ..Default::default()
            },
            backend_settings: backend_settings(),
            ..Default::default()
        };
        let mut manager = AudioManager::<Backend>::new(settings).map_err(|e| format!("{e:?}"))?;
        self.reverb = Reverb::open(&mut manager, self.reverb_settings);
        for bus in &mut self.buses {
            bus.open(&mut manager, self.reverb.as_ref());
        }
        manager
            .main_track()
            .set_volume(amplitude_to_db(self.master), SMOOTH);
        self.device.probe =
            match manager.add_clock(ClockSpeed::TicksPerSecond(PROBE_TICKS_PER_SECOND)) {
                Ok(mut clock) => {
                    clock.start();
                    Some(Probe {
                        clock,
                        ticks: 0,
                        changed: now,
                    })
                }
                // Without a heartbeat the output is simply trusted.
                Err(err) => {
                    log::warn!("audio: no heartbeat clock, a lost device won't be noticed: {err}");
                    None
                }
            };
        self.manager = Some(manager);
        Ok(())
    }

    /// Once a frame: notices a dead output and tries to bring it back.
    pub(crate) fn supervise(&mut self, now: Instant) {
        if !self.device.wanted {
            return;
        }
        if self.manager.is_some() {
            if let Some(probe) = &mut self.device.probe {
                let ticks = probe.clock.time().ticks;
                if ticks != probe.ticks {
                    probe.ticks = ticks;
                    probe.changed = now;
                } else if now.duration_since(probe.changed) > STALL {
                    self.lose(now);
                }
            }
            return;
        }
        if self.device.retry_at.is_some_and(|at| now < at) {
            return;
        }
        match self.connect(now) {
            Ok(()) => {
                self.device.reconnects += 1;
                let after = self
                    .device
                    .lost_at
                    .take()
                    .map(|t| format!(" after {:.1} s", now.duration_since(t).as_secs_f32()))
                    .unwrap_or_default();
                log::warn!("audio: output is back{after}; waiting loops and music restart");
                self.device.retry_at = None;
            }
            Err(err) => {
                log::debug!("audio: still no output device: {err}");
                self.device.retry_at = Some(now + RETRY);
            }
        }
    }

    /// The output stopped: drop the mixer, keep what should come back.
    pub(crate) fn lose(&mut self, now: Instant) {
        log::error!(
            "audio: the output stopped (device unplugged or the audio server went down). \
             Sounds keep their state and come back when a device does; trying every {} s",
            RETRY.as_secs()
        );
        self.manager = None;
        self.device.probe = None;
        self.reverb = None;
        for bus in &mut self.buses {
            bus.hw = None;
        }
        let music = self.music;
        for (id, p) in &mut self.playing {
            p.handle = None;
            p.occ_track = None;
            // Loops and music wait for a voice; a one-shot is gone with its
            // device (the frame update forgets it and clears its source).
            p.virtualized = p.params.looping || Some(*id) == music;
        }
        self.device.lost_at = Some(now);
        self.device.retry_at = Some(now + RETRY);
    }

    /// Times the output came back after being lost (or appeared after a start
    /// without a device).
    pub fn reconnect_count(&self) -> u32 {
        self.device.reconnects
    }
}

//! Mixer buses: kira sub-tracks with an EQ each, plus one shared reverb send.
//!
//! The settings live here as plain data, so they can be read back and set
//! without an audio device; with a device they are mirrored onto kira tracks.

use kira::effect::eq_filter::{EqFilterBuilder, EqFilterHandle, EqFilterKind};
use kira::effect::reverb::{ReverbBuilder, ReverbHandle};
use kira::track::{SendTrackBuilder, SendTrackHandle, TrackBuilder, TrackHandle};
use kira::{AudioManager, Decibels, Mix};

use super::{Backend, SMOOTH, amplitude_to_db};

/// A mixer bus. Sounds are routed to one through [`PlayParams::bus`](crate::PlayParams::bus);
/// every bus feeds the master volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BusId(pub(crate) usize);

impl BusId {
    /// Effects and general one-shots (the default bus).
    pub const SFX: BusId = BusId(0);
    pub const MUSIC: BusId = BusId(1);
    pub const AMBIENT: BusId = BusId(2);
    pub const VOICE: BusId = BusId(3);
}

/// Names of the buses every [`Audio`](crate::Audio) starts with, by [`BusId`] index.
pub(crate) const DEFAULT_BUSES: [&str; 4] = ["sfx", "music", "ambient", "voice"];

/// Gain cap of every EQ band, in dB.
pub const EQ_MAX_DB: f32 = 24.0;

/// Three-band equalizer of a bus: low shelf (250 Hz), bell (1 kHz), high
/// shelf (4 kHz). 0 dB everywhere is a flat response.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EqSettings {
    pub low_db: f32,
    pub mid_db: f32,
    pub high_db: f32,
}

impl EqSettings {
    fn clamped(self) -> Self {
        let c = |v: f32| v.clamp(-EQ_MAX_DB, EQ_MAX_DB);
        Self {
            low_db: c(self.low_db),
            mid_db: c(self.mid_db),
            high_db: c(self.high_db),
        }
    }
}

const EQ_BANDS: [(EqFilterKind, f64, f64); 3] = [
    (EqFilterKind::LowShelf, 250.0, 0.7),
    (EqFilterKind::Bell, 1000.0, 0.7),
    (EqFilterKind::HighShelf, 4000.0, 0.7),
];

/// The shared reverb every bus can send to (see [`Audio::set_bus_reverb`](crate::Audio::set_bus_reverb)).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbSettings {
    /// Size of the room, 0..1; near 1 the tail lasts very long.
    pub feedback: f32,
    /// How fast high frequencies die in the tail, 0..1.
    pub damping: f32,
    /// 0 is mono, 1 fully stereo.
    pub stereo_width: f32,
}

impl Default for ReverbSettings {
    fn default() -> Self {
        Self {
            feedback: 0.8,
            damping: 0.5,
            stereo_width: 1.0,
        }
    }
}

impl ReverbSettings {
    fn clamped(self) -> Self {
        Self {
            feedback: self.feedback.clamp(0.0, 0.98),
            damping: self.damping.clamp(0.0, 1.0),
            stereo_width: self.stereo_width.clamp(0.0, 1.0),
        }
    }
}

pub(crate) struct BusHw {
    pub track: TrackHandle,
    eq: [EqFilterHandle; 3],
}

pub(crate) struct Bus {
    pub name: String,
    pub volume: f32,
    pub muted: bool,
    pub eq: EqSettings,
    /// Amount sent to the reverb, 0..1.
    pub reverb_send: f32,
    /// Held by the game pause (all but music by default).
    pub pausable: bool,
    pub hw: Option<BusHw>,
}

impl Bus {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            volume: 1.0,
            muted: false,
            eq: EqSettings::default(),
            reverb_send: 0.0,
            pausable: name != DEFAULT_BUSES[BusId::MUSIC.0],
            hw: None,
        }
    }

    /// Creates the kira track under the main track.
    pub fn open(&mut self, manager: &mut AudioManager<Backend>, reverb: Option<&Reverb>) {
        let mut builder = TrackBuilder::new()
            .sub_track_capacity(OCCLUSION_TRACKS_PER_BUS)
            .sound_capacity(256);
        if let Some(r) = reverb {
            builder = builder.with_send(r.send.id(), amplitude_to_db(self.reverb_send));
        }
        let [lo, mid, hi] = EQ_BANDS.map(|(kind, freq, q)| {
            builder.add_effect(EqFilterBuilder::new(kind, freq, Decibels::IDENTITY, q))
        });
        match manager.add_sub_track(builder) {
            Ok(track) => {
                let mut hw = BusHw {
                    track,
                    eq: [lo, mid, hi],
                };
                self.push_volume(&mut hw);
                self.push_eq(&mut hw);
                self.hw = Some(hw);
            }
            Err(err) => log::warn!("audio: bus {}: {err}", self.name),
        }
    }

    fn push_volume(&self, hw: &mut BusHw) {
        let v = if self.muted { 0.0 } else { self.volume };
        hw.track.set_volume(amplitude_to_db(v), SMOOTH);
    }

    fn push_eq(&self, hw: &mut BusHw) {
        let gains = [self.eq.low_db, self.eq.mid_db, self.eq.high_db];
        for (band, db) in hw.eq.iter_mut().zip(gains) {
            band.set_gain(Decibels(db), SMOOTH);
        }
    }

    pub fn apply_volume(&mut self) {
        if let Some(mut hw) = self.hw.take() {
            self.push_volume(&mut hw);
            self.hw = Some(hw);
        }
    }

    pub fn set_eq(&mut self, eq: EqSettings) {
        self.eq = eq.clamped();
        if let Some(mut hw) = self.hw.take() {
            self.push_eq(&mut hw);
            self.hw = Some(hw);
        }
    }

    pub fn set_reverb_send(&mut self, level: f32, reverb: Option<&Reverb>) {
        self.reverb_send = level.clamp(0.0, 1.0);
        if let (Some(hw), Some(r)) = (&mut self.hw, reverb) {
            let _ = hw
                .track
                .set_send(r.send.id(), amplitude_to_db(self.reverb_send), SMOOTH);
        }
    }
}

/// Occlusion tracks one bus can host (every positional sound gets one).
pub(crate) const OCCLUSION_TRACKS_PER_BUS: usize = 256;

/// The reverb send track, fully wet: buses decide how much goes into it.
pub(crate) struct Reverb {
    send: SendTrackHandle,
    effect: ReverbHandle,
}

impl Reverb {
    pub fn open(manager: &mut AudioManager<Backend>, settings: ReverbSettings) -> Option<Self> {
        let mut builder = SendTrackBuilder::new();
        let effect = builder.add_effect(
            ReverbBuilder::new()
                .feedback(settings.feedback as f64)
                .damping(settings.damping as f64)
                .stereo_width(settings.stereo_width as f64)
                .mix(Mix::WET),
        );
        match manager.add_send_track(builder) {
            Ok(send) => Some(Self { send, effect }),
            Err(err) => {
                log::warn!("audio: reverb: {err}");
                None
            }
        }
    }

    pub fn apply(&mut self, settings: ReverbSettings) {
        let s = settings.clamped();
        self.effect.set_feedback(s.feedback as f64, SMOOTH);
        self.effect.set_damping(s.damping as f64, SMOOTH);
        self.effect.set_stereo_width(s.stereo_width as f64, SMOOTH);
    }
}

pub(crate) fn clamp_reverb(settings: ReverbSettings) -> ReverbSettings {
    settings.clamped()
}

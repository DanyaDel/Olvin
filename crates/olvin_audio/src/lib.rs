//! Audio on kira. Entities with an `AudioSourceDesc` (from scenes or code)
//! play their clip; the entity with [`AudioListener`] is the ears. The
//! [`Audio`] resource plays one-shots and streamed music from code.
//!
//! 3D sound is done by hand: every frame each positional sound gets a volume
//! (distance attenuation between min/max distance) and a stereo pan from the
//! listener's `GlobalTransform`, tweened over a few milliseconds. There are no
//! mixer-track limits and it works the same on every backend.
//!
//! Mixing: every sound plays on a [`BusId`] (sfx, music, ambient, voice, or
//! your own through [`Audio::add_bus`]) with its own volume, mute and 3-band
//! EQ, and each bus can send into one shared reverb. Positional sounds get
//! occlusion: [`Audio::set_occluder`] takes a function that says how blocked
//! the way from the sound to the listener is (e.g. a physics ray cast), and
//! the sound is quieted and low-passed by it.
//!
//! Voices: at most [`Audio::max_voices`] sounds play at once. The rest, ranked
//! by [`PlayParams::priority`] and then by how loud they would be, are
//! virtualized: looping ones wait silently (and restart from the beginning
//! when they win a voice back), one-shots are dropped. Streamed files are
//! limited separately by [`Audio::max_streams`].
//!
//! Pause: while `Time` is paused (`Time::pause`, e.g. a pause menu), sounds
//! on pausable buses (all but `music` by default, see
//! [`Audio::set_bus_pausable`]) hold where they are and continue on resume.
//! Sounds started during the pause (menu clicks) play normally. Single sounds
//! can be held with [`Audio::pause_sound`].
//!
//! Without an audio device (CI, servers, headless tests) the plugin logs once
//! and every call becomes a no-op; the bookkeeping (ids, which sources started)
//! still works so gameplay code doesn't have to care.

mod device;
mod frame;
mod mixer;
mod mixing;
mod pause;
mod playback;
mod sources;
mod spatial;

pub use mixer::{BusId, EQ_MAX_DB, EqSettings, ReverbSettings};
pub use olvin_scene::file::AudioSourceDesc;
pub use spatial::{attenuation, occlusion_cutoff, occlusion_gain, pan};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use kira::PlaySoundError;
use kira::effect::filter::FilterHandle;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::streaming::StreamingSoundHandle;
use kira::sound::{FromFileError, PlaybackState, SoundData};
use kira::track::TrackHandle;
use kira::{AudioManager, Decibels, Tween};
use olvin_core::{App, Plugin, ProjectPaths, Resources, Stage, Time};
use olvin_ecs::{EcsPlugin, Entity, World};
use olvin_scene::Transform;
use olvin_transform::GlobalTransform;

use mixer::{Bus, Reverb};

/// The mixer backend: the sound card, or in tests a mock that renders on demand.
#[cfg(not(test))]
type Backend = kira::DefaultBackend;
#[cfg(test)]
type Backend = kira::backend::mock::MockBackend;

#[cfg(not(test))]
fn backend_settings() -> <Backend as kira::backend::Backend>::Settings {
    Default::default()
}

#[cfg(test)]
fn backend_settings() -> <Backend as kira::backend::Backend>::Settings {
    kira::backend::mock::MockBackendSettings {
        sample_rate: 44_100,
    }
}

/// Marks the entity that hears audio (usually the camera).
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioListener;

/// Added to an entity once its `AudioSourceDesc` was picked up.
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioSourceState {
    /// The sound it plays right now (from play-on-awake or [`Audio::play_source`]).
    pub sound: Option<SoundId>,
}

/// A sound started through [`Audio`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SoundId(u64);

/// How to play a sound. `position` makes it 3D at a fixed point.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayParams {
    pub volume: f32,
    pub pitch: f32,
    pub looping: bool,
    pub position: Option<Vec3>,
    pub min_distance: f32,
    pub max_distance: f32,
    /// Stream from disk instead of decoding the whole file first (long music).
    pub stream: bool,
    /// Mixer bus the sound plays on.
    pub bus: BusId,
    /// Who keeps a voice when there are too many sounds: 0 is the most
    /// important, 255 the least (like Unity's `AudioSource.priority`).
    pub priority: u8,
    /// Whether the sound is muffled by obstacles (positional sounds only).
    /// Costs a mixer track per sound.
    pub occludable: bool,
}

/// Priority of everything that doesn't say otherwise.
pub const DEFAULT_PRIORITY: u8 = 128;

impl Default for PlayParams {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pitch: 1.0,
            looping: false,
            position: None,
            min_distance: 1.0,
            max_distance: 50.0,
            stream: false,
            bus: BusId::SFX,
            priority: DEFAULT_PRIORITY,
            occludable: true,
        }
    }
}

impl PlayParams {
    pub fn at(position: Vec3) -> Self {
        Self {
            position: Some(position),
            ..Default::default()
        }
    }
}

/// Files bigger than this are streamed when played from an `AudioSourceDesc`
/// (see [`Audio::set_stream_threshold`]).
pub const STREAM_THRESHOLD_BYTES: u64 = 2 * 1024 * 1024;
/// Default for [`Audio::max_voices`].
pub const DEFAULT_MAX_VOICES: usize = 64;
/// Default for [`Audio::max_streams`].
pub const DEFAULT_MAX_STREAMS: usize = 8;

#[derive(Clone, Debug)]
pub struct AudioPlugin {
    /// Clip paths are relative to this folder. `None`: the project's assets
    /// (`ProjectPaths::assets`); a relative path is inside the project folder.
    pub assets_dir: Option<PathBuf>,
    /// `false` never opens a device (tools, tests); same as having none.
    pub enabled: bool,
}

impl Default for AudioPlugin {
    fn default() -> Self {
        Self {
            assets_dir: None,
            enabled: true,
        }
    }
}

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        let dir = {
            let paths = app.resources.fetch::<ProjectPaths>();
            match &self.assets_dir {
                Some(dir) => paths.resolve(dir),
                None => paths.assets.clone(),
            }
        };
        let audio = if self.enabled {
            Audio::open(dir)
        } else {
            Audio::disabled(dir)
        };
        // After transforms are propagated (PostUpdate), before the next frame.
        app.add_plugin(EcsPlugin)
            .insert_resource(audio)
            .add_system(Stage::Last, update);
    }
}

enum Handle {
    Static(StaticSoundHandle),
    Streaming(StreamingSoundHandle<FromFileError>),
}

impl Handle {
    fn state(&self) -> PlaybackState {
        match self {
            Self::Static(h) => h.state(),
            Self::Streaming(h) => h.state(),
        }
    }

    fn position(&self) -> f64 {
        match self {
            Self::Static(h) => h.position(),
            Self::Streaming(h) => h.position(),
        }
    }

    fn set_volume(&mut self, amplitude: f32, tween: Tween) {
        let db = amplitude_to_db(amplitude);
        match self {
            Self::Static(h) => h.set_volume(db, tween),
            Self::Streaming(h) => h.set_volume(db, tween),
        }
    }

    fn set_panning(&mut self, pan: f32, tween: Tween) {
        match self {
            Self::Static(h) => h.set_panning(pan, tween),
            Self::Streaming(h) => h.set_panning(pan, tween),
        }
    }

    fn set_pitch(&mut self, pitch: f32, tween: Tween) {
        let rate = kira::PlaybackRate(pitch.max(0.01) as f64);
        match self {
            Self::Static(h) => h.set_playback_rate(rate, tween),
            Self::Streaming(h) => h.set_playback_rate(rate, tween),
        }
    }

    fn pause(&mut self, tween: Tween) {
        match self {
            Self::Static(h) => h.pause(tween),
            Self::Streaming(h) => h.pause(tween),
        }
    }

    fn resume(&mut self, tween: Tween) {
        match self {
            Self::Static(h) => h.resume(tween),
            Self::Streaming(h) => h.resume(tween),
        }
    }

    fn stop(&mut self, tween: Tween) {
        match self {
            Self::Static(h) => h.stop(tween),
            Self::Streaming(h) => h.stop(tween),
        }
    }

    /// A decoding error of a streamed file, if one happened since the last call.
    fn pop_error(&mut self) -> Option<String> {
        match self {
            Self::Static(_) => None,
            Self::Streaming(h) => h.pop_error().map(|e| e.to_string()),
        }
    }
}

/// A sound's own mixer track with a low-pass filter, for occlusion.
struct OcclusionTrack {
    _track: TrackHandle,
    filter: FilterHandle,
}

#[derive(Default)]
struct Occlusion {
    /// Set by [`Audio::set_occlusion`]; otherwise the occluder function decides.
    manual: Option<f32>,
    /// Eased value that is actually applied, 0 clear .. 1 blocked.
    current: f32,
    initialized: bool,
}

struct Playing {
    path: PathBuf,
    handle: Option<Handle>,
    /// No voice right now: waits for one (looping sounds only).
    virtualized: bool,
    params: PlayParams,
    /// The source entity it follows (position, live volume/pitch).
    entity: Option<Entity>,
    occlusion: Occlusion,
    occ_track: Option<OcclusionTrack>,
    /// Held by [`Audio::pause_sound`].
    user_paused: bool,
    /// Started while the game was paused: plays through that pause.
    pause_exempt: bool,
    /// The voice is paused right now (by the user or the game pause).
    held: bool,
}

type Occluder = Box<dyn FnMut(Vec3, Vec3) -> f32>;

/// The audio engine: one-shots, music, sources. Every method is safe to call
/// without a device (it then does nothing).
pub struct Audio {
    manager: Option<AudioManager<Backend>>,
    assets: PathBuf,
    clips: HashMap<PathBuf, StaticSoundData>,
    playing: HashMap<SoundId, Playing>,
    next_id: u64,
    music: Option<SoundId>,
    master: f32,
    listener: (Vec3, Quat),
    warned: HashSet<String>,
    buses: Vec<Bus>,
    reverb: Option<Reverb>,
    reverb_settings: ReverbSettings,
    occluder: Option<Occluder>,
    max_voices: usize,
    max_streams: usize,
    stream_threshold: u64,
    last_update: Option<Instant>,
    /// Fade-in of the sound being started (music crossfades).
    fade_in: Duration,
    /// Game pause (see [`Audio::set_paused`]).
    paused: bool,
    follow_time: bool,
    /// Watching and reconnecting the output device.
    device: device::Device,
}

fn amplitude_to_db(amplitude: f32) -> Decibels {
    // NaN must not reach the mixer as NaN dB.
    if amplitude.is_nan() || amplitude <= 0.001 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * amplitude.log10())
    }
}

/// Short ramp for per-frame changes: no zipper noise, no audible lag.
const SMOOTH: Tween = Tween {
    start_time: kira::StartTime::Immediate,
    duration: Duration::from_millis(30),
    easing: kira::Easing::Linear,
};

fn fade(duration: Duration) -> Tween {
    Tween { duration, ..SMOOTH }
}

/// How fast the applied occlusion follows the measured one (1/s): ray casts
/// flicker at doorways, ears shouldn't.
const OCCLUSION_RATE: f32 = 8.0;

impl Audio {
    /// Opens the default output device. Without one it stays silent and keeps
    /// looking; a device lost later is reconnected the same way (see `device.rs`).
    pub fn open(assets: impl Into<PathBuf>) -> Self {
        let mut audio = Self::disabled(assets);
        audio.device.wanted = true;
        let now = Instant::now();
        if let Err(err) = audio.connect(now) {
            log::warn!(
                "audio: no output device yet ({err}); silent until one appears (checked every {} s)",
                device::RETRY.as_secs()
            );
            audio.device.lost_at = Some(now);
            audio.device.retry_at = Some(now + device::RETRY);
        }
        audio
    }

    /// Never touches a device.
    pub fn disabled(assets: impl Into<PathBuf>) -> Self {
        Self {
            manager: None,
            assets: assets.into(),
            clips: HashMap::new(),
            playing: HashMap::new(),
            next_id: 0,
            music: None,
            master: 1.0,
            listener: (Vec3::ZERO, Quat::IDENTITY),
            warned: HashSet::new(),
            buses: mixer::DEFAULT_BUSES.iter().map(|n| Bus::new(n)).collect(),
            reverb: None,
            reverb_settings: ReverbSettings::default(),
            occluder: None,
            max_voices: DEFAULT_MAX_VOICES,
            max_streams: DEFAULT_MAX_STREAMS,
            stream_threshold: STREAM_THRESHOLD_BYTES,
            last_update: None,
            fade_in: Duration::ZERO,
            paused: false,
            follow_time: true,
            device: device::Device::default(),
        }
    }

    /// Whether sound actually comes out.
    pub fn is_available(&self) -> bool {
        self.manager.is_some()
    }

    pub fn assets_dir(&self) -> &Path {
        &self.assets
    }
}

/// Plays on the given mixer track, or on the main one.
fn play_on<D: SoundData>(
    manager: &mut AudioManager<Backend>,
    track: Option<&mut TrackHandle>,
    data: D,
) -> Result<D::Handle, String>
where
    PlaySoundError<D::Error>: std::fmt::Display,
{
    match track {
        Some(t) => t.play(data).map_err(|e| e.to_string()),
        None => manager.play(data).map_err(|e| e.to_string()),
    }
}

fn world_position(world: &World, e: Entity) -> Option<Vec3> {
    world
        .get::<&GlobalTransform>(e)
        .map(|g| g.0.w_axis.truncate())
        .ok()
        .or_else(|| world.get::<&Transform>(e).map(|t| t.translation).ok())
}

/// The plugin's per-frame system: follows the `Time` pause, then [`Audio::update`].
pub fn update(res: &mut Resources) {
    let mut audio = res.fetch_mut::<Audio>();
    if audio.follow_time
        && let Some(time) = res.get::<Time>()
    {
        audio.set_paused(time.paused());
    }
    let mut world = res.fetch_mut::<World>();
    audio.update(&mut world);
}

#[cfg(test)]
mod tests;

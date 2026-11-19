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
//! Without an audio device (CI, servers, headless tests) the plugin logs once
//! and every call becomes a no-op; the bookkeeping (ids, which sources started)
//! still works so gameplay code doesn't have to care.

mod mixer;
mod spatial;

pub use mixer::{BusId, EQ_MAX_DB, EqSettings, ReverbSettings};
pub use olvin_scene::file::AudioSourceDesc;
pub use spatial::{attenuation, occlusion_cutoff, occlusion_gain, pan};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use kira::PlaySoundError;
use kira::effect::filter::{FilterBuilder, FilterHandle, FilterMode};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{FromFileError, PlaybackState, SoundData};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Capacities, Decibels, Mix, Tween};
use olvin_core::{App, Plugin, Resources, Stage};
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

pub struct AudioPlugin {
    /// Clip paths are relative to this folder.
    pub assets_dir: PathBuf,
    /// `false` never opens a device (tools, tests); same as having none.
    pub enabled: bool,
}

impl Default for AudioPlugin {
    fn default() -> Self {
        Self {
            assets_dir: PathBuf::from("assets"),
            enabled: true,
        }
    }
}

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        let audio = if self.enabled {
            Audio::open(&self.assets_dir)
        } else {
            Audio::disabled(&self.assets_dir)
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
}

fn amplitude_to_db(amplitude: f32) -> Decibels {
    if amplitude <= 0.001 {
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
    /// Opens the default output device; without one, logs once and stays silent.
    pub fn open(assets: impl Into<PathBuf>) -> Self {
        let mut audio = Self::disabled(assets);
        let settings = AudioManagerSettings::<Backend> {
            capacities: Capacities {
                sub_track_capacity: 1024,
                ..Default::default()
            },
            backend_settings: backend_settings(),
            ..Default::default()
        };
        match AudioManager::<Backend>::new(settings) {
            Ok(mut manager) => {
                audio.reverb = Reverb::open(&mut manager, audio.reverb_settings);
                for bus in &mut audio.buses {
                    bus.open(&mut manager, audio.reverb.as_ref());
                }
                audio.manager = Some(manager);
            }
            Err(err) => log::warn!("audio disabled, no output device: {err:?}"),
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
        }
    }

    /// Whether sound actually comes out.
    pub fn is_available(&self) -> bool {
        self.manager.is_some()
    }

    pub fn assets_dir(&self) -> &Path {
        &self.assets
    }

    /// Plays a 2D sound once at full volume. `path` is relative to `assets/`.
    pub fn play(&mut self, path: impl AsRef<Path>) -> SoundId {
        self.play_with(path, PlayParams::default())
    }

    /// Plays a one-shot 3D sound at a point.
    pub fn play_at(&mut self, path: impl AsRef<Path>, position: Vec3) -> SoundId {
        self.play_with(path, PlayParams::at(position))
    }

    pub fn play_with(&mut self, path: impl AsRef<Path>, params: PlayParams) -> SoundId {
        self.start(path.as_ref(), params, None)
    }

    /// Streams a long file (music) as the single music track, replacing the previous one.
    pub fn play_music(&mut self, path: impl AsRef<Path>, volume: f32, looping: bool) -> SoundId {
        self.play_music_fade(path, volume, looping, Duration::ZERO)
    }

    /// Like [`play_music`](Self::play_music), crossfading: the old track fades
    /// out and the new one in over `fade`.
    pub fn play_music_fade(
        &mut self,
        path: impl AsRef<Path>,
        volume: f32,
        looping: bool,
        fade: Duration,
    ) -> SoundId {
        if let Some(old) = self.music.take() {
            self.stop_fade(old, fade);
        }
        let id = self.start_fading(
            path.as_ref(),
            PlayParams {
                volume,
                looping,
                stream: true,
                bus: BusId::MUSIC,
                priority: 0,
                occludable: false,
                ..Default::default()
            },
            None,
            fade,
        );
        self.music = Some(id);
        id
    }

    pub fn stop_music(&mut self) {
        if let Some(id) = self.music.take() {
            self.stop(id);
        }
    }

    /// The music track, if one is playing.
    pub fn music(&self) -> Option<SoundId> {
        self.music
    }

    pub fn stop(&mut self, id: SoundId) {
        self.stop_fade(id, SMOOTH.duration);
    }

    /// Stops a sound, fading it out over `fade`.
    pub fn stop_fade(&mut self, id: SoundId, fade: Duration) {
        if let Some(mut p) = self.playing.remove(&id) {
            if let Some(h) = &mut p.handle {
                h.stop(self::fade(fade));
            }
            if self.music == Some(id) {
                self.music = None;
            }
        }
    }

    pub fn stop_all(&mut self) {
        let ids: Vec<SoundId> = self.playing.keys().copied().collect();
        for id in ids {
            self.stop(id);
        }
        self.music = None;
    }

    /// Volume of one sound (0..1+, linear).
    pub fn set_volume(&mut self, id: SoundId, volume: f32) {
        if let Some(p) = self.playing.get_mut(&id) {
            p.params.volume = volume;
        }
    }

    pub fn set_pitch(&mut self, id: SoundId, pitch: f32) {
        if let Some(p) = self.playing.get_mut(&id) {
            p.params.pitch = pitch;
            if let Some(h) = &mut p.handle {
                h.set_pitch(pitch, SMOOTH);
            }
        }
    }

    /// Moves a positional sound.
    pub fn set_position(&mut self, id: SoundId, position: Vec3) {
        if let Some(p) = self.playing.get_mut(&id) {
            p.params.position = Some(position);
        }
    }

    /// Changes who wins a voice when there are too many sounds (0 = most important).
    pub fn set_priority(&mut self, id: SoundId, priority: u8) {
        if let Some(p) = self.playing.get_mut(&id) {
            p.params.priority = priority;
            self.rebalance();
        }
    }

    pub fn master_volume(&self) -> f32 {
        self.master
    }

    pub fn set_master_volume(&mut self, volume: f32) {
        self.master = volume.max(0.0);
        if let Some(m) = &mut self.manager {
            m.main_track()
                .set_volume(amplitude_to_db(self.master), SMOOTH);
        }
    }

    /// Whether the sound is still going (always `false` without a device).
    /// A virtualized sound (no voice) isn't playing, but stays tracked.
    pub fn is_playing(&self, id: SoundId) -> bool {
        self.playing
            .get(&id)
            .and_then(|p| p.handle.as_ref())
            .is_some_and(|h| !matches!(h.state(), PlaybackState::Stopped))
    }

    /// Sounds being tracked (playing, virtualized, or started without a device).
    pub fn active_count(&self) -> usize {
        self.playing.len()
    }

    /// Tracked sounds that hold a voice (or would, without a device).
    pub fn voice_count(&self) -> usize {
        self.playing.values().filter(|p| !p.virtualized).count()
    }

    /// Whether the sound is waiting for a voice (see [`max_voices`](Self::max_voices)).
    pub fn is_virtual(&self, id: SoundId) -> bool {
        self.playing.get(&id).is_some_and(|p| p.virtualized)
    }

    /// Streamed sounds holding a voice.
    pub fn stream_count(&self) -> usize {
        self.playing
            .values()
            .filter(|p| !p.virtualized && p.params.stream)
            .count()
    }

    /// Playback position in seconds (`None` without a device or voice).
    pub fn position(&self, id: SoundId) -> Option<f64> {
        self.playing.get(&id)?.handle.as_ref().map(Handle::position)
    }

    pub fn max_voices(&self) -> usize {
        self.max_voices
    }

    /// Sounds that may play at once (at least 1); the rest wait or are dropped.
    pub fn set_max_voices(&mut self, max: usize) {
        self.max_voices = max.max(1);
        self.rebalance();
    }

    pub fn max_streams(&self) -> usize {
        self.max_streams
    }

    /// How many files may be streamed at once; every stream is a decoder
    /// reading from disk. Streams over the limit count as having no voice.
    pub fn set_max_streams(&mut self, max: usize) {
        self.max_streams = max;
        self.rebalance();
    }

    /// Files bigger than this (bytes) are streamed when played from an
    /// `AudioSourceDesc`; smaller ones are decoded once and cached.
    pub fn set_stream_threshold(&mut self, bytes: u64) {
        self.stream_threshold = bytes;
    }

    pub fn stream_threshold(&self) -> u64 {
        self.stream_threshold
    }

    /// Decodes a clip ahead of time so its first play doesn't stall.
    pub fn preload(&mut self, path: impl AsRef<Path>) -> bool {
        self.manager.is_some() && self.clip(path.as_ref()).is_some()
    }

    /// Clips decoded and kept in memory.
    pub fn cached_clips(&self) -> usize {
        self.clips.len()
    }

    /// Forgets a decoded clip (playing instances finish); returns whether it was cached.
    pub fn unload_clip(&mut self, path: impl AsRef<Path>) -> bool {
        self.clips.remove(path.as_ref()).is_some()
    }

    /// Forgets every decoded clip.
    pub fn clear_clips(&mut self) {
        self.clips.clear();
    }

    // Mixer.

    /// A bus by name (`sfx`, `music`, `ambient`, `voice`, or one you added).
    pub fn bus(&self, name: &str) -> Option<BusId> {
        self.buses.iter().position(|b| b.name == name).map(BusId)
    }

    /// Creates a bus (or returns the one with that name).
    pub fn add_bus(&mut self, name: &str) -> BusId {
        if let Some(id) = self.bus(name) {
            return id;
        }
        let mut bus = Bus::new(name);
        if let Some(m) = &mut self.manager {
            bus.open(m, self.reverb.as_ref());
        }
        self.buses.push(bus);
        BusId(self.buses.len() - 1)
    }

    pub fn bus_name(&self, bus: BusId) -> Option<&str> {
        self.buses.get(bus.0).map(|b| b.name.as_str())
    }

    pub fn bus_volume(&self, bus: BusId) -> f32 {
        self.buses.get(bus.0).map_or(0.0, |b| b.volume)
    }

    /// Volume of everything on the bus (linear).
    pub fn set_bus_volume(&mut self, bus: BusId, volume: f32) {
        if let Some(b) = self.buses.get_mut(bus.0) {
            b.volume = volume.max(0.0);
            b.apply_volume();
        }
    }

    pub fn is_bus_muted(&self, bus: BusId) -> bool {
        self.buses.get(bus.0).is_some_and(|b| b.muted)
    }

    /// Silences a bus without losing its volume setting.
    pub fn set_bus_muted(&mut self, bus: BusId, muted: bool) {
        if let Some(b) = self.buses.get_mut(bus.0) {
            b.muted = muted;
            b.apply_volume();
        }
    }

    pub fn bus_eq(&self, bus: BusId) -> EqSettings {
        self.buses.get(bus.0).map(|b| b.eq).unwrap_or_default()
    }

    /// Equalizer of the bus; gains are clamped to ±[`EQ_MAX_DB`].
    pub fn set_bus_eq(&mut self, bus: BusId, eq: EqSettings) {
        if let Some(b) = self.buses.get_mut(bus.0) {
            b.set_eq(eq);
        }
    }

    pub fn reverb(&self) -> ReverbSettings {
        self.reverb_settings
    }

    /// Room of the shared reverb (all buses that send into it).
    pub fn set_reverb(&mut self, settings: ReverbSettings) {
        self.reverb_settings = mixer::clamp_reverb(settings);
        if let Some(r) = &mut self.reverb {
            r.apply(self.reverb_settings);
        }
    }

    pub fn bus_reverb(&self, bus: BusId) -> f32 {
        self.buses.get(bus.0).map_or(0.0, |b| b.reverb_send)
    }

    /// How much of the bus goes into the reverb, 0 (none) .. 1.
    pub fn set_bus_reverb(&mut self, bus: BusId, level: f32) {
        if let Some(b) = self.buses.get_mut(bus.0) {
            b.set_reverb_send(level, self.reverb.as_ref());
        }
    }

    // Occlusion.

    /// Tells the mixer how blocked the way between two points is: from the
    /// listener to a sound, 0 (clear) .. 1 (fully behind walls). Typically a
    /// physics ray cast; partial values suit thin or partial obstacles. Called
    /// once per frame for every positional occludable sound with a voice.
    pub fn set_occluder(&mut self, occluder: impl FnMut(Vec3, Vec3) -> f32 + 'static) {
        self.occluder = Some(Box::new(occluder));
    }

    pub fn clear_occluder(&mut self) {
        self.occluder = None;
    }

    /// Sets a sound's occlusion by hand (0..1), overriding the occluder;
    /// `None` gives control back to it.
    pub fn set_occlusion(&mut self, id: SoundId, occlusion: Option<f32>) {
        if let Some(p) = self.playing.get_mut(&id) {
            p.occlusion.manual = occlusion.map(|o| o.clamp(0.0, 1.0));
        }
    }

    /// The occlusion currently applied to a sound (eased), 0..1.
    pub fn occlusion(&self, id: SoundId) -> Option<f32> {
        self.playing.get(&id).map(|p| p.occlusion.current)
    }

    // Sources.

    /// Plays an entity's `AudioSourceDesc` (Unity's `AudioSource.Play`).
    pub fn play_source(&mut self, world: &mut World, entity: Entity) -> Option<SoundId> {
        let desc = AudioSourceDesc::clone(&*world.get::<&AudioSourceDesc>(entity).ok()?);
        if let Some(old) = world
            .get::<&AudioSourceState>(entity)
            .ok()
            .and_then(|s| s.sound)
        {
            self.stop(old);
        }
        let position = world_position(world, entity);
        let stream = std::fs::metadata(self.assets.join(&desc.clip))
            .is_ok_and(|m| m.len() > self.stream_threshold);
        let params = PlayParams {
            volume: desc.volume,
            pitch: desc.pitch,
            looping: desc.looping,
            position: desc.spatial.then_some(position.unwrap_or(Vec3::ZERO)),
            min_distance: desc.min_distance,
            max_distance: desc.max_distance,
            stream,
            ..Default::default()
        };
        let id = self.start(Path::new(&desc.clip), params, Some(entity));
        // Dropped on the spot for lack of a voice: nothing to point at.
        let sound = self.playing.contains_key(&id).then_some(id);
        let _ = world.insert_one(entity, AudioSourceState { sound });
        Some(id)
    }

    pub fn stop_source(&mut self, world: &mut World, entity: Entity) {
        if let Ok(mut s) = world.get::<&mut AudioSourceState>(entity)
            && let Some(id) = s.sound.take()
        {
            self.stop(id);
        }
    }

    fn start(&mut self, path: &Path, params: PlayParams, entity: Option<Entity>) -> SoundId {
        self.start_fading(path, params, entity, Duration::ZERO)
    }

    fn start_fading(
        &mut self,
        path: &Path,
        params: PlayParams,
        entity: Option<Entity>,
        fade_in: Duration,
    ) -> SoundId {
        let id = SoundId(self.next_id);
        self.next_id += 1;
        self.fade_in = fade_in;
        self.playing.insert(
            id,
            Playing {
                path: path.to_owned(),
                handle: None,
                virtualized: true,
                params,
                entity,
                occlusion: Occlusion::default(),
                occ_track: None,
            },
        );
        // Takes a voice if there is one (or steals it from a worse sound).
        self.rebalance();
        self.fade_in = Duration::ZERO;
        id
    }

    /// Gives `id` a voice: starts it on the mixer.
    fn activate(&mut self, id: SoundId) {
        let Some(p) = self.playing.get(&id) else {
            return;
        };
        let gain = self.gain(&p.params, p.occlusion.current);
        let (path, params) = (p.path.clone(), p.params.clone());
        let started = if self.manager.is_some() {
            self.start_handle(&path, &params, gain)
        } else {
            None
        };
        if let Some(p) = self.playing.get_mut(&id) {
            p.virtualized = false;
            if let Some((handle, track)) = started {
                p.handle = Some(handle);
                p.occ_track = track;
            }
        }
    }

    /// Takes the voice away. One-shots are gone for good, loops wait.
    fn deactivate(&mut self, id: SoundId) -> Option<Option<Entity>> {
        let p = self.playing.get_mut(&id)?;
        if let Some(h) = &mut p.handle {
            h.stop(SMOOTH);
        }
        if p.params.looping {
            p.handle = None;
            p.occ_track = None;
            p.virtualized = true;
            return None;
        }
        let entity = p.entity;
        self.playing.remove(&id);
        if self.music == Some(id) {
            self.music = None;
        }
        Some(entity)
    }

    /// Hands the voices to the most important sounds and takes them from the
    /// rest. Returns the entities of one-shots that lost theirs for good.
    fn rebalance(&mut self) -> Vec<Entity> {
        let mut ranked: Vec<(SoundId, u8, f32)> = self
            .playing
            .iter()
            .map(|(id, p)| {
                let loudness = self.gain(&p.params, p.occlusion.current).0;
                (*id, p.params.priority, loudness)
            })
            .collect();
        // Important first, then loud, then new (a fresh hit beats an old one).
        ranked.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then(b.2.total_cmp(&a.2))
                .then(b.0.0.cmp(&a.0.0))
        });
        let mut voices = 0;
        let mut streams = 0;
        let mut dropped = Vec::new();
        for (id, _, _) in ranked {
            let stream = self.playing[&id].params.stream;
            let fits = voices < self.max_voices && (!stream || streams < self.max_streams);
            if fits {
                voices += 1;
                streams += usize::from(stream);
                if self.playing[&id].virtualized {
                    self.activate(id);
                }
            } else if (!self.playing[&id].virtualized || !self.playing[&id].params.looping)
                && let Some(Some(entity)) = self.deactivate(id)
            {
                dropped.push(entity);
            }
        }
        dropped
    }

    /// Output volume and pan of a sound for the current listener, `occlusion` included.
    fn gain(&self, p: &PlayParams, occlusion: f32) -> (f32, f32) {
        match p.position {
            Some(at) => {
                let (lp, lr) = self.listener;
                let d = at.distance(lp);
                (
                    p.volume
                        * attenuation(d, p.min_distance, p.max_distance)
                        * spatial::occlusion_gain(occlusion),
                    pan(lp, lr, at),
                )
            }
            None => (p.volume, 0.0),
        }
    }

    fn start_handle(
        &mut self,
        path: &Path,
        params: &PlayParams,
        (volume, pan): (f32, f32),
    ) -> Option<(Handle, Option<OcclusionTrack>)> {
        let full = self.assets.join(path);
        let rate = kira::PlaybackRate(params.pitch.max(0.01) as f64);
        let fade_in = (!self.fade_in.is_zero()).then(|| fade(self.fade_in));
        let bus = params.bus.0.min(self.buses.len() - 1);
        let occlude = params.occludable && params.position.is_some();
        let clip = if params.stream {
            None
        } else {
            Some(self.clip(path)?)
        };
        let mut warning = None;
        // Own track (with a low-pass) under the bus, when occlusion applies.
        let mut occ_track = None;
        let mut bus_track = None;
        if let Some(hw) = self.buses[bus].hw.as_mut() {
            if occlude {
                let mut builder = TrackBuilder::new()
                    .sound_capacity(1)
                    .sub_track_capacity(0)
                    .persist_until_sounds_finish(true);
                let filter = builder.add_effect(
                    FilterBuilder::new()
                        .mode(FilterMode::LowPass)
                        .cutoff(spatial::OPEN_CUTOFF)
                        .mix(Mix::WET),
                );
                match hw.track.add_sub_track(builder) {
                    Ok(track) => {
                        occ_track = Some(OcclusionTrack {
                            _track: track,
                            filter,
                        })
                    }
                    Err(err) => warning = Some(format!("occlusion track: {err}")),
                }
            }
            bus_track = Some(&mut hw.track);
        }
        let result = if params.stream {
            match StreamingSoundData::from_file(&full) {
                Ok(mut data) => {
                    if params.looping {
                        data = data.loop_region(..);
                    }
                    let data = data
                        .volume(amplitude_to_db(volume))
                        .panning(pan)
                        .playback_rate(rate)
                        .fade_in_tween(fade_in);
                    let track = occ_track.as_mut().map(|t| &mut t._track).or(bus_track);
                    play_on(self.manager.as_mut()?, track, data).map(Handle::Streaming)
                }
                Err(e) => Err(e.to_string()),
            }
        } else {
            let mut data = clip?
                .volume(amplitude_to_db(volume))
                .panning(pan)
                .playback_rate(rate)
                .fade_in_tween(fade_in);
            if params.looping {
                data = data.loop_region(..);
            }
            let track = occ_track.as_mut().map(|t| &mut t._track).or(bus_track);
            play_on(self.manager.as_mut()?, track, data).map(Handle::Static)
        };
        if let Some(w) = warning {
            self.warn_once(w);
        }
        match result {
            Ok(h) => Some((h, occ_track)),
            Err(err) => {
                self.warn_once(format!("{}: {err}", full.display()));
                None
            }
        }
    }

    fn clip(&mut self, path: &Path) -> Option<StaticSoundData> {
        if let Some(c) = self.clips.get(path) {
            return Some(c.clone());
        }
        let full = self.assets.join(path);
        match StaticSoundData::from_file(&full) {
            Ok(data) => {
                self.clips.insert(path.to_owned(), data.clone());
                Some(data)
            }
            Err(err) => {
                self.warn_once(format!("{}: {err}", full.display()));
                None
            }
        }
    }

    fn warn_once(&mut self, message: String) {
        if self.warned.insert(message.clone()) {
            log::warn!("audio: {message}");
        }
    }

    /// One frame: listener pose, new sources, live source settings, occlusion,
    /// voices, 3D mix, cleanup. Measures the frame time itself.
    pub fn update(&mut self, world: &mut World) {
        let now = Instant::now();
        let dt = self
            .last_update
            .replace(now)
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32());
        self.update_with(world, dt);
    }

    /// [`update`](Self::update) with the frame time given in seconds.
    pub fn update_with(&mut self, world: &mut World, dt: f32) {
        if let Some(pose) = world
            .query::<(&AudioListener, Option<&GlobalTransform>, Option<&Transform>)>()
            .iter()
            .find_map(|(_, g, t)| {
                g.map(|g| {
                    let (_, r, p) = g.0.to_scale_rotation_translation();
                    (p, r)
                })
                .or(t.map(|t| (t.translation, t.rotation)))
            })
        {
            self.listener = pose;
        }

        // New sources: play on awake.
        let fresh: Vec<(Entity, bool)> = world
            .query::<(Entity, &AudioSourceDesc)>()
            .without::<&AudioSourceState>()
            .iter()
            .map(|(e, d)| (e, d.play_on_awake && !d.clip.is_empty()))
            .collect();
        for (e, awake) in fresh {
            if awake {
                self.play_source(world, e);
            } else {
                let _ = world.insert_one(e, AudioSourceState::default());
            }
        }

        // Follow source entities; forget finished and orphaned sounds.
        let mut done = Vec::new();
        for (id, p) in &mut self.playing {
            if let Some(e) = p.entity {
                match world.get::<&AudioSourceDesc>(e) {
                    Ok(desc) => {
                        p.params.volume = desc.volume;
                        if p.params.pitch != desc.pitch {
                            p.params.pitch = desc.pitch;
                            if let Some(h) = &mut p.handle {
                                h.set_pitch(desc.pitch, SMOOTH);
                            }
                        }
                        p.params.min_distance = desc.min_distance;
                        p.params.max_distance = desc.max_distance;
                        if desc.spatial {
                            p.params.position = world_position(world, e).or(p.params.position);
                        } else {
                            p.params.position = None;
                        }
                    }
                    // Despawned (or the component removed): stop it.
                    Err(_) => {
                        if let Some(h) = &mut p.handle {
                            h.stop(SMOOTH);
                        }
                        done.push(*id);
                        continue;
                    }
                }
            }
            if let Some(err) = p.handle.as_mut().and_then(Handle::pop_error) {
                let message = format!("{}: {err}", p.path.display());
                if self.warned.insert(message.clone()) {
                    log::warn!("audio: stream: {message}");
                }
            }
            match &p.handle {
                Some(h) if matches!(h.state(), PlaybackState::Stopped) => done.push(*id),
                // Without a device, one-shots end right away; looping/entity sounds stay tracked.
                None if !p.virtualized && !p.params.looping && p.entity.is_none() => done.push(*id),
                _ => {}
            }
        }
        for id in done {
            self.forget(world, id);
        }

        self.update_occlusion(dt);
        // Loudness changed with the listener: who deserves a voice may have too.
        for entity in self.rebalance() {
            if let Ok(mut s) = world.get::<&mut AudioSourceState>(entity) {
                s.sound = None;
            }
        }

        // 3D mix.
        let mixes: Vec<(SoundId, (f32, f32))> = self
            .playing
            .iter()
            .filter(|(_, p)| p.handle.is_some())
            .map(|(id, p)| (*id, self.gain(&p.params, p.occlusion.current)))
            .collect();
        for (id, (volume, pan)) in mixes {
            if let Some(p) = self.playing.get_mut(&id) {
                if let Some(h) = p.handle.as_mut() {
                    h.set_volume(volume, SMOOTH);
                    h.set_panning(pan, SMOOTH);
                }
                if let Some(t) = &mut p.occ_track {
                    t.filter
                        .set_cutoff(spatial::occlusion_cutoff(p.occlusion.current), SMOOTH);
                }
            }
        }
    }

    /// Removes a finished sound and clears its entity's state.
    fn forget(&mut self, world: &mut World, id: SoundId) {
        if let Some(p) = self.playing.remove(&id)
            && let Some(e) = p.entity
            && let Ok(mut s) = world.get::<&mut AudioSourceState>(e)
            && s.sound == Some(id)
        {
            s.sound = None;
        }
        if self.music == Some(id) {
            self.music = None;
        }
    }

    /// Measures the way to each positional sound and eases its occlusion.
    fn update_occlusion(&mut self, dt: f32) {
        let mut occluder = self.occluder.take();
        let listener = self.listener.0;
        let k = 1.0 - (-OCCLUSION_RATE * dt).exp();
        for p in self.playing.values_mut() {
            let Some(at) = p.params.position.filter(|_| p.params.occludable) else {
                p.occlusion.current = 0.0;
                continue;
            };
            let target = match (p.occlusion.manual, &mut occluder) {
                (Some(m), _) => m,
                (None, Some(f)) => f(listener, at).clamp(0.0, 1.0),
                (None, None) => 0.0,
            };
            if p.occlusion.initialized {
                p.occlusion.current += (target - p.occlusion.current) * k;
            } else {
                p.occlusion.current = target;
                p.occlusion.initialized = true;
            }
        }
        self.occluder = occluder;
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

/// The plugin's per-frame system.
pub fn update(res: &mut Resources) {
    let mut audio = res.fetch_mut::<Audio>();
    let mut world = res.fetch_mut::<World>();
    audio.update(&mut world);
}

#[cfg(test)]
mod tests;

//! Audio on kira. Entities with an `AudioSourceDesc` (from scenes or code)
//! play their clip; the entity with [`AudioListener`] is the ears. The
//! [`Audio`] resource plays one-shots and streamed music from code.
//!
//! 3D sound is done by hand: every frame each positional sound gets a volume
//! (distance attenuation between min/max distance) and a stereo pan from the
//! listener's `GlobalTransform`, tweened over a few milliseconds. There are no
//! mixer-track limits and it works the same on every backend.
//!
//! Without an audio device (CI, servers, headless tests) the plugin logs once
//! and every call becomes a no-op; the bookkeeping (ids, which sources started)
//! still works so gameplay code doesn't have to care.

mod spatial;

pub use olvin_scene::file::AudioSourceDesc;
pub use spatial::{attenuation, pan};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use glam::{Quat, Vec3};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{FromFileError, PlaybackState};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween};
use olvin_core::{App, Plugin, Resources, Stage};
use olvin_ecs::{EcsPlugin, Entity, World};
use olvin_scene::Transform;
use olvin_transform::GlobalTransform;

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
}

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

/// Files bigger than this are streamed when played from an `AudioSourceDesc`.
pub const STREAM_THRESHOLD_BYTES: u64 = 2 * 1024 * 1024;

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
}

struct Playing {
    handle: Option<Handle>,
    params: PlayParams,
    /// The source entity it follows (position, live volume/pitch).
    entity: Option<Entity>,
}

/// The audio engine: one-shots, music, sources. Every method is safe to call
/// without a device (it then does nothing).
pub struct Audio {
    manager: Option<AudioManager<DefaultBackend>>,
    assets: PathBuf,
    clips: HashMap<PathBuf, StaticSoundData>,
    playing: HashMap<SoundId, Playing>,
    next_id: u64,
    music: Option<SoundId>,
    master: f32,
    listener: (Vec3, Quat),
    warned: HashSet<String>,
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

impl Audio {
    /// Opens the default output device; without one, logs once and stays silent.
    pub fn open(assets: impl Into<PathBuf>) -> Self {
        let mut audio = Self::disabled(assets);
        match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(manager) => audio.manager = Some(manager),
            Err(err) => log::warn!("audio disabled, no output device: {err}"),
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
        if let Some(old) = self.music.take() {
            self.stop(old);
        }
        let id = self.play_with(
            path,
            PlayParams {
                volume,
                looping,
                stream: true,
                ..Default::default()
            },
        );
        self.music = Some(id);
        id
    }

    pub fn stop_music(&mut self) {
        if let Some(id) = self.music.take() {
            self.stop(id);
        }
    }

    pub fn stop(&mut self, id: SoundId) {
        if let Some(mut p) = self.playing.remove(&id)
            && let Some(h) = &mut p.handle
        {
            h.stop(SMOOTH);
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
    pub fn is_playing(&self, id: SoundId) -> bool {
        self.playing
            .get(&id)
            .and_then(|p| p.handle.as_ref())
            .is_some_and(|h| !matches!(h.state(), PlaybackState::Stopped))
    }

    /// Sounds being tracked (playing, or started without a device).
    pub fn active_count(&self) -> usize {
        self.playing.len()
    }

    /// Decodes a clip ahead of time so its first play doesn't stall.
    pub fn preload(&mut self, path: impl AsRef<Path>) -> bool {
        self.manager.is_some() && self.clip(path.as_ref()).is_some()
    }

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
            .is_ok_and(|m| m.len() > STREAM_THRESHOLD_BYTES);
        let params = PlayParams {
            volume: desc.volume,
            pitch: desc.pitch,
            looping: desc.looping,
            position: desc.spatial.then_some(position.unwrap_or(Vec3::ZERO)),
            min_distance: desc.min_distance,
            max_distance: desc.max_distance,
            stream,
        };
        let id = self.start(Path::new(&desc.clip), params, Some(entity));
        let _ = world.insert_one(entity, AudioSourceState { sound: Some(id) });
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
        let id = SoundId(self.next_id);
        self.next_id += 1;
        let gain = self.gain(&params);
        let handle = if self.manager.is_some() {
            self.start_handle(path, &params, gain)
        } else {
            None
        };
        self.playing.insert(
            id,
            Playing {
                handle,
                params,
                entity,
            },
        );
        id
    }

    fn start_handle(
        &mut self,
        path: &Path,
        params: &PlayParams,
        (volume, pan): (f32, f32),
    ) -> Option<Handle> {
        let full = self.assets.join(path);
        let result = if params.stream {
            match StreamingSoundData::from_file(&full) {
                Ok(mut data) => {
                    if params.looping {
                        data = data.loop_region(..);
                    }
                    let data = data
                        .volume(amplitude_to_db(volume))
                        .panning(pan)
                        .playback_rate(kira::PlaybackRate(params.pitch.max(0.01) as f64));
                    self.manager
                        .as_mut()?
                        .play(data)
                        .map(Handle::Streaming)
                        .map_err(|e| e.to_string())
                }
                Err(e) => Err(e.to_string()),
            }
        } else {
            let data = self.clip(path)?;
            let mut data = data
                .volume(amplitude_to_db(volume))
                .panning(pan)
                .playback_rate(kira::PlaybackRate(params.pitch.max(0.01) as f64));
            if params.looping {
                data = data.loop_region(..);
            }
            self.manager
                .as_mut()?
                .play(data)
                .map(Handle::Static)
                .map_err(|e| e.to_string())
        };
        match result {
            Ok(h) => Some(h),
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

    /// Volume and pan of a sound for the current listener.
    fn gain(&self, p: &PlayParams) -> (f32, f32) {
        match p.position {
            Some(at) => {
                let (lp, lr) = self.listener;
                let d = at.distance(lp);
                (
                    p.volume * attenuation(d, p.min_distance, p.max_distance),
                    pan(lp, lr, at),
                )
            }
            None => (p.volume, 0.0),
        }
    }

    /// One frame: listener pose, new sources, live source settings, 3D mix, cleanup.
    pub fn update(&mut self, world: &mut World) {
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
            match &p.handle {
                Some(h) if matches!(h.state(), PlaybackState::Stopped) => done.push(*id),
                // Without a device, one-shots end right away; looping/entity sounds stay tracked.
                None if !p.params.looping && p.entity.is_none() => done.push(*id),
                _ => {}
            }
        }
        for id in done {
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

        // 3D mix.
        let mixes: Vec<(SoundId, (f32, f32))> = self
            .playing
            .iter()
            .filter(|(_, p)| p.handle.is_some())
            .map(|(id, p)| (*id, self.gain(&p.params)))
            .collect();
        for (id, (volume, pan)) in mixes {
            if let Some(h) = self.playing.get_mut(&id).and_then(|p| p.handle.as_mut()) {
                h.set_volume(volume, SMOOTH);
                h.set_panning(pan, SMOOTH);
            }
        }
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

//! Entity audio sources and the voice machinery (start, virtualize, route).

use std::path::Path;
use std::time::Duration;

use glam::Vec3;
use kira::effect::filter::{FilterBuilder, FilterMode};
use kira::sound::static_sound::StaticSoundData;
use kira::sound::streaming::StreamingSoundData;
use kira::track::TrackBuilder;
use kira::{Mix, Tween};
use olvin_ecs::{Entity, World};

use super::*;

impl Audio {
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

    pub(super) fn start(
        &mut self,
        path: &Path,
        params: PlayParams,
        entity: Option<Entity>,
    ) -> SoundId {
        self.start_fading(path, params, entity, Duration::ZERO)
    }

    pub(super) fn start_fading(
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
                user_paused: false,
                pause_exempt: self.paused,
                held: false,
            },
        );
        // Takes a voice if there is one (or steals it from a worse sound).
        self.rebalance();
        self.fade_in = Duration::ZERO;
        id
    }

    /// Gives `id` a voice: starts it on the mixer.
    pub(super) fn activate(&mut self, id: SoundId) {
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
            if let Some((mut handle, track)) = started {
                // Won a voice back while held: it waits paused, like the rest.
                if p.held {
                    handle.pause(Tween::default());
                }
                p.handle = Some(handle);
                p.occ_track = track;
            }
        }
    }

    /// Takes the voice away. One-shots are gone for good, loops wait.
    pub(super) fn deactivate(&mut self, id: SoundId) -> Option<Option<Entity>> {
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
    pub(super) fn rebalance(&mut self) -> Vec<Entity> {
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
    pub(super) fn gain(&self, p: &PlayParams, occlusion: f32) -> (f32, f32) {
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

    pub(super) fn start_handle(
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

    pub(super) fn clip(&mut self, path: &Path) -> Option<StaticSoundData> {
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

    pub(super) fn warn_once(&mut self, message: String) {
        if self.warned.insert(message.clone()) {
            log::warn!("audio: {message}");
        }
    }
}

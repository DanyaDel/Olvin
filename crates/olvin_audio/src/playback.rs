//! Playing, stopping and tuning sounds, voice limits, the clip cache.

use std::path::Path;
use std::time::Duration;

use glam::Vec3;
use kira::sound::PlaybackState;

use super::*;

impl Audio {
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
}

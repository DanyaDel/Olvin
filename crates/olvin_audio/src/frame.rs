//! The per-frame update: listener, sources, occlusion, voices, 3D mix.

use std::time::Instant;

use kira::sound::PlaybackState;
use olvin_ecs::{Entity, World};
use olvin_scene::Transform;
use olvin_transform::GlobalTransform;

use super::*;

impl Audio {
    /// One frame: listener pose, new sources, live source settings, occlusion,
    /// voices, 3D mix, cleanup. Measures the frame time itself.
    pub fn update(&mut self, world: &mut World) {
        let now = Instant::now();
        self.supervise(now);
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
        let has_device = self.manager.is_some();
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
                // With a device a voice without a handle means the file would
                // not load: forget it, or looping ghosts hold voices forever.
                None if !p.virtualized && has_device => done.push(*id),
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
    pub(super) fn forget(&mut self, world: &mut World, id: SoundId) {
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
    pub(super) fn update_occlusion(&mut self, dt: f32) {
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

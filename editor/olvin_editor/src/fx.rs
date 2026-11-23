//! Live preview of particle systems in the Scene view. The edited scene is
//! rebuilt into a throwaway world every frame, so the emitters live in a world
//! of their own: one `ParticleEmitter` per scene object that has `particles`
//! and is selected (or has "Simulate" on). Edits to the description are
//! applied to the running emitter in place; Play / Pause / Restart / Stop are
//! the inspector's buttons.

use std::collections::{HashMap, HashSet};

use glam::Mat4;
use olvin::ecs::{Entity, World};
use olvin::particles::{emitter, emitter_params};
use olvin::render::{ParticleEmitter, ParticleMode, Textures};
use olvin::render_assets::RenderAssets;
use olvin::scene::file::{ParticleDesc, SceneFile};
use olvin::transform::GlobalTransform;

/// What a button of the inspector asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    Play,
    Restart,
    Stop,
}

/// The preview controls of one scene object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FxControl {
    /// Frozen: particles stay on screen, nothing moves.
    pub paused: bool,
    /// Keep simulating when the object is not selected.
    pub simulate: bool,
    stopped: bool,
    request: Option<Request>,
}

impl FxControl {
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }
}

struct Entry {
    entity: Entity,
    desc: ParticleDesc,
}

/// The emitters previewed in the Scene view.
#[derive(Default)]
pub struct FxPreview {
    /// Entities with `GlobalTransform` + `ParticleEmitter`, drawn by the
    /// Scene view's particle renderer.
    pub world: World,
    entries: HashMap<u64, Entry>,
    controls: HashMap<u64, FxControl>,
}

impl FxPreview {
    pub fn control(&mut self, id: u64) -> &mut FxControl {
        self.controls.entry(id).or_default()
    }

    /// Resume after a Pause or Stop (a finished one-shot starts over).
    pub fn play(&mut self, id: u64) {
        let c = self.control(id);
        c.paused = false;
        c.request = Some(Request::Play);
    }

    pub fn pause(&mut self, id: u64) {
        self.control(id).paused = true;
    }

    /// Clear the particles and start the cycle over.
    pub fn restart(&mut self, id: u64) {
        let c = self.control(id);
        c.paused = false;
        c.request = Some(Request::Restart);
    }

    /// Clear the particles and stop until Play.
    pub fn stop(&mut self, id: u64) {
        self.control(id).request = Some(Request::Stop);
    }

    /// Runs `f` on the emitter previewed for object `id`, if there is one.
    pub fn with_emitter<R>(&self, id: u64, f: impl FnOnce(&ParticleEmitter) -> R) -> Option<R> {
        let e = self.entries.get(&id)?;
        let em = self.world.get::<&ParticleEmitter>(e.entity).ok()?;
        Some(f(&em))
    }

    /// Live CPU particles of `id`'s preview (`None` when not previewed or on the GPU).
    pub fn count(&self, id: u64) -> Option<usize> {
        self.with_emitter(id, |e| (e.mode() == ParticleMode::Cpu).then(|| e.count()))
            .flatten()
    }

    /// Whether `id`'s emitter simulates on the GPU right now.
    pub fn on_gpu(&self, id: u64) -> bool {
        self.with_emitter(id, |e| e.mode() == ParticleMode::Gpu)
            .unwrap_or(false)
    }

    /// Steps the previews by `dt` seconds. `selected`: the selected objects;
    /// `worlds`: world matrices of the scene's objects. Returns whether
    /// something is moving (the editor keeps repainting).
    pub fn tick(
        &mut self,
        scene: &SceneFile,
        selected: &[u64],
        worlds: &HashMap<u64, Mat4>,
        dt: f32,
        assets: &mut RenderAssets,
        textures: &mut Textures,
    ) -> bool {
        let mut keep = HashSet::new();
        let mut moving = false;
        for e in &scene.entities {
            let Some(desc) = &e.particles else { continue };
            if !scene.active_in_hierarchy(e.id) {
                continue;
            }
            let control = self.controls.entry(e.id).or_default();
            if !(selected.contains(&e.id) || control.simulate) {
                continue;
            }
            keep.insert(e.id);
            let matrix = worlds.get(&e.id).copied().unwrap_or(Mat4::IDENTITY);
            let entry = self.entries.entry(e.id).or_insert_with(|| Entry {
                entity: self
                    .world
                    .spawn((GlobalTransform(matrix), emitter(desc, assets, textures))),
                desc: desc.clone(),
            });
            let Ok(mut em) = self.world.get::<&mut ParticleEmitter>(entry.entity) else {
                continue;
            };
            if entry.desc != *desc {
                // Edited while running: the particles keep flying with the new numbers.
                em.params = emitter_params(desc);
                if entry.desc.texture != desc.texture {
                    em.texture = Some(assets.particle_texture(desc.texture.as_deref(), textures));
                }
                entry.desc = desc.clone();
            }
            drop(em);
            if let Ok(mut g) = self.world.get::<&mut GlobalTransform>(entry.entity) {
                g.0 = matrix;
            }
            let Ok(mut em) = self.world.get::<&mut ParticleEmitter>(entry.entity) else {
                continue;
            };
            match control.request.take() {
                Some(Request::Play) => {
                    let was_stopped = std::mem::take(&mut control.stopped);
                    em.playing = true;
                    if was_stopped || em.is_done() {
                        em.restart();
                    }
                }
                Some(Request::Restart) => {
                    control.stopped = false;
                    em.clear();
                    em.restart();
                }
                Some(Request::Stop) => {
                    control.stopped = true;
                    em.clear();
                    em.playing = false;
                }
                None => {}
            }
            if !control.paused && !control.stopped {
                em.update(dt, &matrix);
                moving |= !em.is_done();
            }
        }
        // Deselected: the preview disappears with its particles.
        let gone: Vec<u64> = self
            .entries
            .keys()
            .copied()
            .filter(|id| !keep.contains(id))
            .collect();
        for id in gone {
            if let Some(entry) = self.entries.remove(&id) {
                let _ = self.world.despawn(entry.entity);
            }
        }
        moving
    }
}

impl crate::EditorState {
    /// Steps the Scene view's particle previews (once per editor frame).
    /// Returns whether anything is moving.
    pub fn tick_particle_previews(&mut self, dt: f32) -> bool {
        if self.play.is_some() {
            return false;
        }
        let mut selected = self.multi.clone();
        selected.extend(self.selection);
        let worlds = self.world_matrices();
        let assets = self.assets_dir();
        let p = &mut self.engine_scene;
        p.assets.set_root(assets);
        p.fx.tick(
            &self.scene,
            &selected,
            &worlds,
            dt.clamp(0.0, 0.1),
            &mut p.assets,
            &mut p.textures,
        )
    }

    /// Live CPU particles of the previewed object (`None`: not previewed, or GPU).
    pub fn particle_preview_count(&self, id: u64) -> Option<usize> {
        self.engine_scene.fx.count(id)
    }

    /// The Scene view's particle previews (their Play / Pause / Restart / Stop).
    pub fn particle_preview(&mut self) -> &mut FxPreview {
        &mut self.engine_scene.fx
    }
}

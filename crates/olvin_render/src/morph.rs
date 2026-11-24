//! Morph targets (blend shapes): a face that smiles, a cheek that puffs.
//!
//! A [`MorphMesh`] on an entity keeps the mesh at weight zero and a list of
//! targets, each a set of per-vertex offsets. [`MorphMesh::weights`] says how
//! much of each target is on; when they change, [`update`] (run by
//! [`skin_meshes`](crate::skin_meshes), so before skinning) builds the blended
//! mesh and re-uploads it into the entity's mesh slot. A skinned mesh is
//! morphed in its bind pose first and then posed, on either tier.
//!
//! Weights come from game code (`set`), from a [`MorphPlayer`] that plays a
//! [`MorphClip`] (glTF weight animations), or both (the player writes every
//! weight it has a track for).
//!
//! Like skinned meshes, a morphed one must own its mesh slot: the blended
//! copy replaces the slot's geometry.

use std::sync::Arc;

use glam::Vec3;
use olvin_core::{Resources, Time};
use olvin_ecs::{Entity, World};

use crate::{MeshData, MeshRenderer, Meshes, SkinnedMesh};

/// A weight below this changes nothing worth drawing.
const NEGLIGIBLE: f32 = 1e-4;

/// One blend shape: offsets added to the base mesh at weight 1.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MorphTarget {
    pub name: String,
    /// One per vertex.
    pub positions: Vec<Vec3>,
    /// One per vertex, or empty when the target leaves normals alone.
    pub normals: Vec<Vec3>,
}

/// Component: the base mesh, its targets and their current weights.
#[derive(Clone, Debug)]
pub struct MorphMesh {
    base: Arc<MeshData>,
    targets: Arc<Vec<MorphTarget>>,
    /// One per target, normally 0..1 (anything goes: -1 can pull the other way).
    pub weights: Vec<f32>,
    /// The weights the mesh slot was last built with.
    applied: Option<Vec<f32>>,
}

impl MorphMesh {
    /// `weights` are the starting ones (missing entries are 0). A target
    /// whose offsets do not match the mesh's vertices is dropped.
    pub fn new(base: MeshData, targets: Vec<MorphTarget>, weights: &[f32]) -> Self {
        let n = base.vertices.len();
        let kept: Vec<(usize, MorphTarget)> = targets
            .into_iter()
            .enumerate()
            .filter(|(i, t)| {
                let ok = t.positions.len() == n && (t.normals.is_empty() || t.normals.len() == n);
                if !ok {
                    log::warn!(
                        "morph target {i} `{}` does not fit the mesh: dropped",
                        t.name
                    );
                }
                ok
            })
            .collect();
        let weights = kept
            .iter()
            .map(|(i, _)| weights.get(*i).copied().unwrap_or(0.0))
            .collect();
        Self {
            base: Arc::new(base),
            targets: Arc::new(kept.into_iter().map(|(_, t)| t).collect()),
            weights,
            applied: None,
        }
    }

    pub fn len(&self) -> usize {
        self.targets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// Target names, in index order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.targets.iter().map(|t| t.name.as_str())
    }

    /// Index of the target called `name`.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.targets.iter().position(|t| t.name == name)
    }

    /// Sets the weight of `name`; false if there is no such target.
    pub fn set(&mut self, name: &str, weight: f32) -> bool {
        match self.index_of(name) {
            Some(i) => self.set_index(i, weight),
            None => false,
        }
    }

    /// Sets the weight of target `index`; false if out of range or not finite.
    pub fn set_index(&mut self, index: usize, weight: f32) -> bool {
        match self.weights.get_mut(index) {
            Some(w) if weight.is_finite() => {
                *w = weight;
                true
            }
            _ => false,
        }
    }

    pub fn weight(&self, name: &str) -> Option<f32> {
        self.index_of(name).map(|i| self.weights[i])
    }

    /// The mesh with the current weights on.
    pub fn morphed(&self) -> MeshData {
        blend(&self.base, &self.targets, &self.weights)
    }

    fn changed(&self) -> bool {
        self.applied.as_ref() != Some(&self.weights)
    }
}

fn blend(base: &MeshData, targets: &[MorphTarget], weights: &[f32]) -> MeshData {
    let mut mesh = base.clone();
    let mut touched_normals = false;
    for (target, &w) in targets.iter().zip(weights) {
        if w.abs() < NEGLIGIBLE {
            continue;
        }
        for (v, d) in mesh.vertices.iter_mut().zip(&target.positions) {
            let p = Vec3::from(v.position) + *d * w;
            v.position = p.into();
        }
        for (v, d) in mesh.vertices.iter_mut().zip(&target.normals) {
            let n = Vec3::from(v.normal) + *d * w;
            v.normal = n.into();
            touched_normals = true;
        }
    }
    if touched_normals {
        for v in &mut mesh.vertices {
            v.normal = Vec3::from(v.normal).normalize_or(Vec3::Y).into();
        }
    }
    mesh
}

/// Weight keys of one target.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MorphTrack {
    /// Index of the target (as in [`MorphMesh`]).
    pub target: usize,
    /// Seconds, ascending.
    pub times: Vec<f32>,
    pub values: Vec<f32>,
    /// Hold each value until the next key instead of blending.
    pub step: bool,
}

impl MorphTrack {
    fn sample(&self, t: f32) -> Option<f32> {
        let (first, last) = (*self.times.first()?, *self.times.last()?);
        if self.values.len() != self.times.len() {
            return None;
        }
        if t <= first {
            return self.values.first().copied();
        }
        if t >= last {
            return self.values.last().copied();
        }
        let next = self.times.partition_point(|&k| k <= t);
        let (a, b) = (next - 1, next);
        if self.step {
            return Some(self.values[a]);
        }
        let span = (self.times[b] - self.times[a]).max(1e-9);
        let f = (t - self.times[a]) / span;
        Some(self.values[a] + (self.values[b] - self.values[a]) * f)
    }
}

/// A weight animation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MorphClip {
    pub name: String,
    /// Seconds: the latest key of any track.
    pub duration: f32,
    pub tracks: Vec<MorphTrack>,
}

impl MorphClip {
    pub fn new(name: impl Into<String>, tracks: Vec<MorphTrack>) -> Self {
        let duration = tracks
            .iter()
            .filter_map(|t| t.times.last().copied())
            .fold(0.0, f32::max);
        Self {
            name: name.into(),
            duration,
            tracks,
        }
    }

    /// Writes the weight of every track at time `t` into `weights`.
    pub fn sample(&self, t: f32, weights: &mut [f32]) {
        for track in &self.tracks {
            if let (Some(slot), Some(v)) = (weights.get_mut(track.target), track.sample(t)) {
                *slot = v;
            }
        }
    }
}

/// Component: the clips of this object by name (what a script can play).
#[derive(Clone, Debug, Default)]
pub struct MorphClips(pub Vec<Arc<MorphClip>>);

impl MorphClips {
    pub fn find(&self, name: &str) -> Option<Arc<MorphClip>> {
        self.0.iter().find(|c| c.name == name).cloned()
    }
}

/// Component: plays a [`MorphClip`] on this object's [`MorphMesh`].
#[derive(Clone, Debug)]
pub struct MorphPlayer {
    pub clip: Arc<MorphClip>,
    /// Seconds into the clip.
    pub time: f32,
    pub speed: f32,
    pub looping: bool,
    pub playing: bool,
}

impl MorphPlayer {
    pub fn new(clip: Arc<MorphClip>, looping: bool) -> Self {
        Self {
            clip,
            time: 0.0,
            speed: 1.0,
            looping,
            playing: true,
        }
    }

    fn advance(&mut self, dt: f32) {
        if !self.playing {
            return;
        }
        self.time += dt * self.speed;
        let d = self.clip.duration;
        if d <= 0.0 {
            self.playing = false;
            self.time = 0.0;
        } else if self.looping {
            self.time = self.time.rem_euclid(d);
        } else if self.time >= d || self.time < 0.0 {
            self.time = self.time.clamp(0.0, d);
            self.playing = false;
        }
    }
}

/// Plays the players, then rebuilds the mesh of every [`MorphMesh`] whose
/// weights changed. Called by [`skin_meshes`](crate::skin_meshes).
pub fn update(res: &mut Resources) {
    let dt = res.get::<Time>().map_or(0.0, |t| t.delta());
    let world = res.fetch::<World>();
    for (player, morph) in world.query::<(&mut MorphPlayer, &mut MorphMesh)>().iter() {
        player.advance(dt);
        let time = player.time;
        player.clip.sample(time, &mut morph.weights);
    }
    let mut meshes = res.fetch_mut::<Meshes>();
    let jobs: Vec<Entity> = world
        .query::<(Entity, &MorphMesh)>()
        .iter()
        .filter(|(_, m)| m.changed())
        .map(|(e, _)| e)
        .collect();
    for e in jobs {
        let Ok(mut morph) = world.get::<&mut MorphMesh>(e) else {
            continue;
        };
        let blended = morph.morphed();
        morph.applied = Some(morph.weights.clone());
        if let Ok(mut skin) = world.get::<&mut SkinnedMesh>(e) {
            // Posed from the blended bind pose at the next `skin_meshes`.
            skin.set_bind(blended);
        } else if let Ok(renderer) = world.get::<&MeshRenderer>(e) {
            meshes.set(renderer.mesh, blended);
        }
    }
}

#[cfg(test)]
mod tests;

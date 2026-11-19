use std::sync::Arc;

use glam::Mat4;
use olvin_core::{App, Events, Plugin, Stage, Time};
use olvin_ecs::{EcsPlugin, Entity, World};

use crate::clip::Clip;
use crate::graph::{AnimGraph, LayerState, Params};
use crate::skeleton::{Pose, Skeleton};

/// One graph playing over (part of) the skeleton. Layers apply in order:
/// each blends its pose over the result so far by `weight`, per joint where
/// `mask` allows.
#[derive(Clone, Debug)]
pub struct Layer {
    pub graph: Arc<AnimGraph>,
    pub weight: f32,
    /// Per joint; `None`: all joints.
    pub mask: Option<Vec<bool>>,
    pub state: LayerState,
}

impl Layer {
    pub fn new(graph: Arc<AnimGraph>) -> Self {
        let state = LayerState::new(&graph);
        Self {
            graph,
            weight: 1.0,
            mask: None,
            state,
        }
    }
}

/// A clip event reached during playback.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationEvent {
    pub entity: Entity,
    pub name: String,
}

/// Component: drives a skeleton. [`AnimPlugin`] advances it every frame and
/// leaves the skinning matrices in `matrices` (read by the renderer).
#[derive(Clone, Debug)]
pub struct Animator {
    pub skeleton: Arc<Skeleton>,
    pub clips: Arc<[Clip]>,
    pub params: Params,
    pub layers: Vec<Layer>,
    /// Playback speed multiplier for all layers.
    pub speed: f32,
    pub pose: Pose,
    /// `Skeleton::skin_matrices` of `pose`.
    pub matrices: Vec<Mat4>,
    /// Bumped whenever `matrices` is recomputed.
    pub version: u64,
    /// Clip events of the last `update`.
    pub events: Vec<String>,
}

impl Animator {
    pub fn new(skeleton: Arc<Skeleton>, clips: Arc<[Clip]>) -> Self {
        let pose = skeleton.rest_pose();
        let matrices = skeleton.skin_matrices(&pose);
        Self {
            skeleton,
            clips,
            params: Params::default(),
            layers: Vec::new(),
            speed: 1.0,
            pose,
            matrices,
            version: 0,
            events: Vec::new(),
        }
    }

    /// An animator with one layer running `graph`.
    pub fn with_graph(mut self, graph: AnimGraph) -> Self {
        self.layers.push(Layer::new(Arc::new(graph)));
        self.refresh();
        self
    }

    /// An animator looping one clip.
    pub fn looping(skeleton: Arc<Skeleton>, clips: Arc<[Clip]>, clip: usize) -> Self {
        Self::new(skeleton, clips).with_graph(AnimGraph::single(clip))
    }

    /// Advances all layers and recomputes pose and matrices.
    pub fn update(&mut self, dt: f32) {
        self.events.clear();
        let dt = dt * self.speed;
        for l in &mut self.layers {
            l.state.update(
                &l.graph,
                &self.clips,
                &mut self.params,
                dt,
                &mut self.events,
            );
        }
        self.refresh();
    }

    /// Recomputes pose and matrices without advancing time.
    pub fn refresh(&mut self) {
        let mut pose = self.skeleton.rest_pose();
        for (i, l) in self.layers.iter().enumerate() {
            let layer = l
                .state
                .pose(&l.graph, &self.clips, &self.params, &self.skeleton);
            if i == 0 && l.mask.is_none() && l.weight >= 1.0 {
                pose = layer;
                continue;
            }
            for (j, (dst, src)) in pose.joints.iter_mut().zip(&layer.joints).enumerate() {
                let on = l
                    .mask
                    .as_ref()
                    .is_none_or(|m| m.get(j).copied().unwrap_or(false));
                if on {
                    *dst = dst.lerp(src, l.weight.clamp(0.0, 1.0));
                }
            }
        }
        self.matrices = self.skeleton.skin_matrices(&pose);
        self.pose = pose;
        self.version += 1;
    }
}

/// Advances every [`Animator`] in `Stage::Update` by the simulation time and
/// sends [`AnimationEvent`]s.
pub struct AnimPlugin;

impl Plugin for AnimPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(EcsPlugin)
            .init_resource::<Events<AnimationEvent>>()
            .add_system(Stage::Update, |res| {
                let dt = res.get::<Time>().map_or(0.0, |t| t.delta());
                let mut fired = Vec::new();
                for (e, a) in res
                    .fetch_mut::<World>()
                    .query_mut::<(Entity, &mut Animator)>()
                {
                    a.update(dt);
                    fired.extend(a.events.iter().map(|name| AnimationEvent {
                        entity: e,
                        name: name.clone(),
                    }));
                }
                if !fired.is_empty() {
                    res.fetch_mut::<Events<AnimationEvent>>().send_batch(fired);
                }
            });
    }
}

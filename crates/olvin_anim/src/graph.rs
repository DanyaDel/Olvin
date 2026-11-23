//! Animation state graph: states play a motion (a clip or a 1D blend of
//! clips driven by a parameter), transitions between states fire on
//! parameter conditions and cross-fade. Definitions ([`AnimGraph`]) are
//! immutable and shareable; the running state lives in [`LayerState`].

use std::collections::HashMap;

use crate::clip::Clip;
use crate::skeleton::{Pose, Skeleton};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Bool(bool),
    /// One-shot: set, then consumed by the first transition that uses it.
    Trigger(bool),
}

impl ParamValue {
    fn as_f32(self) -> f32 {
        match self {
            Self::Float(v) => v,
            Self::Bool(b) | Self::Trigger(b) => f32::from(b),
        }
    }
}

/// Named parameters shared by all layers of an animator.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Params(HashMap<String, ParamValue>);

impl Params {
    pub fn set_float(&mut self, name: &str, v: f32) {
        self.0.insert(name.into(), ParamValue::Float(v));
    }

    pub fn set_bool(&mut self, name: &str, v: bool) {
        self.0.insert(name.into(), ParamValue::Bool(v));
    }

    pub fn set_trigger(&mut self, name: &str) {
        self.0.insert(name.into(), ParamValue::Trigger(true));
    }

    pub fn get(&self, name: &str) -> Option<ParamValue> {
        self.0.get(name).copied()
    }

    /// Value as a number (bools and triggers are 0 / 1); 0 when unset.
    pub fn float(&self, name: &str) -> f32 {
        self.get(name).map_or(0.0, ParamValue::as_f32)
    }

    pub fn bool(&self, name: &str) -> bool {
        self.float(name) != 0.0
    }

    fn consume(&mut self, name: &str) {
        if let Some(ParamValue::Trigger(on)) = self.0.get_mut(name) {
            *on = false;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Test {
    Greater(f32),
    Less(f32),
    True,
    False,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub param: String,
    pub test: Test,
}

impl Condition {
    pub fn new(param: impl Into<String>, test: Test) -> Self {
        Self {
            param: param.into(),
            test,
        }
    }

    fn holds(&self, params: &Params) -> bool {
        let v = params.float(&self.param);
        match self.test {
            Test::Greater(x) => v > x,
            Test::Less(x) => v < x,
            Test::True => v != 0.0,
            Test::False => v == 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Motion {
    /// Index into the animator's clips.
    Clip(usize),
    /// Clips placed along one parameter; between two neighbours they are
    /// blended (poses and durations), outside the range the end clip plays.
    Blend1D {
        param: String,
        /// (threshold, clip), ascending by threshold.
        points: Vec<(f32, usize)>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub name: String,
    pub motion: Motion,
    pub speed: f32,
    pub looping: bool,
}

impl State {
    pub fn new(name: impl Into<String>, motion: Motion) -> Self {
        Self {
            name: name.into(),
            motion,
            speed: 1.0,
            looping: true,
        }
    }

    pub fn once(mut self) -> Self {
        self.looping = false;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    /// `None`: from any state (never back into the target itself).
    pub from: Option<usize>,
    pub to: usize,
    /// All must hold (none: always, subject to `exit_time`).
    pub conditions: Vec<Condition>,
    /// Fires only once the source has played this fraction of its length
    /// (1.0 = the end).
    pub exit_time: Option<f32>,
    /// Cross-fade length in seconds.
    pub duration: f32,
}

impl Transition {
    pub fn new(from: Option<usize>, to: usize, duration: f32) -> Self {
        Self {
            from,
            to,
            conditions: Vec::new(),
            exit_time: None,
            duration,
        }
    }

    pub fn when(mut self, param: &str, test: Test) -> Self {
        self.conditions.push(Condition::new(param, test));
        self
    }

    pub fn at_exit_time(mut self, fraction: f32) -> Self {
        self.exit_time = Some(fraction);
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimGraph {
    pub states: Vec<State>,
    pub transitions: Vec<Transition>,
    /// Entry state.
    pub default: usize,
}

impl AnimGraph {
    /// One looping state playing `clip`.
    pub fn single(clip: usize) -> Self {
        Self {
            states: vec![State::new("clip", Motion::Clip(clip))],
            ..Default::default()
        }
    }

    pub fn state_index(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|s| s.name == name)
    }
}

/// Clip weights of a motion for the current parameters (at most two entries).
fn motion_weights(motion: &Motion, params: &Params) -> Vec<(usize, f32)> {
    match motion {
        Motion::Clip(c) => vec![(*c, 1.0)],
        Motion::Blend1D { param, points } => {
            let v = params.float(param);
            match points.iter().position(|p| p.0 > v) {
                None => points.last().map(|p| (p.1, 1.0)).into_iter().collect(),
                Some(0) => vec![(points[0].1, 1.0)],
                Some(hi) => {
                    let (a, b) = (points[hi - 1], points[hi]);
                    let t = (v - a.0) / (b.0 - a.0).max(1e-9);
                    vec![(a.1, 1.0 - t), (b.1, t)]
                }
            }
        }
    }
}

fn motion_duration(motion: &Motion, params: &Params, clips: &[Clip]) -> f32 {
    motion_weights(motion, params)
        .iter()
        .filter_map(|&(c, w)| clips.get(c).map(|c| c.duration * w))
        .sum()
}

/// Pose of a motion at `phase` (0..1 of its length).
fn motion_pose(
    motion: &Motion,
    params: &Params,
    phase: f32,
    clips: &[Clip],
    skeleton: &Skeleton,
) -> Pose {
    let mut pose: Option<Pose> = None;
    let mut done = 0.0;
    for (c, w) in motion_weights(motion, params) {
        let Some(clip) = clips.get(c) else { continue };
        let mut p = skeleton.rest_pose();
        clip.apply(phase * clip.duration, skeleton, &mut p);
        done += w;
        match &mut pose {
            None => pose = Some(p),
            Some(acc) => acc.blend(&p, w / done.max(1e-9)),
        }
    }
    pose.unwrap_or_else(|| skeleton.rest_pose())
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Fade {
    to: usize,
    /// Seconds into the target state.
    time: f32,
    elapsed: f32,
    duration: f32,
}

/// Running state of one layer: the current state, and the cross-fade
/// towards the next one, if any.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerState {
    pub current: usize,
    /// Seconds into the current state.
    pub time: f32,
    fade: Option<Fade>,
}

impl LayerState {
    pub fn new(graph: &AnimGraph) -> Self {
        Self {
            current: graph.default.min(graph.states.len().saturating_sub(1)),
            time: 0.0,
            fade: None,
        }
    }

    pub fn is_transitioning(&self) -> bool {
        self.fade.is_some()
    }

    /// Jumps to a state without a cross-fade.
    pub fn set_state(&mut self, state: usize) {
        self.current = state;
        self.time = 0.0;
        self.fade = None;
    }

    /// Advances `dt` seconds: plays the state, fires clip events into
    /// `events`, finishes or starts transitions (consuming triggers). While
    /// a cross-fade runs no new transition starts.
    pub fn update(
        &mut self,
        graph: &AnimGraph,
        clips: &[Clip],
        params: &mut Params,
        dt: f32,
        events: &mut Vec<String>,
    ) {
        let Some(state) = graph.states.get(self.current) else {
            return;
        };
        self.time = advance(state, params, clips, self.time, dt, events);
        if let Some(fade) = &mut self.fade {
            if let Some(next) = graph.states.get(fade.to) {
                fade.time = advance(next, params, clips, fade.time, dt, events);
            }
            fade.elapsed += dt;
            if fade.elapsed >= fade.duration
                && let Some(f) = self.fade.take()
            {
                self.current = f.to;
                self.time = f.time;
            }
            return;
        }
        let len = motion_duration(&state.motion, params, clips).max(1e-6);
        let normalized = self.time / len;
        let fired = graph.transitions.iter().find(|t| {
            t.from.map_or(t.to != self.current, |f| f == self.current)
                && t.to < graph.states.len()
                && t.exit_time.is_none_or(|e| normalized >= e)
                && t.conditions.iter().all(|c| c.holds(params))
        });
        if let Some(t) = fired {
            for c in &t.conditions {
                params.consume(&c.param);
            }
            if t.duration <= 0.0 {
                self.set_state(t.to);
            } else {
                self.fade = Some(Fade {
                    to: t.to,
                    time: 0.0,
                    elapsed: 0.0,
                    duration: t.duration,
                });
            }
        }
    }

    /// Local pose of the layer now.
    pub fn pose(
        &self,
        graph: &AnimGraph,
        clips: &[Clip],
        params: &Params,
        skeleton: &Skeleton,
    ) -> Pose {
        let sample = |i: usize, time: f32| match graph.states.get(i) {
            Some(s) => {
                let len = motion_duration(&s.motion, params, clips).max(1e-6);
                motion_pose(&s.motion, params, time / len, clips, skeleton)
            }
            None => skeleton.rest_pose(),
        };
        let mut pose = sample(self.current, self.time);
        if let Some(f) = &self.fade {
            pose.blend(
                &sample(f.to, f.time),
                (f.elapsed / f.duration).clamp(0.0, 1.0),
            );
        }
        pose
    }
}

/// Plays `state` for `dt`; returns the new time (wrapped or clamped).
fn advance(
    state: &State,
    params: &Params,
    clips: &[Clip],
    time: f32,
    dt: f32,
    events: &mut Vec<String>,
) -> f32 {
    let len = motion_duration(&state.motion, params, clips);
    let new = time + dt * state.speed;
    // Events come from the clip with the largest weight, in its own time.
    let main = motion_weights(&state.motion, params)
        .into_iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .and_then(|(c, _)| clips.get(c));
    let scale = main.map_or(1.0, |c| if len > 1e-6 { c.duration / len } else { 1.0 });
    let fire = |from: f32, to: f32, events: &mut Vec<String>| {
        if let Some(c) = main {
            c.events_between(from * scale, to * scale, events);
        }
    };
    if len <= 1e-6 {
        return 0.0;
    }
    if new < len {
        fire(time, new, events);
        new.max(0.0)
    } else if state.looping {
        fire(time, len, events);
        let wrapped = new % len;
        fire(-1.0, wrapped, events);
        wrapped
    } else {
        fire(time, len, events);
        len
    }
}

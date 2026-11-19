use glam::{Quat, Vec3, Vec4};

use crate::skeleton::{Pose, Skeleton};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    Translation,
    Rotation,
    Scale,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    #[default]
    Linear,
    /// glTF cubic spline: three values per key (in-tangent, value, out-tangent).
    CubicSpline,
}

/// Keyframes of one property of one target. Values are xyz for translation
/// and scale, xyzw for rotation.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub target: usize,
    pub property: Property,
    pub interpolation: Interpolation,
    /// Seconds, ascending.
    pub times: Vec<f32>,
    /// `times.len()` entries, or three times that for `CubicSpline`.
    pub values: Vec<Vec4>,
}

impl Channel {
    /// Value at `time`, clamped to the first and last key. `None` for an
    /// empty or inconsistent channel.
    pub fn sample(&self, time: f32) -> Option<Vec4> {
        let n = self.times.len();
        let cubic = self.interpolation == Interpolation::CubicSpline;
        if n == 0 || self.values.len() != if cubic { n * 3 } else { n } {
            return None;
        }
        let key = |i: usize| {
            if cubic {
                self.values[i * 3 + 1]
            } else {
                self.values[i]
            }
        };
        if n == 1 || time <= self.times[0] {
            return Some(key(0));
        }
        if time >= self.times[n - 1] {
            return Some(key(n - 1));
        }
        let hi = self.times.partition_point(|&t| t <= time).min(n - 1);
        let lo = hi - 1;
        let dt = self.times[hi] - self.times[lo];
        let t = if dt > 1e-9 {
            (time - self.times[lo]) / dt
        } else {
            1.0
        };
        let (a, b) = (key(lo), key(hi));
        let rotation = self.property == Property::Rotation;
        Some(match self.interpolation {
            Interpolation::Step => a,
            Interpolation::Linear if rotation => {
                Vec4::from(Quat::from_vec4(a).slerp(Quat::from_vec4(b), t))
            }
            Interpolation::Linear => a.lerp(b, t),
            Interpolation::CubicSpline => {
                let (out_a, in_b) = (self.values[lo * 3 + 2], self.values[hi * 3]);
                let (t2, t3) = (t * t, t * t * t);
                let v = a * (2.0 * t3 - 3.0 * t2 + 1.0)
                    + out_a * dt * (t3 - 2.0 * t2 + t)
                    + b * (-2.0 * t3 + 3.0 * t2)
                    + in_b * dt * (t3 - t2);
                if rotation { v.normalize_or(a) } else { v }
            }
        })
    }
}

/// A named moment in a clip, reported when playback passes it.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipEvent {
    pub time: f32,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Seconds: the latest key of any channel.
    pub duration: f32,
    pub channels: Vec<Channel>,
    /// Ascending by time.
    pub events: Vec<ClipEvent>,
}

impl Clip {
    pub fn new(name: impl Into<String>, channels: Vec<Channel>) -> Self {
        let duration = channels
            .iter()
            .filter_map(|c| c.times.last().copied())
            .fold(0.0, f32::max);
        Self {
            name: name.into(),
            duration,
            channels,
            events: Vec::new(),
        }
    }

    pub fn with_event(mut self, time: f32, name: impl Into<String>) -> Self {
        self.events.push(ClipEvent {
            time,
            name: name.into(),
        });
        self.events.sort_by(|a, b| a.time.total_cmp(&b.time));
        self
    }

    /// Writes the animated properties at `time` into `pose` (joints without
    /// a channel keep their value, so start from the rest pose or a blend).
    pub fn apply(&self, time: f32, skeleton: &Skeleton, pose: &mut Pose) {
        for c in &self.channels {
            let (Some(j), Some(v)) = (skeleton.joint_for_target(c.target), c.sample(time)) else {
                continue;
            };
            let Some(joint) = pose.joints.get_mut(j) else {
                continue;
            };
            match c.property {
                Property::Translation => joint.translation = Vec3::new(v.x, v.y, v.z),
                Property::Scale => joint.scale = Vec3::new(v.x, v.y, v.z),
                Property::Rotation => joint.rotation = Quat::from_vec4(v).normalize(),
            }
        }
    }

    /// Events with `from < time <= to`; at `from == 0` also those at 0.
    pub fn events_between(&self, from: f32, to: f32, out: &mut Vec<String>) {
        out.extend(
            self.events
                .iter()
                .filter(|e| (e.time > from || (from <= 0.0 && e.time <= 0.0)) && e.time <= to)
                .map(|e| e.name.clone()),
        );
    }
}

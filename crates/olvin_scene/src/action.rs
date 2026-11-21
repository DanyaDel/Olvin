//! Object animation as Blender does it: an [`Action`] is a set of F-curves,
//! one per animated channel (`X Location`, `Y Euler Rotation`, ...), each a
//! list of Bezier keyframes with handles. Pure data and math, no editor and
//! no ECS, so the editor, the saved scene and the runtime share one
//! evaluator.
//!
//! The maths follow Blender 4.x (`BKE_fcurve_handles_recalc`,
//! `BKE_nurb_handle_calc`, `fcurve_eval_keyframes_interpolate`): Auto
//! Clamped handles, handle-overlap correction, constant / linear
//! extrapolation, per-key interpolation (Constant, Linear, Bezier).
//! Rotation channels are Euler degrees interpolated per channel.

use glam::Vec3;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Blender's `BEZT_BINARYSEARCH_THRESH`: frames closer than this are the same frame.
pub const FRAME_EPS: f32 = 0.01;
/// Handle length factor of the automatic handles (`2.5614` in Blender).
const AUTO_HANDLE: f32 = 2.5614;

/// Key selection flags (`Key::select`).
pub const SEL_KEY: u8 = 1;
pub const SEL_LEFT: u8 = 2;
pub const SEL_RIGHT: u8 = 4;
pub const SEL_ALL: u8 = SEL_KEY | SEL_LEFT | SEL_RIGHT;

/// The object property an F-curve drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Prop {
    Location,
    /// Euler angles in degrees, as `EntityDesc::rotation`.
    Rotation,
    Scale,
}

impl Prop {
    pub const ALL: [Prop; 3] = [Prop::Location, Prop::Rotation, Prop::Scale];

    /// Blender's channel label: "X Location", "Y Euler Rotation", "Z Scale".
    pub fn channel_name(self, index: u8) -> String {
        let axis = ["X", "Y", "Z"].get(index as usize).copied().unwrap_or("?");
        let what = match self {
            Prop::Location => "Location",
            Prop::Rotation => "Euler Rotation",
            Prop::Scale => "Scale",
        };
        format!("{axis} {what}")
    }

    pub fn label(self) -> &'static str {
        match self {
            Prop::Location => "Location",
            Prop::Rotation => "Rotation",
            Prop::Scale => "Scale",
        }
    }
}

/// How the curve runs from a key to the next one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Interp {
    Constant,
    Linear,
    #[default]
    Bezier,
}

/// Handle behaviour. `AutoClamped` is Blender's default for new keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum HandleType {
    Free,
    Auto,
    Vector,
    Aligned,
    #[default]
    AutoClamped,
}

impl HandleType {
    fn is_auto(self) -> bool {
        matches!(self, Self::Auto | Self::AutoClamped)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Extrapolation {
    #[default]
    Constant,
    Linear,
}

/// A keyframe (Blender's `BezTriple`): the key and its two handles, all as
/// absolute `[frame, value]` points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Key {
    pub frame: f32,
    pub value: f32,
    pub left: [f32; 2],
    pub right: [f32; 2],
    /// Interpolation of the segment that starts at this key.
    #[cfg_attr(feature = "serde", serde(default))]
    pub interp: Interp,
    #[cfg_attr(feature = "serde", serde(default))]
    pub left_type: HandleType,
    #[cfg_attr(feature = "serde", serde(default))]
    pub right_type: HandleType,
    /// `SEL_*` flags. Not saved.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub select: u8,
}

impl Key {
    /// A new Bezier key with Auto Clamped handles (placeholders until the
    /// curve recalculates them).
    pub fn new(frame: f32, value: f32) -> Self {
        Self {
            frame,
            value,
            left: [frame - 1.0, value],
            right: [frame + 1.0, value],
            interp: Interp::Bezier,
            left_type: HandleType::AutoClamped,
            right_type: HandleType::AutoClamped,
            select: 0,
        }
    }

    pub fn is_selected(&self) -> bool {
        self.select & SEL_KEY != 0
    }

    /// Puts a handle at `to` (never past its key in time). Automatic
    /// handles become aligned, vector ones free; an aligned handle drags its
    /// twin around to stay on one line, keeping the twin's length.
    pub fn set_handle(&mut self, left: bool, mut to: [f32; 2]) {
        let key = [self.frame, self.value];
        if left {
            to[0] = to[0].min(key[0]);
        } else {
            to[0] = to[0].max(key[0]);
        }
        let (own, other) = if left {
            (&mut self.left_type, &mut self.right_type)
        } else {
            (&mut self.right_type, &mut self.left_type)
        };
        *own = match *own {
            HandleType::Auto | HandleType::AutoClamped => HandleType::Aligned,
            HandleType::Vector => HandleType::Free,
            t => t,
        };
        if matches!(*other, HandleType::Auto | HandleType::AutoClamped)
            && *own == HandleType::Aligned
        {
            *other = HandleType::Aligned;
        }
        let aligned = *own == HandleType::Aligned && *other == HandleType::Aligned;
        if left {
            self.left = to;
        } else {
            self.right = to;
        }
        if aligned {
            let dir = [key[0] - to[0], key[1] - to[1]];
            let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
            let twin = if left { self.right } else { self.left };
            let twin_len = ((twin[0] - key[0]).powi(2) + (twin[1] - key[1]).powi(2)).sqrt();
            if len > 1e-6 {
                let p = [
                    key[0] + dir[0] / len * twin_len,
                    key[1] + dir[1] / len * twin_len,
                ];
                if left {
                    self.right = p;
                } else {
                    self.left = p;
                }
            }
        }
    }

    /// Moves the key and both handles by `(df, dv)`.
    fn translate(&mut self, df: f32, dv: f32) {
        self.frame += df;
        self.value += dv;
        self.left[0] += df;
        self.left[1] += dv;
        self.right[0] += df;
        self.right[1] += dv;
    }
}

/// Keyframes of one channel of one object.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FCurve {
    /// `EntityDesc::id`.
    pub object: u64,
    pub prop: Prop,
    /// 0..3 = X, Y, Z.
    pub index: u8,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "is_default_extrapolation")
    )]
    pub extrapolation: Extrapolation,
    /// Ascending by frame.
    pub keys: Vec<Key>,
}

#[cfg(feature = "serde")]
fn is_default_extrapolation(e: &Extrapolation) -> bool {
    *e == Extrapolation::Constant
}

impl FCurve {
    pub fn new(object: u64, prop: Prop, index: u8) -> Self {
        Self {
            object,
            prop,
            index,
            extrapolation: Extrapolation::Constant,
            keys: Vec::new(),
        }
    }

    pub fn matches(&self, object: u64, prop: Prop, index: u8) -> bool {
        self.object == object && self.prop == prop && self.index == index
    }

    pub fn name(&self) -> String {
        self.prop.channel_name(self.index)
    }

    /// Recomputes the automatic and vector handles
    /// (`BKE_fcurve_handles_recalc`).
    pub fn recalc_handles(&mut self) {
        let n = self.keys.len();
        for i in 0..n {
            let prev = (i > 0).then(|| self.keys[i - 1]);
            let next = (i + 1 < n).then(|| self.keys[i + 1]);
            calc_handle(&mut self.keys[i], prev, next);
            // Automatic first and last keys ease in and out when the curve
            // holds its value outside the key range.
            let key = &mut self.keys[i];
            if key.left_type.is_auto()
                && key.right_type.is_auto()
                && (i == 0 || i == n - 1)
                && self.extrapolation == Extrapolation::Constant
            {
                key.left[1] = key.value;
                key.right[1] = key.value;
            }
        }
    }

    /// Sorts by frame, then recalculates handles.
    pub fn sort(&mut self) {
        self.keys.sort_by(|a, b| a.frame.total_cmp(&b.frame));
        self.recalc_handles();
    }

    /// The key at `frame` (within [`FRAME_EPS`]).
    pub fn key_at(&self, frame: f32) -> Option<usize> {
        self.keys
            .iter()
            .position(|k| (k.frame - frame).abs() < FRAME_EPS)
    }

    /// Inserts a key like Blender's `insert_vert_fcurve`: an existing key on
    /// that frame only takes the new value (its handles follow), a new key
    /// between others inherits the interpolation of the previous key.
    /// Returns the key's index.
    pub fn insert_key(&mut self, frame: f32, value: f32) -> usize {
        if let Some(i) = self.key_at(frame) {
            let k = &mut self.keys[i];
            let dv = value - k.value;
            k.value = value;
            k.left[1] += dv;
            k.right[1] += dv;
            self.recalc_handles();
            return i;
        }
        let mut key = Key::new(frame, value);
        let at = self.keys.partition_point(|k| k.frame < frame);
        if self.keys.len() >= 2 && at > 0 {
            key.interp = self.keys[at - 1].interp;
        }
        self.keys.insert(at, key);
        self.recalc_handles();
        at
    }

    /// Deletes the key at `frame`; `true` if there was one.
    pub fn delete_key(&mut self, frame: f32) -> bool {
        let Some(i) = self.key_at(frame) else {
            return false;
        };
        self.keys.remove(i);
        self.recalc_handles();
        true
    }

    /// The curve's value at `frame`, `None` without keys
    /// (`fcurve_eval_keyframes`).
    pub fn eval(&self, frame: f32) -> Option<f32> {
        let (first, last) = (self.keys.first()?, self.keys.last()?);
        if let Some(k) = self.key_at(frame) {
            return Some(self.keys[k].value);
        }
        if frame < first.frame {
            return Some(self.extrapolate_before(first, frame));
        }
        if frame > last.frame {
            return Some(self.extrapolate_after(last, frame));
        }
        let hi = self.keys.partition_point(|k| k.frame <= frame);
        let (a, b) = (&self.keys[hi - 1], &self.keys[hi]);
        Some(match a.interp {
            Interp::Constant => a.value,
            Interp::Linear => {
                let dx = b.frame - a.frame;
                if dx == 0.0 {
                    a.value
                } else {
                    a.value + (b.value - a.value) * (frame - a.frame) / dx
                }
            }
            Interp::Bezier => bezier_segment(a, b, frame),
        })
    }

    fn extrapolate_before(&self, first: &Key, frame: f32) -> f32 {
        if self.extrapolation == Extrapolation::Constant || self.keys.len() < 2 {
            return first.value;
        }
        let slope = match first.interp {
            Interp::Bezier => slope(first.left, [first.frame, first.value]),
            Interp::Constant => 0.0,
            Interp::Linear => {
                let next = &self.keys[1];
                slope([first.frame, first.value], [next.frame, next.value])
            }
        };
        first.value + slope * (frame - first.frame)
    }

    fn extrapolate_after(&self, last: &Key, frame: f32) -> f32 {
        if self.extrapolation == Extrapolation::Constant || self.keys.len() < 2 {
            return last.value;
        }
        let prev = &self.keys[self.keys.len() - 2];
        let slope = match last.interp {
            Interp::Bezier => slope([last.frame, last.value], last.right),
            Interp::Constant => 0.0,
            Interp::Linear => slope([prev.frame, prev.value], [last.frame, last.value]),
        };
        last.value + slope * (frame - last.frame)
    }

    /// Frame range of the keys.
    pub fn range(&self) -> Option<(f32, f32)> {
        Some((self.keys.first()?.frame, self.keys.last()?.frame))
    }
}

fn slope(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = b[0] - a[0];
    if dx == 0.0 { 0.0 } else { (b[1] - a[1]) / dx }
}

/// `BKE_nurb_handle_calc` for F-curves (no smoothing mode).
fn calc_handle(bezt: &mut Key, prev: Option<Key>, next: Option<Key>) {
    let p2 = [bezt.frame, bezt.value];
    let (p1, p3) = match (prev, next) {
        (None, None) => return,
        (Some(p), Some(n)) => ([p.frame, p.value], [n.frame, n.value]),
        // The missing neighbour is the other one mirrored through the key.
        (None, Some(n)) => (
            [2.0 * p2[0] - n.frame, 2.0 * p2[1] - n.value],
            [n.frame, n.value],
        ),
        (Some(p), None) => (
            [p.frame, p.value],
            [2.0 * p2[0] - p.frame, 2.0 * p2[1] - p.value],
        ),
    };
    let dvec_a = [p2[0] - p1[0], p2[1] - p1[1]];
    let dvec_b = [p3[0] - p2[0], p3[1] - p2[1]];
    let mut len_a = dvec_a[0];
    let mut len_b = dvec_b[0];
    if len_a == 0.0 {
        len_a = 1.0;
    }
    if len_b == 0.0 {
        len_b = 1.0;
    }

    if bezt.left_type.is_auto() || bezt.right_type.is_auto() {
        let tvec = [
            dvec_b[0] / len_b + dvec_a[0] / len_a,
            dvec_b[1] / len_b + dvec_a[1] / len_a,
        ];
        let len = tvec[0] * AUTO_HANDLE;
        if len != 0.0 {
            let (mut left_violate, mut right_violate) = (false, false);
            if len_a > 5.0 * len_b {
                len_a = 5.0 * len_b;
            }
            if len_b > 5.0 * len_a {
                len_b = 5.0 * len_a;
            }
            let clamped_pair = prev.zip(next);
            if bezt.left_type.is_auto() {
                let f = len_a / len;
                bezt.left = [p2[0] - tvec[0] * f, p2[1] - tvec[1] * f];
                if bezt.left_type == HandleType::AutoClamped
                    && let Some((p, n)) = clamped_pair
                {
                    let ydiff1 = p.value - bezt.value;
                    let ydiff2 = n.value - bezt.value;
                    if (ydiff1 <= 0.0 && ydiff2 <= 0.0) || (ydiff1 >= 0.0 && ydiff2 >= 0.0) {
                        bezt.left[1] = bezt.value;
                        left_violate = true;
                    } else if ydiff1 <= 0.0 {
                        if p.value > bezt.left[1] {
                            bezt.left[1] = p.value;
                            left_violate = true;
                        }
                    } else if p.value < bezt.left[1] {
                        bezt.left[1] = p.value;
                        left_violate = true;
                    }
                }
            }
            if bezt.right_type.is_auto() {
                let f = len_b / len;
                bezt.right = [p2[0] + tvec[0] * f, p2[1] + tvec[1] * f];
                if bezt.right_type == HandleType::AutoClamped
                    && let Some((p, n)) = clamped_pair
                {
                    let ydiff1 = p.value - bezt.value;
                    let ydiff2 = n.value - bezt.value;
                    if (ydiff1 <= 0.0 && ydiff2 <= 0.0) || (ydiff1 >= 0.0 && ydiff2 >= 0.0) {
                        bezt.right[1] = bezt.value;
                        right_violate = true;
                    } else if ydiff1 <= 0.0 {
                        if n.value < bezt.right[1] {
                            bezt.right[1] = n.value;
                            right_violate = true;
                        }
                    } else if n.value > bezt.right[1] {
                        bezt.right[1] = n.value;
                        right_violate = true;
                    }
                }
            }
            if left_violate || right_violate {
                // Keep both handles on one line through the key.
                let h1_x = bezt.left[0] - p2[0];
                let h2_x = p2[0] - bezt.right[0];
                if left_violate {
                    bezt.right[1] = p2[1] + (p2[1] - bezt.left[1]) / h1_x * h2_x;
                } else {
                    bezt.left[1] = p2[1] + (p2[1] - bezt.right[1]) / h2_x * h1_x;
                }
            }
        }
    }
    if bezt.left_type == HandleType::Vector {
        bezt.left = [p2[0] - dvec_a[0] / 3.0, p2[1] - dvec_a[1] / 3.0];
    }
    if bezt.right_type == HandleType::Vector {
        bezt.right = [p2[0] + dvec_b[0] / 3.0, p2[1] + dvec_b[1] / 3.0];
    }
}

/// Bezier segment value at `frame` (`BKE_fcurve_correct_bezpart` + `findzero`).
fn bezier_segment(a: &Key, b: &Key, frame: f32) -> f32 {
    let v1 = [a.frame, a.value];
    let mut v2 = a.right;
    let mut v3 = b.left;
    let v4 = [b.frame, b.value];
    let eps = f32::EPSILON;
    if (v1[1] - v4[1]).abs() < eps && (v2[1] - v3[1]).abs() < eps && (v3[1] - v4[1]).abs() < eps {
        return v1[1];
    }
    // Handles may not overlap or loop.
    let (h1, h2) = (v1[0] - v2[0], v4[0] - v3[0]);
    let len = v4[0] - v1[0];
    if len >= eps {
        let (len1, len2) = (h1.abs(), h2.abs());
        if len1 + len2 != 0.0 && len1 + len2 > len {
            let fac = len / (len1 + len2);
            v2 = [v1[0] - fac * h1, v1[1] - fac * (v1[1] - v2[1])];
            v3 = [v4[0] - fac * h2, v4[1] - fac * (v4[1] - v3[1])];
        }
    }
    let (x1, x2, x3, x4) = (v1[0] as f64, v2[0] as f64, v3[0] as f64, v4[0] as f64);
    let target = frame as f64;
    let bez = |p1: f64, p2: f64, p3: f64, p4: f64, t: f64| {
        let s = 1.0 - t;
        s * s * s * p1 + 3.0 * s * s * t * p2 + 3.0 * s * t * t * p3 + t * t * t * p4
    };
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..64 {
        let mid = 0.5 * (lo + hi);
        if bez(x1, x2, x3, x4, mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let t = 0.5 * (lo + hi);
    bez(v1[1] as f64, v2[1] as f64, v3[1] as f64, v4[1] as f64, t) as f32
}

/// Which channels "I" keys (Blender's Default Key Channels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyChannels {
    pub location: bool,
    pub rotation: bool,
    pub scale: bool,
}

impl Default for KeyChannels {
    fn default() -> Self {
        Self {
            location: true,
            rotation: true,
            scale: true,
        }
    }
}

impl KeyChannels {
    pub fn includes(self, prop: Prop) -> bool {
        match prop {
            Prop::Location => self.location,
            Prop::Rotation => self.rotation,
            Prop::Scale => self.scale,
        }
    }
}

/// How an animated property is shown in the UI (Blender's field colours).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    /// Not animated.
    None,
    /// Animated, the frame holds a key (yellow).
    Keyed,
    /// Animated, between keys (green).
    Animated,
    /// Animated but the value was changed since evaluation (orange).
    Changed,
}

/// The animation of a scene: F-curves and the playback range.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Action {
    pub fps: f32,
    pub frame_start: i32,
    pub frame_end: i32,
    /// The game starts the animation by itself.
    pub autoplay: bool,
    /// The game loops it.
    pub looping: bool,
    pub curves: Vec<FCurve>,
}

impl Default for Action {
    /// Blender's scene defaults: 24 fps, frames 1..250.
    fn default() -> Self {
        Self {
            fps: 24.0,
            frame_start: 1,
            frame_end: 250,
            autoplay: true,
            looping: true,
            curves: Vec::new(),
        }
    }
}

/// Keys copied by Ctrl+C: (object, prop, index, keys).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyClipboard {
    pub entries: Vec<(u64, Prop, u8, Vec<Key>)>,
}

impl Action {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn curve(&self, object: u64, prop: Prop, index: u8) -> Option<&FCurve> {
        self.curves.iter().find(|c| c.matches(object, prop, index))
    }

    /// Objects with curves, in order of first appearance.
    pub fn objects(&self) -> Vec<u64> {
        let mut out = Vec::new();
        for c in &self.curves {
            if !out.contains(&c.object) {
                out.push(c.object);
            }
        }
        out
    }

    pub fn has_object(&self, object: u64) -> bool {
        self.curves.iter().any(|c| c.object == object)
    }

    /// Same curves and keys, ignoring selection.
    pub fn same_data(&self, other: &Action) -> bool {
        self.fps == other.fps
            && self.frame_start == other.frame_start
            && self.frame_end == other.frame_end
            && self.curves.len() == other.curves.len()
            && self.curves.iter().zip(&other.curves).all(|(a, b)| {
                a.matches(b.object, b.prop, b.index)
                    && a.extrapolation == b.extrapolation
                    && a.keys.len() == b.keys.len()
                    && a.keys
                        .iter()
                        .zip(&b.keys)
                        .all(|(x, y)| Key { select: 0, ..*x } == Key { select: 0, ..*y })
            })
    }

    /// Evaluates one channel (`None`: not animated).
    pub fn eval(&self, object: u64, prop: Prop, index: u8, frame: f32) -> Option<f32> {
        self.curve(object, prop, index)?.eval(frame)
    }

    /// Inserts a key on one channel, creating the curve if needed.
    pub fn insert_key(&mut self, object: u64, prop: Prop, index: u8, frame: f32, value: f32) {
        let i = match self
            .curves
            .iter()
            .position(|c| c.matches(object, prop, index))
        {
            Some(i) => i,
            None => {
                self.curves.push(FCurve::new(object, prop, index));
                self.curves.len() - 1
            }
        };
        self.curves[i].insert_key(frame, value);
    }

    /// Keys all three components of `prop`.
    pub fn insert_vec3(&mut self, object: u64, prop: Prop, frame: f32, v: Vec3) {
        for i in 0..3 {
            self.insert_key(object, prop, i as u8, frame, v[i]);
        }
    }

    /// Deletes the keys of `object` on `frame` (all channels or only `prop`);
    /// curves left without keys disappear. Returns how many keys went.
    pub fn delete_keys_at(&mut self, object: u64, prop: Option<Prop>, frame: f32) -> usize {
        let mut removed = 0;
        for c in &mut self.curves {
            if c.object == object && prop.is_none_or(|p| p == c.prop) {
                removed += usize::from(c.delete_key(frame));
            }
        }
        self.curves.retain(|c| !c.keys.is_empty());
        removed
    }

    /// The colour state of one property component at `frame`, given its
    /// current value.
    pub fn key_state(
        &self,
        object: u64,
        prop: Prop,
        index: u8,
        frame: f32,
        current: f32,
    ) -> KeyState {
        let Some(c) = self.curve(object, prop, index) else {
            return KeyState::None;
        };
        let Some(v) = c.eval(frame) else {
            return KeyState::None;
        };
        if (v - current).abs() > 1e-4 * v.abs().max(1.0) {
            KeyState::Changed
        } else if c.key_at(frame).is_some() {
            KeyState::Keyed
        } else {
            KeyState::Animated
        }
    }

    /// The nearest key frame after (`forward`) or before `frame` among the
    /// curves of `objects` (everything when `None`).
    pub fn jump_frame(&self, frame: f32, forward: bool, objects: Option<&[u64]>) -> Option<f32> {
        let frames = self
            .curves
            .iter()
            .filter(|c| objects.is_none_or(|o| o.contains(&c.object)))
            .flat_map(|c| c.keys.iter().map(|k| k.frame));
        if forward {
            frames
                .filter(|f| *f > frame + FRAME_EPS)
                .min_by(|a, b| a.total_cmp(b))
        } else {
            frames
                .filter(|f| *f < frame - FRAME_EPS)
                .max_by(|a, b| a.total_cmp(b))
        }
    }

    // --- selection ------------------------------------------------------

    pub fn select_all(&mut self, select: bool) {
        for k in self.curves.iter_mut().flat_map(|c| &mut c.keys) {
            k.select = if select { SEL_ALL } else { 0 };
        }
    }

    pub fn selected_count(&self) -> usize {
        self.curves
            .iter()
            .flat_map(|c| &c.keys)
            .filter(|k| k.is_selected())
            .count()
    }

    // --- editing selected keys -----------------------------------------

    /// Re-sorts every curve and recomputes handles.
    pub fn resort(&mut self) {
        for c in &mut self.curves {
            c.sort();
        }
    }

    /// Moves the selected keys (and handles) by `df` frames, then like
    /// Blender removes unselected keys that were landed on.
    pub fn move_selected(&mut self, df: f32) {
        for c in &mut self.curves {
            for k in c.keys.iter_mut().filter(|k| k.is_selected()) {
                k.translate(df, 0.0);
            }
        }
        self.finish_transform();
    }

    /// Graph Editor grab: selected keys move by `(df, dv)` with their
    /// handles; a handle selected on its own moves alone (an automatic
    /// handle turns into an aligned one, like Blender).
    pub fn translate_selected(&mut self, df: f32, dv: f32) {
        for c in &mut self.curves {
            for k in &mut c.keys {
                if k.select & SEL_KEY != 0 {
                    k.translate(df, dv);
                    continue;
                }
                for left in [true, false] {
                    let bit = if left { SEL_LEFT } else { SEL_RIGHT };
                    if k.select & bit != 0 {
                        let h = if left { k.left } else { k.right };
                        k.set_handle(left, [h[0] + df, h[1] + dv]);
                    }
                }
            }
        }
        self.finish_transform();
    }

    /// Scales the selected keys' frames around `origin`.
    pub fn scale_selected(&mut self, origin: f32, factor: f32, snap: bool) {
        for c in &mut self.curves {
            for k in c.keys.iter_mut().filter(|k| k.is_selected()) {
                let mut f = origin + (k.frame - origin) * factor;
                if snap {
                    f = f.round();
                }
                let (dl, dr) = (k.left[0] - k.frame, k.right[0] - k.frame);
                k.frame = f;
                k.left[0] = f + dl * factor;
                k.right[0] = f + dr * factor;
            }
        }
        self.finish_transform();
    }

    /// Rounds the selected keys to whole frames.
    pub fn snap_selected(&mut self) {
        for c in &mut self.curves {
            for k in c.keys.iter_mut().filter(|k| k.is_selected()) {
                let df = k.frame.round() - k.frame;
                k.translate(df, 0.0);
            }
        }
        self.finish_transform();
    }

    fn finish_transform(&mut self) {
        for c in &mut self.curves {
            // An unselected key under a selected one is overwritten.
            let moved: Vec<f32> = c
                .keys
                .iter()
                .filter(|k| k.is_selected())
                .map(|k| k.frame)
                .collect();
            c.keys.retain(|k| {
                k.is_selected() || !moved.iter().any(|f| (f - k.frame).abs() < FRAME_EPS)
            });
            c.sort();
        }
    }

    /// Removes the selected keys and the curves left empty.
    pub fn delete_selected(&mut self) {
        for c in &mut self.curves {
            c.keys.retain(|k| !k.is_selected());
            c.recalc_handles();
        }
        self.curves.retain(|c| !c.keys.is_empty());
    }

    /// Copies the selected keys next to the originals' selection state:
    /// the copies end up selected, the originals deselected (Shift+D).
    pub fn duplicate_selected(&mut self) {
        for c in &mut self.curves {
            let copies: Vec<Key> = c.keys.iter().filter(|k| k.is_selected()).copied().collect();
            for k in c.keys.iter_mut() {
                k.select = 0;
            }
            c.keys.extend(copies);
            c.keys.sort_by(|a, b| a.frame.total_cmp(&b.frame));
            c.recalc_handles();
        }
    }

    pub fn set_interp_selected(&mut self, interp: Interp) {
        for k in self.curves.iter_mut().flat_map(|c| &mut c.keys) {
            if k.is_selected() {
                k.interp = interp;
            }
        }
    }

    pub fn set_handle_type_selected(&mut self, kind: HandleType) {
        for c in &mut self.curves {
            for k in c.keys.iter_mut().filter(|k| k.is_selected()) {
                k.left_type = kind;
                k.right_type = kind;
            }
            c.recalc_handles();
        }
    }

    pub fn set_extrapolation_selected_curves(
        &mut self,
        e: Extrapolation,
        only: &[(u64, Prop, u8)],
    ) {
        for c in &mut self.curves {
            if only.contains(&(c.object, c.prop, c.index)) {
                c.extrapolation = e;
                c.recalc_handles();
            }
        }
    }

    pub fn copy_selected(&self) -> KeyClipboard {
        let entries = self
            .curves
            .iter()
            .filter_map(|c| {
                let keys: Vec<Key> = c.keys.iter().filter(|k| k.is_selected()).copied().collect();
                (!keys.is_empty()).then_some((c.object, c.prop, c.index, keys))
            })
            .collect();
        KeyClipboard { entries }
    }

    /// Pastes with the earliest copied key landing on `frame`; the pasted
    /// keys are selected, everything else deselected.
    pub fn paste(&mut self, clip: &KeyClipboard, frame: f32) {
        let Some(first) = clip
            .entries
            .iter()
            .flat_map(|e| e.3.iter().map(|k| k.frame))
            .min_by(|a, b| a.total_cmp(b))
        else {
            return;
        };
        self.select_all(false);
        for (object, prop, index, keys) in &clip.entries {
            let i = match self
                .curves
                .iter()
                .position(|c| c.matches(*object, *prop, *index))
            {
                Some(i) => i,
                None => {
                    self.curves.push(FCurve::new(*object, *prop, *index));
                    self.curves.len() - 1
                }
            };
            let c = &mut self.curves[i];
            let moved: Vec<Key> = keys
                .iter()
                .map(|k| {
                    let mut k = *k;
                    k.translate(frame - first, 0.0);
                    k.select = SEL_ALL;
                    k
                })
                .collect();
            c.keys
                .retain(|k| !moved.iter().any(|m| (m.frame - k.frame).abs() < FRAME_EPS));
            c.keys.extend(moved);
            c.sort();
        }
    }

    /// Frame range of every key.
    pub fn key_range(&self) -> Option<(f32, f32)> {
        let mut out: Option<(f32, f32)> = None;
        for (a, b) in self.curves.iter().filter_map(FCurve::range) {
            out = Some(out.map_or((a, b), |(lo, hi)| (lo.min(a), hi.max(b))));
        }
        out
    }

    /// Frames of the whole range, both ends included.
    pub fn frame_count(&self) -> i32 {
        (self.frame_end - self.frame_start + 1).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(keys: &[(f32, f32)]) -> FCurve {
        let mut c = FCurve::new(1, Prop::Location, 0);
        for (f, v) in keys {
            c.insert_key(*f, *v);
        }
        c
    }

    fn near(a: f32, b: f32) {
        assert!((a - b).abs() < 2e-4, "{a} != {b}");
    }

    /// Reference values from an independent Python implementation of
    /// Blender's handle and bezier code (see the test names).
    #[test]
    fn auto_clamped_peak_has_flat_middle_handles() {
        let c = curve(&[(1.0, 0.0), (10.0, 10.0), (20.0, 0.0)]);
        let mid = c.keys[1];
        assert_eq!(mid.left[1], 10.0);
        assert_eq!(mid.right[1], 10.0);
        near(mid.left[0], 6.4863);
        near(mid.right[0], 13.9041);
        // Ends ease in and out (constant extrapolation).
        assert_eq!(c.keys[0].right[1], 0.0);
        assert_eq!(c.keys[2].left[1], 0.0);
        near(c.keys[0].right[0], 4.5137);
        // Values never exceed the key on either side: no overshoot.
        for i in 0..=190 {
            let v = c.eval(1.0 + i as f32 / 10.0).unwrap();
            assert!((-1e-4..=10.0 + 1e-4).contains(&v), "overshoot {v}");
        }
        assert_eq!(c.eval(10.0), Some(10.0));
        near(c.eval(5.5).unwrap(), 5.0);
        near(c.eval(15.0).unwrap(), 5.0);
    }

    #[test]
    fn bezier_matches_reference_values() {
        let c = curve(&[(1.0, 0.0), (10.0, 10.0), (20.0, 0.0)]);
        // From the python reference (bisection on x(t), then y(t)).
        for (frame, expected) in [
            (2.0, 0.273937),
            (3.0, 1.095835),
            (7.25, 7.955148),
            (12.5, 8.617156),
            (18.0, 0.888889),
        ] {
            near(c.eval(frame).unwrap(), expected);
        }
    }

    #[test]
    fn monotonic_keys_keep_auto_handles_inside() {
        let c = curve(&[(1.0, 0.0), (11.0, 5.0), (21.0, 10.0)]);
        let mid = c.keys[1];
        // Straight line through three equally spaced keys.
        near((mid.right[1] - mid.value) / (mid.right[0] - mid.frame), 0.5);
        let v = c.eval(16.0).unwrap();
        assert!(v > 5.0 && v < 10.0);
    }

    #[test]
    fn clamps_when_a_neighbour_is_much_lower() {
        // Steep rise then a plateau: the left handle must not dip below the
        // previous key.
        let c = curve(&[(1.0, 0.0), (3.0, 1.0), (30.0, 100.0)]);
        let mid = c.keys[1];
        assert!(mid.left[1] >= 0.0, "left handle below previous key");
        for i in 0..=290 {
            let v = c.eval(1.0 + i as f32 / 10.0).unwrap();
            assert!((-1e-3..=100.001).contains(&v), "{v}");
        }
        // The first segment stays between its keys.
        for i in 0..=20 {
            let v = c.eval(1.0 + i as f32 / 10.0).unwrap();
            assert!((-1e-3..=1.001).contains(&v), "{v}");
        }
    }

    #[test]
    fn linear_and_constant_interpolation() {
        let mut c = curve(&[(1.0, 0.0), (11.0, 10.0), (21.0, 0.0)]);
        c.keys[0].interp = Interp::Linear;
        c.keys[1].interp = Interp::Constant;
        near(c.eval(3.5).unwrap(), 2.5);
        near(c.eval(15.0).unwrap(), 10.0);
        near(c.eval(20.9).unwrap(), 10.0);
        assert_eq!(c.eval(21.0), Some(0.0));
    }

    #[test]
    fn constant_extrapolation_by_default() {
        let c = curve(&[(10.0, 3.0), (20.0, 7.0)]);
        assert_eq!(c.eval(-100.0), Some(3.0));
        assert_eq!(c.eval(1000.0), Some(7.0));
        assert_eq!(FCurve::new(1, Prop::Scale, 0).eval(1.0), None);
    }

    #[test]
    fn linear_extrapolation_follows_the_end_slope() {
        let mut c = curve(&[(1.0, 0.0), (11.0, 10.0)]);
        for k in &mut c.keys {
            k.interp = Interp::Linear;
        }
        c.extrapolation = Extrapolation::Linear;
        c.recalc_handles();
        near(c.eval(21.0).unwrap(), 20.0);
        near(c.eval(-9.0).unwrap(), -10.0);
    }

    #[test]
    fn insert_replaces_on_the_same_frame_and_inherits_interpolation() {
        let mut c = curve(&[(1.0, 0.0), (10.0, 5.0), (20.0, 0.0)]);
        c.keys[1].interp = Interp::Linear;
        c.insert_key(10.0, 8.0);
        assert_eq!(c.keys.len(), 3);
        assert_eq!(c.keys[1].value, 8.0);
        assert_eq!(c.keys[1].interp, Interp::Linear);
        c.insert_key(15.0, 1.0);
        assert_eq!(c.keys[2].frame, 15.0);
        assert_eq!(
            c.keys[2].interp,
            Interp::Linear,
            "inherited from the previous key"
        );
        assert!(c.delete_key(15.0));
        assert!(!c.delete_key(15.0));
    }

    #[test]
    fn action_keys_and_states() {
        let mut a = Action::default();
        a.insert_vec3(7, Prop::Location, 1.0, Vec3::new(0.0, 0.0, 0.0));
        a.insert_vec3(7, Prop::Location, 11.0, Vec3::new(10.0, 0.0, 0.0));
        let st = |f, v| a.key_state(7, Prop::Location, 0, f, v);
        assert_eq!(st(1.0, 0.0), KeyState::Keyed);
        assert_eq!(
            st(6.0, a.eval(7, Prop::Location, 0, 6.0).unwrap()),
            KeyState::Animated
        );
        assert_eq!(st(6.0, 99.0), KeyState::Changed);
        assert_eq!(st(1.0, 3.0), KeyState::Changed);
        assert_eq!(a.key_state(7, Prop::Scale, 0, 1.0, 1.0), KeyState::None);
        assert_eq!(a.jump_frame(1.0, true, None), Some(11.0));
        assert_eq!(a.jump_frame(11.0, false, Some(&[7])), Some(1.0));
        assert_eq!(a.jump_frame(11.0, true, None), None);
        assert_eq!(a.jump_frame(1.0, true, Some(&[8])), None);
        assert_eq!(a.delete_keys_at(7, None, 11.0), 3);
        assert_eq!(a.delete_keys_at(7, Some(Prop::Location), 1.0), 3);
        assert!(a.curves.is_empty());
    }

    #[test]
    fn move_scale_duplicate_paste_delete() {
        let mut a = Action::default();
        for f in [1.0, 5.0, 9.0] {
            a.insert_key(1, Prop::Location, 0, f, f);
        }
        a.curves[0].keys[1].select = SEL_ALL;
        a.curves[0].keys[2].select = SEL_ALL;
        a.move_selected(3.0);
        let frames: Vec<f32> = a.curves[0].keys.iter().map(|k| k.frame).collect();
        assert_eq!(frames, [1.0, 8.0, 12.0]);
        // Landing on an unselected key replaces it.
        a.move_selected(-7.0);
        let frames: Vec<f32> = a.curves[0].keys.iter().map(|k| k.frame).collect();
        assert_eq!(frames, [1.0, 5.0], "{frames:?}");
        a.select_all(true);
        a.scale_selected(1.0, 2.0, true);
        let frames: Vec<f32> = a.curves[0].keys.iter().map(|k| k.frame).collect();
        assert_eq!(frames, [1.0, 9.0]);
        a.select_all(false);
        a.curves[0].keys[1].select = SEL_ALL;
        let clip = a.copy_selected();
        a.paste(&clip, 20.0);
        assert_eq!(a.curves[0].keys.len(), 3);
        assert_eq!(a.curves[0].keys[2].frame, 20.0);
        a.duplicate_selected();
        assert_eq!(a.curves[0].keys.len(), 4);
        a.delete_selected();
        assert_eq!(a.curves[0].keys.len(), 3);
        a.select_all(true);
        a.delete_selected();
        assert!(a.curves.is_empty());
    }

    #[test]
    fn dragging_an_auto_handle_makes_it_aligned_and_twists_the_twin() {
        let mut c = curve(&[(1.0, 0.0), (10.0, 10.0), (20.0, 0.0)]);
        let len_before = {
            let k = c.keys[1];
            (k.left[0] - k.frame).hypot(k.left[1] - k.value)
        };
        c.keys[1].set_handle(false, [14.0, 6.0]);
        let k = c.keys[1];
        assert_eq!(
            (k.left_type, k.right_type),
            (HandleType::Aligned, HandleType::Aligned)
        );
        assert_eq!(k.right, [14.0, 6.0]);
        assert!(k.left[1] > 10.0, "twin goes the other way: {:?}", k.left);
        // The twin keeps its own length and the two stay collinear.
        let twin = (k.left[0] - k.frame).hypot(k.left[1] - k.value);
        assert!((twin - len_before).abs() < 1e-3);
        let cross = (k.right[0] - k.frame) * (k.left[1] - k.value)
            - (k.right[1] - k.value) * (k.left[0] - k.frame);
        assert!(cross.abs() < 1e-3);
        // Aligned handles survive recalculation; handles never cross their key.
        c.recalc_handles();
        assert_eq!(c.keys[1].right, [14.0, 6.0]);
        c.keys[1].set_handle(false, [3.0, 6.0]);
        assert_eq!(c.keys[1].right[0], 10.0);
    }

    #[test]
    fn graph_translate_moves_keys_with_handles_and_lone_handles_alone() {
        let mut a = Action::default();
        for f in [1.0, 10.0] {
            a.insert_key(1, Prop::Location, 0, f, 0.0);
        }
        a.curves[0].keys[1].select = SEL_ALL;
        a.translate_selected(2.0, 5.0);
        let k = a.curves[0].keys[1];
        assert_eq!((k.frame, k.value), (12.0, 5.0));
        a.select_all(false);
        a.curves[0].keys[1].select = SEL_LEFT;
        a.translate_selected(0.0, 1.0);
        let k = a.curves[0].keys[1];
        assert_eq!(k.frame, 12.0);
        assert_eq!(k.left[1], 6.0 + 0.0 * k.value);
    }

    #[test]
    fn selection_is_not_data() {
        let mut a = Action::default();
        a.insert_key(1, Prop::Scale, 1, 1.0, 1.0);
        let b = a.clone();
        a.select_all(true);
        assert!(a.same_data(&b));
        assert_ne!(a, b);
    }
}

//! The cutscene file format and its checks.

use std::fmt;
use std::path::Path;

use glam::Vec3;
use olvin_scene::Transform;
use olvin_scene::file::euler_to_quat;
use serde::{Deserialize, Serialize};

/// Limits that keep a damaged or hostile file from eating memory or time.
const MAX_TRACKS: usize = 256;
const MAX_KEYS: usize = 20_000;
const MAX_CUES: usize = 4096;
const MAX_NAME: usize = 256;
const MAX_DURATION: f32 = 3600.0;

/// How a key blends into the next one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ease {
    #[default]
    Linear,
    /// Slow in, slow out (smoothstep).
    Smooth,
    /// Holds the value until the next key.
    Step,
}

/// One key of a track. A channel left `None` is not touched by this key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Key {
    pub time: f32,
    pub position: Option<[f32; 3]>,
    /// Euler angles in degrees, as the scene stores them.
    pub rotation: Option<[f32; 3]>,
    pub scale: Option<[f32; 3]>,
    /// For the segment that starts at this key.
    pub ease: Ease,
}

/// Moves the object called `target`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TransformTrack {
    pub target: String,
    pub keys: Vec<Key>,
}

/// A value carried by a message cue.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum CueValue {
    #[default]
    Nil,
    Bool(bool),
    Number(f64),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CueAction {
    /// Draw this camera (an object with a `Camera`); an empty name hands the
    /// view back to the game's own camera.
    Camera(String),
    /// A script message: to the object `target`, or to everyone.
    Message {
        name: String,
        #[serde(default)]
        value: CueValue,
        #[serde(default)]
        target: Option<String>,
    },
    /// A sound from the assets folder (needs the `audio` feature).
    Sound {
        path: String,
        #[serde(default = "full_volume")]
        volume: f32,
    },
}

fn full_volume() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    pub time: f32,
    pub action: CueAction,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cutscene {
    pub name: String,
    /// Seconds. A track or cue past it is clamped to it.
    pub duration: f32,
    pub tracks: Vec<TransformTrack>,
    pub cues: Vec<Cue>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CutsceneError {
    Io(String),
    Parse(String),
    Invalid(String),
}

impl fmt::Display for CutsceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) | Self::Parse(e) | Self::Invalid(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for CutsceneError {}

impl Cutscene {
    /// Parses and checks RON text; cues come back sorted by time.
    pub fn from_ron(text: &str) -> Result<Self, CutsceneError> {
        let mut cutscene: Cutscene =
            ron::from_str(text).map_err(|e| CutsceneError::Parse(e.to_string()))?;
        cutscene.check()?;
        Ok(cutscene)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, CutsceneError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| CutsceneError::Io(format!("{}: {e}", path.display())))?;
        Self::from_ron(&text)
    }

    pub fn to_ron(&self) -> Result<String, CutsceneError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new())
            .map_err(|e| CutsceneError::Parse(e.to_string()))
    }

    /// Rejects unusable numbers and absurd sizes, sorts keys and cues.
    pub fn check(&mut self) -> Result<(), CutsceneError> {
        let bad = |what: String| Err(CutsceneError::Invalid(what));
        if !self.duration.is_finite() || !(0.0..=MAX_DURATION).contains(&self.duration) {
            return bad(format!("duration must be 0..{MAX_DURATION} seconds"));
        }
        if self.tracks.len() > MAX_TRACKS || self.cues.len() > MAX_CUES {
            return bad(format!("more than {MAX_TRACKS} tracks or {MAX_CUES} cues"));
        }
        if self.tracks.iter().map(|t| t.keys.len()).sum::<usize>() > MAX_KEYS {
            return bad(format!("more than {MAX_KEYS} keys"));
        }
        let finite3 = |v: &Option<[f32; 3]>| v.is_none_or(|a| a.iter().all(|x| x.is_finite()));
        for track in &mut self.tracks {
            if track.target.len() > MAX_NAME {
                return bad("a track target name is too long".into());
            }
            for key in &track.keys {
                if !key.time.is_finite() || key.time < 0.0 {
                    return bad(format!(
                        "track `{}`: key times must be 0 or more",
                        track.target
                    ));
                }
                if !(finite3(&key.position) && finite3(&key.rotation) && finite3(&key.scale)) {
                    return bad(format!(
                        "track `{}`: a key holds a non-finite number",
                        track.target
                    ));
                }
            }
            track.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        }
        for cue in &self.cues {
            if !cue.time.is_finite() || cue.time < 0.0 {
                return bad("cue times must be 0 or more".into());
            }
            match &cue.action {
                CueAction::Camera(n) if n.len() > MAX_NAME => {
                    return bad("a camera name is too long".into());
                }
                CueAction::Message { name, target, .. }
                    if name.len() > MAX_NAME
                        || target.as_ref().is_some_and(|t| t.len() > MAX_NAME) =>
                {
                    return bad("a message name is too long".into());
                }
                CueAction::Sound { path, volume } => {
                    if !volume.is_finite() {
                        return bad("a sound volume is not a number".into());
                    }
                    if !safe_path(path) {
                        return bad(format!(
                            "sound path `{path}` must stay inside the assets folder"
                        ));
                    }
                }
                _ => {}
            }
        }
        self.cues.sort_by(|a, b| a.time.total_cmp(&b.time));
        Ok(())
    }
}

/// A relative path with no `..`, no root and no drive.
pub(crate) fn safe_path(path: &str) -> bool {
    use std::path::Component;
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

impl TransformTrack {
    /// The object's transform at `t` seconds, starting from `base`: a channel
    /// no key sets stays as in `base`; before the first key of a channel it
    /// holds that key, after the last it holds the last.
    pub fn sample(&self, t: f32, base: Transform) -> Transform {
        let mut out = base;
        if let Some(p) = channel(&self.keys, t, |k| k.position) {
            out.translation = Vec3::from(p);
        }
        if let Some(r) = channel(&self.keys, t, |k| k.rotation) {
            out.rotation = euler_to_quat(Vec3::from(r));
        }
        if let Some(s) = channel(&self.keys, t, |k| k.scale) {
            out.scale = Vec3::from(s);
        }
        out
    }
}

fn channel(keys: &[Key], t: f32, get: impl Fn(&Key) -> Option<[f32; 3]>) -> Option<[f32; 3]> {
    let mut before: Option<(&Key, [f32; 3])> = None;
    let mut after: Option<(&Key, [f32; 3])> = None;
    for key in keys {
        let Some(v) = get(key) else { continue };
        if key.time <= t {
            before = Some((key, v));
        } else {
            after = Some((key, v));
            break;
        }
    }
    match (before, after) {
        (Some((a, va)), Some((b, vb))) => {
            let span = (b.time - a.time).max(1e-6);
            let f = ((t - a.time) / span).clamp(0.0, 1.0);
            let f = match a.ease {
                Ease::Linear => f,
                Ease::Smooth => f * f * (3.0 - 2.0 * f),
                Ease::Step => 0.0,
            };
            Some(std::array::from_fn(|i| va[i] + (vb[i] - va[i]) * f))
        }
        (Some((_, v)), None) | (None, Some((_, v))) => Some(v),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(body: &str) -> String {
        format!("(name: \"t\", duration: 2.0, {body})")
    }

    #[test]
    fn a_file_round_trips_and_sorts_keys_and_cues() {
        let src = text(
            r#"tracks: [(target: "A", keys: [(time: 2.0, position: Some((1.0, 0.0, 0.0))), (time: 0.0)])],
               cues: [(time: 1.5, action: Camera("B")), (time: 0.5, action: Sound(path: "sfx/a.ogg"))]"#,
        );
        let c = Cutscene::from_ron(&src).unwrap();
        assert_eq!(c.tracks[0].keys[0].time, 0.0);
        assert_eq!(c.cues[0].time, 0.5);
        assert!(matches!(c.cues[0].action, CueAction::Sound { volume, .. } if volume == 1.0));
        assert_eq!(Cutscene::from_ron(&c.to_ron().unwrap()).unwrap(), c);
    }

    #[test]
    fn bad_numbers_names_and_paths_are_refused_with_a_reason() {
        for (body, what) in [
            ("duration: -1.0", "duration"),
            (
                "tracks: [(target: \"A\", keys: [(time: -1.0)])]",
                "key times",
            ),
            ("cues: [(time: -2.0, action: Camera(\"A\"))]", "cue times"),
            (
                "cues: [(time: 0.0, action: Sound(path: \"../x.ogg\"))]",
                "inside the assets",
            ),
            (
                "cues: [(time: 0.0, action: Sound(path: \"/etc/passwd\"))]",
                "inside the assets",
            ),
            (
                "cues: [(time: 0.0, action: Sound(path: \"a.ogg\", volume: inf))]",
                "volume",
            ),
        ] {
            let src = if what == "duration" {
                "(name: \"t\", duration: -1.0)".to_string()
            } else {
                text(body)
            };
            let err = Cutscene::from_ron(&src).unwrap_err().to_string();
            assert!(err.contains(what), "{body}: {err}");
        }
        let long = "x".repeat(300);
        let src = text(&format!("cues: [(time: 0.0, action: Camera(\"{long}\"))]"));
        assert!(Cutscene::from_ron(&src).is_err());
        let many = (0..300)
            .map(|i| format!("(target: \"T{i}\")"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(Cutscene::from_ron(&text(&format!("tracks: [{many}]"))).is_err());
        assert!(Cutscene::from_ron("(not ron").is_err());
    }

    #[test]
    fn channels_blend_hold_and_ease() {
        let track = TransformTrack {
            target: "A".into(),
            keys: vec![
                Key {
                    time: 1.0,
                    position: Some([0.0, 0.0, 0.0]),
                    ease: Ease::Smooth,
                    ..Default::default()
                },
                Key {
                    time: 3.0,
                    position: Some([4.0, 0.0, 0.0]),
                    ease: Ease::Step,
                    ..Default::default()
                },
                Key {
                    time: 5.0,
                    position: Some([8.0, 0.0, 0.0]),
                    ..Default::default()
                },
            ],
        };
        let x = |t: f32| track.sample(t, Transform::IDENTITY).translation.x;
        assert_eq!(x(0.0), 0.0, "before the first key it holds the first");
        assert_eq!(x(2.0), 2.0, "smoothstep is 0.5 at the middle");
        assert!(x(1.5) < 1.0, "slow start: {}", x(1.5));
        assert_eq!(x(4.0), 4.0, "a step key holds its value");
        assert_eq!(x(9.0), 8.0, "after the last it holds the last");
    }
}

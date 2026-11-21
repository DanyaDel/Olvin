//! Animation setups as plain data: what a scene's model plays and how
//! (`ModelDesc::animation`), and animation graphs, inline in a scene or in
//! their own `*.animgraph.ron` file. Clips and states are named, not
//! indexed, so a re-exported model keeps working. Turned into the runtime
//! `olvin_anim::AnimGraph` by `olvin_assets` (which knows the model's clips).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// What a model's skeleton plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AnimationDesc {
    /// Loops one clip of the model, by name.
    Clip(String),
    /// A graph in its own file (`*.animgraph.ron`), path relative to `assets/`.
    Graph(String),
    /// A graph written into the scene.
    Inline(AnimGraphDesc),
}

/// A state machine over the model's clips (one layer).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnimGraphDesc {
    pub states: Vec<AnimStateDesc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<TransitionDesc>,
    /// Entry state by name; empty: the first one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub initial: String,
    /// Parameters and their starting values.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamDesc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimStateDesc {
    pub name: String,
    pub motion: MotionDesc,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub speed: f32,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub looping: bool,
}

impl AnimStateDesc {
    /// A looping state playing the clip `clip` at normal speed.
    pub fn clip(name: impl Into<String>, clip: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            motion: MotionDesc::Clip(clip.into()),
            speed: 1.0,
            looping: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MotionDesc {
    /// A clip of the model by name.
    Clip(String),
    /// Clips along one parameter: (threshold, clip name), ascending.
    Blend1D {
        param: String,
        points: Vec<(f32, String)>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransitionDesc {
    /// Source state by name; `None`: from any state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub to: String,
    /// All must hold: (parameter, test).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<(String, TestDesc)>,
    /// Fires only after this fraction of the source's length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_time: Option<f32>,
    /// Cross-fade, seconds.
    #[serde(default)]
    pub duration: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum TestDesc {
    Greater(f32),
    Less(f32),
    True,
    False,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParamDesc {
    Float(f32),
    Bool(bool),
    /// Starts unset.
    Trigger,
}

fn one() -> f32 {
    1.0
}

fn is_one(v: &f32) -> bool {
    *v == 1.0
}

fn yes() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

impl AnimGraphDesc {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        let config = ron::ser::PrettyConfig::new().struct_names(false);
        let text = ron::ser::to_string_pretty(self, config).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

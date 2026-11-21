//! Scene animation setups (`olvin_scene::anim`) turned into runtime graphs
//! over a model's clips: names are resolved to indices here.

use olvin_anim::{AnimGraph, Animator, Clip, Condition, Motion, Params, State, Test, Transition};
use olvin_scene::anim::{AnimGraphDesc, AnimationDesc, MotionDesc, ParamDesc, TestDesc};

use crate::ModelData;

fn clip_index(clips: &[Clip], name: &str) -> Result<usize, String> {
    clips
        .iter()
        .position(|c| c.name == name)
        .ok_or_else(|| format!("no clip `{name}`"))
}

/// The runtime graph of `desc` over `clips` (by name) and its starting
/// parameters. Unknown clips or states are errors.
pub fn build_graph(desc: &AnimGraphDesc, clips: &[Clip]) -> Result<(AnimGraph, Params), String> {
    if desc.states.is_empty() {
        return Err("animation graph without states".into());
    }
    let state = |name: &str| {
        desc.states
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| format!("no state `{name}`"))
    };
    let states = desc
        .states
        .iter()
        .map(|s| {
            let motion = match &s.motion {
                MotionDesc::Clip(c) => Motion::Clip(clip_index(clips, c)?),
                MotionDesc::Blend1D { param, points } => Motion::Blend1D {
                    param: param.clone(),
                    points: points
                        .iter()
                        .map(|(t, c)| Ok((*t, clip_index(clips, c)?)))
                        .collect::<Result<_, String>>()?,
                },
            };
            Ok(State {
                name: s.name.clone(),
                motion,
                speed: s.speed,
                looping: s.looping,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let transitions = desc
        .transitions
        .iter()
        .map(|t| {
            Ok(Transition {
                from: t.from.as_deref().map(state).transpose()?,
                to: state(&t.to)?,
                conditions: t
                    .when
                    .iter()
                    .map(|(param, test)| {
                        let test = match *test {
                            TestDesc::Greater(x) => Test::Greater(x),
                            TestDesc::Less(x) => Test::Less(x),
                            TestDesc::True => Test::True,
                            TestDesc::False => Test::False,
                        };
                        Condition::new(param.clone(), test)
                    })
                    .collect(),
                exit_time: t.exit_time,
                duration: t.duration,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let default = if desc.initial.is_empty() {
        0
    } else {
        state(&desc.initial)?
    };
    let mut params = Params::default();
    for (name, p) in &desc.params {
        match *p {
            ParamDesc::Float(v) => params.set_float(name, v),
            ParamDesc::Bool(v) => params.set_bool(name, v),
            // Declared, not fired: reads as 0 until set.
            ParamDesc::Trigger => params.set_bool(name, false),
        }
    }
    Ok((
        AnimGraph {
            states,
            transitions,
            default,
        },
        params,
    ))
}

impl ModelData {
    /// An [`Animator`] over skeleton `skin` running `setup` (`None`: rest
    /// pose, no layers). `file` is the loaded graph for
    /// [`AnimationDesc::Graph`]; without it that setup is an error.
    pub fn animator_with(
        &self,
        skin: usize,
        setup: Option<&AnimationDesc>,
        file: Option<&AnimGraphDesc>,
    ) -> Result<Animator, String> {
        let animator = self
            .animator(skin)
            .ok_or_else(|| format!("{}: no skin {skin}", self.name))?;
        let (graph, params) = match setup {
            None => return Ok(animator),
            Some(AnimationDesc::Clip(name)) => (
                AnimGraph::single(clip_index(&self.clips, name)?),
                Params::default(),
            ),
            Some(AnimationDesc::Inline(desc)) => build_graph(desc, &self.clips)?,
            Some(AnimationDesc::Graph(path)) => {
                let desc = file.ok_or_else(|| format!("{path}: graph not loaded"))?;
                build_graph(desc, &self.clips).map_err(|e| format!("{path}: {e}"))?
            }
        };
        let mut animator = animator;
        animator.params = params;
        Ok(animator.with_graph(graph))
    }
}

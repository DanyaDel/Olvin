//! Olvin animation, pure data and logic: no GPU, no window.
//!
//! - [`Skeleton`], [`Pose`]: joints, rest pose, skinning matrices.
//! - [`Clip`], [`Channel`]: keyframes (step / linear / cubic spline), sampling, events.
//! - [`AnimGraph`], [`LayerState`], [`Params`]: states, cross-fading transitions,
//!   1D blend motions, parameters and triggers.
//! - [`Animator`], [`AnimPlugin`]: the ECS component and its per-frame system.

mod animator;
mod clip;
mod graph;
mod skeleton;

#[cfg(test)]
mod tests;

pub use animator::{AnimPlugin, AnimationEvent, Animator, Layer};
pub use clip::{Channel, Clip, ClipEvent, Interpolation, Property};
pub use graph::{
    AnimGraph, Condition, LayerState, Motion, ParamValue, Params, State, Test, Transition,
};
pub use skeleton::{Joint, JointPose, Pose, Skeleton};

//! Hand-made 3D audio: distance attenuation and stereo pan.

use glam::{Quat, Vec3};

/// Gain at `distance`: 1 up to `min`, then inverse-distance (`min / d`, like
/// Unity's logarithmic rolloff) faded smoothly to 0 at `max`.
pub fn attenuation(distance: f32, min: f32, max: f32) -> f32 {
    let min = min.max(1e-3);
    let max = max.max(min + 1e-3);
    if distance.is_nan() {
        return 0.0;
    }
    if distance <= min {
        return 1.0;
    }
    if distance >= max {
        return 0.0;
    }
    let inverse = min / distance;
    let t = (distance - min) / (max - min);
    inverse * (1.0 - t * t)
}

/// Stereo pan (-1 left .. 1 right) of a sound at `source` for a listener at
/// `position` looking down its -Z. Never fully one-sided, so sounds behind
/// or beside the head stay audible in both ears.
pub fn pan(position: Vec3, rotation: Quat, source: Vec3) -> f32 {
    let local = rotation.inverse() * (source - position);
    let len = local.length();
    if !(len >= 1e-4) || !len.is_finite() {
        return 0.0;
    }
    (local.x / len) * 0.8
}

/// Quietest a fully occluded sound gets (linear gain), on top of the filter.
pub const OCCLUDED_GAIN: f32 = 0.35;
/// Low-pass cutoff of a fully occluded sound, in Hz.
pub const OCCLUDED_CUTOFF: f64 = 500.0;
/// Cutoff when nothing is in the way: above hearing, effectively off.
pub const OPEN_CUTOFF: f64 = 20_000.0;

/// Gain of a sound whose path to the listener is `occlusion` blocked
/// (0 clear .. 1 fully behind walls).
pub fn occlusion_gain(occlusion: f32) -> f32 {
    1.0 + (OCCLUDED_GAIN - 1.0) * occlusion.clamp(0.0, 1.0)
}

/// Low-pass cutoff for the same: walls swallow highs first. Exponential in
/// the amount, so the change sounds even.
pub fn occlusion_cutoff(occlusion: f32) -> f64 {
    let t = occlusion.clamp(0.0, 1.0) as f64;
    OPEN_CUTOFF * (OCCLUDED_CUTOFF / OPEN_CUTOFF).powf(t)
}

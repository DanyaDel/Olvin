//! Hand-made 3D audio: distance attenuation and stereo pan.

use glam::{Quat, Vec3};

/// Gain at `distance`: 1 up to `min`, then inverse-distance (`min / d`, like
/// Unity's logarithmic rolloff) faded smoothly to 0 at `max`.
pub fn attenuation(distance: f32, min: f32, max: f32) -> f32 {
    let min = min.max(1e-3);
    let max = max.max(min + 1e-3);
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
    if len < 1e-4 {
        return 0.0;
    }
    (local.x / len) * 0.8
}

//! Byte formats for replicated components. The default is `postcard` of the
//! serde form; a [`Codec`] replaces it for a component that deserves a compact one.

use glam::{Quat, Vec3};
use olvin_scene::Transform;

/// A hand-written byte format for `T`. Decoding must reject malformed input
/// with `Err`, never panic: bytes from the network are untrusted.
pub struct Codec<T> {
    pub encode: EncodeFn<T>,
    pub decode: DecodeFn<T>,
}

pub type EncodeFn<T> = Box<dyn Fn(&T, &mut Vec<u8>)>;
pub type DecodeFn<T> = Box<dyn Fn(&[u8]) -> Result<T, String>>;

impl<T> Codec<T> {
    pub fn new(
        encode: impl Fn(&T, &mut Vec<u8>) + 'static,
        decode: impl Fn(&[u8]) -> Result<T, String> + 'static,
    ) -> Self {
        Self {
            encode: Box::new(encode),
            decode: Box::new(decode),
        }
    }
}

pub(crate) fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

pub(crate) fn get_varint(input: &mut &[u8]) -> Result<u64, String> {
    let mut v = 0u64;
    for shift in (0..70).step_by(7) {
        let (&b, rest) = input.split_first().ok_or("truncated")?;
        *input = rest;
        v |= ((b & 0x7F) as u64) << shift.min(63);
        if b & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err("varint too long".into())
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

const POS_SCALE: f32 = 256.0;
const UNIT_SCALE: u8 = 0b100;

fn quantize_pos(v: f32) -> i64 {
    // Finite and clamped: NaN or infinity must not turn into nonsense.
    let v = if v.is_finite() { v } else { 0.0 };
    (v * POS_SCALE).round().clamp(-2.0e9, 2.0e9) as i64
}

/// Compact `Transform`: position to 1/256 m as varints, rotation as the three
/// smallest quaternion components (16 bits each), scale omitted when it is 1.
/// About 15 bytes instead of 40.
pub fn transform_codec() -> Codec<Transform> {
    Codec::new(encode_transform, decode_transform)
}

fn encode_transform(t: &Transform, out: &mut Vec<u8>) {
    let unit = t.scale.abs_diff_eq(Vec3::ONE, 1e-4);
    // The component with the largest magnitude is rebuilt from the others.
    let q = t.rotation.normalize();
    let q = if q.is_finite() { q } else { Quat::IDENTITY };
    let a = q.to_array();
    let largest = (0..4)
        .max_by(|&i, &j| a[i].abs().total_cmp(&a[j].abs()))
        .unwrap_or(3);
    let sign = if a[largest] < 0.0 { -1.0 } else { 1.0 };
    out.push((largest as u8) | if unit { UNIT_SCALE } else { 0 });
    for v in [t.translation.x, t.translation.y, t.translation.z] {
        put_varint(out, zigzag(quantize_pos(v)));
    }
    const R: f32 = std::f32::consts::FRAC_1_SQRT_2;
    for (i, c) in a.iter().enumerate() {
        if i == largest {
            continue;
        }
        let n = ((c * sign / R).clamp(-1.0, 1.0) * 0.5 + 0.5) * 65535.0;
        out.extend_from_slice(&(n.round() as u16).to_le_bytes());
    }
    if !unit {
        for v in t.scale.to_array() {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

fn decode_transform(bytes: &[u8]) -> Result<Transform, String> {
    let mut input = bytes;
    let (&flags, rest) = input.split_first().ok_or("truncated transform")?;
    input = rest;
    let largest = (flags & 0b11) as usize;
    let mut p = [0f32; 3];
    for v in &mut p {
        *v = unzigzag(get_varint(&mut input)?) as f32 / POS_SCALE;
    }
    const R: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let mut a = [0f32; 4];
    let mut sum = 0.0;
    for (i, slot) in a.iter_mut().enumerate() {
        if i == largest {
            continue;
        }
        let (raw, rest) = input.split_first_chunk::<2>().ok_or("truncated rotation")?;
        input = rest;
        let n = u16::from_le_bytes(*raw) as f32 / 65535.0;
        *slot = (n * 2.0 - 1.0) * R;
        sum += *slot * *slot;
    }
    a[largest] = (1.0 - sum).max(0.0).sqrt();
    let scale = if flags & UNIT_SCALE != 0 {
        Vec3::ONE
    } else {
        let mut s = [0f32; 3];
        for v in &mut s {
            let (raw, rest) = input.split_first_chunk::<4>().ok_or("truncated scale")?;
            input = rest;
            let f = f32::from_le_bytes(*raw);
            *v = if f.is_finite() { f } else { 1.0 };
        }
        Vec3::from_array(s)
    };
    if !input.is_empty() {
        return Err("trailing bytes in transform".into());
    }
    Ok(Transform {
        translation: Vec3::from_array(p),
        rotation: Quat::from_array(a).normalize(),
        scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_roundtrips_within_tolerance_and_is_small() {
        let t = Transform {
            translation: Vec3::new(12.34, -5.0, 1000.5),
            rotation: Quat::from_euler(glam::EulerRot::YXZ, 1.0, -0.4, 0.2),
            scale: Vec3::ONE,
        };
        let mut b = Vec::new();
        encode_transform(&t, &mut b);
        assert!(b.len() <= 16, "{} bytes", b.len());
        let back = decode_transform(&b).unwrap();
        assert!(back.translation.distance(t.translation) < 0.01);
        assert!(back.rotation.angle_between(t.rotation) < 0.001);
        assert_eq!(back.scale, Vec3::ONE);

        let t2 = Transform {
            scale: Vec3::new(2.0, 0.5, 3.0),
            rotation: Quat::from_xyzw(0.0, 0.0, 0.0, -1.0),
            ..t
        };
        let mut b = Vec::new();
        encode_transform(&t2, &mut b);
        let back = decode_transform(&b).unwrap();
        assert_eq!(back.scale, t2.scale);
        assert!(back.rotation.angle_between(Quat::IDENTITY) < 0.001);
    }

    #[test]
    fn decoder_survives_garbage() {
        let mut x = 7u64;
        for len in 0..40 {
            for _ in 0..200 {
                let bytes: Vec<u8> = (0..len)
                    .map(|_| {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                        (x >> 56) as u8
                    })
                    .collect();
                if let Ok(t) = decode_transform(&bytes) {
                    assert!(t.rotation.is_finite() && t.translation.is_finite());
                }
            }
        }
        assert!(decode_transform(&[]).is_err());
        assert!(decode_transform(&[0xFF; 30]).is_err());
    }

    #[test]
    fn nan_position_is_sanitised() {
        let t = Transform {
            translation: Vec3::new(f32::NAN, f32::INFINITY, 1.0),
            ..Default::default()
        };
        let mut b = Vec::new();
        encode_transform(&t, &mut b);
        let back = decode_transform(&b).unwrap();
        assert!(back.translation.is_finite());
    }
}

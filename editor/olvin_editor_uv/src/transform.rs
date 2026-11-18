//! G / R / S in UV space: typed values, U/V axis constraints, Ctrl snapping
//! (moves to the 1/64 pixel grid, rotations to 5°, scales to 0.1), and the
//! 2D move gizmo's hit test. Pure math.

use olvin_editor::glam::{Affine2, Vec2};

/// The pixel grid Ctrl snaps moves to.
pub const PIXEL: f32 = 1.0 / 64.0;
pub const ANGLE_SNAP_DEG: f32 = 5.0;
pub const SCALE_SNAP: f32 = 0.1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Move,
    Rotate,
    Scale,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Move => "Move",
            Kind::Rotate => "Rotate",
            Kind::Scale => "Scale",
        }
    }
}

/// Blender's numeric input: digits, one decimal point (`,` too), `-` toggles the sign.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NumInput {
    digits: String,
    negative: bool,
}

impl NumInput {
    pub fn is_active(&self) -> bool {
        !self.digits.is_empty() || self.negative
    }

    pub fn type_char(&mut self, c: char) -> bool {
        match c {
            '0'..='9' => self.digits.push(c),
            '.' | ',' => {
                if !self.digits.contains('.') {
                    self.digits.push('.');
                }
            }
            '-' => self.negative = !self.negative,
            _ => return false,
        }
        true
    }

    pub fn backspace(&mut self) {
        if self.digits.pop().is_none() {
            self.negative = false;
        }
    }

    pub fn value(&self) -> Option<f32> {
        let v: f32 = match self.digits.as_str() {
            "" if self.negative => 0.0,
            "" => return None,
            "." => 0.0,
            d => d.parse().ok()?,
        };
        Some(if self.negative { -v } else { v })
    }

    pub fn text(&self) -> String {
        format!("{}{}", if self.negative { "-" } else { "" }, self.digits)
    }
}

pub fn snap_to(v: f32, step: f32) -> f32 {
    if step > 0.0 {
        (v / step).round() * step
    } else {
        v
    }
}

/// One frame of a modal transform. `start` and `mouse` are UV positions of
/// the cursor when the operation began and now; `axis` 0 = U, 1 = V.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub kind: Kind,
    pub pivot: Vec2,
    pub start: Vec2,
    pub mouse: Vec2,
    pub axis: Option<usize>,
    pub typed: Option<f32>,
    pub snap: bool,
}

/// The transform to apply to the original UVs, and a status line.
pub fn matrix(p: &Params) -> (Affine2, String) {
    let around =
        |m: Affine2| Affine2::from_translation(p.pivot) * m * Affine2::from_translation(-p.pivot);
    let axis_name = match p.axis {
        Some(0) => "  along U",
        Some(_) => "  along V",
        None => "",
    };
    match p.kind {
        Kind::Move => {
            let mut d = match p.typed {
                Some(t) if p.axis == Some(1) => Vec2::new(0.0, t),
                Some(t) => Vec2::new(t, 0.0),
                None => p.mouse - p.start,
            };
            match p.axis {
                Some(0) => d.y = 0.0,
                Some(_) => d.x = 0.0,
                None => {}
            }
            if p.snap {
                d = Vec2::new(snap_to(d.x, PIXEL), snap_to(d.y, PIXEL));
            }
            let px = d * 64.0;
            (
                Affine2::from_translation(d),
                format!(
                    "Move  U {:.4}  V {:.4}  ({:.1}, {:.1} px @64){axis_name}",
                    d.x, d.y, px.x, px.y
                ),
            )
        }
        Kind::Rotate => {
            let mut deg = match p.typed {
                Some(t) => t,
                None => {
                    let (a, b) = (p.start - p.pivot, p.mouse - p.pivot);
                    if a.length() < 1e-9 || b.length() < 1e-9 {
                        0.0
                    } else {
                        a.angle_to(b).to_degrees()
                    }
                }
            };
            if p.snap {
                deg = snap_to(deg, ANGLE_SNAP_DEG);
            }
            (
                around(Affine2::from_angle(deg.to_radians())),
                format!("Rotate  {deg:.1}°"),
            )
        }
        Kind::Scale => {
            let mut s = match p.typed {
                Some(t) => t,
                None => {
                    let a = (p.start - p.pivot).length();
                    if a < 1e-9 {
                        1.0
                    } else {
                        (p.mouse - p.pivot).length() / a
                    }
                }
            };
            if p.snap {
                s = snap_to(s, SCALE_SNAP);
            }
            let v = match p.axis {
                Some(0) => Vec2::new(s, 1.0),
                Some(_) => Vec2::new(1.0, s),
                None => Vec2::splat(s),
            };
            (
                around(Affine2::from_scale(v)),
                format!("Scale  {s:.3}{axis_name}"),
            )
        }
    }
}

/// Screen length of the gizmo arrows.
pub const GIZMO_ARM: f32 = 64.0;
pub const GIZMO_BOX: f32 = 7.0;

/// Which gizmo handle is at `mouse`: `Some(None)` the centre square (free
/// move), `Some(Some(0))` the U arrow, `Some(Some(1))` the V arrow (up on screen).
pub fn gizmo_hit(center: Vec2, mouse: Vec2) -> Option<Option<usize>> {
    let d = mouse - center;
    if d.x.abs() <= GIZMO_BOX + 2.0 && d.y.abs() <= GIZMO_BOX + 2.0 {
        return Some(None);
    }
    let u_end = center + Vec2::new(GIZMO_ARM, 0.0);
    let v_end = center + Vec2::new(0.0, -GIZMO_ARM);
    let du = crate::select::segment_distance(mouse, center, u_end);
    let dv = crate::select::segment_distance(mouse, center, v_end);
    match (du <= 6.0, dv <= 6.0) {
        (true, false) => Some(Some(0)),
        (false, true) => Some(Some(1)),
        (true, true) => Some(Some(if du <= dv { 0 } else { 1 })),
        _ => None,
    }
}

/// Bounding-box centre of some UVs (the pivot), `None` when empty.
pub fn bounds_center(points: impl IntoIterator<Item = Vec2>) -> Option<Vec2> {
    let (lo, hi) = points.into_iter().fold(
        (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(p), hi.max(p)),
    );
    lo.is_finite().then(|| (lo + hi) * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(kind: Kind) -> Params {
        Params {
            kind,
            pivot: Vec2::splat(0.5),
            start: Vec2::new(1.0, 0.5),
            mouse: Vec2::new(1.0, 0.5),
            axis: None,
            typed: None,
            snap: false,
        }
    }

    #[test]
    fn move_axis_typed_and_snap() {
        let mut p = params(Kind::Move);
        p.mouse = Vec2::new(1.1, 0.73);
        let (m, _) = matrix(&p);
        assert!(
            m.transform_point2(Vec2::ZERO)
                .abs_diff_eq(Vec2::new(0.1, 0.23), 1e-6)
        );
        p.axis = Some(1);
        assert!(
            matrix(&p)
                .0
                .transform_point2(Vec2::ZERO)
                .abs_diff_eq(Vec2::new(0.0, 0.23), 1e-6)
        );
        p.snap = true;
        let y = matrix(&p).0.transform_point2(Vec2::ZERO).y;
        assert!((y * 64.0 - (0.23f32 * 64.0).round()).abs() < 1e-4, "{y}");
        p.typed = Some(-0.25);
        p.snap = false;
        assert_eq!(
            matrix(&p).0.transform_point2(Vec2::ZERO),
            Vec2::new(0.0, -0.25)
        );
        p.axis = None;
        assert_eq!(
            matrix(&p).0.transform_point2(Vec2::ZERO),
            Vec2::new(-0.25, 0.0)
        );
    }

    #[test]
    fn rotate_follows_mouse_and_typed_degrees() {
        let mut p = params(Kind::Rotate);
        // Mouse from right of the pivot to above it: +90° (counter-clockwise).
        p.mouse = Vec2::new(0.5, 1.0);
        let (m, text) = matrix(&p);
        assert!(
            m.transform_point2(Vec2::new(1.0, 0.5))
                .abs_diff_eq(Vec2::new(0.5, 1.0), 1e-5)
        );
        assert!(text.contains("90.0"));
        p.typed = Some(180.0);
        assert!(
            matrix(&p)
                .0
                .transform_point2(Vec2::new(1.0, 0.5))
                .abs_diff_eq(Vec2::new(0.0, 0.5), 1e-5)
        );
        p.typed = None;
        p.mouse = Vec2::new(0.5 + 0.5 * 0.9994, 0.5 + 0.5 * 0.0349); // ~2°
        p.snap = true;
        assert!(
            matrix(&p)
                .0
                .transform_point2(Vec2::new(1.0, 0.5))
                .abs_diff_eq(Vec2::new(1.0, 0.5), 1e-5)
        );
    }

    #[test]
    fn scale_axis_and_snap() {
        let mut p = params(Kind::Scale);
        p.mouse = Vec2::new(1.5, 0.5);
        let m = matrix(&p).0;
        assert!(
            m.transform_point2(Vec2::ONE)
                .abs_diff_eq(Vec2::splat(1.5), 1e-5)
        );
        p.axis = Some(0);
        assert!(
            matrix(&p)
                .0
                .transform_point2(Vec2::ONE)
                .abs_diff_eq(Vec2::new(1.5, 1.0), 1e-5)
        );
        p.axis = None;
        p.mouse = Vec2::new(1.27, 0.5);
        p.snap = true;
        let (m, _) = matrix(&p);
        // 1.54 -> 1.5
        assert!(
            m.transform_point2(Vec2::new(1.0, 0.5))
                .abs_diff_eq(Vec2::new(1.25, 0.5), 1e-5)
        );
        p.typed = Some(-1.0);
        assert!(
            matrix(&p)
                .0
                .transform_point2(Vec2::ZERO)
                .abs_diff_eq(Vec2::ONE, 1e-5)
        );
    }

    #[test]
    fn num_input() {
        let mut n = NumInput::default();
        assert_eq!(n.value(), None);
        for c in "1,5x".chars() {
            n.type_char(c);
        }
        assert_eq!(n.value(), Some(1.5));
        n.type_char('-');
        assert_eq!((n.value(), n.text()), (Some(-1.5), "-1.5".to_owned()));
        n.backspace();
        n.backspace();
        n.backspace();
        assert_eq!(n.value(), Some(-0.0));
        n.backspace();
        assert!(!n.is_active());
    }

    #[test]
    fn gizmo_handles() {
        let c = Vec2::new(100.0, 100.0);
        assert_eq!(gizmo_hit(c, c + Vec2::new(3.0, -2.0)), Some(None));
        assert_eq!(gizmo_hit(c, c + Vec2::new(40.0, 3.0)), Some(Some(0)));
        assert_eq!(gizmo_hit(c, c + Vec2::new(-2.0, -50.0)), Some(Some(1)));
        assert_eq!(gizmo_hit(c, c + Vec2::new(40.0, 40.0)), None);
        assert_eq!(
            bounds_center([Vec2::ZERO, Vec2::new(2.0, 4.0)]),
            Some(Vec2::new(1.0, 2.0))
        );
        assert_eq!(bounds_center([]), None);
    }
}

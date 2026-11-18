//! The modal transform engine shared by G / R / S, extrude, duplicate and
//! the Edit Mode gizmo: constraints, numeric input, snapping, precision.

use olvin_editor::egui::Pos2;
use olvin_editor::ext::{Snap, ViewCamera};
use olvin_editor::glam::{Mat3, Mat4, Quat, Vec3};
use olvin_editor::olvin_scene::file::PolyMesh;

const GLOBAL: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
const ANGLE_STEP: f32 = 5.0;
const SCALE_STEP: f32 = 0.1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Move,
    Rotate,
    Scale,
}

/// Axis or plane a transform is limited to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Constraint {
    pub axis: usize,
    /// Everything except `axis` (Shift+X/Y/Z).
    pub plane: bool,
    /// Local / normal orientation instead of global.
    pub local: bool,
}

/// One press of X/Y/Z (Shift: plane): global → local → off.
pub fn cycle(current: Option<Constraint>, axis: usize, plane: bool) -> Option<Constraint> {
    let global = Constraint {
        axis,
        plane,
        local: false,
    };
    let local = Constraint {
        local: true,
        ..global
    };
    match current {
        Some(c) if c == global => Some(local),
        Some(c) if c == local => None,
        _ => Some(global),
    }
}

/// Which basis axes take part: all, one, or all but one.
pub fn active_axes(c: Option<Constraint>) -> [bool; 3] {
    match c {
        None => [true; 3],
        Some(c) => std::array::from_fn(|i| (i == c.axis) != c.plane),
    }
}

/// Number typed during a modal transform ("G Z 2 Enter").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NumInput {
    digits: String,
    negative: bool,
}

impl NumInput {
    pub fn is_active(&self) -> bool {
        !self.digits.is_empty() || self.negative
    }

    /// Digits, one decimal point (`,` too) and `-` (toggles the sign).
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

/// `v` rounded to multiples of `step`.
pub fn snap_to(v: f32, step: f32) -> f32 {
    if step > 0.0 {
        (v / step).round() * step
    } else {
        v
    }
}

/// Keeps the active basis components of `delta`, each snapped to `step`.
pub fn constrain_move(delta: Vec3, axes: [Vec3; 3], on: [bool; 3], step: Option<f32>) -> Vec3 {
    (0..3)
        .filter(|&i| on[i])
        .map(|i| axes[i] * step.map_or(delta.dot(axes[i]), |s| snap_to(delta.dot(axes[i]), s)))
        .sum()
}

/// Rotation by `angle` around `axis` through `pivot`.
pub fn rotation(pivot: Vec3, axis: Vec3, angle: f32) -> Mat4 {
    Mat4::from_translation(pivot)
        * Mat4::from_quat(Quat::from_axis_angle(axis.normalize_or(Vec3::Z), angle))
        * Mat4::from_translation(-pivot)
}

/// Scale by `factor` around `pivot` along the active (orthonormal) axes.
pub fn scaling(pivot: Vec3, axes: [Vec3; 3], on: [bool; 3], factor: f32) -> Mat4 {
    let mut m = Mat3::IDENTITY;
    for i in (0..3).filter(|&i| on[i]) {
        let a = axes[i];
        m += Mat3::from_cols(a * a.x, a * a.y, a * a.z) * (factor - 1.0);
    }
    Mat4::from_translation(pivot) * Mat4::from_mat3(m) * Mat4::from_translation(-pivot)
}

/// Signed angle wrapped into (-π, π].
pub fn wrap_angle(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let a = a.rem_euclid(TAU);
    if a > PI { a - TAU } else { a }
}

/// `before` with the world-space transform `m` applied to `verts`.
pub fn apply(before: &PolyMesh, verts: &[u32], world: Mat4, m: Mat4) -> PolyMesh {
    let local = world.inverse() * m * world;
    let mut out = before.clone();
    for &v in verts {
        if let Some(p) = out.positions.get_mut(v as usize) {
            *p = local.transform_point3(before.positions[v as usize]);
        }
    }
    out
}

/// Orthonormal basis with `n` as Z and a horizontal X where possible.
pub fn normal_basis(n: Vec3) -> [Vec3; 3] {
    let z = n.normalize_or(Vec3::Y);
    let x = Vec3::Y.cross(z).try_normalize().unwrap_or(Vec3::X);
    [x, z.cross(x), z]
}

/// What the user does this frame while a transform runs.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub mouse: Option<Pos2>,
    /// Shift held: mouse movement counts ×0.1.
    pub precise: bool,
    /// X/Y/Z presses as (axis, shift).
    pub axis_keys: Vec<(usize, bool)>,
    pub chars: Vec<char>,
    pub backspace: bool,
    /// G / R / S pressed again: switch the transform kind.
    pub switch: Option<Kind>,
}

/// A running modal transform of some vertices.
pub struct Transform {
    pub kind: Kind,
    /// "Move", "Extrude", "Duplicate"...
    pub title: &'static str,
    pub before: PolyMesh,
    pub verts: Vec<u32>,
    pub world: Mat4,
    /// Median of the vertices, world space.
    pub pivot: Vec3,
    /// Second orientation for double presses (face normal or object axes).
    pub local: [Vec3; 3],
    pub local_name: &'static str,
    pub constraint: Option<Constraint>,
    pub num: NumInput,
    /// Started by a gizmo drag: ends when the button is released.
    pub drag: bool,
    start: Pos2,
    /// Virtual mouse: real movement, slowed down while Shift is held.
    mouse: Pos2,
    last: Pos2,
    angle: f32,
    /// Status line, e.g. "Move Z  2".
    pub summary: String,
}

impl Transform {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: Kind,
        title: &'static str,
        before: PolyMesh,
        verts: Vec<u32>,
        world: Mat4,
        local: ([Vec3; 3], &'static str),
        mouse: Pos2,
    ) -> Self {
        let pivot = if verts.is_empty() {
            world.transform_point3(Vec3::ZERO)
        } else {
            verts
                .iter()
                .map(|&v| world.transform_point3(before.positions[v as usize]))
                .sum::<Vec3>()
                / verts.len() as f32
        };
        Self {
            kind,
            title,
            before,
            verts,
            world,
            pivot,
            local: local.0,
            local_name: local.1,
            constraint: None,
            num: NumInput::default(),
            drag: false,
            start: mouse,
            mouse,
            last: mouse,
            angle: 0.0,
            summary: String::new(),
        }
    }

    pub fn axes(&self) -> [Vec3; 3] {
        match self.constraint {
            Some(c) if c.local => self.local,
            _ => GLOBAL,
        }
    }

    /// Feeds one frame of input; returns the world-space transform to apply.
    pub fn step(&mut self, camera: &ViewCamera, snap: Snap, input: &Input) -> Mat4 {
        if let Some(kind) = input.switch.filter(|k| *k != self.kind) {
            self.kind = kind;
            self.constraint = None;
            self.num = NumInput::default();
            self.start = self.mouse;
            self.angle = 0.0;
        }
        for &(axis, plane) in &input.axis_keys {
            self.constraint = cycle(self.constraint, axis, plane);
        }
        for &c in &input.chars {
            self.num.type_char(c);
        }
        if input.backspace {
            self.num.backspace();
        }
        if let Some(real) = input.mouse {
            let k = if input.precise { 0.1 } else { 1.0 };
            let previous = self.mouse;
            self.mouse += (real - self.last) * k;
            self.last = real;
            if let Some(c) = camera.project(self.pivot) {
                self.angle += wrap_angle(angle_at(c, self.mouse) - angle_at(c, previous));
            }
        }
        let (m, value) = match self.kind {
            Kind::Move => self.move_matrix(camera, snap, input.precise),
            Kind::Rotate => self.rotate_matrix(camera, snap, input.precise),
            Kind::Scale => self.scale_matrix(camera, snap, input.precise),
        };
        self.summary = self.describe(value);
        m
    }

    fn move_matrix(&self, camera: &ViewCamera, snap: Snap, precise: bool) -> (Mat4, String) {
        let (axes, on) = (self.axes(), active_axes(self.constraint));
        let d = match self.num.value() {
            Some(v) => axes[(0..3).find(|&i| on[i]).unwrap_or(0)] * v,
            None => self.mouse_move(camera, snap, precise, axes, on),
        };
        let text = format!("D {:.3}  {:.3}  {:.3}", d.x, d.y, d.z);
        (Mat4::from_translation(d), text)
    }

    fn mouse_move(
        &self,
        camera: &ViewCamera,
        snap: Snap,
        precise: bool,
        axes: [Vec3; 3],
        on: [bool; 3],
    ) -> Vec3 {
        let origin = self.pivot;
        let point = |p: Pos2| -> Option<Vec3> {
            let ray = camera.ray(p);
            match self.constraint {
                Some(c) if !c.plane => {
                    Some(origin + axes[c.axis] * ray.closest_on_line(origin, axes[c.axis])?)
                }
                Some(c) => Some(ray.at(ray.plane(origin, axes[c.axis])?)),
                None => Some(ray.at(ray.plane(origin, camera.forward)?)),
            }
        };
        let delta = match (point(self.start), point(self.mouse)) {
            (Some(a), Some(b)) => b - a,
            _ => Vec3::ZERO,
        };
        let step = snap
            .enabled
            .then_some(snap.step * if precise { 0.1 } else { 1.0 });
        constrain_move(delta, axes, on, step)
    }

    /// Rotation axis and the sign that makes the mesh follow the mouse.
    fn rotation_axis(&self, camera: &ViewCamera) -> (Vec3, f32) {
        match self.constraint {
            Some(c) => {
                let a = self.axes()[c.axis];
                (
                    a,
                    if a.dot(camera.forward) < 0.0 {
                        -1.0
                    } else {
                        1.0
                    },
                )
            }
            None => (-camera.forward, -1.0),
        }
    }

    fn rotate_matrix(&self, camera: &ViewCamera, snap: Snap, precise: bool) -> (Mat4, String) {
        let (axis, sign) = self.rotation_axis(camera);
        let angle = match self.num.value() {
            Some(deg) => deg.to_radians(),
            None if snap.enabled => {
                let step = if precise { 1.0 } else { ANGLE_STEP };
                snap_to((self.angle * sign).to_degrees(), step).to_radians()
            }
            None => self.angle * sign,
        };
        let text = format!("{:.1}°", angle.to_degrees());
        (rotation(self.pivot, axis, angle), text)
    }

    fn scale_matrix(&self, camera: &ViewCamera, snap: Snap, precise: bool) -> (Mat4, String) {
        let factor = self.num.value().unwrap_or_else(|| {
            let Some(c) = camera.project(self.pivot) else {
                return 1.0;
            };
            let f = self.mouse.distance(c) / self.start.distance(c).max(4.0);
            if snap.enabled {
                snap_to(
                    f,
                    if precise {
                        SCALE_STEP * 0.1
                    } else {
                        SCALE_STEP
                    },
                )
            } else {
                f
            }
        });
        let m = scaling(
            self.pivot,
            self.axes(),
            active_axes(self.constraint),
            factor,
        );
        (m, format!("×{factor:.3}"))
    }

    fn describe(&self, value: String) -> String {
        let axis = match self.constraint {
            None => String::new(),
            Some(c) => {
                let name = ["X", "Y", "Z"][c.axis];
                let orient = if c.local { self.local_name } else { "Global" };
                let not = if c.plane { "not " } else { "" };
                format!(" {not}{name} ({orient})")
            }
        };
        let typed = if self.num.is_active() {
            format!("  [{}|]", self.num.text())
        } else {
            String::new()
        };
        let name = match self.kind {
            Kind::Move => self.title,
            Kind::Rotate => "Rotate",
            Kind::Scale => "Scale",
        };
        format!("{name}{axis}{typed}  {value}")
    }
}

fn angle_at(center: Pos2, p: Pos2) -> f32 {
    (p.y - center.y).atan2(p.x - center.x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_editor::egui::{self, pos2};
    use olvin_editor::glam::camera::rh;

    fn camera() -> ViewCamera {
        // Looking down -Z from z = 10, 800×600 viewport.
        let eye = Vec3::new(0.0, 0.0, 10.0);
        let view = rh::view::look_at_mat4(eye, Vec3::ZERO, Vec3::Y);
        let proj = rh::proj::directx::perspective(1.0, 800.0 / 600.0, 0.1, 100.0);
        ViewCamera {
            view,
            proj,
            view_proj: proj * view,
            eye,
            forward: -Vec3::Z,
            rect: egui::Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 600.0)),
            near: 0.1,
            orthographic: false,
        }
    }

    const NO_SNAP: Snap = Snap {
        enabled: false,
        step: 1.0,
        angle_deg: 15.0,
    };

    fn grab(kind: Kind) -> Transform {
        let mesh = PolyMesh::cube(2.0);
        let verts = (0..8).collect();
        Transform::new(
            kind,
            "Move",
            mesh,
            verts,
            Mat4::IDENTITY,
            (GLOBAL, "Local"),
            pos2(400.0, 300.0),
        )
    }

    fn typed(t: &mut Transform, keys: &[(usize, bool)], text: &str) -> Mat4 {
        let input = Input {
            mouse: Some(pos2(400.0, 300.0)),
            axis_keys: keys.to_vec(),
            chars: text.chars().collect(),
            ..Default::default()
        };
        t.step(&camera(), NO_SNAP, &input)
    }

    #[test]
    fn constraint_cycles_global_local_off() {
        let x = cycle(None, 0, false);
        assert_eq!(
            x,
            Some(Constraint {
                axis: 0,
                plane: false,
                local: false
            })
        );
        let x2 = cycle(x, 0, false);
        assert!(x2.unwrap().local);
        assert_eq!(cycle(x2, 0, false), None);
        // Another axis or the plane variant starts over in global.
        assert_eq!(cycle(x2, 1, false).unwrap().axis, 1);
        assert!(cycle(x, 0, true).unwrap().plane);
        assert_eq!(active_axes(cycle(None, 2, true)), [true, true, false]);
        assert_eq!(active_axes(cycle(None, 2, false)), [false, false, true]);
    }

    #[test]
    fn numeric_input_parses() {
        let mut n = NumInput::default();
        assert_eq!(n.value(), None);
        for c in "-1,5".chars() {
            assert!(n.type_char(c));
        }
        assert!(!n.type_char('x'));
        assert!(n.type_char('.'), "second point is accepted but ignored");
        assert_eq!(n.value(), Some(-1.5));
        assert_eq!(n.text(), "-1.5");
        n.backspace();
        n.backspace();
        assert_eq!(n.value(), Some(-1.0));
        n.backspace();
        assert_eq!(n.value(), None);
        assert!(n.is_active(), "the sign is still typed");
        n.backspace();
        assert!(!n.is_active());
    }

    #[test]
    fn typed_move_rotate_scale() {
        // G Z 2
        let m = typed(&mut grab(Kind::Move), &[(2, false)], "2");
        assert!(m.w_axis.truncate().abs_diff_eq(Vec3::Z * 2.0, 1e-5));
        // R Z 90 turns +X into +Y around the pivot (origin).
        let m = typed(&mut grab(Kind::Rotate), &[(2, false)], "90");
        assert!(m.transform_point3(Vec3::X).abs_diff_eq(Vec3::Y, 1e-5));
        // S 0.5 uniform; S Shift+Z 2 leaves Z alone.
        let m = typed(&mut grab(Kind::Scale), &[], "0.5");
        assert!(
            m.transform_point3(Vec3::ONE)
                .abs_diff_eq(Vec3::splat(0.5), 1e-5)
        );
        let m = typed(&mut grab(Kind::Scale), &[(2, true)], "2");
        assert!(
            m.transform_point3(Vec3::ONE)
                .abs_diff_eq(Vec3::new(2.0, 2.0, 1.0), 1e-5)
        );
        let mut t = grab(Kind::Move);
        typed(&mut t, &[(0, false)], "3");
        assert!(t.summary.contains("Move X (Global)  [3|]"), "{}", t.summary);
    }

    #[test]
    fn mouse_move_snaps_and_slows_down() {
        let mut t = grab(Kind::Move);
        let at = |x: f32, precise: bool| Input {
            mouse: Some(pos2(x, 300.0)),
            precise,
            ..Default::default()
        };
        let snap = Snap {
            enabled: true,
            ..NO_SNAP
        };
        let free = t.step(&camera(), NO_SNAP, &at(500.0, false)).w_axis.x;
        assert!(free > 0.5, "100 px to the right moves right ({free})");
        let snapped = t.step(&camera(), snap, &at(500.0, false)).w_axis;
        assert_eq!(snapped.x, snapped.x.round());
        // Shift: the next 100 px count as 10.
        let slow = t.step(&camera(), NO_SNAP, &at(600.0, true)).w_axis.x;
        assert!((slow - free * 1.1).abs() < 0.05, "{slow} vs {free}");
    }

    #[test]
    fn mouse_rotation_follows_the_cursor() {
        let mut t = grab(Kind::Rotate);
        let at = |p: Pos2| Input {
            mouse: Some(p),
            ..Default::default()
        };
        // Pivot projects to the screen centre; start right of it.
        t.step(&camera(), NO_SNAP, &at(pos2(500.0, 300.0)));
        t.start = pos2(500.0, 300.0);
        t.angle = 0.0;
        // Quarter turn clockwise on screen (y down): right → below.
        let m = t.step(&camera(), NO_SNAP, &at(pos2(400.0, 400.0)));
        let p = m.transform_point3(Vec3::X);
        assert!(p.abs_diff_eq(-Vec3::Y, 1e-3), "clockwise seen from +Z: {p}");
    }

    #[test]
    fn scaling_and_apply_to_mesh() {
        let s = scaling(Vec3::ONE, GLOBAL, [true, false, false], 3.0);
        assert!(
            s.transform_point3(Vec3::new(2.0, 2.0, 2.0))
                .abs_diff_eq(Vec3::new(4.0, 2.0, 2.0), 1e-5)
        );
        let world = Mat4::from_translation(Vec3::X * 10.0);
        let cube = PolyMesh::cube(2.0);
        let moved = apply(&cube, &[0], world, Mat4::from_translation(Vec3::Y));
        assert_eq!(moved.positions[0], cube.positions[0] + Vec3::Y);
        assert_eq!(moved.positions[1], cube.positions[1]);
        let b = normal_basis(Vec3::Z);
        assert!(b[0].cross(b[1]).abs_diff_eq(b[2], 1e-5));
        assert!((wrap_angle(3.0 * std::f32::consts::PI) - std::f32::consts::PI).abs() < 1e-4);
    }
}

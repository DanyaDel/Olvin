//! Blender's Move tool gizmo for Edit Mode: three arrows, three plane
//! squares and a centre circle at the selection pivot, constant on screen.

use olvin_editor::egui::{self, Color32, Pos2, Stroke, vec2};
use olvin_editor::ext::ViewCamera;
use olvin_editor::glam::Vec3;
use olvin_editor::theme;

use crate::transform::Constraint;

/// Arrow length in screen points.
const SIZE: f32 = 80.0;
const HOT: Color32 = Color32::from_rgb(255, 225, 70);
const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Axis(usize),
    Plane(usize),
    Free,
}

impl Handle {
    /// The transform constraint a drag on this handle uses.
    pub fn constraint(self) -> Option<Constraint> {
        match self {
            Handle::Axis(axis) => Some(Constraint {
                axis,
                plane: false,
                local: false,
            }),
            Handle::Plane(axis) => Some(Constraint {
                axis,
                plane: true,
                local: false,
            }),
            Handle::Free => None,
        }
    }
}

/// The gizmo laid out on screen for one frame.
pub struct Gizmo {
    center: Pos2,
    tips: [Option<Pos2>; 3],
    quads: [Option<[Pos2; 4]>; 3],
}

impl Gizmo {
    pub fn new(camera: &ViewCamera, origin: Vec3) -> Option<Self> {
        let center = camera.project(origin)?;
        let len = camera.world_per_point(origin) * SIZE;
        let tips = AXES.map(|a| {
            camera
                .project(origin + a * len)
                .filter(|t| t.distance(center) > 8.0)
        });
        let quads = std::array::from_fn(|i| {
            let (u, v) = (AXES[(i + 1) % 3], AXES[(i + 2) % 3]);
            let (a, b) = (0.2 * len, 0.42 * len);
            let pts = [(a, a), (b, a), (b, b), (a, b)]
                .map(|(x, y)| camera.project(origin + u * x + v * y));
            Some([pts[0]?, pts[1]?, pts[2]?, pts[3]?])
        });
        Some(Self {
            center,
            tips,
            quads,
        })
    }

    pub fn hit(&self, p: Pos2) -> Option<Handle> {
        if p.distance(self.center) < 11.0 {
            return Some(Handle::Free);
        }
        if let Some(i) = (0..3).find(|&i| self.quads[i].is_some_and(|q| in_quad(p, &q))) {
            return Some(Handle::Plane(i));
        }
        (0..3)
            .filter_map(|i| {
                let tip = self.tips[i]?;
                let end = tip + (tip - self.center).normalized() * 12.0;
                Some((i, dist_to_segment(p, self.center, end)))
            })
            .filter(|(_, d)| *d < 7.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| Handle::Axis(i))
    }

    pub fn draw(&self, painter: &egui::Painter, hovered: Option<Handle>) {
        let color = |h: Handle, base: Color32| if hovered == Some(h) { HOT } else { base };
        for (i, quad) in self.quads.iter().enumerate() {
            if let Some(q) = quad {
                let c = color(Handle::Plane(i), theme::AXES[i]);
                painter.add(egui::Shape::convex_polygon(
                    q.to_vec(),
                    c.gamma_multiply(0.35),
                    Stroke::new(1.0, c),
                ));
            }
        }
        for (i, tip) in self.tips.iter().enumerate() {
            let Some(tip) = *tip else { continue };
            let c = color(Handle::Axis(i), theme::AXES[i]);
            let dir = (tip - self.center).normalized();
            let start = self.center + dir * 12.0;
            painter.line_segment([start, tip], (2.5, c));
            let side = vec2(-dir.y, dir.x) * 5.0;
            painter.add(egui::Shape::convex_polygon(
                vec![tip + dir * 12.0, tip + side, tip - side],
                c,
                Stroke::NONE,
            ));
        }
        let c = color(Handle::Free, Color32::WHITE);
        painter.circle_stroke(
            self.center,
            9.0,
            (
                if hovered == Some(Handle::Free) {
                    2.5
                } else {
                    1.5
                },
                c,
            ),
        );
    }
}

fn in_quad(p: Pos2, q: &[Pos2; 4]) -> bool {
    in_tri(p, [q[0], q[1], q[2]]) || in_tri(p, [q[0], q[2], q[3]])
}

fn in_tri(p: Pos2, [a, b, c]: [Pos2; 3]) -> bool {
    let side = |u: Pos2, v: Pos2| (v - u).x * (p - u).y - (v - u).y * (p - u).x;
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

pub(crate) fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_editor::egui::pos2;

    #[test]
    fn handles_are_hit() {
        let g = Gizmo {
            center: pos2(100.0, 100.0),
            tips: [Some(pos2(180.0, 100.0)), Some(pos2(100.0, 20.0)), None],
            quads: [
                None,
                None,
                Some([
                    pos2(116.0, 84.0),
                    pos2(134.0, 84.0),
                    pos2(134.0, 66.0),
                    pos2(116.0, 66.0),
                ]),
            ],
        };
        assert_eq!(g.hit(pos2(102.0, 101.0)), Some(Handle::Free));
        assert_eq!(g.hit(pos2(160.0, 103.0)), Some(Handle::Axis(0)));
        assert_eq!(g.hit(pos2(99.0, 40.0)), Some(Handle::Axis(1)));
        assert_eq!(g.hit(pos2(125.0, 75.0)), Some(Handle::Plane(2)));
        assert_eq!(g.hit(pos2(170.0, 170.0)), None);
        assert!(Handle::Plane(2).constraint().unwrap().plane);
    }
}

//! Transform gizmo: move / rotate / scale handles, their hit testing and
//! applying a drag to the selection.

use egui::{Align2, Color32, FontId, PointerButton, Pos2, Stroke, Vec2, vec2};
use glam::{EulerRot, Mat4, Quat, Vec3};

use super::{AXES, HOT, Viewport};
use crate::ext::{Snap, ViewCamera};
use crate::state::tools;
use crate::{EditorState, theme};

const GIZMO_POINTS: f32 = 95.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Handle {
    Axis(usize),
    Plane(usize),
    Free,
    Rotate(usize),
    RotateView,
    Scale(usize),
    ScaleAll,
}

pub(super) struct Drag {
    handle: Handle,
    entity: u64,
    scale: Vec3,
    world_rotation: Quat,
    parent: Mat4,
    origin: Vec3,
    axes: [Vec3; 3],
    param: f32,
    hit: Vec3,
    mouse: Pos2,
    origin_screen: Pos2,
    axis_screen: Vec2,
    angle: f32,
    /// Other selected objects moved along: (id, world start, parent inverse).
    others: Vec<(u64, Vec3, Mat4)>,
}

impl Viewport {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn gizmo(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        painter: &egui::Painter,
        camera: &ViewCamera,
        snap: Snap,
        state: &mut EditorState,
        worlds: &std::collections::HashMap<u64, Mat4>,
    ) -> bool {
        let Some(sel) = state.selection else {
            self.drag = None;
            return false;
        };
        let Some(&world) = worlds.get(&sel) else {
            return false;
        };
        let (_, world_rotation, origin) = world.to_scale_rotation_translation();
        let tool = state.tool.as_str();
        let local = state.local_space || tool == tools::SCALE;
        let axes = match &self.drag {
            Some(d) => d.axes,
            None if local => AXES.map(|a| (world_rotation * a).normalize()),
            None => AXES,
        };
        let Some(o) = camera.project(origin) else {
            return false;
        };
        let len = camera.world_per_point(origin) * GIZMO_POINTS;
        let tips: [Option<Pos2>; 3] =
            std::array::from_fn(|i| camera.project(origin + axes[i] * len));
        let to_eye = (camera.eye - origin).normalize();

        let hovered = match &self.drag {
            Some(d) => Some(d.handle),
            None => response
                .hover_pos()
                .and_then(|p| hit_handle(tool, p, camera, origin, axes, len, o, &tips)),
        };
        draw_handles(
            painter, tool, camera, origin, axes, len, o, &tips, hovered, to_eye,
        );

        if self.drag.is_none()
            && response.drag_started_by(PointerButton::Primary)
            && let Some(handle) = hovered
            && let Some(e) = state.scene.get(sel)
        {
            let mouse = ui.input(|i| i.pointer.press_origin()).unwrap_or(o);
            let ray = camera.ray(mouse);
            let parent = e
                .parent
                .and_then(|p| worlds.get(&p).copied())
                .unwrap_or(Mat4::IDENTITY);
            let (param, hit) = match handle {
                Handle::Axis(i) => (ray.closest_on_line(origin, axes[i]).unwrap_or(0.0), origin),
                Handle::Plane(i) => (
                    0.0,
                    ray.plane(origin, axes[i])
                        .map(|t| ray.at(t))
                        .unwrap_or(origin),
                ),
                Handle::Free => (
                    0.0,
                    ray.plane(origin, camera.forward)
                        .map(|t| ray.at(t))
                        .unwrap_or(origin),
                ),
                _ => (0.0, origin),
            };
            let axis_screen = match handle {
                Handle::Scale(i) => tips[i].map(|t| t - o).unwrap_or(vec2(1.0, 0.0)),
                _ => vec2(1.0, 0.0),
            };
            let others = state
                .selected_ids()
                .into_iter()
                .filter(|id| *id != sel)
                .filter_map(|id| {
                    let start = worlds.get(&id)?.w_axis.truncate();
                    let parent = state
                        .scene
                        .get(id)?
                        .parent
                        .and_then(|p| worlds.get(&p).copied())
                        .unwrap_or(Mat4::IDENTITY);
                    Some((id, start, parent.inverse()))
                })
                .collect();
            self.drag = Some(Drag {
                others,
                handle,
                entity: sel,
                scale: e.scale,
                world_rotation,
                parent,
                origin,
                axes,
                param,
                hit,
                mouse,
                origin_screen: o,
                axis_screen,
                angle: 0.0,
            });
        }

        if let Some(d) = &mut self.drag
            && response.dragged_by(PointerButton::Primary)
            && let Some(mouse) = response.interact_pointer_pos()
        {
            apply_drag(d, mouse, camera, snap, state);
            if let Handle::Rotate(_) | Handle::RotateView = d.handle {
                painter.text(
                    o + vec2(0.0, -len / camera.world_per_point(origin) - 24.0),
                    Align2::CENTER_CENTER,
                    format!("{:.1}°", d.angle.to_degrees()),
                    FontId::proportional(13.0),
                    HOT,
                );
            }
        }
        if response.drag_stopped() {
            self.drag = None;
        }
        hovered.is_some()
    }
}

#[allow(clippy::too_many_arguments)]
fn hit_handle(
    tool: &str,
    p: Pos2,
    camera: &ViewCamera,
    origin: Vec3,
    axes: [Vec3; 3],
    len: f32,
    o: Pos2,
    tips: &[Option<Pos2>; 3],
) -> Option<Handle> {
    let near_axis = |i: usize| {
        tips[i]
            .filter(|t| t.distance(o) > 6.0)
            .map(|t| dist_to_segment(p, o, t))
    };
    match tool {
        tools::MOVE => {
            if p.distance(o) < 9.0 {
                return Some(Handle::Free);
            }
            for i in 0..3 {
                if let Some(quad) = plane_quad(camera, origin, axes, len, i)
                    && point_in_quad(p, &quad)
                {
                    return Some(Handle::Plane(i));
                }
            }
            closest((0..3).filter_map(|i| Some((Handle::Axis(i), near_axis(i)?))))
        }
        tools::ROTATE => {
            let view_r = view_ring_radius(camera, origin, len, o);
            if (p.distance(o) - view_r).abs() < 6.0 {
                return Some(Handle::RotateView);
            }
            let eye_dir = (camera.eye - origin).normalize();
            closest((0..3).map(|i| {
                let pts = ring(camera, origin, axes[i], len, eye_dir);
                let d = pts
                    .windows(2)
                    .filter(|w| w[0].1 && w[1].1)
                    .map(|w| dist_to_segment(p, w[0].0, w[1].0))
                    .fold(f32::MAX, f32::min);
                (Handle::Rotate(i), d)
            }))
        }
        tools::SCALE => {
            if p.distance(o) < 10.0 {
                return Some(Handle::ScaleAll);
            }
            closest((0..3).filter_map(|i| Some((Handle::Scale(i), near_axis(i)?))))
        }
        _ => None,
    }
}

fn closest(candidates: impl Iterator<Item = (Handle, f32)>) -> Option<Handle> {
    candidates
        .filter(|(_, d)| *d < 7.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| h)
}

fn plane_quad(
    camera: &ViewCamera,
    origin: Vec3,
    axes: [Vec3; 3],
    len: f32,
    normal: usize,
) -> Option<[Pos2; 4]> {
    let (j, k) = ((normal + 1) % 3, (normal + 2) % 3);
    let (a, b) = (0.18 * len, 0.42 * len);
    let pts = [(a, a), (b, a), (b, b), (a, b)]
        .map(|(u, v)| camera.project(origin + axes[j] * u + axes[k] * v));
    Some([pts[0]?, pts[1]?, pts[2]?, pts[3]?])
}

fn view_ring_radius(camera: &ViewCamera, origin: Vec3, len: f32, o: Pos2) -> f32 {
    let right = camera.view.row(0).truncate();
    camera
        .project(origin + right * len)
        .map(|p| p.distance(o) * 1.18)
        .unwrap_or(100.0)
}

/// Ring around `axis`; the flag marks the half facing the camera.
fn ring(
    camera: &ViewCamera,
    origin: Vec3,
    axis: Vec3,
    len: f32,
    eye_dir: Vec3,
) -> Vec<(Pos2, bool)> {
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    (0..=64)
        .filter_map(|i| {
            let t = i as f32 / 64.0 * std::f32::consts::TAU;
            let dir = u * t.cos() + v * t.sin();
            let p = origin + dir * len;
            Some((camera.project(p)?, dir.dot(eye_dir) > -0.08))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn draw_handles(
    painter: &egui::Painter,
    tool: &str,
    camera: &ViewCamera,
    origin: Vec3,
    axes: [Vec3; 3],
    len: f32,
    o: Pos2,
    tips: &[Option<Pos2>; 3],
    hovered: Option<Handle>,
    eye_dir: Vec3,
) {
    let color = |h: Handle, base: Color32| if hovered == Some(h) { HOT } else { base };
    match tool {
        tools::MOVE => {
            for i in 0..3 {
                if let Some(quad) = plane_quad(camera, origin, axes, len, i) {
                    let c = color(Handle::Plane(i), theme::AXES[i]);
                    painter.add(egui::Shape::convex_polygon(
                        quad.to_vec(),
                        c.gamma_multiply(0.35),
                        Stroke::new(1.0, c),
                    ));
                }
            }
            for (i, tip) in tips.iter().enumerate() {
                let Some(tip) = *tip else { continue };
                let c = color(Handle::Axis(i), theme::AXES[i]);
                painter.line_segment([o, tip], (2.5, c));
                let dir = (tip - o).normalized();
                let side = vec2(-dir.y, dir.x) * 5.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![tip + dir * 12.0, tip + side, tip - side],
                    c,
                    Stroke::NONE,
                ));
            }
            let c = color(Handle::Free, Color32::WHITE);
            painter.rect_stroke(
                egui::Rect::from_center_size(o, vec2(10.0, 10.0)),
                1.0,
                (1.5, c),
                egui::StrokeKind::Middle,
            );
        }
        tools::ROTATE => {
            for (i, axis) in axes.iter().enumerate() {
                let c = color(Handle::Rotate(i), theme::AXES[i]);
                let pts = ring(camera, origin, *axis, len, eye_dir);
                for w in pts.windows(2) {
                    let front = w[0].1 && w[1].1;
                    let stroke = if front {
                        Stroke::new(2.0, c)
                    } else {
                        Stroke::new(1.0, c.gamma_multiply(0.25))
                    };
                    painter.line_segment([w[0].0, w[1].0], stroke);
                }
            }
            let r = view_ring_radius(camera, origin, len, o);
            painter.circle_stroke(
                o,
                r,
                (1.5, color(Handle::RotateView, Color32::from_gray(210))),
            );
            painter.circle_filled(o, 2.5, Color32::WHITE);
        }
        tools::SCALE => {
            for (i, tip) in tips.iter().enumerate() {
                let Some(tip) = *tip else { continue };
                let c = color(Handle::Scale(i), theme::AXES[i]);
                painter.line_segment([o, tip], (2.5, c));
                painter.rect_filled(egui::Rect::from_center_size(tip, vec2(9.0, 9.0)), 1.0, c);
            }
            painter.rect_filled(
                egui::Rect::from_center_size(o, vec2(11.0, 11.0)),
                1.0,
                color(Handle::ScaleAll, Color32::from_gray(230)),
            );
        }
        _ => {}
    }
}

fn apply_drag(d: &mut Drag, mouse: Pos2, camera: &ViewCamera, snap: Snap, state: &mut EditorState) {
    let ray = camera.ray(mouse);
    let to_local = d.parent.inverse();
    let world_axes_aligned = d.axes == AXES;
    let snap_world = |mut p: Vec3, components: &[usize]| {
        for &c in components {
            p[c] = snap.value(p[c]);
        }
        p
    };
    let Some(e) = state.scene.get_mut(d.entity) else {
        return;
    };
    let mut moved = None;

    match d.handle {
        Handle::Axis(i) => {
            let Some(t) = ray.closest_on_line(d.origin, d.axes[i]) else {
                return;
            };
            let mut delta = t - d.param;
            if !world_axes_aligned {
                delta = snap.value(delta);
            }
            let mut p = d.origin + d.axes[i] * delta;
            if world_axes_aligned {
                p = snap_world(p, &[i]);
            }
            e.position = to_local.transform_point3(p);
            moved = Some(p - d.origin);
        }
        Handle::Plane(_) | Handle::Free => {
            let normal = match d.handle {
                Handle::Plane(i) => d.axes[i],
                _ => camera.forward,
            };
            let Some(t) = ray.plane(d.origin, normal) else {
                return;
            };
            let delta = ray.at(t) - d.hit;
            let p = match d.handle {
                Handle::Plane(i) if world_axes_aligned => {
                    snap_world(d.origin + delta, &[(i + 1) % 3, (i + 2) % 3])
                }
                Handle::Plane(i) => {
                    let (j, k) = ((i + 1) % 3, (i + 2) % 3);
                    d.origin
                        + d.axes[j] * snap.value(delta.dot(d.axes[j]))
                        + d.axes[k] * snap.value(delta.dot(d.axes[k]))
                }
                _ => snap_world(d.origin + delta, &[0, 1, 2]),
            };
            e.position = to_local.transform_point3(p);
            moved = Some(p - d.origin);
        }
        Handle::Rotate(_) | Handle::RotateView => {
            let axis = match d.handle {
                Handle::Rotate(i) => d.axes[i],
                _ => (camera.eye - d.origin).normalize(),
            };
            let (v0, v1) = (d.mouse - d.origin_screen, mouse - d.origin_screen);
            if v0.length() < 2.0 || v1.length() < 2.0 {
                return;
            }
            // Screen angle (clockwise positive, y is down) -> right-handed angle about the axis.
            let screen = (v0.x * v1.y - v0.y * v1.x).atan2(v0.dot(v1));
            let facing = axis.dot(camera.eye - d.origin).signum();
            d.angle = snap.angle(-screen * facing);
            let world = Quat::from_axis_angle(axis, d.angle) * d.world_rotation;
            let (_, parent_rotation, _) = d.parent.to_scale_rotation_translation();
            let (y, x, z) = (parent_rotation.inverse() * world).to_euler(EulerRot::YXZ);
            e.rotation = Vec3::new(x, y, z).map(f32::to_degrees);
        }
        Handle::Scale(i) => {
            let l = d.axis_screen.length().max(1.0);
            let mut f = 1.0 + (mouse - d.mouse).dot(d.axis_screen / l) / l;
            if snap.enabled {
                f = (f * 10.0).round() / 10.0;
            }
            e.scale[i] = d.scale[i] * f;
        }
        Handle::ScaleAll => {
            let m = mouse - d.mouse;
            let mut f = 1.0 + (m.x - m.y) / 120.0;
            if snap.enabled {
                f = (f * 10.0).round() / 10.0;
            }
            e.scale = d.scale * f;
        }
    }
    e.position = e.position.map(|v| if v.abs() < 1e-6 { 0.0 } else { v });
    // Multi-selection moves together (rotate/scale apply to the active object).
    if let Some(delta) = moved {
        for (id, start, parent_inv) in &d.others {
            if let Some(o) = state.scene.get_mut(*id) {
                o.position = parent_inv.transform_point3(*start + delta);
            }
        }
    }
}

fn point_in_quad(p: Pos2, q: &[Pos2; 4]) -> bool {
    point_in_tri(p, [q[0], q[1], q[2]]) || point_in_tri(p, [q[0], q[2], q[3]])
}

fn point_in_tri(p: Pos2, [a, b, c]: [Pos2; 3]) -> bool {
    let cross = |o: Pos2, u: Pos2, v: Pos2| (u.x - o.x) * (v.y - o.y) - (u.y - o.y) * (v.x - o.x);
    let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

//! Edit Polygons: select vertices, edges or faces of a brush and move them.
//! The brush stays convex: it is always the hull of its vertices.

use olvin_editor::egui::{self, Align2, Color32, FontId, Key, PointerButton, Pos2, Stroke, vec2};
use olvin_editor::ext::{Ray, Snap, ViewCamera, ViewportCtx};
use olvin_editor::glam::{Mat4, Vec3};
use olvin_editor::olvin_scene::file::{BrushDesc, BrushShape};
use olvin_editor::{NewEntity, theme};

use crate::shared;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ElementMode {
    #[default]
    Vertex,
    Edge,
    Face,
}

/// Selection shared with the CSG panel.
#[derive(Default)]
pub struct PolyState {
    pub mode: ElementMode,
    pub brush: Option<u64>,
    /// Selected vertex indices (edges and faces select their vertices).
    pub selected: Vec<usize>,
}

/// The brush's editable geometry in local (unit-cube) space.
pub struct Topology {
    pub vertices: Vec<Vec3>,
    pub faces: Vec<Vec<usize>>,
    pub edges: Vec<[usize; 2]>,
}

impl Topology {
    pub fn of(brush: &BrushDesc) -> Option<Self> {
        let vertices = match &brush.vertices {
            Some(v) => v.clone(),
            None => olvin_csg::shape_vertices(brush.shape)?,
        };
        let hull = olvin_csg::convex_hull(&vertices)?;
        let index = |p: Vec3| vertices.iter().position(|v| v.abs_diff_eq(p, 1e-4));
        let faces: Vec<Vec<usize>> = hull.iter().filter_map(|f| f.iter().map(|p| index(*p)).collect()).collect();
        let mut edges: Vec<[usize; 2]> = Vec::new();
        for f in &faces {
            for i in 0..f.len() {
                let (a, b) = (f[i], f[(i + 1) % f.len()]);
                let e = [a.min(b), a.max(b)];
                if !edges.contains(&e) {
                    edges.push(e);
                }
            }
        }
        Some(Self { vertices, faces, edges })
    }
}

struct Drag {
    /// World direction to move along; `None` moves freely in the view plane.
    axis: Option<Vec3>,
    param: f32,
    hit: Vec3,
    center: Vec3,
    start: Vec<Vec3>,
    snap_world_axis: Option<usize>,
}

#[derive(Default)]
pub struct PolyTool {
    drag: Option<Drag>,
}

const VERTEX: Color32 = Color32::from_rgb(235, 235, 235);
const HOT: Color32 = Color32::from_rgb(255, 225, 70);
const NORMAL: Color32 = Color32::from_rgb(90, 220, 230);

impl PolyTool {
    /// Returns true: the tool owns the pointer while active.
    pub fn interact(&mut self, ctx: &mut ViewportCtx) -> bool {
        ctx.state.tool_owns_keys = true;
        let Some(sel) = ctx.state.selection else {
            self.drag = None;
            hint(ctx, "Select a brush to edit its polygons");
            return true;
        };
        let Some(brush) = ctx.state.scene.get(sel).and_then(|e| e.brush.clone()) else {
            hint(ctx, "Edit Polygons works on CSG brushes");
            return true;
        };
        let Some(topo) = Topology::of(&brush) else {
            hint(ctx, "Stairs are several brushes in one and can't be edited per vertex");
            return true;
        };
        let world = ctx.state.scene.world_matrices().get(&sel).copied().unwrap_or(Mat4::IDENTITY);
        let points: Vec<Vec3> = topo.vertices.iter().map(|v| world.transform_point3(*v)).collect();

        shared(|s| {
            if s.poly.brush != Some(sel) {
                s.poly.brush = Some(sel);
                s.poly.selected.clear();
            }
            s.poly.selected.retain(|&i| i < points.len());
        });
        self.mode_keys(ctx);
        let (mode, selected) = shared(|s| (s.poly.mode, s.poly.selected.clone()));

        // Draw the cage.
        for [a, b] in &topo.edges {
            let on = selected.contains(a) && selected.contains(b);
            let color = if on { theme::SELECTED } else { Color32::from_gray(210) };
            ctx.camera.line(ctx.painter, points[*a], points[*b], (if on { 2.5 } else { 1.0 }, color));
        }
        for f in &topo.faces {
            if f.iter().all(|i| selected.contains(i)) && mode == ElementMode::Face {
                let poly: Option<Vec<Pos2>> = f.iter().map(|&i| ctx.camera.project(points[i])).collect();
                if let Some(poly) = poly {
                    ctx.painter.add(egui::Shape::convex_polygon(poly, theme::SELECTED.gamma_multiply(0.25), Stroke::NONE));
                }
            }
        }

        // Hover.
        let pointer = ctx.response.hover_pos();
        let hovered = if self.drag.is_none() { pointer.and_then(|p| pick(mode, &topo, &points, ctx.camera, p)) } else { None };
        if let Some(h) = &hovered {
            highlight(ctx, h, &topo, &points);
        }
        for (i, p) in points.iter().enumerate() {
            if let Some(s) = ctx.camera.project(*p) {
                let color = if selected.contains(&i) { theme::SELECTED } else { VERTEX };
                let size = if mode == ElementMode::Vertex { 7.0 } else { 4.0 };
                ctx.painter.rect_filled(egui::Rect::from_center_size(s, vec2(size, size)), 1.0, color);
            }
        }

        // Move gizmo on the selection.
        let mut gizmo_hot = None;
        if !selected.is_empty() {
            let center = selected.iter().map(|&i| points[i]).sum::<Vec3>() / selected.len() as f32;
            let normal = (mode == ElementMode::Face).then(|| face_normal(&topo, &points, &selected)).flatten();
            gizmo_hot = self.gizmo(ctx, center, normal, pointer);
            if self.drag.is_none()
                && ctx.response.drag_started_by(PointerButton::Primary)
                && let Some(axis) = gizmo_hot
            {
                let press = ctx.ui.input(|i| i.pointer.press_origin()).unwrap_or_default();
                let ray = ctx.camera.ray(press);
                let (param, hit) = match axis {
                    Some(a) => (ray.closest_on_line(center, a).unwrap_or(0.0), center),
                    None => (0.0, ray.plane(center, ctx.camera.forward).map(|t| ray.at(t)).unwrap_or(center)),
                };
                let snap_world_axis = axis.and_then(|a| (0..3).find(|&i| a[i].abs() > 0.999));
                self.drag = Some(Drag { axis, param, hit, center, start: points.clone(), snap_world_axis });
                make_editable(ctx, sel, &topo);
            }
        }

        if let Some(d) = &self.drag {
            if ctx.response.dragged_by(PointerButton::Primary)
                && let Some(p) = ctx.response.interact_pointer_pos()
            {
                let delta = drag_delta(d, &ctx.camera.ray(p), ctx.camera, &ctx.snap);
                let inverse = world.inverse();
                if let Some(b) = ctx.state.scene.get_mut(sel).and_then(|e| e.brush.as_mut())
                    && let Some(v) = b.vertices.as_mut()
                {
                    for &i in &selected {
                        if i < v.len() {
                            v[i] = inverse.transform_point3(d.start[i] + delta);
                        }
                    }
                }
                ctx.painter.text(
                    p + vec2(14.0, -14.0),
                    Align2::LEFT_BOTTOM,
                    format!("{:.3}, {:.3}, {:.3}", delta.x, delta.y, delta.z),
                    FontId::monospace(12.0),
                    HOT,
                );
            }
            if ctx.response.drag_stopped() {
                self.drag = None;
            }
            return true;
        }

        // Clicks: select elements, or another brush.
        if ctx.response.clicked_by(PointerButton::Primary) && gizmo_hot.is_none() {
            let add = ctx.ui.input(|i| i.modifiers.shift);
            match hovered {
                Some(h) => {
                    let verts = h.vertices(&topo);
                    shared(|s| {
                        let sel = &mut s.poly.selected;
                        if add {
                            if verts.iter().all(|v| sel.contains(v)) {
                                sel.retain(|v| !verts.contains(v));
                            } else {
                                sel.extend(verts.iter().filter(|v| !sel.contains(v)).copied().collect::<Vec<_>>());
                            }
                        } else {
                            *sel = verts;
                        }
                    });
                }
                None => {
                    let other = pointer.and_then(|p| ctx.geometry.raycast(&ctx.camera.ray(p))).and_then(|h| h.entity);
                    match other {
                        Some(id) if id != sel => ctx.state.selection = Some(id),
                        _ if !add => shared(|s| s.poly.selected.clear()),
                        _ => {}
                    }
                }
            }
        }
        if ctx.ui.input(|i| i.key_pressed(Key::Delete)) && !selected.is_empty() {
            delete_vertices(ctx, sel, &topo, &selected);
        }
        true
    }

    fn mode_keys(&mut self, ctx: &ViewportCtx) {
        if !ctx.response.hovered() || ctx.ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        for (key, mode) in [(Key::Num1, ElementMode::Vertex), (Key::Num2, ElementMode::Edge), (Key::Num3, ElementMode::Face)] {
            if ctx.ui.input(|i| i.key_pressed(key)) {
                shared(|s| s.poly.mode = mode);
            }
        }
    }

    /// Axis arrows (+ face normal); returns the axis under the pointer
    /// (`Some(None)` for the free-move centre).
    fn gizmo(&self, ctx: &ViewportCtx, center: Vec3, normal: Option<Vec3>, pointer: Option<Pos2>) -> Option<Option<Vec3>> {
        let o = ctx.camera.project(center)?;
        let len = ctx.camera.world_per_point(center) * 70.0;
        let mut arrows: Vec<(Vec3, Color32)> = vec![(Vec3::X, theme::AXES[0]), (Vec3::Y, theme::AXES[1]), (Vec3::Z, theme::AXES[2])];
        if let Some(n) = normal {
            arrows.push((n, NORMAL));
        }
        let dragging = self.drag.as_ref().map(|d| d.axis);
        let mut hot = dragging;
        if hot.is_none() && pointer.is_some_and(|p| p.distance(o) < 8.0) {
            hot = Some(None);
        }
        for (dir, color) in &arrows {
            let Some(tip) = ctx.camera.project(center + *dir * len) else { continue };
            let is_hot = hot == Some(Some(*dir))
                || (hot.is_none() && pointer.is_some_and(|p| dist_to_segment(p, o, tip) < 7.0));
            if is_hot && hot.is_none() {
                hot = Some(Some(*dir));
            }
            let c = if is_hot { HOT } else { *color };
            ctx.painter.line_segment([o, tip], (2.5, c));
            ctx.painter.circle_filled(tip, 4.5, c);
        }
        let free = if hot == Some(None) { HOT } else { Color32::WHITE };
        ctx.painter.rect_stroke(egui::Rect::from_center_size(o, vec2(10.0, 10.0)), 1.0, (1.5, free), egui::StrokeKind::Middle);
        hot
    }
}

#[derive(Clone, Copy)]
enum Element {
    Vertex(usize),
    Edge(usize),
    Face(usize),
}

impl Element {
    fn vertices(&self, topo: &Topology) -> Vec<usize> {
        match *self {
            Element::Vertex(i) => vec![i],
            Element::Edge(e) => topo.edges[e].to_vec(),
            Element::Face(f) => topo.faces[f].clone(),
        }
    }
}

fn pick(mode: ElementMode, topo: &Topology, points: &[Vec3], camera: &ViewCamera, p: Pos2) -> Option<Element> {
    match mode {
        ElementMode::Vertex => points
            .iter()
            .enumerate()
            .filter_map(|(i, v)| Some((i, camera.project(*v)?.distance(p))))
            .filter(|(_, d)| *d < 9.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| Element::Vertex(i)),
        ElementMode::Edge => topo
            .edges
            .iter()
            .enumerate()
            .filter_map(|(i, [a, b])| Some((i, dist_to_segment(p, camera.project(points[*a])?, camera.project(points[*b])?))))
            .filter(|(_, d)| *d < 7.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| Element::Edge(i)),
        ElementMode::Face => {
            let ray = camera.ray(p);
            topo.faces
                .iter()
                .enumerate()
                .filter_map(|(i, f)| {
                    (1..f.len() - 1)
                        .filter_map(|k| ray.triangle(points[f[0]], points[f[k]], points[f[k + 1]]))
                        .min_by(|a, b| a.total_cmp(b))
                        .map(|t| (i, t))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| Element::Face(i))
        }
    }
}

fn highlight(ctx: &ViewportCtx, h: &Element, topo: &Topology, points: &[Vec3]) {
    match *h {
        Element::Vertex(i) => {
            if let Some(s) = ctx.camera.project(points[i]) {
                ctx.painter.circle_stroke(s, 7.0, (2.0, HOT));
            }
        }
        Element::Edge(e) => {
            let [a, b] = topo.edges[e];
            ctx.camera.line(ctx.painter, points[a], points[b], (3.0, HOT));
        }
        Element::Face(f) => {
            let poly: Option<Vec<Pos2>> = topo.faces[f].iter().map(|&i| ctx.camera.project(points[i])).collect();
            if let Some(poly) = poly {
                ctx.painter.add(egui::Shape::convex_polygon(poly, HOT.gamma_multiply(0.25), Stroke::new(1.5, HOT)));
            }
        }
    }
}

/// Average outward normal of the fully selected faces.
fn face_normal(topo: &Topology, points: &[Vec3], selected: &[usize]) -> Option<Vec3> {
    let center = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    let n: Vec3 = topo
        .faces
        .iter()
        .filter(|f| f.iter().all(|i| selected.contains(i)))
        .map(|f| {
            let (a, b, c) = (points[f[0]], points[f[1]], points[f[2]]);
            let mut n = (b - a).cross(c - a).normalize_or_zero();
            if n.dot(a - center) < 0.0 {
                n = -n;
            }
            n
        })
        .sum();
    (n.length() > 1e-4).then(|| n.normalize())
}

fn drag_delta(d: &Drag, ray: &Ray, camera: &ViewCamera, snap: &Snap) -> Vec3 {
    match d.axis {
        Some(axis) => {
            let Some(t) = ray.closest_on_line(d.center, axis) else { return Vec3::ZERO };
            let raw = t - d.param;
            match d.snap_world_axis {
                // World axis: land the selection centre on the grid.
                Some(i) => {
                    let target = snap.value(d.center[i] + axis[i] * raw);
                    axis * ((target - d.center[i]) * axis[i].signum())
                }
                None => axis * snap.value(raw),
            }
        }
        None => {
            let Some(t) = ray.plane(d.center, camera.forward) else { return Vec3::ZERO };
            snap.vec(d.center + (ray.at(t) - d.hit)) - d.center
        }
    }
}

/// Parametric brushes become vertex brushes the first time they're edited.
fn make_editable(ctx: &mut ViewportCtx, id: u64, topo: &Topology) {
    if let Some(b) = ctx.state.scene.get_mut(id).and_then(|e| e.brush.as_mut())
        && b.vertices.is_none()
    {
        b.vertices = Some(topo.vertices.clone());
    }
}

fn delete_vertices(ctx: &mut ViewportCtx, id: u64, topo: &Topology, selected: &[usize]) {
    let remaining: Vec<Vec3> =
        topo.vertices.iter().enumerate().filter(|(i, _)| !selected.contains(i)).map(|(_, v)| *v).collect();
    if olvin_csg::convex_hull(&remaining).is_none() {
        ctx.state.warn("can't delete: the brush would become flat");
        return;
    }
    if let Some(b) = ctx.state.scene.get_mut(id).and_then(|e| e.brush.as_mut()) {
        b.vertices = Some(remaining);
    }
    shared(|s| s.poly.selected.clear());
}

/// New additive brush: the selected face pushed out by `depth` along its normal.
pub fn extrude_face(state: &mut olvin_editor::EditorState, depth: f32) -> bool {
    let Some(sel) = state.selection else { return false };
    let Some(brush) = state.scene.get(sel).and_then(|e| e.brush.clone()) else { return false };
    let Some(topo) = Topology::of(&brush) else { return false };
    let world = state.scene.world_matrices().get(&sel).copied().unwrap_or(Mat4::IDENTITY);
    let points: Vec<Vec3> = topo.vertices.iter().map(|v| world.transform_point3(*v)).collect();
    let selected = shared(|s| s.poly.selected.clone());
    let Some(face) = topo.faces.iter().find(|f| f.iter().all(|i| selected.contains(i))) else {
        state.warn("select a face first (Face mode)");
        return false;
    };
    let Some(normal) = face_normal(&topo, &points, face) else { return false };
    let base: Vec<Vec3> = face.iter().map(|&i| points[i]).collect();
    let center = base.iter().copied().sum::<Vec3>() / base.len() as f32 + normal * depth * 0.5;
    let mut vertices: Vec<Vec3> = base.iter().map(|p| *p - center).collect();
    vertices.extend(base.iter().map(|p| *p + normal * depth - center));

    let id = state.create(NewEntity::Empty, None);
    let e = state.scene.get_mut(id).unwrap();
    e.name = "Extrusion".into();
    e.position = center;
    e.brush = Some(BrushDesc { shape: BrushShape::Box, op: brush.op, color: brush.color, vertices: Some(vertices) });
    shared(|s| s.poly.selected.clear());
    true
}

pub fn reset_shape(state: &mut olvin_editor::EditorState) {
    if let Some(b) = state.selection.and_then(|id| state.scene.get_mut(id)).and_then(|e| e.brush.as_mut()) {
        b.vertices = None;
    }
    shared(|s| s.poly.selected.clear());
}

fn hint(ctx: &ViewportCtx, text: &str) {
    ctx.painter.text(
        ctx.camera.rect.center_top() + vec2(0.0, 70.0),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(13.0),
        Color32::from_white_alpha(200),
    );
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_topology_has_8_vertices_12_edges_6_faces() {
        let topo = Topology::of(&BrushDesc::default()).unwrap();
        assert_eq!((topo.vertices.len(), topo.edges.len(), topo.faces.len()), (8, 12, 6));
        assert!(Topology::of(&BrushDesc { shape: BrushShape::Stairs { steps: 2 }, ..Default::default() }).is_none());
    }
}

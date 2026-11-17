//! Scene view drawn with egui shapes: CPU transform, near-plane clipping,
//! painter's-algorithm sorting, one mesh per frame. Independent of the GPU
//! renderer, so the editor runs anywhere egui does.

use std::f32::consts::FRAC_PI_2;
use std::sync::LazyLock;

use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use glam::camera::rh::{proj::directx, view};
use glam::{Mat4, Vec3, Vec4Swizzles};
use olvin_scene::file::Shape;

use crate::{EditorState, Panel, theme};

const NEAR: f32 = 0.05;
const FAR: f32 = 5000.0;
const FOV_DEG: f32 = 50.0;
const LIGHT: Vec3 = Vec3::new(-0.4, -1.0, -0.3);
const AXIS_VECS: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];

struct Geometry {
    tris: Vec<[Vec3; 3]>,
    edges: Vec<[Vec3; 2]>,
    double_sided: bool,
}

static CUBE: LazyLock<Geometry> = LazyLock::new(|| {
    let corner = |i: usize| {
        Vec3::new(
            if i & 1 != 0 { 0.5 } else { -0.5 },
            if i & 2 != 0 { 0.5 } else { -0.5 },
            if i & 4 != 0 { 0.5 } else { -0.5 },
        )
    };
    let faces = [[0, 2, 6, 4], [1, 5, 7, 3], [0, 4, 5, 1], [2, 3, 7, 6], [0, 1, 3, 2], [4, 6, 7, 5]];
    let mut tris = Vec::new();
    for face in faces {
        let mut q = face.map(corner);
        let center = q.iter().copied().sum::<Vec3>() / 4.0;
        if (q[1] - q[0]).cross(q[2] - q[0]).dot(center) < 0.0 {
            q.reverse();
        }
        tris.push([q[0], q[1], q[2]]);
        tris.push([q[0], q[2], q[3]]);
    }
    let mut edges = Vec::new();
    for i in 0..8 {
        for bit in [1, 2, 4] {
            if i & bit == 0 {
                edges.push([corner(i), corner(i | bit)]);
            }
        }
    }
    Geometry { tris, edges, double_sided: false }
});

/// Split into tiles: big floors sort poorly against small objects as a single quad.
const PLANE_TILES: usize = 8;

static PLANE: LazyLock<Geometry> = LazyLock::new(|| {
    let at = |i: usize, j: usize| {
        let step = 1.0 / PLANE_TILES as f32;
        Vec3::new(-0.5 + i as f32 * step, 0.0, -0.5 + j as f32 * step)
    };
    let mut tris = Vec::new();
    for i in 0..PLANE_TILES {
        for j in 0..PLANE_TILES {
            let (a, b, c, d) = (at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j));
            tris.push([a, b, c]);
            tris.push([a, c, d]);
        }
    }
    let n = PLANE_TILES;
    let corners = [at(0, 0), at(0, n), at(n, n), at(n, 0)];
    let edges = (0..4).map(|i| [corners[i], corners[(i + 1) % 4]]).collect();
    Geometry { tris, edges, double_sided: true }
});

fn geometry(shape: Shape) -> &'static Geometry {
    match shape {
        Shape::Cube => &CUBE,
        Shape::Plane => &PLANE,
    }
}

struct Orbit {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Orbit {
    fn eye(&self) -> Vec3 {
        let dir = Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        );
        self.target + dir * self.distance
    }

    fn view(&self) -> Mat4 {
        view::look_at_mat4(self.eye(), self.target, Vec3::Y)
    }
}

struct Projector {
    view: Mat4,
    proj: Mat4,
    rect: Rect,
}

impl Projector {
    fn to_view(&self, world: Vec3) -> Vec3 {
        self.view.transform_point3(world)
    }

    fn view_to_screen(&self, v: Vec3) -> Pos2 {
        let clip = self.proj * v.extend(1.0);
        let ndc = clip.xyz() / clip.w;
        pos2(
            self.rect.left() + (ndc.x * 0.5 + 0.5) * self.rect.width(),
            self.rect.top() + (0.5 - ndc.y * 0.5) * self.rect.height(),
        )
    }

    fn project(&self, world: Vec3) -> Option<Pos2> {
        let v = self.to_view(world);
        (v.z < -NEAR).then(|| self.view_to_screen(v))
    }

    /// Draws a world-space segment, clipped against the near plane.
    fn line(&self, painter: &egui::Painter, a: Vec3, b: Vec3, stroke: impl Into<Stroke>) {
        if let Some((a, b)) = clip_segment(self.to_view(a), self.to_view(b)) {
            painter.line_segment([self.view_to_screen(a), self.view_to_screen(b)], stroke);
        }
    }
}

struct Tri {
    pts: [Pos2; 3],
    /// Flat double-sided shapes (floors, walls) draw first, below volumes.
    layer: u8,
    depth: f32,
    color: Color32,
    entity: u64,
}

pub struct Viewport {
    orbit: Orbit,
    show_grid: bool,
    wireframe: bool,
    drag_axis: Option<usize>,
    // Per-frame scratch, kept to avoid reallocating.
    tris: Vec<Tri>,
    poly: Vec<Vec3>,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            orbit: Orbit { target: Vec3::ZERO, yaw: 0.6, pitch: 0.45, distance: 12.0 },
            show_grid: true,
            wireframe: false,
            drag_axis: None,
            tris: Vec::new(),
            poly: Vec::new(),
        }
    }
}

impl Panel for Viewport {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if rect.width() < 4.0 || rect.height() < 4.0 {
            return;
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::VIEWPORT_BG);

        let worlds = state.world_matrices();
        self.navigate(ui, &response, state, &worlds);

        let aspect = rect.width() / rect.height();
        let p = Projector {
            view: self.orbit.view(),
            proj: directx::perspective(FOV_DEG.to_radians(), aspect, NEAR, FAR),
            rect,
        };

        if self.show_grid {
            self.draw_grid(&painter, &p);
        }
        self.build_tris(state, &worlds, &p);
        if self.wireframe {
            for e in &state.scene.entities {
                if let (Some(mesh), Some(m)) = (&e.mesh, worlds.get(&e.id)) {
                    for [a, b] in &geometry(mesh.shape).edges {
                        p.line(&painter, m.transform_point3(*a), m.transform_point3(*b), (1.0, Color32::from_gray(190)));
                    }
                }
            }
        } else {
            let mut mesh = egui::Mesh::default();
            for t in &self.tris {
                let i = mesh.vertices.len() as u32;
                for pt in t.pts {
                    mesh.colored_vertex(pt, t.color);
                }
                mesh.add_triangle(i, i + 1, i + 2);
            }
            painter.add(egui::Shape::mesh(mesh));
        }

        // Selection outline.
        if let Some(sel) = state.selection
            && let Some(e) = state.scene.get(sel)
            && let (Some(mesh), Some(m)) = (&e.mesh, worlds.get(&sel))
        {
            for [a, b] in &geometry(mesh.shape).edges {
                p.line(&painter, m.transform_point3(*a), m.transform_point3(*b), (2.0, theme::SELECTED));
            }
        }

        let camera_icons = self.draw_cameras(&painter, &p, state, &worlds);
        let hovered_axis = self.gizmo(&painter, &p, &response, state, &worlds);

        if response.clicked_by(egui::PointerButton::Primary)
            && hovered_axis.is_none()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            state.selection = self.pick(pointer, &camera_icons);
        }

        self.overlay(ui, rect, &p, state);
    }
}

impl Viewport {
    fn navigate(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        state: &mut EditorState,
        worlds: &std::collections::HashMap<u64, Mat4>,
    ) {
        let delta = response.drag_delta();
        let middle = response.dragged_by(egui::PointerButton::Middle);
        let right = response.dragged_by(egui::PointerButton::Secondary);
        let shift = ui.input(|i| i.modifiers.shift);
        if (middle || right) && shift {
            // Pan in the view plane, scaled so the point under the cursor follows it.
            let view = self.orbit.view();
            let right_v = view.row(0).xyz();
            let up_v = view.row(1).xyz();
            let scale = self.orbit.distance * 0.0016;
            self.orbit.target += (-right_v * delta.x + up_v * delta.y) * scale;
        } else if middle || right {
            self.orbit.yaw -= delta.x * 0.008;
            self.orbit.pitch = (self.orbit.pitch + delta.y * 0.008).clamp(-1.55, 1.55);
        }

        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.orbit.distance = (self.orbit.distance * (-scroll * 0.0015).exp()).clamp(0.2, 4000.0);
            }
            if !ui.ctx().egui_wants_keyboard_input() {
                let keys = ui.input(|i| {
                    [egui::Key::F, egui::Key::Num1, egui::Key::Num3, egui::Key::Num7].map(|k| i.key_pressed(k))
                });
                if keys[0] {
                    state.frame_selection = true;
                }
                if keys[1] {
                    (self.orbit.yaw, self.orbit.pitch) = (0.0, 0.0);
                }
                if keys[2] {
                    (self.orbit.yaw, self.orbit.pitch) = (FRAC_PI_2, 0.0);
                }
                if keys[3] {
                    (self.orbit.yaw, self.orbit.pitch) = (0.0, 1.55);
                }
            }
        }

        if state.frame_selection
            && let Some(sel) = state.selection
            && let Some(m) = worlds.get(&sel)
        {
            let (scale, _, translation) = m.to_scale_rotation_translation();
            self.orbit.target = translation;
            self.orbit.distance = (scale.max_element() * 3.0).clamp(2.0, 500.0);
            state.frame_selection = false;
        }
    }

    fn draw_grid(&self, painter: &egui::Painter, p: &Projector) {
        let step = if self.orbit.distance > 80.0 { 10.0 } else { 1.0 };
        let half = 30.0 * step;
        let cx = (self.orbit.target.x / step).round() * step;
        let cz = (self.orbit.target.z / step).round() * step;
        let minor = Color32::from_rgba_unmultiplied(255, 255, 255, 18);
        let mut i = -half;
        while i <= half {
            let (x, z) = (cx + i, cz + i);
            let cx_line = if x.abs() < step * 0.5 { theme::AXIS_Z.gamma_multiply(0.8) } else { minor };
            let cz_line = if z.abs() < step * 0.5 { theme::AXIS_X.gamma_multiply(0.8) } else { minor };
            p.line(painter, Vec3::new(x, 0.0, cz - half), Vec3::new(x, 0.0, cz + half), (1.0, cx_line));
            p.line(painter, Vec3::new(cx - half, 0.0, z), Vec3::new(cx + half, 0.0, z), (1.0, cz_line));
            i += step;
        }
    }

    fn build_tris(&mut self, state: &EditorState, worlds: &std::collections::HashMap<u64, Mat4>, p: &Projector) {
        self.tris.clear();
        let light = -LIGHT.normalize();
        for e in &state.scene.entities {
            let (Some(mesh), Some(m)) = (&e.mesh, worlds.get(&e.id)) else { continue };
            let geo = geometry(mesh.shape);
            let flipped = m.determinant() < 0.0;
            let [r, g, b, _] = mesh.color;
            for tri in &geo.tris {
                let w = tri.map(|v| m.transform_point3(v));
                let v = w.map(|x| p.to_view(x));
                let facing = ((v[1] - v[0]).cross(v[2] - v[0]).dot(-v[0]) > 0.0) != flipped;
                if !facing && !geo.double_sided {
                    continue;
                }
                let mut n = (w[1] - w[0]).cross(w[2] - w[0]).normalize_or_zero();
                if flipped == facing {
                    n = -n;
                }
                let k = 0.35 + 0.65 * n.dot(light).max(0.0);
                let color: Color32 = egui::Rgba::from_rgb(r * k, g * k, b * k).into();
                let depth = (v[0].z + v[1].z + v[2].z) / 3.0;
                let layer = if geo.double_sided { 0 } else { 1 };

                if v.iter().all(|x| x.z < -NEAR) {
                    self.tris.push(Tri { pts: v.map(|x| p.view_to_screen(x)), layer, depth, color, entity: e.id });
                } else {
                    clip_polygon(&v, &mut self.poly);
                    for i in 1..self.poly.len().saturating_sub(1) {
                        let pts = [self.poly[0], self.poly[i], self.poly[i + 1]].map(|x| p.view_to_screen(x));
                        self.tris.push(Tri { pts, layer, depth, color, entity: e.id });
                    }
                }
            }
        }
        // Painter's algorithm: flat layer first, then farthest first (view z is
        // negative in front of the camera). Approximate by nature; exact
        // occlusion comes with the GPU viewport.
        self.tris.sort_unstable_by(|a, b| a.layer.cmp(&b.layer).then(a.depth.total_cmp(&b.depth)));
    }

    fn draw_cameras(
        &self,
        painter: &egui::Painter,
        p: &Projector,
        state: &EditorState,
        worlds: &std::collections::HashMap<u64, Mat4>,
    ) -> Vec<(u64, Pos2)> {
        let mut icons = Vec::new();
        for e in state.scene.entities.iter().filter(|e| e.camera.is_some()) {
            let Some(m) = worlds.get(&e.id) else { continue };
            let (_, rot, pos) = m.to_scale_rotation_translation();
            let cm = Mat4::from_rotation_translation(rot, pos);
            let color = if state.selection == Some(e.id) { theme::SELECTED } else { Color32::from_gray(225) };
            let apex = cm.transform_point3(Vec3::ZERO);
            let c = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .map(|(x, y)| cm.transform_point3(Vec3::new(0.35 * x, 0.25 * y, -0.6)));
            for i in 0..4 {
                p.line(painter, apex, c[i], (1.5, color));
                p.line(painter, c[i], c[(i + 1) % 4], (1.5, color));
            }
            // "Up" marker so the camera's roll is visible.
            let up = cm.transform_point3(Vec3::new(0.0, 0.42, -0.6));
            p.line(painter, c[2], up, (1.5, color));
            p.line(painter, c[3], up, (1.5, color));
            if let Some(s) = p.project(apex) {
                icons.push((e.id, s));
            }
        }
        icons
    }

    /// Translate gizmo. Returns the axis under the cursor.
    fn gizmo(
        &mut self,
        painter: &egui::Painter,
        p: &Projector,
        response: &egui::Response,
        state: &mut EditorState,
        worlds: &std::collections::HashMap<u64, Mat4>,
    ) -> Option<usize> {
        let sel = state.selection?;
        let world = *worlds.get(&sel)?;
        let origin = world.w_axis.truncate();
        let len = (self.orbit.eye() - origin).length() * 0.15;
        let o = p.project(origin)?;
        let tips: Vec<Option<Pos2>> = AXIS_VECS.iter().map(|a| p.project(origin + *a * len)).collect();

        let pointer = response.hover_pos();
        let hovered = self.drag_axis.or_else(|| {
            let pointer = pointer?;
            (0..3)
                .filter_map(|i| Some((i, dist_to_segment(pointer, o, tips[i]?))))
                .filter(|(_, d)| *d < 8.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        });

        for (i, tip) in tips.iter().enumerate() {
            let Some(tip) = *tip else { continue };
            let color = if hovered == Some(i) { Color32::WHITE } else { theme::AXES[i] };
            painter.line_segment([o, tip], (3.0, color));
            painter.circle_filled(tip, 5.0, color);
        }
        painter.circle_filled(o, 3.5, Color32::WHITE);

        if response.drag_started_by(egui::PointerButton::Primary) {
            self.drag_axis = hovered;
        }
        if let Some(axis) = self.drag_axis
            && response.dragged_by(egui::PointerButton::Primary)
            && let Some(tip) = tips[axis]
        {
            let d = tip - o;
            let pixels = d.length();
            if pixels > 1.0 {
                let amount = response.drag_delta().dot(d / pixels) / pixels * len;
                let parent = state.scene.get(sel).and_then(|e| e.parent);
                let parent_world = parent.and_then(|p| worlds.get(&p).copied()).unwrap_or(Mat4::IDENTITY);
                let local = parent_world.inverse().transform_vector3(AXIS_VECS[axis] * amount);
                if let Some(e) = state.scene.get_mut(sel) {
                    e.position += local;
                }
            }
        }
        if response.drag_stopped() {
            self.drag_axis = None;
        }
        hovered
    }

    fn pick(&self, pointer: Pos2, cameras: &[(u64, Pos2)]) -> Option<u64> {
        let camera = cameras
            .iter()
            .map(|(id, s)| (*id, s.distance(pointer)))
            .filter(|(_, d)| *d < 14.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, _)) = camera {
            return Some(id);
        }
        self.tris.iter().rev().find(|t| point_in_tri(pointer, t.pts)).map(|t| t.entity)
    }

    fn overlay(&mut self, ui: &mut egui::Ui, rect: Rect, p: &Projector, state: &mut EditorState) {
        let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(6.0)));
        bar.horizontal(|ui| {
            ui.toggle_value(&mut self.show_grid, "Сетка");
            ui.toggle_value(&mut self.wireframe, "Каркас");
            if ui.button("⌖").on_hover_text("К выделенному (F)").clicked() {
                // Picked up by `navigate` on the next frame.
                state.frame_selection = true;
            }
        });

        let painter = ui.painter_at(rect);
        // Axis indicator (top right), back to front.
        let center = rect.right_top() + vec2(-42.0, 42.0);
        let mut axes: Vec<(usize, Vec3)> =
            AXIS_VECS.iter().enumerate().map(|(i, a)| (i, p.view.transform_vector3(*a))).collect();
        axes.sort_by(|a, b| a.1.z.total_cmp(&b.1.z));
        painter.circle_filled(center, 34.0, Color32::from_black_alpha(40));
        for (i, d) in axes {
            let tip = center + vec2(d.x, -d.y) * 26.0;
            painter.line_segment([center, tip], (2.0, theme::AXES[i]));
            painter.circle_filled(tip, 8.0, theme::AXES[i]);
            painter.text(tip, egui::Align2::CENTER_CENTER, ["X", "Y", "Z"][i], egui::FontId::proportional(11.0), Color32::BLACK);
        }

        if !state.preview.is_empty() {
            painter.text(
                rect.center_top() + vec2(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                "• превью анимации",
                egui::FontId::proportional(13.0),
                theme::SELECTED,
            );
        }
        painter.text(
            rect.left_bottom() + vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            "СКМ/ПКМ — вращать · Shift — сдвигать · колесо — зум · F — к объекту · 1/3/7 — виды",
            egui::FontId::proportional(11.0),
            Color32::from_white_alpha(90),
        );
    }
}

/// Sutherland–Hodgman against the near plane (keep z < -NEAR).
fn clip_polygon(input: &[Vec3], out: &mut Vec<Vec3>) {
    out.clear();
    let inside = |p: Vec3| p.z < -NEAR;
    for i in 0..input.len() {
        let (a, b) = (input[i], input[(i + 1) % input.len()]);
        match (inside(a), inside(b)) {
            (true, true) => out.push(b),
            (true, false) => out.push(intersect(a, b)),
            (false, true) => {
                out.push(intersect(a, b));
                out.push(b);
            }
            (false, false) => {}
        }
    }
}

fn clip_segment(a: Vec3, b: Vec3) -> Option<(Vec3, Vec3)> {
    match (a.z < -NEAR, b.z < -NEAR) {
        (true, true) => Some((a, b)),
        (true, false) => Some((a, intersect(a, b))),
        (false, true) => Some((intersect(a, b), b)),
        (false, false) => None,
    }
}

fn intersect(a: Vec3, b: Vec3) -> Vec3 {
    let t = (-NEAR - a.z) / (b.z - a.z);
    a + (b - a) * t
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_faces_point_outward() {
        for tri in &CUBE.tris {
            let center = (tri[0] + tri[1] + tri[2]) / 3.0;
            assert!((tri[1] - tri[0]).cross(tri[2] - tri[0]).dot(center) > 0.0);
        }
        assert_eq!(CUBE.edges.len(), 12);
    }

    #[test]
    fn near_clipping() {
        let mut out = Vec::new();
        // One vertex behind the camera: the triangle becomes a quad.
        clip_polygon(&[Vec3::new(0.0, 0.0, -5.0), Vec3::new(1.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 1.0)], &mut out);
        assert_eq!(out.len(), 4);
        assert!(out.iter().all(|p| p.z <= -NEAR + 1e-5));
        assert!(clip_segment(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 2.0)).is_none());
    }
}

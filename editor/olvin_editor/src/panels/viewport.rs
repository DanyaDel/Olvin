//! Scene view. Rendered on the GPU inside egui's render pass (depth buffer,
//! MSAA), with an infinite adaptive grid, raycast picking and Unity-style
//! transform tools. Elements extend it through `ViewportExtension`.

use std::f32::consts::FRAC_PI_2;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Align2, Color32, FontId, Pos2, PointerButton, Sense, Stroke, Vec2, vec2};
use glam::camera::rh::{proj::directx, view};
use glam::{EulerRot, Mat4, Quat, Vec3};
use olvin_scene::file::Shape;

use crate::ext::{Geometry, Snap, ViewCamera, ViewportCtx, flags, linear_color};
use crate::render::{Frame, Uniforms};
use crate::state::tools;
use crate::{EditorState, Panel, theme};

const NEAR: f32 = 0.05;
const FAR: f32 = 4000.0;
const FOV_DEG: f32 = 55.0;
const LIGHT: Vec3 = Vec3::new(-0.45, -1.0, -0.3);
const GIZMO_POINTS: f32 = 95.0;
const HOT: Color32 = Color32::from_rgb(255, 225, 70);
const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
const STEPS: [f32; 11] = [1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct ShapeGeometry {
    tris: Vec<[Vec3; 3]>,
    edges: Vec<[Vec3; 2]>,
}

static CUBE: LazyLock<ShapeGeometry> = LazyLock::new(|| {
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
    ShapeGeometry { tris, edges }
});

static PLANE: LazyLock<ShapeGeometry> = LazyLock::new(|| {
    let c = [
        Vec3::new(-0.5, 0.0, -0.5),
        Vec3::new(-0.5, 0.0, 0.5),
        Vec3::new(0.5, 0.0, 0.5),
        Vec3::new(0.5, 0.0, -0.5),
    ];
    ShapeGeometry {
        tris: vec![[c[0], c[1], c[2]], [c[0], c[2], c[3]]],
        edges: vec![[c[0], c[1]], [c[1], c[2]], [c[2], c[3]], [c[3], c[0]]],
    }
});

fn shape_geometry(shape: Shape) -> &'static ShapeGeometry {
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
    fn back(&self) -> Vec3 {
        Vec3::new(self.pitch.cos() * self.yaw.sin(), self.pitch.sin(), self.pitch.cos() * self.yaw.cos())
    }

    fn eye(&self) -> Vec3 {
        self.target + self.back() * self.distance
    }
}

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

struct Drag {
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
}

pub struct Viewport {
    id: u64,
    orbit: Orbit,
    show_grid: bool,
    wireframe: bool,
    drag: Option<Drag>,
    /// Last frame's geometry: raycasts and the GPU upload.
    geometry: Geometry,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            orbit: Orbit { target: Vec3::ZERO, yaw: 0.6, pitch: 0.45, distance: 14.0 },
            show_grid: true,
            wireframe: false,
            drag: None,
            geometry: Geometry::default(),
        }
    }
}

impl Panel for Viewport {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if rect.width() < 8.0 || rect.height() < 8.0 {
            return;
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::VIEWPORT_BG);
        // The 3D image goes here, under everything drawn later this frame.
        let scene_slot = painter.add(egui::Shape::Noop);

        let worlds = state.world_matrices();
        self.hotkeys(ui, &response, state);
        self.navigate(ui, &response, state, &worlds);
        let camera = self.camera(rect);
        let grid_step = self.grid_step();
        let ctrl = ui.input(|i| i.modifiers.command);
        let snap = Snap {
            enabled: state.snap.enabled != ctrl,
            step: state.snap.step.unwrap_or(grid_step),
            angle_deg: state.snap.angle_deg,
        };

        // Element tools see the pointer first, then the gizmo, then selection.
        let mut extensions = std::mem::take(&mut state.ext.viewport);
        let tool = state.tool.clone();
        let mut consumed = false;
        for ext in &mut extensions {
            let mut ctx = ViewportCtx {
                ui,
                response: &response,
                painter: &painter,
                camera: &camera,
                snap,
                state,
                tool: &tool,
                geometry: &self.geometry,
            };
            consumed |= ext.interact(&mut ctx);
        }
        let transform_tool = [tools::MOVE, tools::ROTATE, tools::SCALE].contains(&tool.as_str());
        let mut gizmo_hot = false;
        if !consumed && transform_tool {
            gizmo_hot = self.gizmo(ui, &response, &painter, &camera, snap, state, &worlds);
        } else {
            self.drag = None;
        }
        if !consumed
            && !gizmo_hot
            && response.clicked_by(PointerButton::Primary)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            state.selection = self.pick(pointer, &camera, state, &worlds);
        }

        // Geometry for this frame, after edits.
        let worlds = state.world_matrices();
        self.geometry.clear();
        self.geometry.wireframe = self.wireframe;
        self.scene_geometry(state, &worlds);
        for ext in &mut extensions {
            ext.geometry(state, &mut self.geometry);
        }
        state.ext.viewport = extensions;
        self.selection_outline(&painter, &camera, state, &worlds);

        let inv = camera.view_proj.inverse();
        let frame = Frame {
            viewport: self.id,
            uniforms: Uniforms {
                view_proj: camera.view_proj.to_cols_array_2d(),
                inv_view_proj: inv.to_cols_array_2d(),
                eye: camera.eye.extend(1.0).to_array(),
                light: LIGHT.normalize().extend(0.0).to_array(),
                grid: [snap.step, 8.0, self.fade_distance(), 0.0],
            },
            vertices: self.geometry.vertices.clone(),
            lines: self.geometry.lines.clone(),
            overlay: self.geometry.overlay_lines.clone(),
            grid: self.show_grid,
        };
        painter.set(scene_slot, eframe::egui_wgpu::Callback::new_paint_callback(rect, frame));

        self.overlay(ui, rect, &camera, state, grid_step);
    }
}

impl Viewport {
    fn eye_height(&self) -> f32 {
        self.orbit.eye().y.abs().max(self.orbit.distance * 0.3).max(0.05)
    }

    /// Power-of-two grid step that keeps roughly the same on-screen density.
    fn grid_step(&self) -> f32 {
        let raw = self.eye_height() / 14.0;
        2f32.powf(raw.log2().round()).clamp(STEPS[0], 64.0)
    }

    fn fade_distance(&self) -> f32 {
        (self.eye_height() * 60.0).clamp(30.0, FAR * 0.8)
    }

    fn camera(&self, rect: egui::Rect) -> ViewCamera {
        let eye = self.orbit.eye();
        let view = view::look_at_mat4(eye, self.orbit.target, Vec3::Y);
        let proj = directx::perspective(FOV_DEG.to_radians(), rect.width() / rect.height(), NEAR, FAR);
        ViewCamera { view, proj, view_proj: proj * view, eye, forward: -self.orbit.back(), rect, near: NEAR }
    }

    fn hotkeys(&mut self, ui: &egui::Ui, response: &egui::Response, state: &mut EditorState) {
        let flying = ui.input(|i| i.pointer.button_down(PointerButton::Secondary));
        if !response.hovered() || flying || ui.ctx().egui_wants_keyboard_input() || self.drag.is_some() {
            return;
        }
        let pressed = |k| ui.input(|i| i.key_pressed(k) && i.modifiers.is_none());
        for (key, tool) in [
            (egui::Key::Q, tools::VIEW),
            (egui::Key::W, tools::MOVE),
            (egui::Key::E, tools::ROTATE),
            (egui::Key::R, tools::SCALE),
        ] {
            if pressed(key) {
                state.tool = tool.to_owned();
            }
        }
        for ext in &state.ext.viewport {
            for t in ext.tools() {
                if t.key.is_some_and(pressed) {
                    state.tool = t.id.to_owned();
                }
            }
        }
        if pressed(egui::Key::X) {
            state.local_space = !state.local_space;
        }
        if pressed(egui::Key::F) {
            state.frame_selection = true;
        }
        for (key, yaw, pitch) in [(egui::Key::Num1, 0.0, 0.0), (egui::Key::Num3, FRAC_PI_2, 0.0), (egui::Key::Num7, 0.0, 1.55)] {
            if pressed(key) {
                (self.orbit.yaw, self.orbit.pitch) = (yaw, pitch);
            }
        }
    }

    fn navigate(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        state: &mut EditorState,
        worlds: &std::collections::HashMap<u64, Mat4>,
    ) {
        let delta = response.drag_delta();
        let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
        let middle = response.dragged_by(PointerButton::Middle);
        let right = response.dragged_by(PointerButton::Secondary);
        let left = response.dragged_by(PointerButton::Primary);
        let hand = left && state.tool == tools::VIEW && !alt;

        if (middle && shift) || hand {
            let view = view::look_at_mat4(self.orbit.eye(), self.orbit.target, Vec3::Y);
            let (r, u) = (view.row(0).truncate(), view.row(1).truncate());
            let scale = self.orbit.distance * 0.0016;
            self.orbit.target += (-r * delta.x + u * delta.y) * scale;
        } else if middle || (left && alt) {
            // Orbit around the target (Blender MMB, Unity Alt+LMB).
            self.orbit.yaw -= delta.x * 0.008;
            self.orbit.pitch = (self.orbit.pitch + delta.y * 0.008).clamp(-1.55, 1.55);
        } else if right {
            // Look around from the eye (Unity RMB), WASD to fly.
            let eye = self.orbit.eye();
            self.orbit.yaw -= delta.x * 0.005;
            self.orbit.pitch = (self.orbit.pitch + delta.y * 0.005).clamp(-1.55, 1.55);
            self.orbit.target = eye - self.orbit.back() * self.orbit.distance;
        }

        if response.hovered() || right {
            let flying = ui.input(|i| i.pointer.button_down(PointerButton::Secondary));
            if flying {
                let (dt, boost) = ui.input(|i| (i.stable_dt.min(0.1), if i.modifiers.shift { 4.0 } else { 1.0 }));
                let forward = -self.orbit.back();
                let right_v = forward.cross(Vec3::Y).normalize_or_zero();
                let axis = |a: egui::Key, b: egui::Key| ui.input(|i| i.key_down(a) as i32 as f32 - i.key_down(b) as i32 as f32);
                let mv = forward * axis(egui::Key::W, egui::Key::S)
                    + right_v * axis(egui::Key::D, egui::Key::A)
                    + Vec3::Y * axis(egui::Key::E, egui::Key::Q);
                if mv != Vec3::ZERO {
                    self.orbit.target += mv.normalize() * (self.orbit.distance * 1.2 + 2.0) * boost * dt;
                    ui.ctx().request_repaint();
                }
            }
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 && response.hovered() {
                self.orbit.distance = (self.orbit.distance * (-scroll * 0.0015).exp()).clamp(0.1, FAR * 0.5);
            }
        }

        if state.frame_selection
            && let Some(sel) = state.selection
            && let Some(m) = worlds.get(&sel)
        {
            let (scale, _, translation) = m.to_scale_rotation_translation();
            self.orbit.target = translation;
            self.orbit.distance = (scale.max_element() * 2.5).clamp(2.0, 800.0);
            state.frame_selection = false;
        }
    }

    fn scene_geometry(&mut self, state: &EditorState, worlds: &std::collections::HashMap<u64, Mat4>) {
        for e in &state.scene.entities {
            let Some(m) = worlds.get(&e.id) else { continue };
            if let Some(mesh) = &e.mesh {
                let color = linear_color(mesh.color);
                let f = if state.selection == Some(e.id) { flags::SELECTED } else { 0 };
                for tri in &shape_geometry(mesh.shape).tris {
                    let w = tri.map(|v| m.transform_point3(v));
                    let n = (w[1] - w[0]).cross(w[2] - w[0]).normalize_or_zero();
                    self.geometry.polygon(e.id, &w, n, color, f);
                }
            }
            if e.camera.is_some() {
                let (_, rot, pos) = m.to_scale_rotation_translation();
                let cm = Mat4::from_rotation_translation(rot, pos);
                let color = if state.selection == Some(e.id) { theme::SELECTED } else { Color32::from_gray(220) };
                let apex = cm.transform_point3(Vec3::ZERO);
                let c = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                    .map(|(x, y)| cm.transform_point3(Vec3::new(0.35 * x, 0.25 * y, -0.6)));
                let up = cm.transform_point3(Vec3::new(0.0, 0.42, -0.6));
                for i in 0..4 {
                    self.geometry.line(apex, c[i], color, false);
                    self.geometry.line(c[i], c[(i + 1) % 4], color, false);
                }
                self.geometry.line(c[2], up, color, false);
                self.geometry.line(c[3], up, color, false);
            }
        }
    }

    fn selection_outline(
        &self,
        painter: &egui::Painter,
        camera: &ViewCamera,
        state: &EditorState,
        worlds: &std::collections::HashMap<u64, Mat4>,
    ) {
        let Some(sel) = state.selection else { return };
        let (Some(e), Some(m)) = (state.scene.get(sel), worlds.get(&sel)) else { return };
        if let Some(mesh) = &e.mesh {
            for [a, b] in &shape_geometry(mesh.shape).edges {
                camera.line(painter, m.transform_point3(*a), m.transform_point3(*b), (1.5, theme::SELECTED));
            }
        }
    }

    fn pick(
        &self,
        pointer: Pos2,
        camera: &ViewCamera,
        state: &EditorState,
        worlds: &std::collections::HashMap<u64, Mat4>,
    ) -> Option<u64> {
        // Cameras have no surface: pick them by their icon.
        let icon = state
            .scene
            .entities
            .iter()
            .filter(|e| e.camera.is_some())
            .filter_map(|e| Some((e.id, camera.project(worlds.get(&e.id)?.w_axis.truncate())?.distance(pointer))))
            .filter(|(_, d)| *d < 16.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, _)) = icon {
            return Some(id);
        }
        self.geometry.raycast(&camera.ray(pointer)).and_then(|h| h.entity)
    }

    // --- gizmo -----------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn gizmo(
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
        let Some(&world) = worlds.get(&sel) else { return false };
        let (_, world_rotation, origin) = world.to_scale_rotation_translation();
        let tool = state.tool.as_str();
        let local = state.local_space || tool == tools::SCALE;
        let axes = match &self.drag {
            Some(d) => d.axes,
            None if local => AXES.map(|a| (world_rotation * a).normalize()),
            None => AXES,
        };
        let Some(o) = camera.project(origin) else { return false };
        let len = camera.world_per_point(origin) * GIZMO_POINTS;
        let tips: [Option<Pos2>; 3] = std::array::from_fn(|i| camera.project(origin + axes[i] * len));
        let to_eye = (camera.eye - origin).normalize();

        let hovered = match &self.drag {
            Some(d) => Some(d.handle),
            None => response.hover_pos().and_then(|p| hit_handle(tool, p, camera, origin, axes, len, o, &tips)),
        };
        draw_handles(painter, tool, camera, origin, axes, len, o, &tips, hovered, to_eye);

        if self.drag.is_none()
            && response.drag_started_by(PointerButton::Primary)
            && let Some(handle) = hovered
            && let Some(e) = state.scene.get(sel)
        {
            let mouse = ui.input(|i| i.pointer.press_origin()).unwrap_or(o);
            let ray = camera.ray(mouse);
            let parent = e.parent.and_then(|p| worlds.get(&p).copied()).unwrap_or(Mat4::IDENTITY);
            let (param, hit) = match handle {
                Handle::Axis(i) => (ray.closest_on_line(origin, axes[i]).unwrap_or(0.0), origin),
                Handle::Plane(i) => (0.0, ray.plane(origin, axes[i]).map(|t| ray.at(t)).unwrap_or(origin)),
                Handle::Free => (0.0, ray.plane(origin, camera.forward).map(|t| ray.at(t)).unwrap_or(origin)),
                _ => (0.0, origin),
            };
            let axis_screen = match handle {
                Handle::Scale(i) => tips[i].map(|t| t - o).unwrap_or(vec2(1.0, 0.0)),
                _ => vec2(1.0, 0.0),
            };
            self.drag = Some(Drag {
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

    // --- overlay -------------------------------------------------------------

    fn overlay(&mut self, ui: &mut egui::Ui, rect: egui::Rect, camera: &ViewCamera, state: &mut EditorState, grid_step: f32) {
        let bar_rect = rect.shrink(6.0);
        let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(bar_rect));
        egui::Frame::new()
            .fill(Color32::from_black_alpha(110))
            .corner_radius(5.0)
            .inner_margin(egui::Margin::symmetric(6, 3))
            .show(&mut bar, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (id, label, key) in [
                        (tools::VIEW, "Hand", "Q"),
                        (tools::MOVE, "Move", "W"),
                        (tools::ROTATE, "Rotate", "E"),
                        (tools::SCALE, "Scale", "R"),
                    ] {
                        if ui.selectable_label(state.tool == id, label).on_hover_text(format!("{label} ({key})")).clicked() {
                            state.tool = id.to_owned();
                        }
                    }
                    let ext_tools: Vec<_> = state.ext.viewport.iter().flat_map(|e| e.tools().iter().copied()).collect();
                    if !ext_tools.is_empty() {
                        ui.separator();
                    }
                    for t in ext_tools {
                        let hint = match t.key {
                            Some(k) => format!("{} ({})", t.tooltip, k.name()),
                            None => t.tooltip.to_owned(),
                        };
                        if ui.selectable_label(state.tool == t.id, t.label).on_hover_text(hint).clicked() {
                            state.tool = t.id.to_owned();
                        }
                    }
                    ui.separator();
                    let space = if state.local_space { "Local" } else { "Global" };
                    if ui.button(space).on_hover_text("Gizmo axes (X)").clicked() {
                        state.local_space = !state.local_space;
                    }
                    ui.separator();
                    ui.toggle_value(&mut state.snap.enabled, "Snap").on_hover_text("Grid snapping (hold Ctrl to invert)");
                    let step_label = |s: f32| if s < 1.0 { format!("1/{}", (1.0 / s).round()) } else { format!("{s}") };
                    egui::ComboBox::from_id_salt(("snap-step", self.id))
                        .width(78.0)
                        .selected_text(match state.snap.step {
                            None => format!("Auto {}", step_label(grid_step)),
                            Some(s) => step_label(s),
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut state.snap.step, None, "Auto (adaptive)");
                            for s in STEPS {
                                ui.selectable_value(&mut state.snap.step, Some(s), step_label(s));
                            }
                        });
                    egui::ComboBox::from_id_salt(("snap-angle", self.id))
                        .width(48.0)
                        .selected_text(format!("{}°", state.snap.angle_deg))
                        .show_ui(ui, |ui| {
                            for a in [1.0, 5.0, 15.0, 30.0, 45.0, 90.0] {
                                ui.selectable_value(&mut state.snap.angle_deg, a, format!("{a}°"));
                            }
                        });
                    ui.separator();
                    ui.toggle_value(&mut self.show_grid, "Grid");
                    ui.toggle_value(&mut self.wireframe, "Wire");
                    if ui.button("Frame").on_hover_text("Frame selection (F)").clicked() {
                        state.frame_selection = true;
                    }
                });
            });

        let painter = ui.painter_at(rect);
        // Axis indicator, back to front.
        let center = rect.right_bottom() + vec2(-44.0, -44.0);
        let mut axes: Vec<(usize, Vec3)> =
            AXES.iter().enumerate().map(|(i, a)| (i, camera.view.transform_vector3(*a))).collect();
        axes.sort_by(|a, b| a.1.z.total_cmp(&b.1.z));
        painter.circle_filled(center, 34.0, Color32::from_black_alpha(50));
        for (i, d) in axes {
            let tip = center + vec2(d.x, -d.y) * 26.0;
            painter.line_segment([center, tip], (2.0, theme::AXES[i]));
            painter.circle_filled(tip, 8.0, theme::AXES[i]);
            painter.text(tip, Align2::CENTER_CENTER, ["X", "Y", "Z"][i], FontId::proportional(11.0), Color32::BLACK);
        }
        if !state.preview.is_empty() {
            painter.text(rect.center_top() + vec2(0.0, 44.0), Align2::CENTER_CENTER, "• animation preview", FontId::proportional(13.0), theme::SELECTED);
        }
        painter.text(
            rect.left_bottom() + vec2(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            "MMB orbit · Shift+MMB pan · RMB look + WASD/QE fly · wheel zoom · F frame",
            FontId::proportional(11.0),
            Color32::from_white_alpha(90),
        );
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
    let near_axis = |i: usize| tips[i].filter(|t| t.distance(o) > 6.0).map(|t| dist_to_segment(p, o, t));
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
    candidates.filter(|(_, d)| *d < 7.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(h, _)| h)
}

fn plane_quad(camera: &ViewCamera, origin: Vec3, axes: [Vec3; 3], len: f32, normal: usize) -> Option<[Pos2; 4]> {
    let (j, k) = ((normal + 1) % 3, (normal + 2) % 3);
    let (a, b) = (0.18 * len, 0.42 * len);
    let pts = [(a, a), (b, a), (b, b), (a, b)].map(|(u, v)| camera.project(origin + axes[j] * u + axes[k] * v));
    Some([pts[0]?, pts[1]?, pts[2]?, pts[3]?])
}

fn view_ring_radius(camera: &ViewCamera, origin: Vec3, len: f32, o: Pos2) -> f32 {
    let right = camera.view.row(0).truncate();
    camera.project(origin + right * len).map(|p| p.distance(o) * 1.18).unwrap_or(100.0)
}

/// Ring around `axis`; the flag marks the half facing the camera.
fn ring(camera: &ViewCamera, origin: Vec3, axis: Vec3, len: f32, eye_dir: Vec3) -> Vec<(Pos2, bool)> {
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
                    painter.add(egui::Shape::convex_polygon(quad.to_vec(), c.gamma_multiply(0.35), Stroke::new(1.0, c)));
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
            painter.rect_stroke(egui::Rect::from_center_size(o, vec2(10.0, 10.0)), 1.0, (1.5, c), egui::StrokeKind::Middle);
        }
        tools::ROTATE => {
            for (i, axis) in axes.iter().enumerate() {
                let c = color(Handle::Rotate(i), theme::AXES[i]);
                let pts = ring(camera, origin, *axis, len, eye_dir);
                for w in pts.windows(2) {
                    let front = w[0].1 && w[1].1;
                    let stroke = if front { Stroke::new(2.0, c) } else { Stroke::new(1.0, c.gamma_multiply(0.25)) };
                    painter.line_segment([w[0].0, w[1].0], stroke);
                }
            }
            let r = view_ring_radius(camera, origin, len, o);
            painter.circle_stroke(o, r, (1.5, color(Handle::RotateView, Color32::from_gray(210))));
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
    let Some(e) = state.scene.get_mut(d.entity) else { return };

    match d.handle {
        Handle::Axis(i) => {
            let Some(t) = ray.closest_on_line(d.origin, d.axes[i]) else { return };
            let mut delta = t - d.param;
            if !world_axes_aligned {
                delta = snap.value(delta);
            }
            let mut p = d.origin + d.axes[i] * delta;
            if world_axes_aligned {
                p = snap_world(p, &[i]);
            }
            e.position = to_local.transform_point3(p);
        }
        Handle::Plane(_) | Handle::Free => {
            let normal = match d.handle {
                Handle::Plane(i) => d.axes[i],
                _ => camera.forward,
            };
            let Some(t) = ray.plane(d.origin, normal) else { return };
            let delta = ray.at(t) - d.hit;
            let p = match d.handle {
                Handle::Plane(i) if world_axes_aligned => snap_world(d.origin + delta, &[(i + 1) % 3, (i + 2) % 3]),
                Handle::Plane(i) => {
                    let (j, k) = ((i + 1) % 3, (i + 2) % 3);
                    d.origin + d.axes[j] * snap.value(delta.dot(d.axes[j])) + d.axes[k] * snap.value(delta.dot(d.axes[k]))
                }
                _ => snap_world(d.origin + delta, &[0, 1, 2]),
            };
            e.position = to_local.transform_point3(p);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::Ray;

    #[test]
    fn cube_faces_point_outward() {
        for tri in &CUBE.tris {
            let center = (tri[0] + tri[1] + tri[2]) / 3.0;
            assert!((tri[1] - tri[0]).cross(tri[2] - tri[0]).dot(center) > 0.0);
        }
        assert_eq!(CUBE.edges.len(), 12);
    }

    #[test]
    fn rays_hit_triangles_and_axes() {
        let ray = Ray { origin: Vec3::new(0.2, 5.0, 0.2), dir: Vec3::NEG_Y };
        let t = ray.triangle(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
        assert!((t - 5.0).abs() < 1e-5);
        // A ray crossing the X axis line at x = 3.
        let ray = Ray { origin: Vec3::new(3.0, 1.0, 5.0), dir: Vec3::new(0.0, -0.2, -1.0).normalize() };
        let along = ray.closest_on_line(Vec3::ZERO, Vec3::X).unwrap();
        assert!((along - 3.0).abs() < 1e-4);
    }

    #[test]
    fn camera_ray_roundtrips_projection() {
        let v = Viewport::default();
        let cam = v.camera(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let p = Vec3::new(1.0, 0.5, -2.0);
        let s = cam.project(p).unwrap();
        let ray = cam.ray(s);
        let closest = ray.origin + ray.dir * (p - ray.origin).dot(ray.dir);
        assert!(closest.distance(p) < 1e-3);
    }
}

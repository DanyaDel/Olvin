//! Scene view. Rendered on the GPU inside egui's render pass (depth buffer,
//! MSAA), with an infinite adaptive grid, raycast picking and Unity-style
//! transform tools. Elements extend it through `ViewportExtension`.

use std::f32::consts::{FRAC_PI_2, PI};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Align2, Color32, FontId, PointerButton, Pos2, Sense, Stroke, Vec2, vec2};
use glam::camera::rh::{proj::directx, view};
use glam::{EulerRot, Mat4, Quat, Vec3};
use olvin_scene::file::{ColliderShape, EntityDesc, LightKind, Shape};
use olvin_scene::primitives::Primitive;

use crate::ext::{Geometry, Snap, ViewCamera, ViewportCtx, flags, linear_color};
use crate::render::{Frame, Uniforms};
use crate::state::tools;
use crate::{EditorState, Panel, theme};

const NEAR: f32 = 0.05;
const FAR: f32 = 4000.0;
const LIGHT: Vec3 = Vec3::new(-0.45, -1.0, -0.3);
const GIZMO_POINTS: f32 = 95.0;
const HOT: Color32 = Color32::from_rgb(255, 225, 70);
const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
const STEPS: [f32; 11] = [
    1.0 / 16.0,
    0.125,
    0.25,
    0.5,
    1.0,
    2.0,
    4.0,
    8.0,
    16.0,
    32.0,
    64.0,
];

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

static PRIMITIVES: LazyLock<Vec<Primitive>> =
    LazyLock::new(|| Shape::ALL.iter().map(|s| Primitive::new(*s)).collect());

fn primitive(shape: Shape) -> &'static Primitive {
    &PRIMITIVES[Shape::ALL.iter().position(|s| *s == shape).unwrap_or(0)]
}

const COLLIDER: Color32 = Color32::from_rgb(145, 244, 139);
const LIGHT_GIZMO: Color32 = Color32::from_rgb(255, 236, 140);
const AUDIO_GIZMO: Color32 = Color32::from_rgb(120, 180, 255);

struct Orbit {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Orbit {
    fn back(&self) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        )
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
    /// Other selected objects moved along: (id, world start, parent inverse).
    others: Vec<(u64, Vec3, Mat4)>,
}

pub struct Viewport {
    id: u64,
    orbit: Orbit,
    orthographic: bool,
    /// Fly speed multiplier (scroll while holding RMB, like Unity).
    fly_speed: f32,
    show_grid: bool,
    wireframe: bool,
    shading: crate::engine::Shading,
    drag: Option<Drag>,
    /// Dragging a light's range or cone handle.
    light_drag: Option<(u64, LightHandle)>,
    /// Bottom of the toolbar last frame: tool overlays start below it.
    toolbar_bottom: f32,
    /// Copy of the editor preferences for this frame (camera feel, FOV).
    prefs: crate::prefs::Preferences,
    /// Shading was set from the preferences (first frame) or by the user.
    shading_set: bool,
    /// Last frame's geometry: raycasts and the GPU upload.
    geometry: Geometry,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            orbit: Orbit {
                target: Vec3::ZERO,
                yaw: 0.6,
                pitch: 0.45,
                distance: 14.0,
            },
            orthographic: false,
            fly_speed: 1.0,
            show_grid: true,
            wireframe: false,
            shading: crate::engine::Shading::default(),
            drag: None,
            light_drag: None,
            toolbar_bottom: 0.0,
            prefs: crate::prefs::Preferences::default(),
            shading_set: false,
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
        self.prefs = state.prefs.clone();
        // The view is always dark, so its overlays keep the Dark theme's
        // widgets whatever the chrome uses.
        *ui.visuals_mut() = olvin_ui::visuals(&theme::Palette::DARK);
        if !self.shading_set {
            self.shading_set = true;
            if self.prefs.solid_by_default {
                self.shading = crate::engine::Shading::Solid;
            }
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
                overlay_top: self.toolbar_bottom.max(rect.top() + 40.0),
            };
            consumed |= ext.interact(&mut ctx);
        }
        let transform_tool = [tools::MOVE, tools::ROTATE, tools::SCALE].contains(&tool.as_str());
        let mut gizmo_hot = false;
        if !consumed && transform_tool {
            gizmo_hot = self.light_handles(ui, &response, &painter, &camera, snap, state, &worlds);
        }
        if gizmo_hot {
            self.drag = None;
        } else if !consumed && transform_tool {
            gizmo_hot = self.gizmo(ui, &response, &painter, &camera, snap, state, &worlds);
        } else {
            self.drag = None;
        }
        if !consumed
            && !gizmo_hot
            && response.clicked_by(PointerButton::Primary)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let hit = self.pick(pointer, &camera, state, &worlds);
            match (hit, ui.input(|i| i.modifiers.command)) {
                (Some(id), true) => state.toggle_selected(id),
                (hit, _) => state.select(hit),
            }
        }

        // The engine draws meshes (the edited scene, or the game while playing).
        let frame_nr = ui.ctx().cumulative_frame_nr();
        let engine = crate::engine::prepare_view(state, self.id, frame_nr, self.shading, |_| {
            Some(camera.view_proj)
        });

        // Editor geometry for this frame, after edits.
        let worlds = state.world_matrices();
        self.geometry.clear();
        self.geometry.wireframe = self.wireframe;
        self.scene_geometry(state, &worlds, engine.is_some());
        if !state.is_playing() {
            // While playing, element geometry (e.g. CSG) is part of the game world.
            for ext in &mut extensions {
                ext.geometry(state, &mut self.geometry);
            }
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
                light: state
                    .scene
                    .sun()
                    .map(|(d, _, _)| d)
                    .unwrap_or(LIGHT)
                    .normalize()
                    .extend(0.0)
                    .to_array(),
                grid: [snap.step, 8.0, self.fade_distance(), 0.0],
            },
            vertices: self.geometry.vertices.clone(),
            lines: self.geometry.lines.clone(),
            overlay: self.geometry.overlay_lines.clone(),
            grid: self.show_grid,
            engine,
        };
        painter.set(
            scene_slot,
            eframe::egui_wgpu::Callback::new_paint_callback(rect, frame),
        );

        self.overlay(ui, rect, &camera, state, grid_step);
    }
}

impl Viewport {
    fn eye_height(&self) -> f32 {
        self.orbit
            .eye()
            .y
            .abs()
            .max(self.orbit.distance * 0.3)
            .max(0.05)
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
        // Pitch is clamped short of ±90°, so Y stays a valid up vector.
        let view = view::look_at_mat4(eye, self.orbit.target, Vec3::Y);
        let aspect = rect.width() / rect.height();
        let proj = if self.orthographic {
            let h = self.orbit.distance * (self.prefs.fov_deg.to_radians() * 0.5).tan();
            directx::orthographic(-h * aspect, h * aspect, -h, h, NEAR, FAR)
        } else {
            directx::perspective(self.prefs.fov_deg.to_radians(), aspect, NEAR, FAR)
        };
        ViewCamera {
            view,
            proj,
            view_proj: proj * view,
            eye,
            forward: -self.orbit.back(),
            rect,
            near: NEAR,
            orthographic: self.orthographic,
        }
    }

    fn hotkeys(&mut self, ui: &egui::Ui, response: &egui::Response, state: &mut EditorState) {
        let flying = ui.input(|i| i.pointer.button_down(PointerButton::Secondary));
        if !response.hovered()
            || flying
            || ui.ctx().egui_wants_keyboard_input()
            || self.drag.is_some()
            // A tool such as Edit Mode uses plain keys (E, R, X, F...) itself.
            || state.keys_owned_last_frame
        {
            return;
        }
        let pressed = |k| ui.input(|i| i.key_pressed(k) && i.modifiers.is_none());
        if ui.input(|i| {
            i.key_pressed(egui::Key::Z) && i.modifiers.matches_exact(egui::Modifiers::SHIFT)
        }) {
            use crate::engine::Shading;
            self.shading = if self.shading == Shading::Solid {
                Shading::Rendered
            } else {
                Shading::Solid
            };
        }
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
    }

    /// Unity scene-view controls: MMB pan, Alt+LMB orbit, Alt+RMB zoom,
    /// RMB look + WASD/QE fly (scroll changes speed), wheel zoom, F frame.
    fn navigate(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        state: &mut EditorState,
        worlds: &std::collections::HashMap<u64, Mat4>,
    ) {
        let delta = response.drag_delta();
        let alt = ui.input(|i| i.modifiers.alt);
        let middle = response.dragged_by(PointerButton::Middle);
        let right = response.dragged_by(PointerButton::Secondary);
        let left = response.dragged_by(PointerButton::Primary);
        let hand = left && state.tool == tools::VIEW && !alt;

        if middle || hand {
            let view = view::look_at_mat4(self.orbit.eye(), self.orbit.target, Vec3::Y);
            let (r, u) = (view.row(0).truncate(), view.row(1).truncate());
            let scale = self.orbit.distance * 0.0016;
            self.orbit.target += (-r * delta.x + u * delta.y) * scale;
        } else if left && alt {
            let (k, dy) = self.look(delta);
            self.orbit.yaw -= delta.x * 0.008 * k;
            self.orbit.pitch = (self.orbit.pitch + dy * 0.008 * k).clamp(-1.55, 1.55);
        } else if right && alt {
            self.zoom((delta.x - delta.y) * 0.01);
        } else if right {
            let eye = self.orbit.eye();
            let (k, dy) = self.look(delta);
            self.orbit.yaw -= delta.x * 0.005 * k;
            self.orbit.pitch = (self.orbit.pitch + dy * 0.005 * k).clamp(-1.55, 1.55);
            self.orbit.target = eye - self.orbit.back() * self.orbit.distance;
        }

        let flying = ui.input(|i| i.pointer.button_down(PointerButton::Secondary)) && !alt;
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if flying && (response.hovered() || right) {
            let (dt, boost) = ui.input(|i| {
                (
                    i.stable_dt.min(0.1),
                    if i.modifiers.shift { 4.0 } else { 1.0 },
                )
            });
            if scroll != 0.0 {
                self.fly_speed = (self.fly_speed * (scroll * 0.004).exp()).clamp(0.05, 50.0);
            }
            let forward = -self.orbit.back();
            let right_v = forward.cross(Vec3::Y).normalize_or_zero();
            let axis = |a: egui::Key, b: egui::Key| {
                ui.input(|i| i.key_down(a) as i32 as f32 - i.key_down(b) as i32 as f32)
            };
            let mv = forward * axis(egui::Key::W, egui::Key::S)
                + right_v * axis(egui::Key::D, egui::Key::A)
                + Vec3::Y * axis(egui::Key::E, egui::Key::Q);
            if mv != Vec3::ZERO {
                self.orbit.target +=
                    mv.normalize() * self.prefs.fly_speed * self.fly_speed * boost * dt;
                ui.ctx().request_repaint();
            }
        } else if scroll != 0.0 && response.hovered() {
            self.zoom(-scroll * 0.0015 * self.prefs.zoom_speed);
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

    /// Look sensitivity and the (optionally inverted) vertical delta.
    fn look(&self, delta: Vec2) -> (f32, f32) {
        let dy = if self.prefs.invert_y {
            -delta.y
        } else {
            delta.y
        };
        (self.prefs.look_sensitivity, dy)
    }

    fn zoom(&mut self, amount: f32) {
        self.orbit.distance = (self.orbit.distance * amount.exp()).clamp(0.1, FAR * 0.5);
    }

    /// `engine_meshes`: the engine renderer draws meshes; only make them pickable.
    fn scene_geometry(
        &mut self,
        state: &EditorState,
        worlds: &std::collections::HashMap<u64, Mat4>,
        engine_meshes: bool,
    ) {
        for e in &state.scene.entities {
            let Some(m) = worlds.get(&e.id) else { continue };
            if !state.scene.active_in_hierarchy(e.id) {
                continue;
            }
            if let Some(poly) = e.poly.as_ref().filter(|_| e.brush.is_none()) {
                let (pos, nrm, idx) = poly.triangulate();
                let color = linear_color(e.mesh.as_ref().map_or([0.8; 4], |m| m.color));
                for t in idx.as_chunks::<3>().0 {
                    let p = t.map(|i| m.transform_point3(pos[i as usize]));
                    if engine_meshes {
                        self.geometry.pick_triangle(e.id, p);
                    } else {
                        let n = (m.transform_vector3(nrm[t[0] as usize])).normalize_or_zero();
                        self.geometry.triangle(e.id, p, [n; 3], color, 0);
                    }
                }
            } else if let Some(mesh) = &e.mesh {
                let prim = primitive(mesh.shape);
                let normal_matrix = glam::Mat3::from_mat4(*m).inverse().transpose();
                let color = linear_color(mesh.color);
                let f = if state.selection == Some(e.id) {
                    flags::SELECTED
                } else {
                    0
                };
                for t in prim.indices.as_chunks::<3>().0 {
                    let idx = [t[0], t[1], t[2]].map(|i| i as usize);
                    let pos = idx.map(|i| m.transform_point3(prim.positions[i]));
                    if engine_meshes {
                        self.geometry.pick_triangle(e.id, pos);
                    } else {
                        let nrm =
                            idx.map(|i| (normal_matrix * prim.normals[i]).normalize_or_zero());
                        self.geometry.triangle(e.id, pos, nrm, color, f);
                    }
                }
            }
            if e.camera.is_some() {
                let (_, rot, pos) = m.to_scale_rotation_translation();
                let cm = Mat4::from_rotation_translation(rot, pos);
                let color = if state.selection == Some(e.id) {
                    theme::SELECTED
                } else {
                    Color32::from_gray(220)
                };
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
            if state.selection == Some(e.id) {
                component_gizmos(&mut self.geometry, e, *m);
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
        for sel in state.selected_ids() {
            let (Some(e), Some(m)) = (state.scene.get(sel), worlds.get(&sel)) else {
                continue;
            };
            let color = if Some(sel) == state.selection {
                theme::SELECTED
            } else {
                theme::SELECTED.gamma_multiply(0.6)
            };
            if let Some(poly) = e.poly.as_ref().filter(|_| e.brush.is_none()) {
                for [a, b] in poly.edges() {
                    let (a, b) = (poly.positions[a as usize], poly.positions[b as usize]);
                    camera.line(
                        painter,
                        m.transform_point3(a),
                        m.transform_point3(b),
                        (1.5, color),
                    );
                }
            } else if let Some(mesh) = &e.mesh {
                for [a, b] in &primitive(mesh.shape).outline {
                    camera.line(
                        painter,
                        m.transform_point3(*a),
                        m.transform_point3(*b),
                        (1.5, color),
                    );
                }
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
        // Cameras, lights and sounds have no surface: pick them by their icon.
        let icon = state
            .scene
            .entities
            .iter()
            .filter(|e| has_icon(e) && state.scene.active_in_hierarchy(e.id))
            .filter_map(|e| {
                Some((
                    e.id,
                    camera
                        .project(worlds.get(&e.id)?.w_axis.truncate())?
                        .distance(pointer),
                ))
            })
            .filter(|(_, d)| *d < 16.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, _)) = icon {
            return Some(id);
        }
        self.geometry
            .raycast(&camera.ray(pointer))
            .and_then(|h| h.entity)
    }

    // --- light handles -----------------------------------------------------------

    /// Unity's light handles on the selected light: dots on the range sphere
    /// (point), the cone tip and rim (spot). Returns true while one is hot.
    #[allow(clippy::too_many_arguments)]
    fn light_handles(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        painter: &egui::Painter,
        camera: &ViewCamera,
        snap: Snap,
        state: &mut EditorState,
        worlds: &std::collections::HashMap<u64, Mat4>,
    ) -> bool {
        let Some(id) = state.selection else {
            self.light_drag = None;
            return false;
        };
        let (Some(light), Some(m)) = (
            state.scene.get(id).and_then(|e| e.light.clone()),
            worlds.get(&id),
        ) else {
            self.light_drag = None;
            return false;
        };
        let handles = light_handle_points(&light, *m, camera);
        if handles.is_empty() {
            return false;
        }
        let pointer = response.hover_pos();
        let hot = self.light_drag.map(|(_, h)| h).or_else(|| {
            let p = pointer?;
            handles
                .iter()
                .filter_map(|(h, w)| Some((*h, camera.project(*w)?.distance(p))))
                .filter(|(_, d)| *d < 9.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(h, _)| h)
        });
        for (h, w) in &handles {
            if let Some(s) = camera.project(*w) {
                let on = hot == Some(*h);
                painter.circle(
                    s,
                    if on { 6.0 } else { 4.5 },
                    if on { Color32::WHITE } else { LIGHT_GIZMO },
                    (1.0, Color32::from_gray(30)),
                );
            }
        }
        if response.drag_started_by(PointerButton::Primary)
            && let Some(h) = hot
        {
            self.light_drag = Some((id, h));
        }
        if let Some((id, h)) = self.light_drag {
            if !ui.input(|i| i.pointer.primary_down()) {
                self.light_drag = None;
                return true;
            }
            if let (Some(p), Some(e)) = (ui.ctx().pointer_hover_pos(), state.scene.get_mut(id))
                && let Some(l) = &mut e.light
            {
                drag_light_handle(l, h, *m, camera, camera.ray(p), snap);
                let text = match h {
                    LightHandle::Angle => format!("angle {:.1}°", l.spot_angle),
                    LightHandle::Range => format!("range {:.2}", l.range),
                };
                painter.text(
                    p + vec2(14.0, -14.0),
                    Align2::LEFT_BOTTOM,
                    text,
                    FontId::monospace(12.0),
                    Color32::WHITE,
                );
            }
            return true;
        }
        hot.is_some()
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

    // --- overlay -------------------------------------------------------------

    /// Unity's scene gizmo: click an axis to look along it, the centre toggles
    /// perspective / orthographic.
    fn view_gizmo(&mut self, ui: &mut egui::Ui, rect: egui::Rect, camera: &ViewCamera) {
        let center = rect.right_top() + vec2(-52.0, 58.0);
        let area = egui::Rect::from_center_size(center, vec2(96.0, 96.0));
        let response = ui.interact(area, egui::Id::new(("view-gizmo", self.id)), Sense::click());
        let pointer = response.hover_pos();
        let painter = ui.painter_at(rect);
        painter.circle_filled(
            center,
            40.0,
            Color32::from_black_alpha(if response.hovered() { 70 } else { 40 }),
        );

        // (direction, axis index, positive); camera ends up on that side looking at the target.
        let mut tips: Vec<(Vec3, usize, bool, Vec3)> = Vec::new();
        for (i, axis) in AXES.iter().enumerate() {
            for positive in [true, false] {
                let dir = if positive { *axis } else { -*axis };
                tips.push((dir, i, positive, camera.view.transform_vector3(dir)));
            }
        }
        tips.sort_by(|a, b| a.3.z.total_cmp(&b.3.z));
        let mut clicked_axis = None;
        for (dir, i, positive, v) in &tips {
            let tip = center + vec2(v.x, -v.y) * 28.0;
            let hot = pointer.is_some_and(|p| p.distance(tip) < 9.0);
            let color = if hot { HOT } else { theme::AXES[*i] };
            if *positive {
                painter.line_segment([center, tip], (2.0, color));
                painter.circle_filled(tip, 8.5, color);
                painter.text(
                    tip,
                    Align2::CENTER_CENTER,
                    ["X", "Y", "Z"][*i],
                    FontId::proportional(11.0),
                    Color32::BLACK,
                );
            } else {
                painter.circle(tip, 6.0, Color32::from_gray(60), (1.5, color));
            }
            if hot && response.clicked() {
                clicked_axis = Some(*dir);
            }
        }
        let center_hot = pointer.is_some_and(|p| p.distance(center) < 8.0);
        painter.circle_filled(
            center,
            6.0,
            if center_hot {
                HOT
            } else {
                Color32::from_gray(200)
            },
        );
        painter.text(
            center + vec2(0.0, 50.0),
            Align2::CENTER_CENTER,
            if self.orthographic {
                "◀ Iso"
            } else {
                "◀ Persp"
            },
            FontId::proportional(11.0),
            Color32::from_white_alpha(170),
        );

        if let Some(dir) = clicked_axis {
            (self.orbit.yaw, self.orbit.pitch) = match dir {
                d if d == Vec3::X => (FRAC_PI_2, 0.0),
                d if d == -Vec3::X => (-FRAC_PI_2, 0.0),
                d if d == Vec3::Y => (0.0, 1.55),
                d if d == -Vec3::Y => (0.0, -1.55),
                d if d == Vec3::Z => (0.0, 0.0),
                _ => (PI, 0.0),
            };
        } else if response.clicked() && center_hot {
            self.orthographic = !self.orthographic;
        }
    }

    fn overlay(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        camera: &ViewCamera,
        state: &mut EditorState,
        grid_step: f32,
    ) {
        let bar_rect = rect.shrink(6.0);
        let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(bar_rect));
        self.toolbar_bottom = egui::Frame::new()
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
                        if ui
                            .selectable_label(state.tool == id, label)
                            .on_hover_text(format!("{label} ({key})"))
                            .clicked()
                        {
                            state.tool = id.to_owned();
                        }
                    }
                    let ext_tools: Vec<_> = state
                        .ext
                        .viewport
                        .iter()
                        .flat_map(|e| e.tools().iter().copied())
                        .collect();
                    if !ext_tools.is_empty() {
                        ui.separator();
                    }
                    for t in ext_tools {
                        let hint = match t.key {
                            Some(k) => format!("{} ({})", t.tooltip, k.name()),
                            None => t.tooltip.to_owned(),
                        };
                        if ui
                            .selectable_label(state.tool == t.id, t.label)
                            .on_hover_text(hint)
                            .clicked()
                        {
                            state.tool = t.id.to_owned();
                        }
                    }
                    ui.separator();
                    let space = if state.local_space { "Local" } else { "Global" };
                    if ui.button(space).on_hover_text("Gizmo axes (X)").clicked() {
                        state.local_space = !state.local_space;
                    }
                    ui.separator();
                    ui.toggle_value(&mut state.snap.enabled, "Snap")
                        .on_hover_text("Grid snapping (hold Ctrl to invert)");
                    let step_label = |s: f32| {
                        if s < 1.0 {
                            format!("1/{}", (1.0 / s).round())
                        } else {
                            format!("{s}")
                        }
                    };
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
                    ui.separator();
                    use crate::engine::Shading;
                    for (mode, label, tip) in [
                        (
                            Shading::Solid,
                            "◐ Solid",
                            "Studio light from the view, scene lights off (Shift+Z)",
                        ),
                        (
                            Shading::Rendered,
                            "☀ Rendered",
                            "Scene lights, shadows, environment and fog (Shift+Z)",
                        ),
                    ] {
                        ui.selectable_value(&mut self.shading, mode, label)
                            .on_hover_text(tip);
                    }
                    if ui
                        .button("Frame")
                        .on_hover_text("Frame selection (F)")
                        .clicked()
                    {
                        state.frame_selection = true;
                    }
                });
            })
            .response
            .rect
            .bottom();

        let painter = ui.painter_at(rect);
        draw_icons(&painter, camera, state);
        self.view_gizmo(ui, rect, camera);
        if self.prefs.show_stats
            && let Some(stats) = state.render_stats.get(&self.id)
        {
            let dt = ui.input(|i| i.stable_dt).max(1e-4);
            let text = format!(
                "{} draw calls · {} objects · {} culled · {:.1} ms ({:.0} fps)",
                stats.draw_calls,
                stats.instances,
                stats.culled,
                dt * 1000.0,
                1.0 / dt
            );
            let galley = painter.layout_no_wrap(
                text,
                FontId::monospace(11.5),
                Color32::from_white_alpha(220),
            );
            let at = rect.right_bottom() - vec2(galley.size().x + 14.0, galley.size().y + 12.0);
            painter.rect_filled(
                egui::Rect::from_min_size(at - vec2(6.0, 3.0), galley.size() + vec2(12.0, 6.0)),
                6.0,
                Color32::from_black_alpha(120),
            );
            painter.galley(at, galley, Color32::WHITE);
        }
        if ui.input(|i| i.pointer.button_down(PointerButton::Secondary)) && self.fly_speed != 1.0 {
            painter.text(
                rect.center_bottom() + vec2(0.0, -28.0),
                Align2::CENTER_CENTER,
                format!("fly speed ×{:.2}", self.fly_speed),
                FontId::proportional(12.0),
                Color32::from_white_alpha(170),
            );
        }
        if !state.preview.is_empty() {
            painter.text(
                rect.center_top() + vec2(0.0, 44.0),
                Align2::CENTER_CENTER,
                "• animation preview",
                FontId::proportional(13.0),
                theme::SELECTED,
            );
        }
        painter.text(
            rect.left_bottom() + vec2(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            "Alt+LMB orbit · MMB pan · RMB look + WASD/QE fly · Alt+RMB / wheel zoom · F frame",
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LightHandle {
    Range,
    Angle,
}

/// World positions of a light's handles.
fn light_handle_points(
    l: &olvin_scene::file::LightDesc,
    m: Mat4,
    camera: &ViewCamera,
) -> Vec<(LightHandle, Vec3)> {
    let pos = m.w_axis.truncate();
    let forward = -m.z_axis.truncate().normalize_or(Vec3::Z);
    match l.kind {
        LightKind::Directional => Vec::new(),
        LightKind::Point => {
            // Facing the camera, so they stay on the sphere's silhouette.
            let right = camera.forward.cross(Vec3::Y).normalize_or(Vec3::X);
            let up = right.cross(camera.forward).normalize_or(Vec3::Y);
            [right, -right, up, -up]
                .into_iter()
                .map(|d| (LightHandle::Range, pos + d * l.range))
                .collect()
        }
        LightKind::Spot => {
            let side = m.x_axis.truncate().normalize_or(Vec3::X);
            let up = m.y_axis.truncate().normalize_or(Vec3::Y);
            let r = spot_radius(l);
            let base = pos + forward * l.range;
            let mut out = vec![(LightHandle::Range, base)];
            out.extend(
                [side, -side, up, -up]
                    .into_iter()
                    .map(|d| (LightHandle::Angle, base + d * r)),
            );
            out
        }
    }
}

fn spot_radius(l: &olvin_scene::file::LightDesc) -> f32 {
    l.range * (l.spot_angle.to_radians() * 0.5).tan()
}

fn drag_light_handle(
    l: &mut olvin_scene::file::LightDesc,
    h: LightHandle,
    m: Mat4,
    camera: &ViewCamera,
    ray: crate::ext::Ray,
    snap: Snap,
) {
    let pos = m.w_axis.truncate();
    let forward = -m.z_axis.truncate().normalize_or(Vec3::Z);
    match (l.kind, h) {
        (LightKind::Point, _) => {
            if let Some(t) = ray.plane(pos, camera.forward) {
                l.range = snap.value(ray.at(t).distance(pos)).max(0.05);
            }
        }
        (LightKind::Spot, LightHandle::Range) => {
            if let Some(t) = ray.closest_on_line(pos, forward) {
                l.range = snap.value(t).max(0.05);
            }
        }
        (LightKind::Spot, LightHandle::Angle) => {
            let base = pos + forward * l.range;
            if let Some(t) = ray.plane(base, forward) {
                let r = ray.at(t).distance(base);
                let deg = (2.0 * (r / l.range.max(1e-3)).atan()).to_degrees();
                l.spot_angle = if snap.enabled {
                    (deg / snap.angle_deg).round() * snap.angle_deg
                } else {
                    deg
                }
                .clamp(1.0, 179.0);
            }
        }
        (LightKind::Directional, _) => {}
    }
}

fn has_icon(e: &EntityDesc) -> bool {
    e.camera.is_some() || e.light.is_some() || e.audio_source.is_some()
}

/// A light's icon: a disc in the light's colour with a glyph for its type,
/// plus a short ray showing where directional and spot lights point.
fn light_icon(
    painter: &egui::Painter,
    camera: &ViewCamera,
    p: Pos2,
    pos: Vec3,
    forward: Vec3,
    light: &olvin_scene::file::LightDesc,
    selected: bool,
) {
    let tint = linear_color([light.color[0], light.color[1], light.color[2], 1.0]);
    let ring = if selected {
        theme::SELECTED
    } else {
        Color32::from_gray(20)
    };
    let glyph = Color32::from_gray(25);
    if light.kind != LightKind::Point
        && let Some(tip) = camera.project(pos + forward * camera.world_per_point(pos) * 34.0)
    {
        let dir = (tip - p).normalized();
        painter.line_segment(
            [p + dir * 11.0, tip],
            (2.0, LIGHT_GIZMO.gamma_multiply(0.8)),
        );
        painter.circle_filled(tip, 2.5, LIGHT_GIZMO);
    }
    if light.kind == LightKind::Directional {
        for i in 0..8 {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            let d = vec2(a.cos(), a.sin());
            painter.line_segment([p + d * 12.0, p + d * 16.0], (2.0, tint));
        }
    }
    painter.circle(p, 9.5, tint, (if selected { 2.0 } else { 1.5 }, ring));
    match light.kind {
        LightKind::Directional => {
            painter.circle_filled(p, 3.5, glyph);
        }
        LightKind::Point => {
            painter.circle_stroke(p - vec2(0.0, 1.5), 3.5, (1.5, glyph));
            painter.line_segment([p + vec2(-2.0, 4.0), p + vec2(2.0, 4.0)], (1.5, glyph));
        }
        LightKind::Spot => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    p + vec2(-2.0, -4.5),
                    p + vec2(2.0, -4.5),
                    p + vec2(4.5, 3.5),
                    p + vec2(-4.5, 3.5),
                ],
                glyph,
                Stroke::NONE,
            ));
        }
    }
}

/// Unity-style icons for things without a surface.
fn draw_icons(painter: &egui::Painter, camera: &ViewCamera, state: &EditorState) {
    let worlds = state.scene.world_matrices();
    for e in &state.scene.entities {
        if !has_icon(e) || e.camera.is_some() || !state.scene.active_in_hierarchy(e.id) {
            continue;
        }
        let Some(p) = worlds
            .get(&e.id)
            .and_then(|m| camera.project(m.w_axis.truncate()))
        else {
            continue;
        };
        let selected = state.selection == Some(e.id);
        if let Some(light) = &e.light {
            let forward = worlds
                .get(&e.id)
                .map(|m| -m.z_axis.truncate().normalize_or_zero())
                .unwrap_or(Vec3::NEG_Z);
            let pos = worlds[&e.id].w_axis.truncate();
            light_icon(painter, camera, p, pos, forward, light, selected);
        } else if e.audio_source.is_some() {
            let color = if selected {
                theme::SELECTED
            } else {
                AUDIO_GIZMO
            };
            painter.rect_filled(
                egui::Rect::from_center_size(p - vec2(4.0, 0.0), vec2(5.0, 7.0)),
                1.0,
                color,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    p + vec2(-2.0, -3.5),
                    p + vec2(5.0, -8.0),
                    p + vec2(5.0, 8.0),
                    p + vec2(-2.0, 3.5),
                ],
                color,
                Stroke::NONE,
            ));
        }
    }
}

/// Gizmos of the selected object's components, in world space.
fn component_gizmos(g: &mut Geometry, e: &EntityDesc, m: Mat4) {
    let (scale, rot, pos) = m.to_scale_rotation_translation();
    let axes = [rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z];
    if let Some(c) = &e.collider {
        let center = m.transform_point3(c.center);
        match c.shape {
            ColliderShape::Box { size } => {
                let local = Mat4::from_translation(c.center) * Mat4::from_scale(size);
                for [a, b] in &primitive(Shape::Cube).outline {
                    g.line(
                        (m * local).transform_point3(*a),
                        (m * local).transform_point3(*b),
                        COLLIDER,
                        true,
                    );
                }
            }
            ColliderShape::Sphere { radius } => {
                let r = radius * scale.abs().max_element();
                for (u, v) in [(0, 1), (1, 2), (0, 2)] {
                    g.circle(center, axes[u], axes[v], r, COLLIDER, true);
                }
            }
            ColliderShape::Capsule { radius, height } => {
                let r = radius * scale.x.abs().max(scale.z.abs());
                let half = (height * scale.y.abs() * 0.5 - r).max(0.0);
                let up = axes[1];
                for end in [-1.0, 1.0] {
                    g.circle(
                        center + up * half * end,
                        axes[0],
                        axes[2],
                        r,
                        COLLIDER,
                        true,
                    );
                }
                for side in [axes[0], -axes[0], axes[2], -axes[2]] {
                    g.line(
                        center + side * r - up * half,
                        center + side * r + up * half,
                        COLLIDER,
                        true,
                    );
                }
                for side in [axes[0], axes[2]] {
                    // Cap arcs.
                    for end in [-1.0f32, 1.0] {
                        let c0 = center + up * half * end;
                        let pts: Vec<Vec3> = (0..=16)
                            .map(|i| {
                                let a = i as f32 / 16.0 * std::f32::consts::PI;
                                c0 + side * (r * a.cos()) + up * (r * a.sin() * end)
                            })
                            .collect();
                        for w in pts.windows(2) {
                            g.line(w[0], w[1], COLLIDER, true);
                        }
                    }
                }
            }
            ColliderShape::Mesh => {
                if let Some(mesh) = &e.mesh {
                    for [a, b] in &primitive(mesh.shape).outline {
                        g.line(
                            m.transform_point3(*a),
                            m.transform_point3(*b),
                            COLLIDER,
                            true,
                        );
                    }
                }
            }
        }
    }
    if let Some(cc) = &e.character_controller {
        let half = (cc.height * 0.5 - cc.radius).max(0.0);
        for end in [-1.0, 1.0] {
            g.circle(
                pos + axes[1] * half * end,
                axes[0],
                axes[2],
                cc.radius,
                COLLIDER,
                true,
            );
        }
        for side in [axes[0], -axes[0], axes[2], -axes[2]] {
            g.line(
                pos + side * cc.radius - axes[1] * half,
                pos + side * cc.radius + axes[1] * half,
                COLLIDER,
                true,
            );
        }
    }
    if let Some(l) = &e.light {
        let forward = -axes[2];
        match l.kind {
            LightKind::Directional => {
                for (dx, dy) in [(0.0, 0.0), (0.3, 0.0), (-0.3, 0.0), (0.0, 0.3), (0.0, -0.3)] {
                    let o = pos + axes[0] * dx + axes[1] * dy;
                    g.line(o, o + forward * 2.0, LIGHT_GIZMO, true);
                }
                g.circle(pos, axes[0], axes[1], 0.3, LIGHT_GIZMO, true);
            }
            LightKind::Point => {
                for (u, v) in [(Vec3::X, Vec3::Y), (Vec3::Y, Vec3::Z), (Vec3::X, Vec3::Z)] {
                    g.circle(pos, u, v, l.range, LIGHT_GIZMO, true);
                }
            }
            LightKind::Spot => {
                let r = l.range * (l.spot_angle.to_radians() * 0.5).tan();
                let base = pos + forward * l.range;
                g.circle(base, axes[0], axes[1], r, LIGHT_GIZMO, true);
                for d in [axes[0], -axes[0], axes[1], -axes[1]] {
                    g.line(pos, base + d * r, LIGHT_GIZMO, true);
                }
            }
        }
    }
    if let Some(a) = &e.audio_source
        && a.spatial
    {
        for r in [a.min_distance, a.max_distance] {
            g.circle(pos, Vec3::X, Vec3::Z, r, AUDIO_GIZMO, true);
            g.circle(
                pos,
                Vec3::X,
                Vec3::Y,
                r,
                AUDIO_GIZMO.gamma_multiply(0.5),
                true,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::Ray;

    #[test]
    fn rays_hit_triangles_and_axes() {
        let ray = Ray {
            origin: Vec3::new(0.2, 5.0, 0.2),
            dir: Vec3::NEG_Y,
        };
        let t = ray
            .triangle(
                Vec3::ZERO,
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 0.0),
            )
            .unwrap();
        assert!((t - 5.0).abs() < 1e-5);
        // A ray crossing the X axis line at x = 3.
        let ray = Ray {
            origin: Vec3::new(3.0, 1.0, 5.0),
            dir: Vec3::new(0.0, -0.2, -1.0).normalize(),
        };
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

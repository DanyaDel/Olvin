//! Scene view. Rendered on the GPU inside egui's render pass (depth buffer,
//! MSAA), with an infinite adaptive grid, raycast picking and Unity-style
//! transform tools. Elements extend it through `ViewportExtension`.

mod gizmo;
mod lights;
mod navigation;
mod overlay;
mod scene;

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Color32, PointerButton, Sense};
use glam::Vec3;
use olvin_scene::file::{EntityDesc, Shape};
use olvin_scene::primitives::Primitive;

use crate::ext::{Geometry, Snap, ViewportCtx};
use crate::panels::assets::{AssetDrag, DropTarget, drop_asset, dropped};
use crate::render::{Frame, Uniforms};
use crate::state::tools;
use crate::{EditorState, Panel, theme};
use gizmo::Drag;
use lights::LightHandle;

const LIGHT: Vec3 = Vec3::new(-0.45, -1.0, -0.3);
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

fn has_icon(e: &EntityDesc) -> bool {
    e.camera.is_some() || e.light.is_some() || e.audio_source.is_some()
}

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
            && !ui.input(|i| i.modifiers.alt)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let hit = self
                .pick(pointer, &camera, state, &worlds)
                .map(|id| state.click_target(id));
            match (hit, ui.input(|i| i.modifiers.command)) {
                (Some(id), true) => state.toggle_selected(id),
                (hit, _) => state.select(hit),
            }
        }

        // An asset dragged out of the Assets panel and released here.
        if response.dnd_hover_payload::<AssetDrag>().is_some() {
            painter.rect_stroke(
                rect.shrink(2.0),
                0.0,
                (2.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(asset) = dropped(&response)
            && let Some(pointer) = ui.ctx().pointer_latest_pos()
        {
            let ray = camera.ray(pointer);
            // Models land on the ground plane (the view's target when the
            // ray points at the sky).
            let at = ray
                .plane(Vec3::ZERO, Vec3::Y)
                .filter(|t| *t > 0.0)
                .map(|t| ray.at(t))
                .unwrap_or(self.orbit.target);
            let pick = self.pick(pointer, &camera, state, &worlds);
            drop_asset(state, &asset.0, DropTarget::Viewport { pick, at: Some(at) });
        }

        // A playing model preview animates every frame.
        if state.model_previews.values().any(|p| p.playing) {
            ui.ctx().request_repaint();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::Ray;
    use egui::{Pos2, vec2};

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

//! Unity's light handles: drag a point light's range or a spot light's
//! range and cone angle.

use egui::{Align2, Color32, FontId, PointerButton, vec2};
use glam::{Mat4, Vec3};
use olvin_scene::file::LightKind;

use super::{LIGHT_GIZMO, Viewport};
use crate::EditorState;
use crate::ext::{Snap, ViewCamera};

impl Viewport {
    /// Unity's light handles on the selected light: dots on the range sphere
    /// (point), the cone tip and rim (spot). Returns true while one is hot.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn light_handles(
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LightHandle {
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

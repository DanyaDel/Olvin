//! Scene geometry for the frame: meshes, camera frustums, component gizmos,
//! selection outlines and raycast picking.

use egui::{Color32, Pos2};
use glam::{Mat4, Vec3};
use olvin_scene::file::{ColliderShape, EntityDesc, LightKind, Shape};

use super::{AUDIO_GIZMO, LIGHT_GIZMO, Viewport, has_icon, primitive};
use crate::ext::{Geometry, ViewCamera, flags, linear_color};
use crate::{EditorState, theme};

const COLLIDER: Color32 = Color32::from_rgb(145, 244, 139);

impl Viewport {
    /// `engine_meshes`: the engine renderer draws meshes; only make them pickable.
    pub(super) fn scene_geometry(
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

    pub(super) fn selection_outline(
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

    pub(super) fn pick(
        &self,
        pointer: Pos2,
        camera: &ViewCamera,
        state: &mut EditorState,
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
        let ray = camera.ray(pointer);
        let hit = self
            .geometry
            .raycast(&ray)
            .and_then(|h| Some((h.distance, h.entity?)));
        // Terrains are drawn by the engine and use their heightfield for picking.
        let root = state.assets_dir();
        let scene = &state.scene;
        let store = &mut state.terrains;
        let terrain = scene
            .entities
            .iter()
            .filter(|e| scene.active_in_hierarchy(e.id))
            .filter_map(|e| {
                let desc = e.terrain.as_ref()?;
                let world = *worlds.get(&e.id)?;
                let data = &store.entry(root.as_deref(), &desc.data).data;
                let inv = world.inverse();
                let local = olvin::terrain::mesh::raycast(
                    data,
                    desc.size,
                    inv.transform_point3(ray.origin),
                    inv.transform_vector3(ray.dir),
                )?;
                Some((world.transform_point3(local).distance(ray.origin), e.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        [hit, terrain]
            .into_iter()
            .flatten()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
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

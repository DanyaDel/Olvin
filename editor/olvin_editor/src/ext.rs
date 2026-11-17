//! Extension points for editor elements: viewport geometry, overlays and
//! tools, inspector components, "create" menu items.

use bytemuck::{Pod, Zeroable};
use egui::{Color32, Pos2, Rect, Stroke};
use glam::{Mat4, Vec3, Vec4Swizzles};
use olvin_scene::file::EntityDesc;

use crate::EditorState;

/// World-space geometry submitted to a viewport for one frame.
#[derive(Default)]
pub struct Geometry {
    pub(crate) vertices: Vec<GpuVertex>,
    /// Owner entity per triangle, for picking.
    pub(crate) tri_entity: Vec<u64>,
    pub(crate) lines: Vec<LineVertex>,
    pub(crate) overlay_lines: Vec<LineVertex>,
    /// Draw meshes as edges only.
    pub wireframe: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GpuVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [u8; 4],
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct LineVertex {
    pub position: [f32; 3],
    pub color: [u8; 4],
}

/// Per-face rendering flags.
pub mod flags {
    /// World-space 1 m grid pattern (level geometry without textures).
    pub const GRID: u32 = 1;
    /// Highlight tint for the selection.
    pub const SELECTED: u32 = 2;
}

fn rgba8(c: Color32) -> [u8; 4] {
    c.to_array()
}

/// Linear RGBA (as stored in scenes) to the sRGB bytes the shaders expect.
pub fn linear_color(c: [f32; 4]) -> Color32 {
    egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]).into()
}

impl Geometry {
    /// A convex polygon (fan-triangulated); in wireframe mode only its edges.
    pub fn polygon(&mut self, entity: u64, vertices: &[Vec3], normal: Vec3, color: Color32, flags: u32) {
        if vertices.len() < 3 {
            return;
        }
        if self.wireframe {
            for i in 0..vertices.len() {
                self.line(vertices[i], vertices[(i + 1) % vertices.len()], Color32::from_gray(200), false);
            }
        }
        let v = |p: Vec3| GpuVertex { position: p.to_array(), normal: normal.to_array(), color: rgba8(color), flags };
        for i in 1..vertices.len() - 1 {
            self.vertices.extend([v(vertices[0]), v(vertices[i]), v(vertices[i + 1])]);
            self.tri_entity.push(entity);
        }
    }

    /// A triangle with per-vertex normals (smooth shading).
    pub fn triangle(&mut self, entity: u64, positions: [Vec3; 3], normals: [Vec3; 3], color: Color32, flags: u32) {
        if self.wireframe {
            for i in 0..3 {
                self.line(positions[i], positions[(i + 1) % 3], Color32::from_gray(200), false);
            }
        }
        for i in 0..3 {
            self.vertices.push(GpuVertex {
                position: positions[i].to_array(),
                normal: normals[i].to_array(),
                color: rgba8(color),
                flags,
            });
        }
        self.tri_entity.push(entity);
    }

    /// A circle of line segments around `center` in the plane spanned by `u`, `v`.
    pub fn circle(&mut self, center: Vec3, u: Vec3, v: Vec3, radius: f32, color: Color32, on_top: bool) {
        const N: usize = 48;
        let p = |i: usize| {
            let a = i as f32 / N as f32 * std::f32::consts::TAU;
            center + (u * a.cos() + v * a.sin()) * radius
        };
        for i in 0..N {
            self.line(p(i), p(i + 1), color, on_top);
        }
    }

    /// A line; `on_top` lines ignore depth (outlines of hidden things).
    pub fn line(&mut self, a: Vec3, b: Vec3, color: Color32, on_top: bool) {
        let target = if on_top { &mut self.overlay_lines } else { &mut self.lines };
        target.push(LineVertex { position: a.to_array(), color: rgba8(color) });
        target.push(LineVertex { position: b.to_array(), color: rgba8(color) });
    }

    pub(crate) fn clear(&mut self) {
        self.vertices.clear();
        self.tri_entity.clear();
        self.lines.clear();
        self.overlay_lines.clear();
    }

    /// Nearest triangle hit along the ray.
    pub fn raycast(&self, ray: &Ray) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        for (i, tri) in self.vertices.as_chunks::<3>().0.iter().enumerate() {
            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(tri[k].position));
            if let Some(t) = ray.triangle(a, b, c)
                && best.as_ref().is_none_or(|h| t < h.distance)
            {
                let mut normal = (b - a).cross(c - a).normalize_or_zero();
                if normal.dot(ray.dir) > 0.0 {
                    normal = -normal;
                }
                best = Some(Hit { point: ray.at(t), normal, distance: t, entity: Some(self.tri_entity[i]) });
            }
        }
        best
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }

    /// Distance to a plane, if hit in front.
    pub fn plane(&self, point: Vec3, normal: Vec3) -> Option<f32> {
        let denom = normal.dot(self.dir);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = normal.dot(point - self.origin) / denom;
        (t >= 0.0).then_some(t)
    }

    /// Möller–Trumbore, double-sided.
    pub fn triangle(&self, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
        let (e1, e2) = (b - a, c - a);
        let p = self.dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-9 {
            return None;
        }
        let inv = 1.0 / det;
        let s = self.origin - a;
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(e1);
        let v = self.dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = e2.dot(q) * inv;
        (t > 1e-5).then_some(t)
    }

    /// Parameter along the line (`point`, `dir`) closest to this ray.
    pub fn closest_on_line(&self, point: Vec3, dir: Vec3) -> Option<f32> {
        let w = self.origin - point;
        let b = dir.dot(self.dir);
        let denom = 1.0 - b * b;
        if denom.abs() < 1e-5 {
            return None;
        }
        Some((dir.dot(w) - b * self.dir.dot(w)) / denom)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub point: Vec3,
    /// Facing the ray origin.
    pub normal: Vec3,
    pub distance: f32,
    /// None for the ground grid.
    pub entity: Option<u64>,
}

/// The camera of the viewport being drawn.
#[derive(Clone, Copy, Debug)]
pub struct ViewCamera {
    pub view: Mat4,
    pub proj: Mat4,
    pub view_proj: Mat4,
    pub eye: Vec3,
    pub forward: Vec3,
    pub rect: Rect,
    pub near: f32,
    pub orthographic: bool,
}

impl ViewCamera {
    pub fn project(&self, p: Vec3) -> Option<Pos2> {
        let v = self.view.transform_point3(p);
        (v.z < -self.near).then(|| self.view_to_screen(v))
    }

    fn view_to_screen(&self, v: Vec3) -> Pos2 {
        let clip = self.proj * v.extend(1.0);
        let ndc = clip.xyz() / clip.w;
        egui::pos2(
            self.rect.left() + (ndc.x * 0.5 + 0.5) * self.rect.width(),
            self.rect.top() + (0.5 - ndc.y * 0.5) * self.rect.height(),
        )
    }

    pub fn ray(&self, screen: Pos2) -> Ray {
        let x = (screen.x - self.rect.left()) / self.rect.width() * 2.0 - 1.0;
        let y = 1.0 - (screen.y - self.rect.top()) / self.rect.height() * 2.0;
        let inv = self.view_proj.inverse();
        let near = inv.project_point3(Vec3::new(x, y, 0.0));
        let far = inv.project_point3(Vec3::new(x, y, 1.0));
        Ray { origin: near, dir: (far - near).normalize() }
    }

    /// World units covered by one screen point at `p`.
    pub fn world_per_point(&self, p: Vec3) -> f32 {
        let fy = self.proj.y_axis.y;
        if self.orthographic {
            return 2.0 / (fy * self.rect.height());
        }
        let depth = (-self.view.transform_point3(p).z).max(self.near);
        2.0 * depth / (fy * self.rect.height())
    }

    /// Draws a world-space segment with egui, clipped at the near plane.
    pub fn line(&self, painter: &egui::Painter, a: Vec3, b: Vec3, stroke: impl Into<Stroke>) {
        let (mut va, mut vb) = (self.view.transform_point3(a), self.view.transform_point3(b));
        let near = -self.near;
        if va.z >= near && vb.z >= near {
            return;
        }
        if va.z >= near {
            va = vb + (va - vb) * ((near - vb.z) / (va.z - vb.z));
        } else if vb.z >= near {
            vb = va + (vb - va) * ((near - va.z) / (vb.z - va.z));
        }
        painter.line_segment([self.view_to_screen(va), self.view_to_screen(vb)], stroke);
    }
}

/// Snapping for the current viewport (grid step follows the adaptive grid).
#[derive(Clone, Copy, Debug)]
pub struct Snap {
    pub enabled: bool,
    pub step: f32,
    pub angle_deg: f32,
}

impl Snap {
    pub fn value(&self, v: f32) -> f32 {
        if self.enabled { (v / self.step).round() * self.step } else { v }
    }

    pub fn vec(&self, v: Vec3) -> Vec3 {
        Vec3::new(self.value(v.x), self.value(v.y), self.value(v.z))
    }

    pub fn angle(&self, radians: f32) -> f32 {
        if self.enabled {
            let step = self.angle_deg.to_radians();
            (radians / step).round() * step
        } else {
            radians
        }
    }
}

/// A viewport tool contributed by an element (shown in the viewport toolbar).
#[derive(Clone, Copy, Debug)]
pub struct ToolDesc {
    pub id: &'static str,
    pub label: &'static str,
    pub tooltip: &'static str,
    pub key: Option<egui::Key>,
}

/// What a viewport extension gets each frame to draw overlays and handle input.
pub struct ViewportCtx<'a> {
    pub ui: &'a egui::Ui,
    pub response: &'a egui::Response,
    pub painter: &'a egui::Painter,
    pub camera: &'a ViewCamera,
    pub snap: Snap,
    pub state: &'a mut EditorState,
    /// Active tool id (`"move"`, `"csg.draw"`, ...).
    pub tool: &'a str,
    /// Last frame's geometry, for raycasts.
    pub geometry: &'a Geometry,
}

impl ViewportCtx<'_> {
    /// Scene surface under the pointer, falling back to the ground plane (y = 0).
    pub fn raycast(&self, screen: Pos2) -> Option<Hit> {
        let ray = self.camera.ray(screen);
        self.geometry.raycast(&ray).or_else(|| {
            let t = ray.plane(Vec3::ZERO, Vec3::Y)?;
            Some(Hit { point: ray.at(t), normal: Vec3::Y, distance: t, entity: None })
        })
    }
}

pub trait ViewportExtension {
    /// Adds world-space geometry for this frame. Called once per viewport.
    fn geometry(&mut self, state: &EditorState, out: &mut Geometry);

    /// Overlays and input. Return `true` to consume the pointer (no selection
    /// click, no gizmo) this frame.
    fn interact(&mut self, _ctx: &mut ViewportCtx) -> bool {
        false
    }

    fn tools(&self) -> &[ToolDesc] {
        &[]
    }
}

/// An inspector section for a component stored on [`EntityDesc`].
#[derive(Clone, Copy)]
pub struct ComponentEditor {
    pub title: &'static str,
    pub has: fn(&EntityDesc) -> bool,
    pub add: fn(&mut EntityDesc),
    pub remove: fn(&mut EntityDesc),
    pub ui: fn(&mut egui::Ui, &mut EntityDesc),
}

/// An entry of the "create" menus (hierarchy "+", GameObject menu).
#[derive(Clone, Copy)]
pub struct CreateItem {
    /// `"CSG/Box"` puts "Box" in a "CSG" submenu.
    pub path: &'static str,
    /// Returns the new entity id.
    pub create: fn(&mut EditorState, Option<u64>) -> u64,
}

#[derive(Default)]
pub(crate) struct Extensions {
    pub viewport: Vec<Box<dyn ViewportExtension>>,
    pub components: Vec<ComponentEditor>,
    pub create: Vec<CreateItem>,
}

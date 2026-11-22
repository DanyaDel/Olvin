//! CSG in the scene view: live mesh, brush outlines, the Draw and Edit tools.

use std::time::Instant;

use olvin_editor::egui::{self, Align2, Color32, FontId, Key, PointerButton, Pos2};
use olvin_editor::ext::{
    Geometry, Snap, ToolDesc, ViewCamera, ViewportCtx, ViewportExtension, flags, linear_color,
};
use olvin_editor::glam::{Mat4, Vec3};
use olvin_editor::olvin_scene::file::{BrushDesc, CsgOp};
use olvin_editor::{EditorState, NewEntity, theme};

use crate::{DRAW_TOOL, EDIT_TOOL, op_color, shared};

const TOOLS: [ToolDesc; 2] = [
    ToolDesc {
        id: DRAW_TOOL,
        label: "Draw Brush",
        tooltip: "Drag a rectangle on any surface, then set the height: up adds, down carves. Shift: subtract",
        key: Some(Key::B),
    },
    ToolDesc {
        id: EDIT_TOOL,
        label: "Resize",
        tooltip: "Drag the faces of the selected brush",
        key: Some(Key::T),
    },
];

/// An axis-aligned drawing plane picked from the surface under the cursor.
#[derive(Clone, Copy)]
struct DrawPlane {
    point: Vec3,
    normal: Vec3,
    axis: usize,
}

impl DrawPlane {
    fn from_hit(point: Vec3, normal: Vec3) -> Self {
        let a = normal.abs();
        let axis = if a.x >= a.y && a.x >= a.z {
            0
        } else if a.y >= a.z {
            1
        } else {
            2
        };
        let mut n = Vec3::ZERO;
        n[axis] = normal[axis].signum();
        Self {
            point,
            normal: n,
            axis,
        }
    }

    /// Snaps the in-plane coordinates, keeps the plane coordinate exact.
    fn snap(&self, p: Vec3, snap: &Snap) -> Vec3 {
        let mut s = snap.vec(p);
        s[self.axis] = self.point[self.axis];
        s
    }
}

enum Draw {
    Idle,
    Base {
        plane: DrawPlane,
        start: Vec3,
        end: Vec3,
    },
    Height {
        plane: DrawPlane,
        start: Vec3,
        end: Vec3,
        height: f32,
    },
}

struct FaceDrag {
    entity: u64,
    axis: usize,
    sign: f32,
    dir: Vec3,
    param: f32,
    face: Vec3,
    position: Vec3,
    scale: Vec3,
    extent: f32,
    parent_inv: Mat4,
}

pub struct CsgViewport {
    cache_key: Vec<olvin_csg::BrushInput>,
    polygons: Vec<olvin_csg::Polygon>,
    draw: Draw,
    face: Option<FaceDrag>,
}

impl Default for CsgViewport {
    fn default() -> Self {
        Self {
            cache_key: Vec::new(),
            polygons: Vec::new(),
            draw: Draw::Idle,
            face: None,
        }
    }
}

impl CsgViewport {
    fn rebuild(&mut self, state: &EditorState) {
        let brushes = state.scene.csg_brushes();
        if brushes == self.cache_key {
            return;
        }
        let started = Instant::now();
        self.polygons = olvin_csg::build(&brushes);
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        shared(|s| {
            s.stats.brushes = brushes.len();
            s.stats.polygons = self.polygons.len();
            s.stats.triangles = self
                .polygons
                .iter()
                .map(|p| p.vertices.len().saturating_sub(2))
                .sum();
            s.stats.build_ms = ms;
        });
        self.cache_key = brushes;
    }
}

impl ViewportExtension for CsgViewport {
    fn tools(&self) -> &[ToolDesc] {
        &TOOLS
    }

    fn geometry(&mut self, state: &EditorState, out: &mut Geometry) {
        self.rebuild(state);
        let (grid, outlines) = shared(|s| (s.grid_texture, s.outlines));
        let base = if grid { flags::GRID } else { 0 };
        for p in &self.polygons {
            let f = if state.selection == Some(p.brush) {
                base | flags::SELECTED
            } else {
                base
            };
            // Brushes with a material are drawn textured by the engine; keep them pickable.
            if state
                .scene
                .get(p.brush)
                .is_some_and(|e| e.material.is_some())
            {
                for i in 1..p.vertices.len().saturating_sub(1) {
                    out.pick_triangle(p.brush, [p.vertices[0], p.vertices[i], p.vertices[i + 1]]);
                }
                continue;
            }
            out.polygon(p.brush, &p.vertices, p.normal, linear_color(p.color), f);
        }
        if outlines {
            for b in &self.cache_key {
                if state.selection == Some(b.id) {
                    continue; // drawn thicker in `interact`
                }
                let c = op_color(b.op, true);
                for [a, e] in olvin_csg::brush_edges(b) {
                    out.line(a, e, c.gamma_multiply(0.55), false);
                    // Hidden parts stay faintly visible: subtractive brushes live inside walls.
                    out.line(a, e, c.gamma_multiply(0.12), true);
                }
            }
        }
    }

    fn interact(&mut self, ctx: &mut ViewportCtx) -> bool {
        self.selected_outline(ctx);
        match ctx.tool {
            DRAW_TOOL => {
                self.face = None;
                self.draw_tool(ctx);
                true
            }
            EDIT_TOOL => {
                self.draw = Draw::Idle;
                self.edit_tool(ctx)
            }
            _ => {
                self.draw = Draw::Idle;
                self.face = None;
                false
            }
        }
    }
}

impl CsgViewport {
    fn selected_outline(&self, ctx: &ViewportCtx) {
        let Some(sel) = ctx.state.selection else {
            return;
        };
        let Some(b) = self.cache_key.iter().find(|b| b.id == sel) else {
            return;
        };
        for [a, e] in olvin_csg::brush_edges(b) {
            ctx.camera.line(ctx.painter, a, e, (2.0, theme::SELECTED));
        }
    }

    fn draw_tool(&mut self, ctx: &mut ViewportCtx) {
        let pointer = ctx.response.hover_pos();
        let (pressed, released, esc, shift) = ctx.ui.input(|i| {
            (
                // Alt+LMB orbits the view: it must not start or finish a brush.
                i.pointer.button_pressed(PointerButton::Primary) && !i.modifiers.alt,
                i.pointer.button_released(PointerButton::Primary),
                i.key_pressed(Key::Escape) || i.pointer.button_pressed(PointerButton::Secondary),
                i.modifiers.shift,
            )
        });
        if esc {
            self.draw = Draw::Idle;
        }
        let hovered = ctx.response.hovered();

        match &mut self.draw {
            Draw::Idle => {
                let Some(p) = pointer else { return };
                let Some(hit) = ctx.raycast(p) else { return };
                let (point, normal) = (hit.point, hit.normal);
                let plane = DrawPlane::from_hit(point, normal);
                let at = plane.snap(point, &ctx.snap);
                cross(
                    ctx.painter,
                    ctx.camera,
                    at,
                    plane,
                    ctx.snap.step.max(0.1),
                    theme::SELECTED,
                );
                if pressed && hovered {
                    self.draw = Draw::Base {
                        plane,
                        start: at,
                        end: at,
                    };
                }
            }
            Draw::Base { plane, start, end } => {
                if let Some(p) = pointer {
                    let ray = ctx.camera.ray(p);
                    if let Some(t) = ray.plane(plane.point, plane.normal) {
                        *end = plane.snap(ray.at(t), &ctx.snap);
                    }
                }
                rect_preview(ctx, *plane, *start, *end, 0.0);
                if released {
                    let d = (*end - *start).abs();
                    let flat = (0..3).filter(|&i| i != plane.axis).any(|i| d[i] < 1e-4);
                    self.draw = if flat {
                        Draw::Idle
                    } else {
                        Draw::Height {
                            plane: *plane,
                            start: *start,
                            end: *end,
                            height: 0.0,
                        }
                    };
                }
            }
            Draw::Height {
                plane,
                start,
                end,
                height,
            } => {
                if let Some(p) = pointer {
                    let center = (*start + *end) * 0.5;
                    if let Some(t) = ctx.camera.ray(p).closest_on_line(center, plane.normal) {
                        *height = ctx.snap.value(t);
                    }
                }
                rect_preview(ctx, *plane, *start, *end, *height);
                if pressed && hovered {
                    let (plane, start, end, height) = (*plane, *start, *end, *height);
                    self.draw = Draw::Idle;
                    if height.abs() > 1e-4 {
                        create_brush(ctx.state, plane, start, end, height, shift);
                    }
                }
            }
        }
        ctx.ui.ctx().request_repaint();
    }

    /// Face handles on the selected brush. Returns true while a handle is hot.
    fn edit_tool(&mut self, ctx: &mut ViewportCtx) -> bool {
        let Some(sel) = ctx.state.selection else {
            return false;
        };
        let Some(b) = self.cache_key.iter().find(|b| b.id == sel) else {
            return false;
        };
        let m = b.transform;
        let pointer = ctx.response.hover_pos();

        let faces: Vec<(usize, f32, Vec3, Vec3, Pos2)> = (0..3)
            .flat_map(|axis| [(axis, -1.0), (axis, 1.0)])
            .filter_map(|(axis, sign)| {
                let mut local = Vec3::ZERO;
                local[axis] = 0.5 * sign;
                let world = m.transform_point3(local);
                let dir = m.transform_vector3(local).normalize_or_zero();
                Some((axis, sign, world, dir, ctx.camera.project(world)?))
            })
            .collect();
        let hot = match &self.face {
            Some(f) => Some((f.axis, f.sign)),
            None => pointer.and_then(|p| {
                faces
                    .iter()
                    .map(|f| (f, f.4.distance(p)))
                    .filter(|(_, d)| *d < 10.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(f, _)| (f.0, f.1))
            }),
        };
        for (axis, sign, _, _, s) in &faces {
            let is_hot = hot == Some((*axis, *sign));
            let color = if is_hot {
                Color32::from_rgb(255, 225, 70)
            } else {
                theme::AXES[*axis]
            };
            ctx.painter.circle(
                *s,
                if is_hot { 7.0 } else { 5.5 },
                color,
                (1.0, Color32::BLACK),
            );
        }

        if self.face.is_none()
            && ctx.response.drag_started_by(PointerButton::Primary)
            && !ctx.ui.input(|i| i.modifiers.alt)
            && let Some((axis, sign)) = hot
            && let Some(face) = faces.iter().find(|f| f.0 == axis && f.1 == sign)
            && let Some(e) = ctx.state.scene.get(sel)
        {
            let press = ctx.ui.input(|i| i.pointer.press_origin()).unwrap_or(face.4);
            let worlds = ctx.state.scene.world_matrices();
            let parent = e
                .parent
                .and_then(|p| worlds.get(&p).copied())
                .unwrap_or(Mat4::IDENTITY);
            let mut full = Vec3::ZERO;
            full[axis] = 1.0;
            self.face = Some(FaceDrag {
                entity: sel,
                axis,
                sign,
                dir: face.3,
                param: ctx
                    .camera
                    .ray(press)
                    .closest_on_line(face.2, face.3)
                    .unwrap_or(0.0),
                face: face.2,
                position: e.position,
                scale: e.scale,
                extent: m.transform_vector3(full).length(),
                // A parent scaled to zero has no inverse.
                parent_inv: if parent.determinant().abs() > 1e-12 {
                    parent.inverse()
                } else {
                    Mat4::IDENTITY
                },
            });
        }
        if let Some(f) = &self.face
            && ctx.response.dragged_by(PointerButton::Primary)
            && let Some(p) = ctx.response.interact_pointer_pos()
            && let Some(t) = ctx.camera.ray(p).closest_on_line(f.face, f.dir)
        {
            let mut delta = t - f.param;
            // Axis-aligned faces land exactly on grid lines; others move in grid steps.
            let aligned = (0..3).find(|&i| f.dir[i].abs() > 0.999);
            delta = match aligned {
                Some(i) => {
                    let target = ctx.snap.value(f.face[i] + f.dir[i] * delta);
                    (target - f.face[i]) * f.dir[i].signum()
                }
                None => ctx.snap.value(delta),
            };
            let extent = (f.extent + delta).max(0.01);
            let applied = extent - f.extent;
            if let Some(e) = ctx.state.scene.get_mut(f.entity) {
                // A flat brush (scale 0) has no extent to scale: set it.
                e.scale[f.axis] = if f.extent < 1e-6 {
                    extent
                } else {
                    f.scale[f.axis] * extent / f.extent
                };
                e.position = f.position + f.parent_inv.transform_vector3(f.dir * applied * 0.5);
            }
            ctx.painter.text(
                p + egui::vec2(14.0, -14.0),
                Align2::LEFT_BOTTOM,
                format!("{extent:.3}"),
                FontId::monospace(12.0),
                Color32::from_rgb(255, 225, 70),
            );
        }
        if ctx.response.drag_stopped() {
            self.face = None;
        }
        hot.is_some()
    }
}

fn corners(plane: DrawPlane, start: Vec3, end: Vec3) -> [Vec3; 4] {
    let (j, k) = ((plane.axis + 1) % 3, (plane.axis + 2) % 3);
    let at = |a: f32, b: f32| {
        let mut p = start;
        p[j] = a;
        p[k] = b;
        p
    };
    [
        at(start[j], start[k]),
        at(end[j], start[k]),
        at(end[j], end[k]),
        at(start[j], end[k]),
    ]
}

fn rect_preview(ctx: &ViewportCtx, plane: DrawPlane, start: Vec3, end: Vec3, height: f32) {
    let base = corners(plane, start, end);
    let top = base.map(|c| c + plane.normal * height);
    let carve = height < 0.0 || ctx.ui.input(|i| i.modifiers.shift);
    let color = if carve {
        op_color(CsgOp::Subtract, true)
    } else {
        op_color(CsgOp::Add, true)
    };
    for i in 0..4 {
        ctx.camera
            .line(ctx.painter, base[i], base[(i + 1) % 4], (2.0, color));
        if height != 0.0 {
            ctx.camera
                .line(ctx.painter, top[i], top[(i + 1) % 4], (2.0, color));
            ctx.camera.line(ctx.painter, base[i], top[i], (1.5, color));
        }
    }
    let size = (end - start).abs() + (plane.normal * height).abs();
    if let Some(p) = ctx
        .camera
        .project((start + end) * 0.5 + plane.normal * height)
    {
        ctx.painter.text(
            p,
            Align2::CENTER_BOTTOM,
            format!("{:.2} × {:.2} × {:.2}", size.x, size.y, size.z),
            FontId::monospace(12.0),
            color,
        );
    }
}

fn cross(
    painter: &egui::Painter,
    camera: &ViewCamera,
    at: Vec3,
    plane: DrawPlane,
    size: f32,
    color: Color32,
) {
    let (j, k) = ((plane.axis + 1) % 3, (plane.axis + 2) % 3);
    let (mut u, mut v) = (Vec3::ZERO, Vec3::ZERO);
    u[j] = size * 0.5;
    v[k] = size * 0.5;
    camera.line(painter, at - u, at + u, (1.5, color));
    camera.line(painter, at - v, at + v, (1.5, color));
}

fn create_brush(
    state: &mut EditorState,
    plane: DrawPlane,
    start: Vec3,
    end: Vec3,
    height: f32,
    force_subtract: bool,
) {
    let far = (start + end) * 0.5 + plane.normal * height;
    let pts = [
        start,
        end,
        start + plane.normal * height,
        end + plane.normal * height,
        far,
    ];
    let min = pts.iter().copied().fold(Vec3::splat(f32::MAX), Vec3::min);
    let max = pts.iter().copied().fold(Vec3::splat(f32::MIN), Vec3::max);
    let (shape, default_op, color) = shared(|s| (s.shape, s.op, s.color));
    // Extruding into a surface carves it, like Realtime CSG.
    let op = if force_subtract || height < 0.0 {
        CsgOp::Subtract
    } else {
        default_op
    };

    let id = state.create(NewEntity::Empty, None);
    let e = state.scene.get_mut(id).unwrap();
    e.name = format!(
        "{} ({})",
        shape.label(),
        if op == CsgOp::Subtract {
            "carve"
        } else {
            "brush"
        }
    );
    e.position = (min + max) * 0.5;
    e.scale = (max - min).max(Vec3::splat(0.01));
    e.brush = Some(BrushDesc {
        shape,
        op,
        color,
        ..Default::default()
    });
    let message = format!(
        "{} {:.2}×{:.2}×{:.2}",
        op.label(),
        e.scale.x,
        e.scale.y,
        e.scale.z
    );
    state.info(message);
}

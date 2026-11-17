//! Edit Mode in the scene view: the cage, selection and Blender's modal
//! operations (grab, extrude, inset, loop cut, knife).

use olvin_editor::egui::{
    self, Align2, Color32, FontId, Key, Modifiers, PointerButton, Pos2, Stroke, vec2,
};
use olvin_editor::ext::{Geometry, ToolDesc, ViewCamera, ViewportCtx, ViewportExtension};
use olvin_editor::glam::{Mat4, Vec3};
use olvin_editor::olvin_scene::file::PolyMesh;
use olvin_editor::{EditorState, theme, tools};

use crate::ops::{self, Op, make_editable};
use crate::{EDIT_TOOL, Mode, face_verts, selected_edges, selected_faces, shared};

const TOOLS: [ToolDesc; 1] = [ToolDesc {
    id: EDIT_TOOL,
    label: "Edit Mesh",
    tooltip: "Blender-style Edit Mode (Tab): 1/2/3 select mode, G E I F K X M, Ctrl+R",
    key: None,
}];

const HOT: Color32 = Color32::from_rgb(255, 225, 70);
const WIRE: Color32 = Color32::from_rgb(20, 22, 26);

enum Modal {
    /// Move the selection; `axis` constrains it (X/Y/Z).
    Grab {
        before: PolyMesh,
        verts: Vec<u32>,
        axis: Option<Vec3>,
        center: Vec3,
        start: Pos2,
    },
    Inset {
        before: PolyMesh,
        faces: Vec<usize>,
        center: Pos2,
        start: f32,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum Sub {
    None,
    LoopCut,
    Knife(Option<(u32, u32, f32)>),
}

pub struct EditMode {
    modal: Option<Modal>,
    sub: Sub,
}

impl Default for EditMode {
    fn default() -> Self {
        Self {
            modal: None,
            sub: Sub::None,
        }
    }
}

#[derive(Clone, Copy)]
enum Hover {
    Vertex(u32),
    Edge(u32, u32, f32),
    Face(usize),
}

impl ViewportExtension for EditMode {
    fn geometry(&mut self, _state: &EditorState, _out: &mut Geometry) {}

    fn tools(&self) -> &[ToolDesc] {
        &TOOLS
    }

    fn interact(&mut self, ctx: &mut ViewportCtx) -> bool {
        let keys_free = ctx.response.hovered() && !ctx.ui.ctx().egui_wants_keyboard_input();
        if keys_free && pressed(ctx, Key::Tab) {
            self.toggle(ctx);
        }
        if ctx.tool != EDIT_TOOL {
            self.modal = None;
            self.sub = Sub::None;
            return false;
        }
        ctx.state.tool_owns_keys = true;
        let Some(id) = ctx.state.selection else {
            hint(ctx, "Select a mesh, then Tab");
            return true;
        };
        let Some(mesh) = ctx.state.scene.get(id).and_then(|e| e.poly.clone()) else {
            hint(ctx, "Tab converts the selected object to an editable mesh");
            return true;
        };
        if shared(|s| s.entity) != Some(id) {
            shared(|s| {
                s.entity = Some(id);
                s.verts.clear();
            });
        }
        let world = ctx
            .state
            .scene
            .world_matrices()
            .get(&id)
            .copied()
            .unwrap_or(Mat4::IDENTITY);
        let points: Vec<Vec3> = mesh
            .positions
            .iter()
            .map(|p| world.transform_point3(*p))
            .collect();

        if let Some(modal) = &mut self.modal {
            ctx.state.busy = true;
            match update_modal(ctx, modal, id, world) {
                ModalResult::Continue => {}
                ModalResult::Confirm => self.modal = None,
                ModalResult::Cancel => {
                    if let Some(before) = modal_before(modal)
                        && let Some(e) = ctx.state.scene.get_mut(id)
                    {
                        e.poly = Some(before);
                    }
                    self.modal = None;
                }
            }
            draw_cage(ctx, &mesh, &points, None);
            return true;
        }

        if keys_free {
            self.hotkeys(ctx, &mesh, &points, world, id);
        }
        let mode = shared(|s| s.mode);
        let pointer = ctx.response.hover_pos();
        let hover = pointer.and_then(|p| {
            let m = match self.sub {
                Sub::LoopCut | Sub::Knife(_) => Mode::Edge,
                Sub::None => mode,
            };
            pick(m, &mesh, &points, ctx.camera, p)
        });
        draw_cage(ctx, &mesh, &points, hover);
        self.overlay(ctx, mode);

        if ctx.ui.input(|i| i.key_pressed(Key::Escape)) {
            self.sub = Sub::None;
        }
        if ctx.response.clicked_by(PointerButton::Primary) {
            self.click(ctx, id, hover);
        }
        true
    }
}

enum ModalResult {
    Continue,
    Confirm,
    Cancel,
}

fn modal_before(m: &Modal) -> Option<PolyMesh> {
    match m {
        Modal::Grab { before, .. } | Modal::Inset { before, .. } => Some(before.clone()),
    }
}

impl EditMode {
    fn toggle(&mut self, ctx: &mut ViewportCtx) {
        if ctx.tool == EDIT_TOOL {
            ctx.state.tool = tools::MOVE.to_owned();
            self.modal = None;
            self.sub = Sub::None;
        } else if let Some(id) = ctx.state.selection {
            match make_editable(ctx.state, id) {
                Ok(()) => {
                    ctx.state.tool = EDIT_TOOL.to_owned();
                    shared(|s| {
                        s.entity = Some(id);
                        s.verts.clear();
                    });
                }
                Err(e) => ctx.state.warn(e),
            }
        }
    }

    fn hotkeys(
        &mut self,
        ctx: &mut ViewportCtx,
        mesh: &PolyMesh,
        points: &[Vec3],
        world: Mat4,
        id: u64,
    ) {
        for (key, mode) in [
            (Key::Num1, Mode::Vertex),
            (Key::Num2, Mode::Edge),
            (Key::Num3, Mode::Face),
        ] {
            if pressed(ctx, key) {
                shared(|s| s.mode = mode);
            }
        }
        let shift_n = ctx.ui.input(|i| i.key_pressed(Key::N) && i.modifiers.shift);
        let ctrl_r = ctx
            .ui
            .input(|i| i.key_pressed(Key::R) && i.modifiers.command);
        let op = if pressed(ctx, Key::A) {
            Some(Op::SelectAll)
        } else if pressed(ctx, Key::F) {
            Some(Op::Fill)
        } else if pressed(ctx, Key::X) || pressed(ctx, Key::Delete) {
            Some(Op::Delete)
        } else if pressed(ctx, Key::M) {
            Some(Op::Merge)
        } else if shift_n {
            Some(Op::RecalculateNormals)
        } else {
            None
        };
        if let Some(op) = op
            && let Err(e) = ops::apply(ctx.state, op)
        {
            ctx.state.warn(e);
        }
        if ctrl_r {
            self.sub = Sub::LoopCut;
        } else if pressed(ctx, Key::K) {
            self.sub = Sub::Knife(None);
        }

        let verts = shared(|s| s.verts.clone());
        if verts.is_empty() {
            return;
        }
        let start = ctx.response.hover_pos().unwrap_or_default();
        let center = verts.iter().map(|&v| points[v as usize]).sum::<Vec3>() / verts.len() as f32;
        if pressed(ctx, Key::G) {
            self.modal = Some(Modal::Grab {
                before: mesh.clone(),
                verts,
                axis: None,
                center,
                start,
            });
        } else if pressed(ctx, Key::E) {
            self.extrude(ctx, mesh, world, id, center, start);
        } else if pressed(ctx, Key::I) {
            let faces = selected_faces(mesh, &verts);
            if let (false, Some(c)) = (faces.is_empty(), ctx.camera.project(center)) {
                self.modal = Some(Modal::Inset {
                    before: mesh.clone(),
                    faces,
                    center: c,
                    start: start.distance(c),
                });
            }
        }
    }

    /// E: extrude, then grab (faces along their normal, like Blender).
    fn extrude(
        &mut self,
        ctx: &mut ViewportCtx,
        mesh: &PolyMesh,
        world: Mat4,
        id: u64,
        center: Vec3,
        start: Pos2,
    ) {
        let (mode, verts) = shared(|s| (s.mode, s.verts.clone()));
        let mut m = mesh.clone();
        let (moved, axis) = match mode {
            Mode::Face => {
                let faces = selected_faces(&m, &verts);
                if faces.is_empty() {
                    return;
                }
                let normal: Vec3 = faces.iter().map(|&f| m.face_normal(f)).sum();
                let top = m.extrude_faces(&faces, Vec3::ZERO);
                (
                    face_verts(&m, &top),
                    Some(world.transform_vector3(normal).normalize_or_zero()),
                )
            }
            Mode::Edge => {
                let edges = selected_edges(&m, &verts);
                (m.extrude_edges(&edges, Vec3::ZERO), None)
            }
            Mode::Vertex => (m.extrude_vertices(&verts, Vec3::ZERO), None),
        };
        if moved.is_empty() {
            return;
        }
        if let Some(e) = ctx.state.scene.get_mut(id) {
            e.poly = Some(m.clone());
        }
        shared(|s| s.verts = moved.clone());
        self.modal = Some(Modal::Grab {
            before: m,
            verts: moved,
            axis: axis.filter(|a| *a != Vec3::ZERO),
            center,
            start,
        });
    }

    fn click(&mut self, ctx: &mut ViewportCtx, id: u64, hover: Option<Hover>) {
        let Some(mesh) = ctx.state.scene.get_mut(id).and_then(|e| e.poly.as_mut()) else {
            return;
        };
        match (self.sub, hover) {
            (Sub::LoopCut, Some(Hover::Edge(a, b, _))) => {
                let cuts = shared(|s| s.cuts.max(1));
                match mesh.loop_cut(a, b, cuts) {
                    Ok(new) => shared(|s| s.verts = new),
                    Err(e) => ctx.state.warn(format!("loop cut: {e}")),
                }
                self.sub = Sub::None;
            }
            (Sub::Knife(None), Some(Hover::Edge(a, b, t))) => {
                self.sub = Sub::Knife(Some((a, b, t)))
            }
            (Sub::Knife(Some(first)), Some(Hover::Edge(a, b, t))) => {
                match mesh.knife(first, (a, b, t)) {
                    Ok(new) => shared(|s| s.verts = new.to_vec()),
                    Err(e) => ctx.state.warn(format!(
                        "knife: both points must be on edges of one face ({e})"
                    )),
                }
                self.sub = Sub::None;
            }
            (Sub::None, hover) => {
                let add = ctx.ui.input(|i| i.modifiers.shift);
                let picked = match hover {
                    Some(Hover::Vertex(v)) => vec![v],
                    Some(Hover::Edge(a, b, _)) => vec![a, b],
                    Some(Hover::Face(f)) => mesh.faces[f].clone(),
                    None => Vec::new(),
                };
                shared(|s| {
                    if !add {
                        s.verts = picked;
                    } else if !picked.is_empty() && picked.iter().all(|v| s.verts.contains(v)) {
                        s.verts.retain(|v| !picked.contains(v));
                    } else {
                        for v in picked {
                            if !s.verts.contains(&v) {
                                s.verts.push(v);
                            }
                        }
                    }
                });
            }
            _ => {}
        }
    }

    fn overlay(&self, ctx: &ViewportCtx, mode: Mode) {
        let rect = ctx.camera.rect;
        let count = shared(|s| s.verts.len());
        let mode_name = match self.sub {
            Sub::LoopCut => "LOOP CUT · click an edge · Esc cancel".to_owned(),
            Sub::Knife(None) => "KNIFE · click the first edge".to_owned(),
            Sub::Knife(Some(_)) => "KNIFE · click a second edge of the same face".to_owned(),
            Sub::None => format!("EDIT MODE · {mode:?} · {count} selected"),
        };
        let pos = rect.center_top() + vec2(0.0, 56.0);
        let galley =
            ctx.painter
                .layout_no_wrap(mode_name, FontId::proportional(13.0), Color32::WHITE);
        let bg = egui::Rect::from_center_size(pos, galley.size() + vec2(20.0, 8.0));
        ctx.painter
            .rect_filled(bg, 8.0, theme::ACCENT.gamma_multiply(0.85));
        ctx.painter
            .galley(bg.min + vec2(10.0, 4.0), galley, Color32::WHITE);
        ctx.painter.text(
            rect.left_bottom() + vec2(8.0, -24.0),
            Align2::LEFT_BOTTOM,
            "1/2/3 mode · A all · G move (X/Y/Z axis) · E extrude · I inset · Ctrl+R loop cut · K knife · F fill · X delete · M merge · Tab exit",
            FontId::proportional(11.0),
            Color32::from_white_alpha(130),
        );
    }
}

fn update_modal(ctx: &mut ViewportCtx, modal: &mut Modal, id: u64, world: Mat4) -> ModalResult {
    let (confirm, cancel) = ctx.ui.input(|i| {
        (
            i.pointer.button_pressed(PointerButton::Primary) || i.key_pressed(Key::Enter),
            i.pointer.button_pressed(PointerButton::Secondary) || i.key_pressed(Key::Escape),
        )
    });
    if cancel {
        return ModalResult::Cancel;
    }
    let Some(mouse) = ctx.ui.ctx().pointer_hover_pos() else {
        return ModalResult::Continue;
    };
    let inverse = world.inverse();
    match modal {
        Modal::Grab {
            before,
            verts,
            axis,
            center,
            start,
        } => {
            for (key, a) in [(Key::X, Vec3::X), (Key::Y, Vec3::Y), (Key::Z, Vec3::Z)] {
                if ctx.ui.input(|i| i.key_pressed(key)) {
                    *axis = if *axis == Some(a) { None } else { Some(a) };
                }
            }
            let (axis_now, origin) = (*axis, *center);
            let point = |p: Pos2| -> Option<Vec3> {
                let ray = ctx.camera.ray(p);
                match axis_now {
                    Some(a) => Some(origin + a * ray.closest_on_line(origin, a)?),
                    None => Some(ray.at(ray.plane(origin, ctx.camera.forward)?)),
                }
            };
            let (Some(from), Some(to)) = (point(*start), point(mouse)) else {
                return ModalResult::Continue;
            };
            let delta = match axis {
                Some(a) => *a * ctx.snap.value((to - from).dot(*a)),
                None => ctx.snap.vec(to - from),
            };
            let mut m = before.clone();
            m.translate_vertices(verts, inverse.transform_vector3(delta));
            if let Some(e) = ctx.state.scene.get_mut(id) {
                e.poly = Some(m);
            }
            let label = match axis {
                Some(a) if *a == Vec3::X => "X",
                Some(a) if *a == Vec3::Y => "Y",
                Some(a) if *a == Vec3::Z => "Z",
                Some(_) => "normal",
                None => "free",
            };
            status(
                ctx,
                mouse,
                format!("move {label}  {:.3} {:.3} {:.3}", delta.x, delta.y, delta.z),
            );
        }
        Modal::Inset {
            before,
            faces,
            center,
            start,
        } => {
            let pixels = (*start - mouse.distance(*center)).max(0.0);
            let amount = ctx.snap.value(
                pixels
                    * ctx
                        .camera
                        .world_per_point(world.transform_point3(before.face_center(faces[0]))),
            );
            let mut m = before.clone();
            let inner = m.inset_faces(faces, amount.max(0.0));
            shared(|s| s.verts = face_verts(&m, &inner));
            if let Some(e) = ctx.state.scene.get_mut(id) {
                e.poly = Some(m);
            }
            status(
                ctx,
                mouse,
                format!("inset {amount:.3}  (move towards the centre)"),
            );
        }
    }
    if confirm {
        ModalResult::Confirm
    } else {
        ModalResult::Continue
    }
}

fn draw_cage(ctx: &ViewportCtx, mesh: &PolyMesh, points: &[Vec3], hover: Option<Hover>) {
    let (mode, verts) = shared(|s| (s.mode, s.verts.clone()));
    let project = |i: u32| ctx.camera.project(points[i as usize]);
    // Faces: selected (face mode) and hovered tints.
    for f in 0..mesh.faces.len() {
        let selected = mode == Mode::Face && mesh.faces[f].iter().all(|v| verts.contains(v));
        let hot = matches!(hover, Some(Hover::Face(h)) if h == f);
        if !(selected || hot) {
            continue;
        }
        let fill = if hot {
            HOT.gamma_multiply(0.22)
        } else {
            theme::SELECTED.gamma_multiply(0.28)
        };
        for [a, b, c] in mesh.face_triangles(f) {
            if let (Some(a), Some(b), Some(c)) = (project(a), project(b), project(c)) {
                ctx.painter.add(egui::Shape::convex_polygon(
                    vec![a, b, c],
                    fill,
                    Stroke::NONE,
                ));
            }
        }
    }
    for [a, b] in mesh.edges() {
        let on = verts.contains(&a) && verts.contains(&b);
        let (pa, pb) = (points[a as usize], points[b as usize]);
        ctx.camera.line(ctx.painter, pa, pb, (2.5, WIRE));
        ctx.camera.line(
            ctx.painter,
            pa,
            pb,
            (
                if on { 2.0 } else { 1.0 },
                if on {
                    theme::SELECTED
                } else {
                    Color32::from_gray(200)
                },
            ),
        );
    }
    if let Some(Hover::Edge(a, b, t)) = hover {
        let (pa, pb) = (points[a as usize], points[b as usize]);
        ctx.camera.line(ctx.painter, pa, pb, (3.0, HOT));
        if let Some(s) = ctx.camera.project(pa.lerp(pb, t)) {
            ctx.painter.circle_filled(s, 3.5, HOT);
        }
    }
    let size = if mode == Mode::Vertex { 6.0 } else { 3.5 };
    for (i, p) in points.iter().enumerate() {
        if let Some(s) = ctx.camera.project(*p) {
            let color = if verts.contains(&(i as u32)) {
                theme::SELECTED
            } else {
                Color32::from_gray(15)
            };
            ctx.painter.rect_filled(
                egui::Rect::from_center_size(s, vec2(size, size)),
                1.0,
                color,
            );
            ctx.painter.rect_stroke(
                egui::Rect::from_center_size(s, vec2(size, size)),
                1.0,
                (1.0, Color32::from_gray(220)),
                egui::StrokeKind::Middle,
            );
        }
    }
    if let Some(Hover::Vertex(v)) = hover
        && let Some(s) = project(v)
    {
        ctx.painter.circle_stroke(s, 7.0, (2.0, HOT));
    }
}

fn pick(
    mode: Mode,
    mesh: &PolyMesh,
    points: &[Vec3],
    camera: &ViewCamera,
    p: Pos2,
) -> Option<Hover> {
    match mode {
        Mode::Vertex => points
            .iter()
            .enumerate()
            .filter_map(|(i, v)| Some((i, camera.project(*v)?.distance(p))))
            .filter(|(_, d)| *d < 10.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| Hover::Vertex(i as u32)),
        Mode::Edge => mesh
            .edges()
            .into_iter()
            .filter_map(|[a, b]| {
                let (sa, sb) = (
                    camera.project(points[a as usize])?,
                    camera.project(points[b as usize])?,
                );
                let ab = sb - sa;
                let t = ((p - sa).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.02, 0.98);
                Some((a, b, t, p.distance(sa + ab * t)))
            })
            .filter(|e| e.3 < 8.0)
            .min_by(|x, y| x.3.total_cmp(&y.3))
            .map(|(a, b, t, _)| Hover::Edge(a, b, t)),
        Mode::Face => {
            let ray = camera.ray(p);
            (0..mesh.faces.len())
                .filter_map(|f| {
                    mesh.face_triangles(f)
                        .into_iter()
                        .filter_map(|[a, b, c]| {
                            ray.triangle(points[a as usize], points[b as usize], points[c as usize])
                        })
                        .min_by(|x, y| x.total_cmp(y))
                        .map(|t| (f, t))
                })
                .min_by(|x, y| x.1.total_cmp(&y.1))
                .map(|(f, _)| Hover::Face(f))
        }
    }
}

fn pressed(ctx: &ViewportCtx, key: Key) -> bool {
    ctx.ui
        .input(|i| i.key_pressed(key) && i.modifiers.matches_exact(Modifiers::NONE))
}

fn hint(ctx: &ViewportCtx, text: &str) {
    ctx.painter.text(
        ctx.camera.rect.center_top() + vec2(0.0, 56.0),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(13.0),
        Color32::from_white_alpha(200),
    );
}

fn status(ctx: &ViewportCtx, at: Pos2, text: String) {
    ctx.painter.text(
        at + vec2(16.0, -16.0),
        Align2::LEFT_BOTTOM,
        text,
        FontId::monospace(12.0),
        HOT,
    );
}

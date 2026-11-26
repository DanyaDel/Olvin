//! Edit Mode in the scene view: the cage, selection (click, box, loops,
//! X-ray), the move gizmo and Blender's modal operations.

use olvin_editor::egui::{
    self, Align2, Color32, Event, FontId, Key, PointerButton, Pos2, Rect, Sense, Stroke, vec2,
};
use olvin_editor::ext::{Geometry, ToolDesc, ViewportCtx, ViewportExtension};
use olvin_editor::glam::{Mat4, Vec3};
use olvin_editor::keymap::Command;
use olvin_editor::olvin_scene::file::PolyMesh;
use olvin_editor::{EditorState, theme, tools};

use crate::cage::{Cage, HOT, Hover};
use crate::gizmo::Gizmo;
use crate::ops::{self, Op, make_editable};
use crate::select::{Combine, combine};
use crate::transform::{self, Constraint, Input, Kind, NumInput, Transform, normal_basis};
use crate::{EDIT_TOOL, Mode, face_verts, selected_edges, selected_faces, shared};

const TOOLS: [ToolDesc; 1] = [ToolDesc {
    id: EDIT_TOOL,
    label: "Edit Mesh",
    tooltip: "Edit mesh vertices, edges and faces",
    key: None,
}];

enum Modal {
    Transform(Box<Transform>),
    Inset {
        before: PolyMesh,
        faces: Vec<usize>,
        center: Pos2,
        start: f32,
        num: NumInput,
    },
}

impl Modal {
    fn before(&self) -> &PolyMesh {
        match self {
            Modal::Transform(t) => &t.before,
            Modal::Inset { before, .. } => before,
        }
    }
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
    cage: Cage,
    /// Box select in progress: where the drag started.
    boxing: Option<Pos2>,
    /// Edit Mode was active last frame (to win back plain keys the core
    /// viewport turns into tool switches).
    was_editing: bool,
    /// egui's Ctrl+=/- zoom is off while editing (it is grow / shrink here).
    zoom_off: bool,
}

impl Default for EditMode {
    fn default() -> Self {
        Self {
            modal: None,
            sub: Sub::None,
            cage: Cage::default(),
            boxing: None,
            was_editing: false,
            zoom_off: false,
        }
    }
}

impl ViewportExtension for EditMode {
    fn geometry(&mut self, _state: &EditorState, _out: &mut Geometry) {}

    fn tools(&self) -> &[ToolDesc] {
        &TOOLS
    }

    fn interact(&mut self, ctx: &mut ViewportCtx) -> bool {
        // While RMB is held the keys fly the camera (WASD/QE), never edit.
        let flying = ctx.ui.input(|i| i.pointer.secondary_down());
        let keys_free = ctx.response.hovered()
            && !flying
            && !ctx.state.capturing_key
            && !ctx.ui.ctx().egui_wants_keyboard_input();
        let mut editing = self.reclaim(ctx, keys_free);
        if keys_free
            && self.modal.is_none()
            && ctx
                .ui
                .input_mut(|i| ctx.state.prefs.keymap.pressed(i, Command::EditToggle))
        {
            editing = self.toggle(ctx, editing);
        }
        self.keyboard_zoom(ctx, editing);
        self.was_editing = editing;
        if !editing {
            ctx.state.mesh_selection = None;
            self.modal = None;
            self.sub = Sub::None;
            self.boxing = None;
            return false;
        }
        ctx.state.tool_owns_keys = true;
        let Some(id) = ctx.state.selection else {
            hint(ctx, "Select a mesh to edit");
            return true;
        };
        // Picking another brush while editing: it becomes editable at once.
        if ctx
            .state
            .scene
            .get(id)
            .is_some_and(|e| e.brush.is_some() && e.poly.is_none())
        {
            let _ = make_editable(ctx.state, id);
        }
        let Some(mesh) = ctx.state.scene.get(id).and_then(|e| e.poly.clone()) else {
            hint(ctx, "Tab converts the selected object to an editable mesh");
            return true;
        };
        if shared(|s| s.entity) != Some(id) {
            shared(|s| {
                s.entity = Some(id);
                s.verts.clear();
                s.hidden.clear();
            });
        }
        // Undo / redo swap the mesh under the selection: forget vertices it no longer has.
        let count = mesh.positions.len() as u32;
        shared(|s| {
            s.verts.retain(|v| *v < count);
            s.hidden.retain(|v| *v < count);
        });
        let world = ctx
            .state
            .scene
            .world_matrices()
            .get(&id)
            .copied()
            .unwrap_or(Mat4::IDENTITY);

        if self.modal.is_some() {
            ctx.state.busy = true;
            if !ctx.state.capturing_key {
                self.run_modal(ctx, id);
            }
            let mesh = ctx
                .state
                .scene
                .get(id)
                .and_then(|e| e.poly.clone())
                .unwrap_or(mesh);
            self.cage.update(&mesh, world, ctx.camera, true);
            let mode = shared(|s| s.mode);
            self.cage.draw(ctx.painter, ctx.camera, mode, None);
            self.draw_modal_guides(ctx);
            self.overlay(ctx, mode);
            return true;
        }
        self.cage.update(&mesh, world, ctx.camera, false);

        let (mode, xray, show_gizmo) = shared(|s| (s.mode, s.xray, s.gizmo));
        let (alt, shift, ctrl) = ctx
            .ui
            .input(|i| (i.modifiers.alt, i.modifiers.shift, i.modifiers.command));
        let pointer = ctx.response.hover_pos();
        let gizmo = self
            .cage
            .pivot()
            .filter(|_| show_gizmo && self.sub == Sub::None && self.boxing.is_none())
            .and_then(|p| Gizmo::new(ctx.camera, p));
        let gizmo_hover = gizmo.as_ref().zip(pointer).and_then(|(g, p)| g.hit(p));
        let pick_mode = if alt || self.sub != Sub::None {
            Mode::Edge
        } else {
            mode
        };
        let hover = pointer
            .filter(|_| gizmo_hover.is_none() && self.boxing.is_none())
            .and_then(|p| self.cage.pick(pick_mode, ctx.camera, p, xray));
        self.cage.draw(ctx.painter, ctx.camera, mode, hover);
        if let Some(g) = &gizmo {
            g.draw(ctx.painter, gizmo_hover);
        }

        if keys_free {
            self.hotkeys(ctx, &mesh, world, id, hover);
        }
        if !ctx.state.capturing_key && ctx.ui.input(|i| i.key_pressed(Key::Escape)) {
            self.sub = Sub::None;
        }

        // LMB drag: a gizmo handle moves the selection, empty space boxes.
        if self.modal.is_none() && !alt && ctx.response.drag_started_by(PointerButton::Primary) {
            let origin = ctx.ui.input(|i| i.pointer.press_origin()).or(pointer);
            let handle = gizmo.as_ref().zip(origin).and_then(|(g, p)| g.hit(p));
            if let Some(h) = handle {
                let start = origin.unwrap_or_default();
                self.start_transform(ctx, Kind::Move, "Move", mesh.clone(), world, start);
                if let Some(Modal::Transform(t)) = &mut self.modal {
                    t.drag = true;
                    t.constraint = h.constraint();
                }
            } else if self.sub == Sub::None {
                self.boxing = origin;
            }
        }
        if let Some(start) = self.boxing {
            let end = pointer.unwrap_or(start);
            let r = Rect::from_two_pos(start, end);
            ctx.painter
                .rect_filled(r, 0.0, Color32::from_white_alpha(12));
            ctx.painter.rect_stroke(
                r,
                0.0,
                (1.0, Color32::from_white_alpha(200)),
                egui::StrokeKind::Middle,
            );
            if !ctx.ui.input(|i| i.pointer.primary_down()) {
                let how = if ctrl {
                    Combine::Subtract
                } else if shift {
                    Combine::Add
                } else {
                    Combine::Replace
                };
                let picked = self.cage.boxed(mode, &mesh, r);
                shared(|s| combine(&mut s.verts, &picked, how));
                self.boxing = None;
            }
        }
        if ctx.response.clicked_by(PointerButton::Primary) && gizmo_hover.is_none() {
            self.click(ctx, id, hover, alt, shift, ctrl);
        }
        shared(|s| s.status.clear());
        ctx.state.mesh_selection = Some((id, shared(|s| s.verts.clone())));
        self.overlay(ctx, mode);
        true
    }
}

impl EditMode {
    /// Whether Edit Mode is on. The core viewport leaves plain keys alone
    /// while a tool owns them (`EditorState::keys_owned_last_frame`).
    fn reclaim(&self, ctx: &mut ViewportCtx, _keys_free: bool) -> bool {
        ctx.tool == EDIT_TOOL
    }

    fn keyboard_zoom(&mut self, ctx: &ViewportCtx, editing: bool) {
        if editing != self.zoom_off {
            ctx.ui
                .ctx()
                .options_mut(|o| o.zoom_with_keyboard = !editing);
            self.zoom_off = editing;
        }
    }

    /// Tab: enter or leave Edit Mode. Returns whether Edit Mode is now on.
    fn toggle(&mut self, ctx: &mut ViewportCtx, editing: bool) -> bool {
        if editing {
            ctx.state.tool = tools::MOVE.to_owned();
            self.modal = None;
            self.sub = Sub::None;
            return false;
        }
        let Some(id) = ctx.state.selection else {
            return false;
        };
        match make_editable(ctx.state, id) {
            Ok(()) => {
                ctx.state.tool = EDIT_TOOL.to_owned();
                shared(|s| {
                    s.entity = Some(id);
                    s.verts.clear();
                    s.hidden.clear();
                });
                true
            }
            Err(e) => {
                ctx.state.warn(e);
                false
            }
        }
    }

    fn hotkeys(
        &mut self,
        ctx: &mut ViewportCtx,
        mesh: &PolyMesh,
        world: Mat4,
        id: u64,
        hover: Option<Hover>,
    ) {
        let keys = ctx.state.prefs.keymap.clone();
        let pressed = |command| ctx.ui.input_mut(|i| keys.pressed(i, command));
        for (key, mode) in [
            (Command::EditVertex, Mode::Vertex),
            (Command::EditEdge, Mode::Edge),
            (Command::EditFace, Mode::Face),
        ] {
            if pressed(key) {
                shared(|s| s.mode = mode);
            }
        }
        if pressed(Command::EditXray) {
            shared(|s| s.xray = !s.xray);
        }
        let op = if pressed(Command::EditSelectAll) {
            Some(Op::SelectAll)
        } else if pressed(Command::EditSelectNone) {
            Some(Op::SelectNone)
        } else if pressed(Command::EditInvert) {
            Some(Op::SelectInvert)
        } else if pressed(Command::EditLinked) {
            Some(Op::SelectLinked)
        } else if pressed(Command::EditGrow) {
            Some(Op::SelectGrow)
        } else if pressed(Command::EditShrink) {
            Some(Op::SelectShrink)
        } else if pressed(Command::EditHide) {
            Some(Op::Hide)
        } else if pressed(Command::EditHideOthers) {
            Some(Op::HideUnselected)
        } else if pressed(Command::EditReveal) {
            Some(Op::Reveal)
        } else if pressed(Command::EditFill) {
            Some(Op::Fill)
        } else if pressed(Command::EditConnect) {
            Some(Op::Connect)
        } else if pressed(Command::EditDelete) {
            Some(Op::Delete)
        } else if pressed(Command::EditMerge) {
            Some(Op::Merge)
        } else if pressed(Command::EditNormals) {
            Some(Op::RecalculateNormals)
        } else {
            None
        };
        if let Some(op) = op
            && let Err(e) = ops::apply(ctx.state, op)
        {
            ctx.state.warn(e);
        }
        // L / Shift+L: (de)select everything linked to the element under the cursor.
        for (command, how) in [
            (Command::EditPickLinked, Combine::Add),
            (Command::EditUnpickLinked, Combine::Subtract),
        ] {
            if pressed(command)
                && let Some(h) = hover
            {
                let linked = mesh.linked(&h.verts(mesh));
                shared(|s| combine(&mut s.verts, &linked, how));
            }
        }
        if pressed(Command::EditLoopCut) {
            self.sub = Sub::LoopCut;
        } else if pressed(Command::EditKnife) {
            self.sub = Sub::Knife(None);
        }

        let verts = shared(|s| s.verts.clone());
        if verts.is_empty() {
            return;
        }
        let start = ctx.response.hover_pos().unwrap_or(ctx.camera.rect.center());
        for (key, kind) in [
            (Command::EditMove, Kind::Move),
            (Command::EditRotate, Kind::Rotate),
            (Command::EditScale, Kind::Scale),
        ] {
            if pressed(key) {
                self.start_transform(ctx, kind, "Move", mesh.clone(), world, start);
            }
        }
        if pressed(Command::EditDuplicate) {
            match ops::apply(ctx.state, Op::Duplicate) {
                Ok(()) => {
                    if let Some(m) = ctx.state.scene.get(id).and_then(|e| e.poly.clone()) {
                        self.start_transform(ctx, Kind::Move, "Duplicate", m, world, start);
                    }
                }
                Err(e) => ctx.state.warn(e),
            }
        } else if pressed(Command::EditExtrude) {
            self.extrude(ctx, mesh, world, id, start);
        } else if pressed(Command::EditInset) {
            let faces = selected_faces(mesh, &verts);
            let center = self.cage.pivot().and_then(|p| ctx.camera.project(p));
            if let (false, Some(c)) = (faces.is_empty(), center) {
                self.modal = Some(Modal::Inset {
                    before: mesh.clone(),
                    faces,
                    center: c,
                    start: start.distance(c),
                    num: NumInput::default(),
                });
            }
        }
    }

    /// Starts G / R / S on the current selection of `mesh` (already stored
    /// in the scene).
    fn start_transform(
        &mut self,
        ctx: &ViewportCtx,
        kind: Kind,
        title: &'static str,
        mesh: PolyMesh,
        world: Mat4,
        mouse: Pos2,
    ) {
        let verts = shared(|s| s.verts.clone());
        if verts.is_empty() {
            return;
        }
        let local = orientation(&mesh, &verts, world);
        let mut t = Transform::new(kind, title, mesh, verts, world, local, mouse);
        // Seed the summary so the first frame already shows the op name.
        t.step(ctx.camera, ctx.snap, &Input::default());
        self.modal = Some(Modal::Transform(Box::new(t)));
    }

    /// E: extrude, then grab (faces along their normal, like Blender).
    fn extrude(
        &mut self,
        ctx: &mut ViewportCtx,
        mesh: &PolyMesh,
        world: Mat4,
        id: u64,
        start: Pos2,
    ) {
        let (mode, verts) = shared(|s| (s.mode, s.verts.clone()));
        let mut m = mesh.clone();
        let (moved, along_normal) = match mode {
            Mode::Face => {
                let faces = selected_faces(&m, &verts);
                if faces.is_empty() {
                    return;
                }
                let top = m.extrude_faces(&faces, Vec3::ZERO);
                (face_verts(&m, &top), true)
            }
            Mode::Edge => {
                let edges = selected_edges(&m, &verts);
                (m.extrude_edges(&edges, Vec3::ZERO), false)
            }
            Mode::Vertex => (m.extrude_vertices(&verts, Vec3::ZERO), false),
        };
        if moved.is_empty() {
            return;
        }
        // The normal orientation comes from the faces before extrusion.
        let local = orientation(mesh, &verts, world);
        if let Some(e) = ctx.state.scene.get_mut(id) {
            e.poly = Some(m.clone());
        }
        shared(|s| s.verts = moved);
        self.start_transform(ctx, Kind::Move, "Extrude", m, world, start);
        if let Some(Modal::Transform(t)) = &mut self.modal {
            (t.local, t.local_name) = local;
            if along_normal {
                t.constraint = Some(Constraint {
                    axis: 2,
                    plane: false,
                    local: true,
                });
            }
        }
    }

    fn run_modal(&mut self, ctx: &mut ViewportCtx, id: u64) {
        let Some(modal) = &mut self.modal else {
            return;
        };
        let (enter, cancel, click, released) = ctx.ui.input(|i| {
            (
                i.key_pressed(Key::Enter) || i.key_pressed(Key::Space),
                i.pointer.button_pressed(PointerButton::Secondary) || i.key_pressed(Key::Escape),
                i.pointer.button_pressed(PointerButton::Primary),
                !i.pointer.primary_down(),
            )
        });
        if cancel {
            let before = modal.before().clone();
            if let Some(e) = ctx.state.scene.get_mut(id) {
                e.poly = Some(before);
            }
            self.modal = None;
            return;
        }
        let input = modal_input(ctx);
        let (mesh, confirm, status) = match modal {
            Modal::Transform(t) => {
                let m = t.step(ctx.camera, ctx.snap, &input);
                let confirm = enter || if t.drag { released } else { click };
                (
                    transform::apply(&t.before, &t.verts, t.world, m),
                    confirm,
                    t.summary.clone(),
                )
            }
            Modal::Inset {
                before,
                faces,
                center,
                start,
                num,
            } => {
                for c in &input.chars {
                    num.type_char(*c);
                }
                if input.backspace {
                    num.backspace();
                }
                let amount = num.value().unwrap_or_else(|| {
                    let mouse = input.mouse.unwrap_or(*center);
                    let pixels = (*start - mouse.distance(*center)).max(0.0);
                    let world = ctx
                        .state
                        .scene
                        .world_matrices()
                        .get(&id)
                        .copied()
                        .unwrap_or(Mat4::IDENTITY);
                    let at = world.transform_point3(before.face_center(faces[0]));
                    ctx.snap.value(pixels * ctx.camera.world_per_point(at))
                });
                let mut m = before.clone();
                let inner = m.inset_faces(faces, amount.max(0.0));
                shared(|s| s.verts = face_verts(&m, &inner));
                let typed = if num.is_active() {
                    format!("  [{}|]", num.text())
                } else {
                    String::new()
                };
                (m, enter || click, format!("Inset{typed}  {amount:.3}"))
            }
        };
        if let Some(e) = ctx.state.scene.get_mut(id) {
            e.poly = Some(mesh);
        }
        shared(|s| s.status = status);
        if confirm {
            self.modal = None;
        }
    }

    /// Constraint axis lines, the pivot and the dashed line to the mouse.
    fn draw_modal_guides(&self, ctx: &ViewportCtx) {
        let mouse = ctx.ui.ctx().pointer_hover_pos();
        let status = shared(|s| s.status.clone());
        if let Some(Modal::Transform(t)) = &self.modal {
            let axes = t.axes();
            let on = transform::active_axes(t.constraint);
            if t.constraint.is_some() {
                let far = ctx.camera.world_per_point(t.pivot) * 4000.0;
                for i in (0..3).filter(|&i| on[i]) {
                    let (a, b) = (t.pivot - axes[i] * far, t.pivot + axes[i] * far);
                    ctx.camera
                        .line(ctx.painter, a, b, (1.5, theme::AXES[i].gamma_multiply(0.9)));
                }
            }
            if let Some(c) = ctx.camera.project(t.pivot) {
                ctx.painter.circle_filled(c, 3.0, Color32::WHITE);
                if t.kind != Kind::Move
                    && let Some(m) = mouse
                {
                    let stroke = Stroke::new(1.0, Color32::from_white_alpha(170));
                    ctx.painter
                        .extend(egui::Shape::dashed_line(&[c, m], stroke, 6.0, 4.0));
                }
            }
        }
        if let Some(m) = mouse {
            ctx.painter.text(
                m + vec2(16.0, -16.0),
                Align2::LEFT_BOTTOM,
                status,
                FontId::monospace(12.0),
                HOT,
            );
        }
    }

    fn click(
        &mut self,
        ctx: &mut ViewportCtx,
        id: u64,
        hover: Option<Hover>,
        alt: bool,
        shift: bool,
        ctrl: bool,
    ) {
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
            (Sub::None, Some(Hover::Edge(a, b, _))) if alt => {
                // Alt: edge loop; Ctrl+Alt: edge ring; face mode: face loop.
                let mode = shared(|s| s.mode);
                let picked: Vec<u32> = if mode == Mode::Face {
                    face_verts(mesh, &mesh.ring(a, b).0)
                } else if ctrl {
                    mesh.ring(a, b).1.into_iter().flatten().collect()
                } else {
                    mesh.edge_loop(a, b).into_iter().flatten().collect()
                };
                let how = if shift {
                    Combine::Add
                } else {
                    Combine::Replace
                };
                shared(|s| combine(&mut s.verts, &picked, how));
            }
            (Sub::None, hover) => {
                let picked = hover.map(|h| h.verts(mesh)).unwrap_or_default();
                shared(|s| {
                    let all_on = !picked.is_empty() && picked.iter().all(|v| s.verts.contains(v));
                    let how = match (shift, all_on) {
                        (false, _) => Combine::Replace,
                        (true, true) => Combine::Subtract,
                        (true, false) => Combine::Add,
                    };
                    combine(&mut s.verts, &picked, how);
                });
            }
            _ => {}
        }
    }

    /// Header pill (top left): select mode, X-ray and gizmo toggles and what
    /// is going on; plus a short key hint at the bottom.
    fn overlay(&self, ctx: &ViewportCtx, mode: Mode) {
        let rect = ctx.camera.rect;
        let font = FontId::proportional(12.5);
        let (count, xray, gizmo, status) =
            shared(|s| (s.verts.len(), s.xray, s.gizmo, s.status.clone()));
        let text = match self.sub {
            Sub::LoopCut => "Loop Cut: click an edge".to_owned(),
            Sub::Knife(None) => "Knife: click the first edge".to_owned(),
            Sub::Knife(Some(_)) => "Knife: click a second edge of the same face".to_owned(),
            Sub::None if !status.is_empty() => status,
            Sub::None => format!("{count} selected"),
        };
        let galley = ctx.painter.layout_no_wrap(text, font.clone(), theme::TEXT);
        const BUTTON: f32 = 52.0;
        const H: f32 = 22.0;
        let origin = egui::pos2(rect.left() + 8.0, ctx.overlay_top + 6.0);
        let width = 5.0 * BUTTON + 2.0 * 8.0 + galley.size().x + 22.0;
        let bg = Rect::from_min_size(origin, vec2(width, H + 6.0));
        ctx.painter
            .rect_filled(bg, 7.0, Color32::from_black_alpha(130));

        let mut x = origin.x + 3.0;
        let button = |x: &mut f32, label: &str, on: bool, tip: &str| -> bool {
            let r = Rect::from_min_size(Pos2::new(*x, origin.y + 3.0), vec2(BUTTON - 2.0, H));
            *x += BUTTON;
            let id = ctx.response.id.with(("edit-pill", label));
            let resp = ctx.ui.interact(r, id, Sense::click());
            let fill = if on {
                theme::ACCENT
            } else if resp.hovered() {
                Color32::from_white_alpha(28)
            } else {
                Color32::TRANSPARENT
            };
            ctx.painter.rect_filled(r, 5.0, fill);
            let color = if on || resp.hovered() {
                Color32::WHITE
            } else {
                theme::TEXT_WEAK
            };
            ctx.painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                label,
                font.clone(),
                color,
            );
            resp.on_hover_text(tip).clicked()
        };
        for (m, label, tip) in [
            (Mode::Vertex, "Vertex", "Vertex select (1)"),
            (Mode::Edge, "Edge", "Edge select (2)"),
            (Mode::Face, "Face", "Face select (3)"),
        ] {
            if button(&mut x, label, mode == m, tip) {
                shared(|s| s.mode = m);
            }
        }
        x += 8.0;
        if button(
            &mut x,
            "X-ray",
            xray,
            "See and select through the mesh (Alt+Z)",
        ) {
            shared(|s| s.xray = !s.xray);
        }
        if button(&mut x, "Gizmo", gizmo, "Move gizmo at the selection") {
            shared(|s| s.gizmo = !s.gizmo);
        }
        x += 8.0;
        let text_pos = Pos2::new(x + 6.0, bg.center().y - galley.size().y * 0.5);
        ctx.painter.galley(text_pos, galley, theme::TEXT);

        let keys = match (&self.modal, self.sub) {
            (Some(Modal::Transform(_)), _) => {
                "LMB/Enter confirm · RMB/Esc cancel · X/Y/Z axis, twice local · Shift+X/Y/Z plane · type a value · Ctrl snap · Shift precise"
            }
            (Some(Modal::Inset { .. }), _) => {
                "move towards the centre · type a value · LMB/Enter confirm · RMB/Esc cancel"
            }
            (None, Sub::LoopCut | Sub::Knife(_)) => "click an edge · Esc cancel",
            (None, Sub::None) => {
                "G/R/S transform · E extrude · I inset · Shift+D duplicate · Ctrl+R loop cut · K knife · J connect · drag box · Alt+click loop · L linked · H hide · Alt+Z X-ray · Tab exit"
            }
        };
        ctx.painter.text(
            rect.left_bottom() + vec2(8.0, -24.0),
            Align2::LEFT_BOTTOM,
            keys,
            FontId::proportional(11.0),
            Color32::from_white_alpha(130),
        );
    }
}

/// Second orientation for double axis presses: the average normal of the
/// selected faces, else the object's own axes.
fn orientation(mesh: &PolyMesh, verts: &[u32], world: Mat4) -> ([Vec3; 3], &'static str) {
    let faces = selected_faces(mesh, verts);
    let normal: Vec3 = faces.iter().map(|&f| mesh.face_area_vector(f)).sum();
    if normal.length_squared() > 1e-12 {
        let n = world.inverse().transpose().transform_vector3(normal);
        return (normal_basis(n), "Normal");
    }
    let (_, rotation, _) = world.to_scale_rotation_translation();
    ([Vec3::X, Vec3::Y, Vec3::Z].map(|a| rotation * a), "Local")
}

/// This frame's input for a running modal operation.
fn modal_input(ctx: &ViewportCtx) -> Input {
    ctx.ui.input(|i| {
        let plain = !i.modifiers.command && !i.modifiers.alt;
        let key = |k| plain && i.key_pressed(k);
        Input {
            mouse: i.pointer.hover_pos(),
            precise: i.modifiers.shift,
            axis_keys: [(Key::X, 0), (Key::Y, 1), (Key::Z, 2)]
                .into_iter()
                .filter(|(k, _)| key(*k))
                .map(|(_, a)| (a, i.modifiers.shift))
                .collect(),
            chars: i
                .events
                .iter()
                .filter_map(|e| match e {
                    Event::Text(t) => Some(t.chars()),
                    _ => None,
                })
                .flatten()
                .collect(),
            backspace: i.key_pressed(Key::Backspace),
            switch: [
                (Key::G, Kind::Move),
                (Key::R, Kind::Rotate),
                (Key::S, Kind::Scale),
            ]
            .into_iter()
            .find(|(k, _)| key(*k) && !i.modifiers.shift)
            .map(|(_, kind)| kind),
        }
    })
}

fn hint(ctx: &ViewportCtx, text: &str) {
    ctx.painter.text(
        egui::pos2(ctx.camera.rect.center().x, ctx.overlay_top + 20.0),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(13.0),
        Color32::from_white_alpha(200),
    );
}

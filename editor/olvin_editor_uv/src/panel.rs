//! The UV Editor panel: toolbar, 2D canvas, selection, modal G/R/S, gizmo.

use olvin_editor::egui::{
    self, Align2, Color32, Event, FontId, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Stroke,
};
use olvin_editor::glam::Vec2;
use olvin_editor::{EditorState, Panel, theme};
use olvin_mesh::{PolyMesh, UvCorner};

use crate::ops::{self, Target, UvOp};
use crate::select::{Combine, Hit, SelectMode, Selection, Topo};
use crate::transform::{self, GIZMO_ARM, GIZMO_BOX, Kind, NumInput, Params};
use crate::view::View;
use crate::{Background, hooks};

const CHECKER: [Color32; 2] = [
    Color32::from_rgb(78, 82, 92),
    Color32::from_rgb(100, 105, 116),
];
const CHECKER_TILES: usize = 8;
const EDGE: Color32 = Color32::from_rgb(196, 202, 214);

fn p2(v: Vec2) -> Pos2 {
    Pos2::new(v.x, v.y)
}

fn v2(p: Pos2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

/// A G / R / S in progress: one undo step (`state.busy` while it runs).
struct Modal {
    kind: Kind,
    /// The mesh as it was (restored on cancel).
    original: PolyMesh,
    /// `original` with UVs ensured; every frame starts from it.
    before: PolyMesh,
    corners: Vec<UvCorner>,
    pivot: Vec2,
    start: Vec2,
    axis: Option<usize>,
    num: NumInput,
    /// Started by dragging the gizmo: confirms on release.
    drag: bool,
    summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    Op(UvOp),
    SelectAll,
    SelectNone,
    SelectInvert,
    SelectLinked,
}

pub struct UvEditor {
    pub view: View,
    framed: bool,
    pub mode: SelectMode,
    pub sel: Selection,
    entity: Option<u64>,
    active_face: Option<usize>,
    modal: Option<Modal>,
    boxing: Option<Pos2>,
    /// The press that confirmed a modal must not also select on release.
    swallow_click: bool,
    pub texture_px: u32,
    pub margin: f32,
    pub only_selected_3d: bool,
    pub gizmo: bool,
    /// Canvas centre of the last frame.
    last_origin: Vec2,
}

impl Default for UvEditor {
    fn default() -> Self {
        Self {
            view: View::default(),
            framed: false,
            mode: SelectMode::Vertex,
            sel: Selection::default(),
            entity: None,
            active_face: None,
            modal: None,
            boxing: None,
            swallow_click: false,
            texture_px: 1024,
            margin: 0.02,
            only_selected_3d: false,
            gizmo: true,
            last_origin: Vec2::ZERO,
        }
    }
}

/// The entity's mesh with UVs (box-projected on the fly if it has none yet).
fn display(state: &EditorState, id: u64) -> Option<PolyMesh> {
    let mut m = state.scene.get(id)?.poly.clone()?;
    m.ensure_uvs();
    Some(m)
}

fn poly_mut(state: &mut EditorState, id: u64) -> Option<&mut PolyMesh> {
    state.scene.get_mut(id)?.poly.as_mut()
}

fn chord(ui: &egui::Ui, mods: Modifiers, key: Key) -> bool {
    ui.input(|i| i.key_pressed(key) && i.modifiers.matches_exact(mods))
}

impl Panel for UvEditor {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let id = state
            .selection
            .filter(|id| state.scene.get(*id).is_some_and(|e| e.poly.is_some()));
        if id != self.entity {
            self.entity = id;
            self.sel.clear();
            self.active_face = None;
            self.modal = None;
            self.boxing = None;
        }
        let mut request = None;
        self.toolbar(ui, state, id, &mut request);

        let size = ui.available_size().max(egui::vec2(32.0, 32.0));
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 4.0, theme::VIEWPORT_BG);
        let origin = v2(rect.center());
        self.last_origin = origin;
        if !self.framed && rect.width() > 1.0 {
            self.view
                .frame(v2(rect.size().to_pos2()), Vec2::ZERO, Vec2::ONE);
            self.framed = true;
        }
        if self.modal.is_none() {
            self.navigate(ui, &response, origin);
        }

        let Some((id, mut mesh)) = id.and_then(|id| Some((id, display(state, id)?))) else {
            self.draw_background(&painter, origin, state, None);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Select a mesh object (Modeling: Make Editable) to edit its UVs",
                FontId::proportional(13.0),
                theme::TEXT_WEAK,
            );
            return;
        };
        let mut visible = self.visible(state, id, &mesh);
        self.sel.sanitize(&mesh);
        if response.hovered() {
            state.tool_owns_keys = true;
        }
        let keys =
            (response.hovered() || self.modal.is_some()) && !ui.ctx().egui_wants_keyboard_input();

        if self.modal.is_some() {
            state.busy = true;
            self.run_modal(ui, state, id, origin);
        } else {
            if keys {
                self.hotkeys(
                    ui,
                    state,
                    id,
                    &mesh,
                    &visible,
                    origin,
                    rect,
                    &response,
                    &mut request,
                );
            }
            if self.modal.is_none() {
                self.pointer(ui, &response, state, id, &mesh, &visible, origin);
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) && !response.clicked() {
            self.swallow_click = false;
        }
        if let Some(r) = request {
            self.run_request(r, state, id, &mesh, &visible);
        }
        if let Some(m) = display(state, id) {
            mesh = m;
            visible = self.visible(state, id, &mesh);
            self.sel.sanitize(&mesh);
        }
        let real_uvs = state
            .scene
            .get(id)
            .and_then(|e| e.poly.as_ref())
            .is_some_and(|p| p.has_uvs() || p.faces.is_empty());
        self.draw(
            ui, &painter, rect, &response, state, id, &mesh, visible, real_uvs,
        );
    }
}

impl UvEditor {
    fn visible(&self, state: &EditorState, id: u64, mesh: &PolyMesh) -> Vec<usize> {
        let all = || (0..mesh.faces.len()).collect();
        match hooks().face_filter {
            Some(filter) if self.only_selected_3d => filter(state, id).unwrap_or_else(all),
            _ => all(),
        }
    }

    // --- toolbar ------------------------------------------------------------

    fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        state: &EditorState,
        id: Option<u64>,
        request: &mut Option<Request>,
    ) {
        let busy = self.modal.is_some();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (mode, label) in SelectMode::ALL {
                    ui.selectable_value(&mut self.mode, mode, label);
                }
                ui.separator();
                let mut op = |ui: &mut egui::Ui, op: UvOp, label: &str| {
                    if ui.button(label).on_hover_text(op.tooltip()).clicked() {
                        *request = Some(Request::Op(op));
                        ui.close();
                    }
                };
                op(ui, UvOp::Unwrap, "Unwrap  U");
                ui.menu_button("Project", |ui| {
                    for o in UvOp::PROJECT {
                        op(ui, o, o.label());
                    }
                });
                ui.menu_button("Seams", |ui| {
                    for o in UvOp::SEAMS {
                        op(ui, o, o.label());
                    }
                });
                op(ui, UvOp::Pack, "Pack  Ctrl+P");
                ui.menu_button("Arrange", |ui| {
                    for o in UvOp::ARRANGE {
                        op(ui, o, o.label());
                    }
                });
                ui.menu_button("Select", |ui| {
                    for (r, label) in [
                        (Request::SelectAll, "All  A"),
                        (Request::SelectNone, "None  Alt+A"),
                        (Request::SelectInvert, "Invert  Ctrl+I"),
                        (Request::SelectLinked, "Linked  Ctrl+L"),
                    ] {
                        if ui.button(label).clicked() {
                            *request = Some(r);
                            ui.close();
                        }
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                let wired = hooks().face_filter.is_some();
                ui.add_enabled(wired, egui::Checkbox::new(&mut self.only_selected_3d, "Only selected in 3D"))
                    .on_hover_text("Show only the faces selected in Edit Mode (Blender's UV sync off)")
                    .on_disabled_hover_text(
                        "Not connected yet: the Modeling element has to provide its Edit Mode selection (olvin_editor_uv::set_face_filter)",
                    );
                ui.toggle_value(&mut self.gizmo, "Gizmo");
                ui.label("Texture");
                ui.add(egui::DragValue::new(&mut self.texture_px).range(16..=16384).suffix(" px"));
                ui.label("Margin");
                ui.add(egui::DragValue::new(&mut self.margin).range(0.0..=0.2).speed(0.001));
                if let Some(mesh) = id.and_then(|id| display(state, id)) {
                    let topo = Topo::new(&mesh, self.visible(state, id.unwrap_or(0), &mesh));
                    let mut faces = self.sel.faces(&topo);
                    let scope = if faces.is_empty() {
                        faces = topo.visible.clone();
                        "all"
                    } else {
                        "selected"
                    };
                    match mesh.texel_density(&faces, self.texture_px as f32) {
                        Some(d) => ui.label(format!("Texel density ({scope}): {d:.1} px/m")),
                        None => ui.weak("Texel density: —"),
                    };
                }
            });
        });
    }

    // --- navigation ----------------------------------------------------------

    fn navigate(&mut self, ui: &egui::Ui, response: &egui::Response, origin: Vec2) {
        let (scroll, zoom, alt) =
            ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta(), i.modifiers.alt));
        if response.hovered()
            && let Some(mouse) = response.hover_pos()
        {
            let factor = (scroll * 0.0015).exp() * zoom;
            if (factor - 1.0).abs() > 1e-6 {
                self.view.zoom_at(origin, v2(mouse), factor);
            }
        }
        if response.dragged_by(PointerButton::Middle)
            || (alt && response.dragged_by(PointerButton::Primary))
        {
            self.view.pan(v2(response.drag_delta().to_pos2()));
        }
    }

    fn frame(&mut self, rect: Rect, mesh: &PolyMesh, visible: &[usize], selected_only: bool) {
        let topo = Topo::new(mesh, visible.to_vec());
        let corners: Vec<UvCorner> = if selected_only && !self.sel.is_empty() {
            self.sel
                .corners
                .iter()
                .copied()
                .filter(|c| topo.is_shown(c.0))
                .collect()
        } else {
            topo.corners()
        };
        let mut pts: Vec<Vec2> = corners.iter().map(|&c| topo.uv(c)).collect();
        if !selected_only || pts.is_empty() {
            pts.extend([Vec2::ZERO, Vec2::ONE]);
        }
        let (lo, hi) = pts
            .iter()
            .fold((Vec2::MAX, Vec2::MIN), |(l, h), &p| (l.min(p), h.max(p)));
        self.view.frame(v2(rect.size().to_pos2()), lo, hi);
    }

    // --- hotkeys --------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn hotkeys(
        &mut self,
        ui: &egui::Ui,
        state: &mut EditorState,
        id: u64,
        mesh: &PolyMesh,
        visible: &[usize],
        origin: Vec2,
        rect: Rect,
        response: &egui::Response,
        request: &mut Option<Request>,
    ) {
        let none = Modifiers::NONE;
        for (key, mode) in [
            (Key::Num1, SelectMode::Vertex),
            (Key::Num2, SelectMode::Edge),
            (Key::Num3, SelectMode::Face),
            (Key::Num4, SelectMode::Island),
        ] {
            if chord(ui, none, key) {
                self.mode = mode;
            }
        }
        let r = if chord(ui, none, Key::A) {
            Some(Request::SelectAll)
        } else if chord(ui, Modifiers::ALT, Key::A) {
            Some(Request::SelectNone)
        } else if chord(ui, Modifiers::COMMAND, Key::I) {
            Some(Request::SelectInvert)
        } else if chord(ui, Modifiers::COMMAND, Key::L) {
            Some(Request::SelectLinked)
        } else if chord(ui, none, Key::U) {
            Some(Request::Op(UvOp::Unwrap))
        } else if chord(ui, Modifiers::COMMAND, Key::E) {
            Some(Request::Op(UvOp::MarkSeam))
        } else if chord(ui, Modifiers::COMMAND, Key::P) {
            Some(Request::Op(UvOp::Pack))
        } else {
            None
        };
        if r.is_some() {
            *request = r;
        }
        if chord(ui, none, Key::F) {
            self.frame(rect, mesh, visible, true);
        } else if chord(ui, none, Key::Home) {
            self.frame(rect, mesh, visible, false);
        }
        let mouse = response.hover_pos().map(v2);
        // L / Shift+L: (de)select the island under the cursor.
        for (mods, how) in [(none, Combine::Add), (Modifiers::SHIFT, Combine::Subtract)] {
            if chord(ui, mods, Key::L)
                && let Some(m) = mouse
            {
                let topo = Topo::new(mesh, visible.to_vec());
                let view = self.view;
                if let Some(hit) = topo.pick(SelectMode::Face, |t| view.to_screen(origin, t), m) {
                    let picked = topo.expand(hit, SelectMode::Island);
                    self.sel.combine(&picked, how);
                } else if let Some(hit) =
                    topo.pick(SelectMode::Vertex, |t| view.to_screen(origin, t), m)
                {
                    let (Hit::Vertex(c) | Hit::Edge(c)) = hit else {
                        continue;
                    };
                    let picked = topo.expand(Hit::Face(c.0), SelectMode::Island);
                    self.sel.combine(&picked, how);
                }
            }
        }
        let start = mouse.unwrap_or(origin);
        for (key, kind) in [
            (Key::G, Kind::Move),
            (Key::R, Kind::Rotate),
            (Key::S, Kind::Scale),
        ] {
            if chord(ui, none, key) {
                self.start_transform(
                    state,
                    id,
                    mesh,
                    visible,
                    kind,
                    self.view.to_uv(origin, start),
                    false,
                    None,
                );
            }
        }
    }

    // --- pointer ----------------------------------------------------------------

    /// Pivot of the selection (sticky corners' bounding-box centre).
    fn pivot(&self, topo: &Topo) -> Option<(Vec<UvCorner>, Vec2)> {
        let corners = topo.sticky(self.sel.corners.iter().copied());
        let pivot = transform::bounds_center(corners.iter().map(|&c| topo.uv(c)))?;
        Some((corners, pivot))
    }

    #[allow(clippy::too_many_arguments)]
    fn pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        state: &mut EditorState,
        id: u64,
        mesh: &PolyMesh,
        visible: &[usize],
        origin: Vec2,
    ) {
        let (alt, shift, ctrl) =
            ui.input(|i| (i.modifiers.alt, i.modifiers.shift, i.modifiers.command));
        let topo = Topo::new(mesh, visible.to_vec());
        let view = self.view;
        let to_screen = |t: Vec2| view.to_screen(origin, t);
        let gizmo_at = self
            .pivot(&topo)
            .filter(|_| self.gizmo)
            .map(|(_, p)| to_screen(p));

        if !alt && response.drag_started_by(PointerButton::Primary) {
            let press = ui
                .input(|i| i.pointer.press_origin())
                .or(response.hover_pos());
            let handle = gizmo_at
                .zip(press)
                .and_then(|(g, p)| transform::gizmo_hit(g, v2(p)));
            if let (Some(axis), Some(p)) = (handle, press) {
                let start = view.to_uv(origin, v2(p));
                self.start_transform(state, id, mesh, visible, Kind::Move, start, true, axis);
                return;
            }
            self.boxing = press;
        }
        if let Some(start) = self.boxing {
            if !ui.input(|i| i.pointer.primary_down()) {
                let end = response
                    .hover_pos()
                    .or(ui.input(|i| i.pointer.latest_pos()))
                    .unwrap_or(start);
                let (a, b) = (view.to_uv(origin, v2(start)), view.to_uv(origin, v2(end)));
                let (lo, hi) = (a.min(b), a.max(b));
                let picked = topo.boxed(self.mode, |t| t.cmpge(lo).all() && t.cmple(hi).all());
                let how = if ctrl {
                    Combine::Subtract
                } else if shift {
                    Combine::Add
                } else {
                    Combine::Replace
                };
                self.sel.combine(&picked, how);
                self.boxing = None;
            }
            return;
        }
        if response.clicked_by(PointerButton::Primary) && !alt && !self.swallow_click {
            let Some(mouse) = response.hover_pos().map(v2) else {
                return;
            };
            if gizmo_at.is_some_and(|g| transform::gizmo_hit(g, mouse).is_some()) {
                return;
            }
            match topo.pick(self.mode, to_screen, mouse) {
                Some(hit) => {
                    let picked = topo.expand(hit, self.mode);
                    self.active_face = Some(match hit {
                        Hit::Vertex(c) | Hit::Edge(c) => c.0,
                        Hit::Face(f) => f,
                    });
                    let how = if shift {
                        Combine::Toggle
                    } else {
                        Combine::Replace
                    };
                    self.sel.combine(&picked, how);
                }
                None if !shift => self.sel.clear(),
                None => {}
            }
        }
    }

    // --- modal transforms -------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn start_transform(
        &mut self,
        state: &mut EditorState,
        id: u64,
        mesh: &PolyMesh,
        visible: &[usize],
        kind: Kind,
        start: Vec2,
        drag: bool,
        axis: Option<usize>,
    ) {
        let topo = Topo::new(mesh, visible.to_vec());
        let Some((corners, pivot)) = self.pivot(&topo) else {
            return;
        };
        let Some(original) = state.scene.get(id).and_then(|e| e.poly.clone()) else {
            return;
        };
        let mut before = original.clone();
        before.ensure_uvs();
        self.boxing = None;
        self.modal = Some(Modal {
            kind,
            original,
            before,
            corners,
            pivot,
            start,
            axis,
            num: NumInput::default(),
            drag,
            summary: kind.label().to_owned(),
        });
        state.busy = true;
    }

    fn run_modal(&mut self, ui: &egui::Ui, state: &mut EditorState, id: u64, origin: Vec2) {
        let view = self.view;
        let Some(modal) = &mut self.modal else { return };
        struct In {
            enter: bool,
            cancel: bool,
            click: bool,
            released: bool,
            snap: bool,
            mouse: Option<Pos2>,
            chars: Vec<char>,
            backspace: bool,
            axis: Option<usize>,
            switch: Option<Kind>,
        }
        let input = ui.input(|i| {
            let plain = !i.modifiers.command && !i.modifiers.alt && !i.modifiers.shift;
            let key = |k| plain && i.key_pressed(k);
            In {
                enter: i.key_pressed(Key::Enter) || i.key_pressed(Key::Space),
                cancel: i.pointer.button_pressed(PointerButton::Secondary)
                    || i.key_pressed(Key::Escape),
                click: i.pointer.button_pressed(PointerButton::Primary),
                released: !i.pointer.primary_down(),
                snap: i.modifiers.command,
                mouse: i.pointer.latest_pos(),
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
                axis: [(Key::X, 0), (Key::Y, 1)]
                    .into_iter()
                    .find(|(k, _)| key(*k))
                    .map(|(_, a)| a),
                switch: [
                    (Key::G, Kind::Move),
                    (Key::R, Kind::Rotate),
                    (Key::S, Kind::Scale),
                ]
                .into_iter()
                .find(|(k, _)| key(*k))
                .map(|(_, k)| k),
            }
        });
        if input.cancel {
            let original = modal.original.clone();
            if let Some(e) = state.scene.get_mut(id) {
                e.poly = Some(original);
            }
            self.modal = None;
            self.swallow_click = true;
            return;
        }
        if let Some(a) = input.axis {
            modal.axis = if modal.axis == Some(a) { None } else { Some(a) };
        }
        if let Some(k) = input.switch
            && k != modal.kind
        {
            modal.kind = k;
            modal.num = NumInput::default();
        }
        for c in input.chars {
            modal.num.type_char(c);
        }
        if input.backspace {
            modal.num.backspace();
        }
        let mouse = input
            .mouse
            .map(|p| view.to_uv(origin, v2(p)))
            .unwrap_or(modal.start);
        let params = Params {
            kind: modal.kind,
            pivot: modal.pivot,
            start: modal.start,
            mouse,
            axis: modal.axis,
            typed: modal.num.value(),
            snap: input.snap,
        };
        let (m, mut summary) = transform::matrix(&params);
        if modal.num.is_active() {
            summary.push_str(&format!("   [{}|]", modal.num.text()));
        }
        modal.summary = summary;
        let mut mesh = modal.before.clone();
        mesh.transform_uvs(&modal.corners, m);
        if let Some(e) = state.scene.get_mut(id) {
            e.poly = Some(mesh);
        }
        let confirm = input.enter
            || if modal.drag {
                input.released
            } else {
                input.click
            };
        if confirm {
            self.swallow_click = !modal.drag;
            self.modal = None;
        }
    }

    // --- operations ---------------------------------------------------------------

    fn run_request(
        &mut self,
        r: Request,
        state: &mut EditorState,
        id: u64,
        mesh: &PolyMesh,
        visible: &[usize],
    ) {
        let topo = Topo::new(mesh, visible.to_vec());
        match r {
            Request::SelectAll => self.sel.combine(&topo.corners(), Combine::Replace),
            Request::SelectNone => self.sel.clear(),
            Request::SelectInvert => {
                let rest: Vec<UvCorner> = topo
                    .corners()
                    .into_iter()
                    .filter(|c| !self.sel.contains(*c))
                    .collect();
                self.sel.combine(&rest, Combine::Replace);
            }
            Request::SelectLinked => {
                let faces: Vec<usize> = self.sel.corners.iter().map(|c| c.0).collect();
                let picked = topo.face_corners(&topo.islands_of(&faces));
                self.sel.combine(&picked, Combine::Add);
            }
            Request::Op(op) => {
                let selected = self.sel.faces(&topo);
                let target = Target {
                    selected: !selected.is_empty(),
                    faces: if selected.is_empty() {
                        topo.visible.clone()
                    } else {
                        selected
                    },
                    edges: self.sel.edges(&topo),
                    corners: topo.sticky(self.sel.corners.iter().copied()),
                    active: self.active_face,
                };
                let margin = self.margin;
                let Some(poly) = poly_mut(state, id) else {
                    return;
                };
                match ops::apply(poly, op, &target, margin) {
                    Ok(msg) if !msg.is_empty() => state.info(format!("UV: {msg}")),
                    Ok(_) => {}
                    Err(e) => state.warn(format!("UV {}: {e}", op.label())),
                }
            }
        }
    }

    // --- drawing ----------------------------------------------------------------------

    /// The 0..1 square's background: the [`crate::set_background`] hook, or
    /// a procedural checker until a texture can be drawn.
    fn draw_background(
        &self,
        painter: &egui::Painter,
        origin: Vec2,
        state: &EditorState,
        id: Option<u64>,
    ) {
        let view = self.view;
        let a = view.to_screen(origin, Vec2::new(0.0, 1.0));
        let b = view.to_screen(origin, Vec2::new(1.0, 0.0));
        let square = Rect::from_min_max(p2(a), p2(b));
        let hook: Option<Background> = hooks().background;
        let drawn = match (hook, id) {
            (Some(draw), Some(id)) => draw(painter, square, state, id),
            _ => false,
        };
        if !drawn {
            background(painter, square);
        }
        painter.rect_stroke(
            square,
            0.0,
            Stroke::new(1.0, theme::TEXT_WEAK),
            egui::StrokeKind::Outside,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        response: &egui::Response,
        state: &EditorState,
        id: u64,
        mesh: &PolyMesh,
        visible: Vec<usize>,
        real_uvs: bool,
    ) {
        let origin = v2(rect.center());
        let view = self.view;
        let to = |t: Vec2| p2(view.to_screen(origin, t));
        self.draw_background(painter, origin, state, Some(id));
        self.draw_grids(painter, rect, origin);

        let topo = Topo::new(mesh, visible);
        let hover = response
            .hover_pos()
            .filter(|_| self.modal.is_none() && self.boxing.is_none())
            .and_then(|m| topo.pick(self.mode, |t| view.to_screen(origin, t), v2(m)));

        // Faces.
        let selected_faces = self.sel.faces(&topo);
        let mut fill = egui::Mesh::default();
        for &f in &topo.visible {
            let color = if selected_faces.contains(&f) {
                theme::SELECTED.gamma_multiply(0.28)
            } else if hover == Some(Hit::Face(f)) {
                Color32::from_white_alpha(30)
            } else {
                Color32::from_white_alpha(10)
            };
            let base = fill.vertices.len() as u32;
            for &t in &mesh.uvs[f] {
                fill.colored_vertex(to(t), color);
            }
            for [a, b, c] in topo.triangles(f) {
                fill.add_triangle(base + a, base + b, base + c);
            }
        }
        painter.add(egui::Shape::mesh(fill));

        // Edges: seams red, selected orange, hovered white.
        for c in topo.corners() {
            let n = topo.next(c);
            let face = &mesh.faces[c.0];
            let (a, b) = (face[c.1], face[n.1]);
            let stroke = if hover == Some(Hit::Edge(c)) {
                Stroke::new(2.5, Color32::WHITE)
            } else if self.sel.has_edge(&topo, c) {
                Stroke::new(1.8, theme::SELECTED)
            } else if mesh.is_seam(a, b) {
                Stroke::new(1.8, theme::AXIS_X)
            } else {
                Stroke::new(1.0, EDGE)
            };
            painter.line_segment([to(topo.uv(c)), to(topo.uv(n))], stroke);
        }

        match self.mode {
            SelectMode::Vertex | SelectMode::Edge => {
                for c in topo.corners() {
                    let (r, color) = if self.sel.contains(c) {
                        (3.5, theme::SELECTED)
                    } else if hover == Some(Hit::Vertex(c)) {
                        (4.0, Color32::WHITE)
                    } else {
                        (2.0, Color32::from_gray(25))
                    };
                    painter.circle_filled(to(topo.uv(c)), r, color);
                }
            }
            SelectMode::Face | SelectMode::Island => {
                for &f in &topo.visible {
                    let color = if selected_faces.contains(&f) {
                        theme::SELECTED
                    } else {
                        Color32::from_gray(30)
                    };
                    painter.circle_filled(to(topo.center(f)), 2.5, color);
                }
            }
        }

        if let Some(start) = self.boxing {
            let end = response.hover_pos().unwrap_or(start);
            let r = Rect::from_two_pos(start, end);
            painter.rect_filled(r, 0.0, Color32::from_white_alpha(12));
            painter.rect_stroke(
                r,
                0.0,
                (1.0, Color32::from_white_alpha(200)),
                egui::StrokeKind::Middle,
            );
        }

        // Gizmo or modal guides.
        if let Some(modal) = &self.modal {
            let p = view.to_screen(origin, modal.pivot);
            if let Some(axis) = modal.axis {
                let dir = if axis == 0 { Vec2::X } else { Vec2::NEG_Y };
                let far = rect.width().max(rect.height()) * 2.0;
                painter.line_segment(
                    [p2(p - dir * far), p2(p + dir * far)],
                    Stroke::new(1.5, theme::AXES[axis]),
                );
            }
            painter.circle_stroke(p2(p), 4.0, Stroke::new(1.5, Color32::WHITE));
            if modal.kind != Kind::Move
                && let Some(m) = ui.input(|i| i.pointer.latest_pos())
            {
                painter.add(egui::Shape::dashed_line(
                    &[p2(p), m],
                    Stroke::new(1.0, Color32::from_white_alpha(160)),
                    5.0,
                    4.0,
                ));
            }
        } else if self.gizmo
            && self.boxing.is_none()
            && let Some((_, pivot)) = self.pivot(&topo)
        {
            let c = view.to_screen(origin, pivot);
            let hot = response
                .hover_pos()
                .and_then(|m| transform::gizmo_hit(c, v2(m)));
            for (axis, dir) in [(0usize, Vec2::X), (1, Vec2::NEG_Y)] {
                let color = if hot == Some(Some(axis)) {
                    Color32::WHITE
                } else {
                    theme::AXES[axis]
                };
                let end = c + dir * GIZMO_ARM;
                painter.line_segment([p2(c), p2(end)], Stroke::new(2.5, color));
                let side = dir.perp() * 5.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![p2(end + dir * 10.0), p2(end + side), p2(end - side)],
                    color,
                    Stroke::NONE,
                ));
            }
            let color = if hot == Some(None) {
                Color32::WHITE
            } else {
                theme::ACCENT
            };
            painter.rect_stroke(
                Rect::from_center_size(p2(c), egui::vec2(GIZMO_BOX * 2.0, GIZMO_BOX * 2.0)),
                2.0,
                Stroke::new(1.5, color),
                egui::StrokeKind::Middle,
            );
        }

        // Status lines.
        let status = match &self.modal {
            Some(m) => format!(
                "{}   X/Y axis · type a value · Ctrl snap · LMB/Enter confirm · RMB/Esc cancel",
                m.summary
            ),
            None => format!(
                "{} corners selected · G/R/S · U unwrap · L island · A all · F/Home frame · MMB/Alt+LMB pan · wheel zoom",
                self.sel.corners.len()
            ),
        };
        painter.text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            status,
            FontId::proportional(12.0),
            theme::TEXT,
        );
        if !real_uvs {
            painter.text(
                rect.left_top() + egui::vec2(8.0, 8.0),
                Align2::LEFT_TOP,
                "No UVs yet: showing a box projection. Unwrap (U) or project to create them.",
                FontId::proportional(12.0),
                ui.visuals().warn_fg_color,
            );
        }
    }

    /// Faint lines on whole UV tiles, and the 1/64 pixel grid when zoomed in.
    fn draw_grids(&self, painter: &egui::Painter, rect: Rect, origin: Vec2) {
        let view = self.view;
        let lo = view.to_uv(origin, v2(rect.left_bottom()));
        let hi = view.to_uv(origin, v2(rect.right_top()));
        let lines = |step: f32, color: Color32, limit: f32| {
            if (hi.x - lo.x) / step > limit || (hi.y - lo.y) / step > limit {
                return;
            }
            let mut u = (lo.x / step).floor() * step;
            while u <= hi.x {
                let (a, b) = (
                    view.to_screen(origin, Vec2::new(u, lo.y)),
                    view.to_screen(origin, Vec2::new(u, hi.y)),
                );
                painter.line_segment([p2(a), p2(b)], Stroke::new(1.0, color));
                u += step;
            }
            let mut v = (lo.y / step).floor() * step;
            while v <= hi.y {
                let (a, b) = (
                    view.to_screen(origin, Vec2::new(lo.x, v)),
                    view.to_screen(origin, Vec2::new(hi.x, v)),
                );
                painter.line_segment([p2(a), p2(b)], Stroke::new(1.0, color));
                v += step;
            }
        };
        lines(1.0, Color32::from_white_alpha(22), 200.0);
        if view.zoom * transform::PIXEL >= 10.0 {
            lines(transform::PIXEL, Color32::from_black_alpha(40), 400.0);
        }
    }
}

/// Procedural checker filling `square` (the 0..1 UV square on screen).
/// This is the fallback of the [`crate::Background`] hook: once materials can
/// hand their base texture to the editor, a hook draws that instead.
pub fn background(painter: &egui::Painter, square: Rect) {
    let n = CHECKER_TILES;
    let step = square.size() / n as f32;
    painter.rect_filled(square, 0.0, CHECKER[0]);
    for i in 0..n {
        for j in 0..n {
            if (i + j) % 2 == 1 {
                let min = square.min + egui::vec2(step.x * i as f32, step.y * j as f32);
                painter.rect_filled(Rect::from_min_size(min, step), 0.0, CHECKER[1]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_editor::NewEntity;

    fn frame(
        ctx: &egui::Context,
        panel: &mut UvEditor,
        state: &mut EditorState,
        events: Vec<Event>,
    ) {
        // egui takes modifiers from ModifiersChanged events, not from key events.
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let mut events = events;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 700.0))),
            events,
            ..Default::default()
        };
        ctx.run_ui(input, |ui| panel.ui(ui, state))
            .textures_delta
            .clear();
    }

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn mesh(state: &EditorState, id: u64) -> &PolyMesh {
        state.scene.get(id).unwrap().poly.as_ref().unwrap()
    }

    #[test]
    fn draws_headless_and_runs_hotkeys() {
        let mut state = EditorState::new(None);
        let ctx = egui::Context::default();
        let mut panel = UvEditor::default();
        // Nothing selected: just the hint.
        frame(&ctx, &mut panel, &mut state, vec![]);
        let id = state.create(NewEntity::Empty, None);
        state.scene.get_mut(id).unwrap().poly = Some(PolyMesh::cube(1.0));
        let at = Pos2::new(400.0, 400.0);
        let hover = || Event::PointerMoved(at);
        for _ in 0..2 {
            frame(&ctx, &mut panel, &mut state, vec![hover()]);
        }
        assert!(!mesh(&state, id).has_uvs(), "drawing does not write UVs");
        // A selects everything, U unwraps it into six islands.
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![hover(), key(Key::A, Modifiers::NONE)],
        );
        assert_eq!(panel.sel.corners.len(), 24);
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![hover(), key(Key::U, Modifiers::NONE)],
        );
        assert!(mesh(&state, id).has_uvs());
        assert_eq!(mesh(&state, id).islands().len(), 6);
        let before = mesh(&state, id).uvs.clone();

        // S, type 0.5, Enter: everything scales about the pivot.
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![hover(), key(Key::S, Modifiers::NONE)],
        );
        assert!(panel.modal.is_some() && state.busy);
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![hover(), Event::Text("0.5".into())],
        );
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![hover(), key(Key::Enter, Modifiers::NONE)],
        );
        assert!(panel.modal.is_none());
        let after = &mesh(&state, id).uvs;
        let span = |uvs: &Vec<Vec<Vec2>>| {
            let (l, h) = uvs
                .iter()
                .flatten()
                .fold((Vec2::MAX, Vec2::MIN), |(l, h), &t| (l.min(t), h.max(t)));
            h - l
        };
        assert!((span(after) * 2.0).abs_diff_eq(span(&before), 1e-4));

        // G, move the mouse, Esc: cancelled, nothing changed.
        let scaled = mesh(&state, id).clone();
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![hover(), key(Key::G, Modifiers::NONE)],
        );
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![Event::PointerMoved(Pos2::new(460.0, 380.0))],
        );
        assert_ne!(mesh(&state, id), &scaled);
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![key(Key::Escape, Modifiers::NONE)],
        );
        assert_eq!(mesh(&state, id), &scaled);

        // G X 0.25 Enter: a typed move along U.
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![hover(), key(Key::G, Modifiers::NONE)],
        );
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![
                hover(),
                key(Key::X, Modifiers::NONE),
                Event::Text("0.25".into()),
            ],
        );
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![hover(), key(Key::Enter, Modifiers::NONE)],
        );
        let moved = &mesh(&state, id).uvs;
        for (a, b) in moved.iter().flatten().zip(scaled.uvs.iter().flatten()) {
            assert!(a.abs_diff_eq(*b + Vec2::new(0.25, 0.0), 1e-5));
        }

        // Island mode, Alt+A, then Ctrl+P packs back into 0..1; every mode draws.
        for (k, mode) in [
            (Key::Num2, SelectMode::Edge),
            (Key::Num3, SelectMode::Face),
            (Key::Num4, SelectMode::Island),
        ] {
            frame(
                &ctx,
                &mut panel,
                &mut state,
                vec![hover(), key(k, Modifiers::NONE)],
            );
            assert_eq!(panel.mode, mode);
        }
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![hover(), key(Key::A, Modifiers::ALT)],
        );
        assert!(panel.sel.is_empty());
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![hover(), key(Key::P, Modifiers::COMMAND)],
        );
        let m = mesh(&state, id);
        assert!(
            m.uvs
                .iter()
                .flatten()
                .all(|t| t.x >= -1e-4 && t.x <= 1.0001 && t.y >= -1e-4 && t.y <= 1.0001)
        );
        m.validate().unwrap();
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![
                hover(),
                key(Key::F, Modifiers::NONE),
                key(Key::Home, Modifiers::NONE),
            ],
        );
    }

    #[test]
    fn click_and_box_select_on_canvas() {
        let mut state = EditorState::new(None);
        let ctx = egui::Context::default();
        let mut panel = UvEditor::default();
        let id = state.create(NewEntity::Empty, None);
        let mut plane = PolyMesh::plane(2.0);
        plane.reset_uvs(&[0]);
        state.scene.get_mut(id).unwrap().poly = Some(plane);
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![Event::PointerMoved(Pos2::new(400.0, 400.0))],
        );
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![Event::PointerMoved(Pos2::new(400.0, 400.0))],
        );
        // Where is UV (0, 0) on screen? Use the panel's own view.
        let canvas_center = panel.last_origin;
        let corner = p2(panel.view.to_screen(canvas_center, Vec2::ZERO));
        let press = |pressed| Event::PointerButton {
            pos: corner,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(
            &ctx,
            &mut panel,
            &mut state,
            vec![Event::PointerMoved(corner), press(true)],
        );
        frame(&ctx, &mut panel, &mut state, vec![press(false)]);
        frame(&ctx, &mut panel, &mut state, vec![]);
        assert_eq!(panel.sel.corners.len(), 1, "clicked the (0, 0) corner");

        // Box from outside the square across everything selects all 4 corners.
        let a = p2(panel.view.to_screen(canvas_center, Vec2::splat(-0.05)));
        let b = p2(panel.view.to_screen(canvas_center, Vec2::splat(1.05)));
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut events = vec![vec![Event::PointerMoved(a), button(a, true)]];
        for k in 1..=6 {
            events.push(vec![Event::PointerMoved(a + (b - a) * (k as f32 / 6.0))]);
        }
        events.push(vec![button(b, false)]);
        events.push(vec![]);
        for e in events {
            frame(&ctx, &mut panel, &mut state, e);
        }
        assert_eq!(panel.sel.corners.len(), 4);
    }
}

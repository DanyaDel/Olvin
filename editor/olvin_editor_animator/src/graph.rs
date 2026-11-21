//! The Graph Editor: F-curves drawn as they are evaluated, with key points
//! and Bezier handles. Select keys and handles (click, Shift+click, box),
//! grab them (G, or drag; X / Y constrain), scale, delete, duplicate,
//! interpolation (T), handle type (V), view all (Home).

use std::collections::HashSet;

use olvin_editor::egui::{
    self, Align2, Color32, FontId, Key, Pos2, Rect, Sense, Shape, pos2, vec2,
};
use olvin_editor::olvin_scene::action::{
    Action, HandleType, Interp, Prop, SEL_ALL, SEL_KEY, SEL_LEFT, SEL_RIGHT,
};
use olvin_editor::{EditorState, Panel};

use crate::clock;
use crate::modal::{Kind, Modal, Outcome};
use crate::ops;
use crate::view::{self, Axis, KEY_SELECTED, RULER_H};

const NAMES_W: f32 = 190.0;
const ROW_H: f32 = 22.0;
const PICK_R: f32 = 9.0;

/// Blender's X / Y / Z axis colours.
const AXIS: [Color32; 3] = [
    Color32::from_rgb(237, 78, 100),
    Color32::from_rgb(139, 220, 0),
    Color32::from_rgb(64, 156, 255),
];

type CurveId = (u64, Prop, u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    Key(usize, usize),
    Left(usize, usize),
    Right(usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmd {
    SelectAll,
    SelectNone,
    Invert,
    Grab,
    Scale,
    Duplicate,
    Delete,
    Copy,
    Paste,
    Interp(Interp),
    Handle(HandleType),
    InsertKeys,
    ViewAll,
    BoxSelect,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Interp,
    Handle,
}

pub struct GraphEditor {
    x: Axis,
    y: Axis,
    only_selected: bool,
    only_selected_handles: bool,
    hidden: HashSet<CurveId>,
    modal: Option<Modal>,
    box_start: Option<Pos2>,
    box_armed: bool,
    scrubbing: bool,
    menu: Option<(Menu, Pos2)>,
    fitted: bool,
}

impl Default for GraphEditor {
    fn default() -> Self {
        Self {
            x: Axis {
                origin: -5.0,
                scale: 4.0,
            },
            y: Axis {
                origin: -1.0,
                scale: -40.0,
            },
            only_selected: true,
            only_selected_handles: false,
            hidden: HashSet::new(),
            modal: None,
            box_start: None,
            box_armed: false,
            scrubbing: false,
            menu: None,
            fitted: false,
        }
    }
}

const INTERPS: [(&str, Interp); 3] = [
    ("Constant", Interp::Constant),
    ("Linear", Interp::Linear),
    ("Bezier", Interp::Bezier),
];

const HANDLES: [(&str, HandleType); 5] = [
    ("Automatic", HandleType::Auto),
    ("Auto Clamped", HandleType::AutoClamped),
    ("Vector", HandleType::Vector),
    ("Aligned", HandleType::Aligned),
    ("Free", HandleType::Free),
];

impl Panel for GraphEditor {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let cmd = self.header(ui);
        let rect = ui.available_rect_before_wrap();
        if rect.width() < NAMES_W + 60.0 || rect.height() < RULER_H + 40.0 {
            return;
        }
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let graph = Rect::from_min_max(pos2(rect.left() + NAMES_W, rect.top()), rect.max);
        let ruler_rect = Rect::from_min_size(graph.min, vec2(graph.width(), RULER_H));
        let body = Rect::from_min_max(pos2(graph.left(), graph.top() + RULER_H), graph.max);
        if !self.fitted {
            self.fitted = true;
            self.view_all(state, body);
        }
        let ctx = ui.ctx().clone();
        let curves = self.shown(state);
        self.navigate(ui, &response, body);
        if response.hovered() {
            state.tool_owns_keys = true;
        }

        let pointer = response.hover_pos();
        let to_view = |p: Pos2| {
            (
                self.x.value(p.x, body.left()),
                self.y.value(p.y, body.bottom()),
            )
        };
        let mouse = pointer.map_or((0.0, 0.0), to_view);
        let current = state.anim_frame as f32;
        let mut cmd = cmd;

        if let Some(modal) = &mut self.modal {
            state.busy = true;
            state.tool_owns_keys = true;
            let outcome = modal.input(&ctx, response.drag_stopped());
            let ctrl = ui.input(|i| i.modifiers.ctrl);
            let result = modal.result(mouse, current, ctrl);
            match outcome {
                Outcome::Running | Outcome::Confirm => state.scene.animation = result,
                Outcome::Cancel => state.scene.animation = modal.snapshot.clone(),
            }
            if outcome != Outcome::Running {
                self.modal = None;
            }
        } else {
            cmd = cmd.or_else(|| self.keyboard(ui, &response));
            self.pointer(ui, state, &response, &curves, ruler_rect, body);
        }
        if let Some(cmd) = cmd {
            self.run(cmd, state, &curves, mouse, current, body);
        }

        self.paint(ui, state, &curves, rect, graph, ruler_rect, body);
        self.menu_popup(&ctx, state);
        self.channel_clicks(state, &response, rect);
    }
}

impl GraphEditor {
    fn header(&mut self, ui: &mut egui::Ui) -> Option<Cmd> {
        let mut cmd = None;
        ui.horizontal(|ui| {
            ui.menu_button("Select", |ui| {
                for (label, c) in [
                    ("All  (A)", Cmd::SelectAll),
                    ("None  (Alt+A)", Cmd::SelectNone),
                    ("Invert  (Ctrl+I)", Cmd::Invert),
                    ("Box Select  (B)", Cmd::BoxSelect),
                ] {
                    if ui.button(label).clicked() {
                        cmd = Some(c);
                        ui.close();
                    }
                }
            });
            ui.menu_button("Key", |ui| {
                for (label, c) in [
                    ("Insert Keyframes  (I)", Cmd::InsertKeys),
                    ("Grab  (G)", Cmd::Grab),
                    ("Scale  (S)", Cmd::Scale),
                    ("Duplicate  (Shift+D)", Cmd::Duplicate),
                    ("Delete Keyframes  (X)", Cmd::Delete),
                    ("Copy  (Ctrl+C)", Cmd::Copy),
                    ("Paste  (Ctrl+V)", Cmd::Paste),
                ] {
                    if ui.button(label).clicked() {
                        cmd = Some(c);
                        ui.close();
                    }
                }
                ui.menu_button("Interpolation Mode  (T)", |ui| {
                    for (label, i) in INTERPS {
                        if ui.button(label).clicked() {
                            cmd = Some(Cmd::Interp(i));
                            ui.close();
                        }
                    }
                });
                ui.menu_button("Handle Type  (V)", |ui| {
                    for (label, h) in HANDLES {
                        if ui.button(label).clicked() {
                            cmd = Some(Cmd::Handle(h));
                            ui.close();
                        }
                    }
                });
            });
            if ui.button("View All  (Home)").clicked() {
                cmd = Some(Cmd::ViewAll);
            }
            ui.checkbox(&mut self.only_selected, "Only Show Selected");
            ui.checkbox(&mut self.only_selected_handles, "Only Selected Handles");
            if let Some(m) = &self.modal {
                ui.label(m.status());
            }
        });
        ui.separator();
        cmd
    }

    /// Curves drawn: visible objects minus the hidden channels.
    fn shown(&self, state: &EditorState) -> Vec<usize> {
        ops::visible_curves(state, self.only_selected)
            .into_iter()
            .filter(|&c| {
                let k = &state.scene.animation.curves[c];
                !self.hidden.contains(&(k.object, k.prop, k.index))
            })
            .collect()
    }

    fn navigate(&mut self, ui: &egui::Ui, response: &egui::Response, body: Rect) {
        let (ctrl, shift) = ui.input(|i| (i.modifiers.ctrl, i.modifiers.shift));
        if response.dragged_by(egui::PointerButton::Middle) {
            let d = response.drag_delta();
            if ctrl {
                let at = response.interact_pointer_pos().unwrap_or(body.center());
                self.x
                    .zoom(at.x, body.left(), (1.0 + d.x * 0.01).max(0.1), (0.2, 400.0));
                self.y.zoom(
                    at.y,
                    body.bottom(),
                    (1.0 - d.y * 0.01).max(0.1),
                    (0.001, 5000.0),
                );
            } else {
                self.x.pan(d.x);
                self.y.pan(d.y);
            }
        }
        if response.hovered() && response.hover_pos().is_some_and(|p| body.contains(p)) {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if wheel != 0.0 {
                let at = response.hover_pos().unwrap_or(body.center());
                if ctrl {
                    self.x.pan(wheel);
                } else if shift {
                    self.y.pan(wheel);
                } else {
                    let f = (1.0 + wheel * 0.002_f32).clamp(0.5, 2.0);
                    self.x.zoom(at.x, body.left(), f, (0.2, 400.0));
                    self.y.zoom(at.y, body.bottom(), f, (0.001, 5000.0));
                }
            }
        }
    }

    fn view_all(&mut self, state: &EditorState, body: Rect) {
        let a = &state.scene.animation;
        let curves = self.shown(state);
        let mut lo = (f32::MAX, f32::MAX);
        let mut hi = (f32::MIN, f32::MIN);
        for &c in &curves {
            for k in &a.curves[c].keys {
                for p in [[k.frame, k.value], k.left, k.right] {
                    lo = (lo.0.min(p[0]), lo.1.min(p[1]));
                    hi = (hi.0.max(p[0]), hi.1.max(p[1]));
                }
            }
        }
        if lo.0 > hi.0 {
            lo = (a.frame_start as f32, -1.0);
            hi = (a.frame_end as f32, 1.0);
        }
        if hi.1 - lo.1 < 1e-3 {
            lo.1 -= 1.0;
            hi.1 += 1.0;
        }
        if hi.0 - lo.0 < 1.0 {
            hi.0 = lo.0 + 10.0;
        }
        self.x = Axis::fit(lo.0, hi.0, body.width(), 30.0, false);
        self.y = Axis::fit(lo.1, hi.1, body.height(), 30.0, true);
    }

    fn keyboard(&mut self, ui: &egui::Ui, response: &egui::Response) -> Option<Cmd> {
        if !response.hovered() || ui.ctx().egui_wants_keyboard_input() {
            return None;
        }
        if ui.input(|i| i.key_pressed(Key::T) && i.modifiers.is_none()) {
            self.menu = response.hover_pos().map(|p| (Menu::Interp, p));
        }
        if ui.input(|i| i.key_pressed(Key::V) && i.modifiers.is_none()) {
            self.menu = response.hover_pos().map(|p| (Menu::Handle, p));
        }
        ui.input(|i| {
            let m = i.modifiers;
            let plain = m.is_none();
            let key = |k| i.key_pressed(k);
            if key(Key::A) && plain {
                Some(Cmd::SelectAll)
            } else if key(Key::A) && m.matches_exact(egui::Modifiers::ALT) {
                Some(Cmd::SelectNone)
            } else if key(Key::I) && m.matches_exact(egui::Modifiers::CTRL) {
                Some(Cmd::Invert)
            } else if key(Key::B) && plain {
                Some(Cmd::BoxSelect)
            } else if key(Key::G) && plain {
                Some(Cmd::Grab)
            } else if key(Key::S) && plain {
                Some(Cmd::Scale)
            } else if key(Key::D) && m.matches_exact(egui::Modifiers::SHIFT) {
                Some(Cmd::Duplicate)
            } else if (key(Key::X) || key(Key::Delete)) && plain {
                Some(Cmd::Delete)
            } else if key(Key::I) && plain {
                Some(Cmd::InsertKeys)
            } else if key(Key::Home) && plain {
                Some(Cmd::ViewAll)
            } else if i.events.iter().any(|e| matches!(e, egui::Event::Copy)) {
                Some(Cmd::Copy)
            } else if i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))
                || (key(Key::V) && m.command)
            {
                Some(Cmd::Paste)
            } else {
                None
            }
        })
    }

    fn run(
        &mut self,
        cmd: Cmd,
        state: &mut EditorState,
        curves: &[usize],
        mouse: (f32, f32),
        current: f32,
        body: Rect,
    ) {
        let frame = state.anim_frame as f32;
        let selected = state.scene.animation.selected_count();
        let any_selected = state
            .scene
            .animation
            .curves
            .iter()
            .flat_map(|c| &c.keys)
            .any(|k| k.select != 0);
        let a = &mut state.scene.animation;
        match cmd {
            Cmd::SelectAll => a.select_all(true),
            Cmd::SelectNone => a.select_all(false),
            Cmd::Invert => ops::invert_selection(a),
            Cmd::BoxSelect => self.box_armed = true,
            Cmd::Grab | Cmd::Scale if any_selected => {
                let kind = if cmd == Cmd::Grab {
                    Kind::Grab
                } else {
                    Kind::Scale
                };
                self.modal = Some(Modal::new(kind, a.clone(), mouse, false, true));
            }
            Cmd::Duplicate if selected > 0 => {
                a.duplicate_selected();
                self.modal = Some(Modal::new(Kind::Grab, a.clone(), mouse, false, true));
            }
            Cmd::Delete => a.delete_selected(),
            Cmd::Copy => {
                let clip = a.copy_selected();
                clock::with(|s| s.clipboard = clip);
            }
            Cmd::Paste => {
                let clip = clock::with(|s| s.clipboard.clone());
                a.paste(&clip, current);
            }
            Cmd::Interp(i) => a.set_interp_selected(i),
            Cmd::Handle(h) => a.set_handle_type_selected(h),
            Cmd::InsertKeys => {
                for &c in curves {
                    let curve = &a.curves[c];
                    if let Some(v) = curve.eval(frame) {
                        let (o, p, i) = (curve.object, curve.prop, curve.index);
                        a.insert_key(o, p, i, frame, v);
                    }
                }
            }
            Cmd::ViewAll => self.view_all(state, body),
            _ => {}
        }
    }

    /// The key or handle nearest to `p` within the pick radius.
    fn pick(&self, action: &Action, curves: &[usize], p: Pos2, body: Rect) -> Option<Pick> {
        let px = |pt: [f32; 2]| {
            pos2(
                self.x.px(pt[0], body.left()),
                self.y.px(pt[1], body.bottom()),
            )
        };
        let mut best: Option<(f32, Pick)> = None;
        let mut consider = |d: f32, pick: Pick| {
            if d <= PICK_R && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, pick));
            }
        };
        for &c in curves {
            for (k, key) in action.curves[c].keys.iter().enumerate() {
                consider(px([key.frame, key.value]).distance(p), Pick::Key(c, k));
                if self.handles_shown(key.select) {
                    // Handles win ties against their own key.
                    consider(px(key.left).distance(p) - 0.5, Pick::Left(c, k));
                    consider(px(key.right).distance(p) - 0.5, Pick::Right(c, k));
                }
            }
        }
        best.map(|(_, pick)| pick)
    }

    fn handles_shown(&self, select: u8) -> bool {
        !self.only_selected_handles || select != 0
    }

    fn pointer(
        &mut self,
        ui: &egui::Ui,
        state: &mut EditorState,
        response: &egui::Response,
        curves: &[usize],
        ruler_rect: Rect,
        body: Rect,
    ) {
        let shift = ui.input(|i| i.modifiers.shift);
        let origin = ui.input(|i| i.pointer.press_origin());
        let at = |p: Pos2| {
            (
                self.x.value(p.x, body.left()),
                self.y.value(p.y, body.bottom()),
            )
        };
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = origin
        {
            if ruler_rect.contains(p) {
                self.scrubbing = true;
            } else if body.contains(p) {
                let hit = if self.box_armed {
                    None
                } else {
                    self.pick(&state.scene.animation, curves, p, body)
                };
                match hit {
                    Some(pick) => {
                        let a = &mut state.scene.animation;
                        select_pick(a, pick, shift, true);
                        self.modal = Some(Modal::new(Kind::Grab, a.clone(), at(p), true, true));
                    }
                    None => self.box_start = Some(p),
                }
                self.box_armed = false;
            }
        }
        if self.scrubbing {
            if let Some(p) = response.interact_pointer_pos() {
                state.anim_frame = view::scrub_frame(&self.x, body, p.x);
            }
            if response.drag_stopped() {
                self.scrubbing = false;
            }
        }
        if let Some(start) = self.box_start
            && response.drag_stopped()
        {
            let end = response.interact_pointer_pos().unwrap_or(start);
            let rect = Rect::from_two_pos(start, end);
            let show_handles = self.only_selected_handles;
            let a = &mut state.scene.animation;
            if !shift {
                a.select_all(false);
            }
            for &c in curves {
                for key in &mut a.curves[c].keys {
                    let inside = |pt: [f32; 2]| {
                        rect.contains(pos2(
                            self.x.px(pt[0], body.left()),
                            self.y.px(pt[1], body.bottom()),
                        ))
                    };
                    if inside([key.frame, key.value]) {
                        key.select = SEL_ALL;
                    } else if !show_handles || key.select != 0 {
                        if inside(key.left) {
                            key.select |= SEL_LEFT;
                        }
                        if inside(key.right) {
                            key.select |= SEL_RIGHT;
                        }
                    }
                }
            }
            self.box_start = None;
        }
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
        {
            if ruler_rect.contains(p) {
                state.anim_frame = view::scrub_frame(&self.x, body, p.x);
            } else if body.contains(p) {
                let a = &mut state.scene.animation;
                match self.pick(a, curves, p, body) {
                    Some(pick) => select_pick(a, pick, shift, false),
                    None if !shift => a.select_all(false),
                    None => {}
                }
            }
        }
    }

    /// The channel list: eye toggles per curve, click selects the object.
    fn channel_clicks(&mut self, state: &mut EditorState, response: &egui::Response, rect: Rect) {
        if !response.clicked_by(egui::PointerButton::Primary) {
            return;
        }
        let Some(p) = response.interact_pointer_pos() else {
            return;
        };
        if p.x > rect.left() + NAMES_W || p.y < rect.top() + RULER_H {
            return;
        }
        let rows = self.channel_rows(state);
        let i = ((p.y - rect.top() - RULER_H) / ROW_H).floor();
        let Some(row) = (i >= 0.0).then(|| rows.get(i as usize)).flatten() else {
            return;
        };
        match row {
            ChannelRow::Object(o, _) => state.select(Some(*o)),
            ChannelRow::Curve(id, _) => {
                if p.x < rect.left() + 34.0 {
                    if !self.hidden.remove(id) {
                        self.hidden.insert(*id);
                    }
                } else {
                    state.select(Some(id.0));
                }
            }
        }
    }

    fn channel_rows(&self, state: &EditorState) -> Vec<ChannelRow> {
        let a = &state.scene.animation;
        let visible = ops::visible_curves(state, self.only_selected);
        let mut rows = Vec::new();
        let mut objects: Vec<u64> = Vec::new();
        for &c in &visible {
            let o = a.curves[c].object;
            if !objects.contains(&o) {
                objects.push(o);
            }
        }
        for o in objects {
            let name = state
                .scene
                .get(o)
                .map_or_else(|| "(deleted)".to_owned(), |e| e.name.clone());
            rows.push(ChannelRow::Object(o, name));
            let mut curves: Vec<_> = visible
                .iter()
                .map(|&c| &a.curves[c])
                .filter(|c| c.object == o)
                .collect();
            curves.sort_by_key(|c| (c.prop, c.index));
            for c in curves {
                rows.push(ChannelRow::Curve((o, c.prop, c.index), c.name()));
            }
        }
        rows
    }

    #[allow(clippy::too_many_arguments)]
    fn paint(
        &self,
        ui: &egui::Ui,
        state: &EditorState,
        curves: &[usize],
        rect: Rect,
        graph: Rect,
        ruler_rect: Rect,
        body: Rect,
    ) {
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);
        let clip = ui.painter_at(body);
        let a = &state.scene.animation;

        // Value grid with labels.
        let step = view::nice_step(self.y.scale, 38.0);
        let first = (self.y.value(body.bottom(), body.bottom()) / step).floor() * step;
        let mut v = first;
        while self.y.px(v, body.bottom()) > body.top() {
            let y = self.y.px(v, body.bottom());
            let zero = v.abs() < step * 1e-3;
            clip.line_segment(
                [pos2(body.left(), y), pos2(body.right(), y)],
                (1.0, Color32::from_white_alpha(if zero { 40 } else { 12 })),
            );
            clip.text(
                pos2(body.left() + 3.0, y - 2.0),
                Align2::LEFT_BOTTOM,
                format_value(v, step),
                FontId::monospace(10.0),
                visuals.weak_text_color(),
            );
            v += step;
        }

        // Curves, drawn as evaluated.
        for &c in curves {
            let curve = &a.curves[c];
            let color = AXIS[curve.index.min(2) as usize];
            let selected_curve = curve.keys.iter().any(|k| k.select != 0);
            let width = if selected_curve { 2.0 } else { 1.2 };
            let mut points = Vec::new();
            let mut x = body.left();
            while x <= body.right() + 2.0 {
                let f = self.x.value(x, body.left());
                if let Some(val) = curve.eval(f) {
                    points.push(pos2(x, self.y.px(val, body.bottom())));
                }
                x += 2.0;
            }
            clip.add(Shape::line(points, (width, color)));
        }
        // Handles and keys on top.
        for &c in curves {
            let curve = &a.curves[c];
            for key in &curve.keys {
                let kp = pos2(
                    self.x.px(key.frame, body.left()),
                    self.y.px(key.value, body.bottom()),
                );
                if key.interp == Interp::Bezier && self.handles_shown(key.select) {
                    for (pt, bit, ty) in [
                        (key.left, SEL_LEFT, key.left_type),
                        (key.right, SEL_RIGHT, key.right_type),
                    ] {
                        let hp = pos2(
                            self.x.px(pt[0], body.left()),
                            self.y.px(pt[1], body.bottom()),
                        );
                        let sel = key.select & bit != 0;
                        let line = handle_color(ty, sel);
                        clip.line_segment([kp, hp], (1.0, line));
                        clip.circle(
                            hp,
                            3.0,
                            if sel { Color32::WHITE } else { line },
                            (1.0, Color32::BLACK),
                        );
                    }
                }
                let fill = if key.select & SEL_KEY != 0 {
                    KEY_SELECTED
                } else {
                    Color32::BLACK
                };
                clip.rect_filled(Rect::from_center_size(kp, vec2(7.0, 7.0)), 1.0, fill);
                clip.rect_stroke(
                    Rect::from_center_size(kp, vec2(7.0, 7.0)),
                    1.0,
                    (1.0, Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
            }
        }
        // Playhead line over the graph.
        let x = self.x.px(state.anim_frame as f32, body.left());
        clip.line_segment(
            [pos2(x, body.top()), pos2(x, body.bottom())],
            (1.5, view::PLAYHEAD),
        );
        view::ruler(
            ui,
            &ui.painter_at(graph),
            ruler_rect,
            body,
            &self.x,
            (a.frame_start, a.frame_end),
            state.anim_frame,
        );

        // Channel list.
        let list = Rect::from_min_max(rect.min, pos2(rect.left() + NAMES_W, rect.bottom()));
        let side = ui.painter_at(list);
        side.rect_filled(list, 0.0, Color32::from_gray(30));
        for (i, row) in self.channel_rows(state).iter().enumerate() {
            let y = list.top() + RULER_H + i as f32 * ROW_H;
            let mid = y + ROW_H / 2.0;
            match row {
                ChannelRow::Object(o, name) => {
                    let sel = state.is_selected(*o);
                    side.rect_filled(
                        Rect::from_min_size(pos2(list.left(), y), vec2(list.width(), ROW_H)),
                        0.0,
                        if sel {
                            Color32::from_rgb(70, 82, 112)
                        } else {
                            Color32::from_gray(52)
                        },
                    );
                    side.text(
                        pos2(list.left() + 8.0, mid),
                        Align2::LEFT_CENTER,
                        name,
                        FontId::proportional(12.5),
                        visuals.text_color(),
                    );
                }
                ChannelRow::Curve(id, name) => {
                    let hidden = self.hidden.contains(id);
                    side.text(
                        pos2(list.left() + 8.0, mid),
                        Align2::LEFT_CENTER,
                        if hidden { "◌" } else { "◉" },
                        FontId::proportional(12.0),
                        visuals.text_color(),
                    );
                    side.circle_filled(
                        pos2(list.left() + 30.0, mid),
                        4.0,
                        AXIS[id.2.min(2) as usize],
                    );
                    side.text(
                        pos2(list.left() + 42.0, mid),
                        Align2::LEFT_CENTER,
                        name,
                        FontId::proportional(12.5),
                        if hidden {
                            visuals.weak_text_color()
                        } else {
                            visuals.text_color()
                        },
                    );
                }
            }
        }
        if a.curves.is_empty() {
            painter.text(
                pos2(graph.left() + 16.0, body.top() + 16.0),
                Align2::LEFT_TOP,
                "No animation. Select an object and press I in the viewport.",
                FontId::proportional(12.5),
                ops::hint_color(ui),
            );
        }
        if let (Some(start), Some(now)) = (self.box_start, ui.input(|i| i.pointer.hover_pos())) {
            painter.rect_stroke(
                Rect::from_two_pos(start, now),
                0.0,
                (1.0, Color32::WHITE),
                egui::StrokeKind::Inside,
            );
        }
    }

    fn menu_popup(&mut self, ctx: &egui::Context, state: &mut EditorState) {
        let Some((menu, pos)) = self.menu else {
            return;
        };
        let done = match menu {
            Menu::Interp => ops::popup_menu(ctx, "graph-interp", pos, &INTERPS).map(|c| {
                if let Some(i) = c {
                    state.scene.animation.set_interp_selected(i);
                }
            }),
            Menu::Handle => ops::popup_menu(ctx, "graph-handle", pos, &HANDLES).map(|c| {
                if let Some(h) = c {
                    state.scene.animation.set_handle_type_selected(h);
                }
            }),
        };
        if done.is_some() {
            self.menu = None;
        }
    }
}

enum ChannelRow {
    Object(u64, String),
    Curve(CurveId, String),
}

/// Applies a click (or the start of a drag) on a key or handle.
fn select_pick(a: &mut Action, pick: Pick, extend: bool, dragging: bool) {
    let (c, k, bits) = match pick {
        Pick::Key(c, k) => (c, k, SEL_ALL),
        Pick::Left(c, k) => (c, k, SEL_LEFT),
        Pick::Right(c, k) => (c, k, SEL_RIGHT),
    };
    let already = a.curves[c].keys[k].select & bits == bits;
    let is_key = matches!(pick, Pick::Key(..));
    if dragging && already && !extend && is_key {
        return; // dragging a selected key moves the whole selection
    }
    if !extend {
        a.select_all(false);
    }
    let key = &mut a.curves[c].keys[k];
    if extend && already {
        key.select &= !bits;
    } else {
        key.select |= bits;
    }
}

fn handle_color(ty: HandleType, selected: bool) -> Color32 {
    let (dim, bright) = match ty {
        HandleType::Free => (
            Color32::from_rgb(90, 90, 90),
            Color32::from_rgb(210, 210, 210),
        ),
        HandleType::Auto => (
            Color32::from_rgb(150, 130, 40),
            Color32::from_rgb(255, 230, 80),
        ),
        HandleType::Vector => (
            Color32::from_rgb(40, 120, 50),
            Color32::from_rgb(90, 230, 100),
        ),
        HandleType::Aligned => (
            Color32::from_rgb(140, 70, 110),
            Color32::from_rgb(255, 130, 200),
        ),
        HandleType::AutoClamped => (
            Color32::from_rgb(130, 60, 60),
            Color32::from_rgb(255, 110, 110),
        ),
    };
    if selected { bright } else { dim }
}

fn format_value(v: f32, step: f32) -> String {
    let decimals = (-step.log10().floor()).clamp(0.0, 5.0) as usize;
    format!("{v:.decimals$}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_a_handle_selects_only_it_and_a_selected_key_keeps_the_selection() {
        let mut a = Action::default();
        for f in [1.0, 10.0] {
            a.insert_key(1, Prop::Location, 0, f, f);
        }
        select_pick(&mut a, Pick::Key(0, 0), false, false);
        assert_eq!(a.curves[0].keys[0].select, SEL_ALL);
        // Starting a drag on a selected key leaves the selection alone.
        select_pick(&mut a, Pick::Key(0, 0), false, true);
        select_pick(&mut a, Pick::Key(0, 1), true, true);
        assert_eq!(a.selected_count(), 2);
        // A handle click replaces everything with that handle.
        select_pick(&mut a, Pick::Right(0, 1), false, false);
        assert_eq!(a.curves[0].keys[1].select, SEL_RIGHT);
        assert_eq!(a.curves[0].keys[0].select, 0);
        // Shift-click toggles it off again.
        select_pick(&mut a, Pick::Right(0, 1), true, false);
        assert_eq!(a.curves[0].keys[1].select, 0);
    }

    #[test]
    fn value_labels_follow_the_step() {
        assert_eq!(format_value(2.0, 1.0), "2");
        assert_eq!(format_value(0.25, 0.05), "0.25");
    }
}

//! The Dope Sheet: channels per object and F-curve group on the left, key
//! diamonds on the frame ruler on the right. Select (click, Shift+click,
//! box), grab, scale, duplicate, delete, copy / paste, interpolation.

use std::collections::HashSet;

use olvin_editor::egui::{self, Align2, FontId, Pos2, Rect, Sense, pos2, vec2};
use olvin_editor::keymap::Command;
use olvin_editor::olvin_scene::action::Interp;
use olvin_editor::{EditorState, Panel, theme};

use crate::clock;
use crate::modal::{Kind, Modal, Outcome};
use crate::ops::{self, KeyRef};
use crate::view::{self, Axis, RULER_H};

const NAMES_W: f32 = 200.0;
const ROW_H: f32 = 22.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    Summary,
    Object(u64),
    Group(u64),
    Channel(usize),
}

struct Row {
    kind: RowKind,
    indent: f32,
    label: String,
    curves: Vec<usize>,
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
    InsertKeys,
    SnapFrames,
    ViewAll,
    BoxSelect,
}

pub struct DopeSheet {
    view: Axis,
    scroll: f32,
    only_selected: bool,
    collapsed: HashSet<(bool, u64)>,
    modal: Option<Modal>,
    box_start: Option<Pos2>,
    box_armed: bool,
    scrubbing: bool,
    interp_menu: Option<Pos2>,
    fitted: bool,
}

impl Default for DopeSheet {
    fn default() -> Self {
        Self {
            view: Axis {
                origin: -5.0,
                scale: 4.0,
            },
            scroll: 0.0,
            only_selected: true,
            collapsed: HashSet::new(),
            modal: None,
            box_start: None,
            box_armed: false,
            scrubbing: false,
            interp_menu: None,
            fitted: false,
        }
    }
}

impl Panel for DopeSheet {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let cmd = self.header(ui, state);
        let rect = ui.available_rect_before_wrap();
        if rect.width() < NAMES_W + 40.0 || rect.height() < RULER_H + ROW_H {
            return;
        }
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        if !self.fitted {
            self.fitted = true;
            self.view_all(state, rect);
        }
        let keys_rect = Rect::from_min_max(pos2(rect.left() + NAMES_W, rect.top()), rect.max);
        let ruler_rect = Rect::from_min_size(keys_rect.min, vec2(keys_rect.width(), RULER_H));
        let rows = self.rows(state);
        let top = rect.top() + RULER_H;
        let max_scroll = (rows.len() as f32 * ROW_H - (rect.bottom() - top)).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max_scroll);

        // Navigation.
        let pointer = response.hover_pos();
        view::navigate_x(ui, &response, keys_rect, &mut self.view, false);
        if let Some(p) = pointer
            && response.hovered()
            && !ui.input(|i| i.modifiers.ctrl)
        {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if p.x > rect.left() && wheel != 0.0 {
                self.scroll = (self.scroll - wheel).clamp(0.0, max_scroll);
            }
        }
        if response.hovered() {
            state.tool_owns_keys = true;
        }

        let ctx = ui.ctx().clone();
        let mouse = pointer.map_or((0.0, 0.0), |p| {
            (self.view.value(p.x, keys_rect.left()), 0.0)
        });
        let current = state.anim_frame as f32;
        let mut cmd = cmd;

        // A running modal operation owns the mouse and the keys.
        if let Some(modal) = &mut self.modal {
            state.busy = true;
            state.tool_owns_keys = true;
            let outcome = if state.capturing_key {
                Outcome::Running
            } else {
                modal.input(&ctx, response.drag_stopped())
            };
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
            cmd = cmd.or_else(|| self.keyboard(ui, &response, state));
            self.pointer(
                ui, state, &response, &rows, rect, keys_rect, ruler_rect, top,
            );
        }
        if let Some(cmd) = cmd {
            self.run(cmd, state, &rows, mouse, current, keys_rect);
        }

        self.paint(ui, state, &rows, rect, keys_rect, ruler_rect, top);
        self.interp_popup(&ctx, state);
        let map = state.prefs.keymap.clone();
        let key_label = |label, command| format!("{label}  ({})", map.shortcut_text(command));
        response.context_menu(|ui| {
            for (label, c) in [
                (key_label("Grab", Command::AnimGrab), Cmd::Grab),
                (key_label("Scale", Command::AnimScale), Cmd::Scale),
                (
                    key_label("Duplicate", Command::AnimDuplicate),
                    Cmd::Duplicate,
                ),
                (
                    key_label("Delete Keyframes", Command::AnimDelete),
                    Cmd::Delete,
                ),
                ("Snap to Frame".into(), Cmd::SnapFrames),
                (key_label("Copy", Command::AnimCopy), Cmd::Copy),
                (key_label("Paste", Command::AnimPaste), Cmd::Paste),
            ] {
                if ui.button(label).clicked() {
                    self.run(c, state, &rows, mouse, current, keys_rect);
                    ui.close();
                }
            }
            ui.menu_button("Interpolation Mode", |ui| {
                for (label, i) in INTERPS {
                    if ui.button(label).clicked() {
                        self.run(Cmd::Interp(i), state, &rows, mouse, current, keys_rect);
                        ui.close();
                    }
                }
            });
        });
    }
}

const INTERPS: [(&str, Interp); 3] = [
    ("Constant", Interp::Constant),
    ("Linear", Interp::Linear),
    ("Bezier", Interp::Bezier),
];

impl DopeSheet {
    fn header(&mut self, ui: &mut egui::Ui, state: &EditorState) -> Option<Cmd> {
        let key_label =
            |label, command| format!("{label}  ({})", state.prefs.keymap.shortcut_text(command));
        let mut cmd = None;
        ui.horizontal(|ui| {
            ui.menu_button("Select", |ui| {
                for (label, c) in [
                    (key_label("All", Command::AnimSelectAll), Cmd::SelectAll),
                    (key_label("None", Command::AnimSelectNone), Cmd::SelectNone),
                    (key_label("Invert", Command::AnimInvert), Cmd::Invert),
                    (key_label("Box Select", Command::AnimBox), Cmd::BoxSelect),
                ] {
                    if ui.button(label).clicked() {
                        cmd = Some(c);
                        ui.close();
                    }
                }
            });
            ui.menu_button("Key", |ui| {
                for (label, c) in [
                    (
                        key_label("Insert Keyframes", Command::AnimInsert),
                        Cmd::InsertKeys,
                    ),
                    (key_label("Grab", Command::AnimGrab), Cmd::Grab),
                    (key_label("Scale", Command::AnimScale), Cmd::Scale),
                    (
                        key_label("Duplicate", Command::AnimDuplicate),
                        Cmd::Duplicate,
                    ),
                    (
                        key_label("Delete Keyframes", Command::AnimDelete),
                        Cmd::Delete,
                    ),
                    ("Snap to Frame".into(), Cmd::SnapFrames),
                    (key_label("Copy", Command::AnimCopy), Cmd::Copy),
                    (key_label("Paste", Command::AnimPaste), Cmd::Paste),
                ] {
                    if ui.button(label).clicked() {
                        cmd = Some(c);
                        ui.close();
                    }
                }
                ui.menu_button(
                    key_label("Interpolation Mode", Command::AnimInterpolation),
                    |ui| {
                        for (label, i) in INTERPS {
                            if ui.button(label).clicked() {
                                cmd = Some(Cmd::Interp(i));
                                ui.close();
                            }
                        }
                    },
                );
            });
            if ui.button("View All  (Home)").clicked() {
                cmd = Some(Cmd::ViewAll);
            }
            ui.checkbox(&mut self.only_selected, "Only Show Selected");
            if let Some(modal) = &self.modal {
                ui.label(modal.status());
            }
        });
        ui.separator();
        cmd
    }

    fn rows(&self, state: &EditorState) -> Vec<Row> {
        let action = &state.scene.animation;
        let visible = ops::visible_curves(state, self.only_selected);
        let mut rows = vec![Row {
            kind: RowKind::Summary,
            indent: 0.0,
            label: "Summary".into(),
            curves: visible.clone(),
        }];
        let mut objects: Vec<u64> = Vec::new();
        for &c in &visible {
            let o = action.curves[c].object;
            if !objects.contains(&o) {
                objects.push(o);
            }
        }
        for o in objects {
            let curves: Vec<usize> = visible
                .iter()
                .copied()
                .filter(|&c| action.curves[c].object == o)
                .collect();
            let name = state
                .scene
                .get(o)
                .map_or_else(|| "(deleted)".to_owned(), |e| e.name.clone());
            rows.push(Row {
                kind: RowKind::Object(o),
                indent: 0.0,
                label: name,
                curves: curves.clone(),
            });
            if self.collapsed.contains(&(false, o)) {
                continue;
            }
            rows.push(Row {
                kind: RowKind::Group(o),
                indent: 14.0,
                label: "Object Transforms".into(),
                curves: curves.clone(),
            });
            if self.collapsed.contains(&(true, o)) {
                continue;
            }
            let mut sorted = curves;
            sorted.sort_by_key(|&c| {
                let k = &action.curves[c];
                (k.prop, k.index)
            });
            for c in sorted {
                rows.push(Row {
                    kind: RowKind::Channel(c),
                    indent: 28.0,
                    label: action.curves[c].name(),
                    curves: vec![c],
                });
            }
        }
        rows
    }

    fn view_all(&mut self, state: &EditorState, rect: Rect) {
        let a = &state.scene.animation;
        let (lo, hi) = a
            .key_range()
            .unwrap_or((a.frame_start as f32, a.frame_end as f32));
        let hi = if hi - lo < 1.0 { lo + 10.0 } else { hi };
        self.view = Axis::fit(lo, hi, rect.width() - NAMES_W, 30.0, false);
    }

    fn keyboard(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        state: &EditorState,
    ) -> Option<Cmd> {
        if state.capturing_key || !response.hovered() || ui.ctx().egui_wants_keyboard_input() {
            return None;
        }
        // T opens the interpolation menu at the mouse.
        let pressed = |command| ui.input_mut(|i| state.prefs.keymap.pressed(i, command));
        if pressed(Command::AnimInterpolation) {
            self.interp_menu = response.hover_pos();
        }
        for (command, action) in [
            (Command::AnimSelectAll, Cmd::SelectAll),
            (Command::AnimSelectNone, Cmd::SelectNone),
            (Command::AnimInvert, Cmd::Invert),
            (Command::AnimBox, Cmd::BoxSelect),
            (Command::AnimGrab, Cmd::Grab),
            (Command::AnimScale, Cmd::Scale),
            (Command::AnimDuplicate, Cmd::Duplicate),
            (Command::AnimDelete, Cmd::Delete),
            (Command::AnimInsert, Cmd::InsertKeys),
            (Command::AnimViewAll, Cmd::ViewAll),
            (Command::AnimCopy, Cmd::Copy),
            (Command::AnimPaste, Cmd::Paste),
        ] {
            if pressed(command) {
                return Some(action);
            }
        }
        ui.input(|i| {
            let keymap = &state.prefs.keymap;
            if keymap.is_default(Command::AnimCopy)
                && i.events.iter().any(|e| matches!(e, egui::Event::Copy))
            {
                Some(Cmd::Copy)
            } else if keymap.is_default(Command::AnimPaste)
                && i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))
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
        rows: &[Row],
        mouse: (f32, f32),
        current: f32,
        _keys_rect: Rect,
    ) {
        let visible: Vec<usize> = rows.first().map(|r| r.curves.clone()).unwrap_or_default();
        let selected = state.scene.animation.selected_count();
        let frame = state.anim_frame as f32;
        let a = &mut state.scene.animation;
        match cmd {
            Cmd::SelectAll => a.select_all(true),
            Cmd::SelectNone => a.select_all(false),
            Cmd::Invert => ops::invert_selection(a),
            Cmd::BoxSelect => self.box_armed = true,
            Cmd::Grab | Cmd::Scale if selected > 0 => {
                let kind = if cmd == Cmd::Grab {
                    Kind::Grab
                } else {
                    Kind::Scale
                };
                self.modal = Some(Modal::new(kind, a.clone(), mouse, false, false));
            }
            Cmd::Duplicate if selected > 0 => {
                a.duplicate_selected();
                self.modal = Some(Modal::new(Kind::Grab, a.clone(), mouse, false, false));
            }
            Cmd::Delete => a.delete_selected(),
            Cmd::Copy => {
                let clip = a.copy_selected();
                clock::with(|s| s.clipboard = clip);
                let n = selected;
                state.info(format!("copied {n} keys"));
            }
            Cmd::Paste => {
                let clip = clock::with(|s| s.clipboard.clone());
                a.paste(&clip, current);
            }
            Cmd::Interp(i) => a.set_interp_selected(i),
            Cmd::SnapFrames => a.snap_selected(),
            Cmd::InsertKeys => {
                for c in visible {
                    let curve = &a.curves[c];
                    if let Some(v) = curve.eval(frame) {
                        let (o, p, i) = (curve.object, curve.prop, curve.index);
                        a.insert_key(o, p, i, frame, v);
                    }
                }
            }
            Cmd::ViewAll => {
                let width = _keys_rect.width();
                let (lo, hi) = a
                    .key_range()
                    .unwrap_or((a.frame_start as f32, a.frame_end as f32));
                let hi = if hi - lo < 1.0 { lo + 10.0 } else { hi };
                self.view = Axis::fit(lo, hi, width, 30.0, false);
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pointer(
        &mut self,
        ui: &egui::Ui,
        state: &mut EditorState,
        response: &egui::Response,
        rows: &[Row],
        rect: Rect,
        keys_rect: Rect,
        ruler_rect: Rect,
        top: f32,
    ) {
        let shift = ui.input(|i| i.modifiers.shift);
        let origin = ui.input(|i| i.pointer.press_origin());
        let frame_at = |x: f32| self.view.value(x, keys_rect.left());

        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = origin
        {
            if ruler_rect.contains(p) {
                self.scrubbing = true;
            } else if keys_rect.contains(p) {
                let hit = if self.box_armed {
                    None
                } else {
                    self.hit(&state.scene.animation, rows, p, rect, keys_rect, top)
                };
                match hit {
                    Some(refs) => {
                        let a = &mut state.scene.animation;
                        if !refs.iter().all(|&(c, k)| a.curves[c].keys[k].is_selected()) {
                            ops::click_select(a, &refs, shift);
                        }
                        let start = (frame_at(p.x), 0.0);
                        self.modal = Some(Modal::new(Kind::Grab, a.clone(), start, true, false));
                    }
                    None => self.box_start = Some(p),
                }
                self.box_armed = false;
            }
        }
        if self.scrubbing {
            if let Some(p) = response.interact_pointer_pos() {
                state.anim_frame = view::scrub_frame(&self.view, keys_rect, p.x);
            }
            if response.drag_stopped() {
                self.scrubbing = false;
            }
        }
        if let Some(start) = self.box_start
            && response.drag_stopped()
        {
            let end = response.interact_pointer_pos().unwrap_or(start);
            let (r0, r1) = (start.y.min(end.y), start.y.max(end.y));
            let mut curves = Vec::new();
            for (i, row) in rows.iter().enumerate() {
                let y = top + i as f32 * ROW_H - self.scroll;
                if y + ROW_H >= r0 && y <= r1 {
                    for &c in &row.curves {
                        if !curves.contains(&c) {
                            curves.push(c);
                        }
                    }
                }
            }
            ops::box_select(
                &mut state.scene.animation,
                &curves,
                (frame_at(start.x), frame_at(end.x)),
                shift,
            );
            self.box_start = None;
        }
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
        {
            if ruler_rect.contains(p) {
                state.anim_frame = view::scrub_frame(&self.view, keys_rect, p.x);
            } else if keys_rect.contains(p) {
                let hit = self
                    .hit(&state.scene.animation, rows, p, rect, keys_rect, top)
                    .unwrap_or_default();
                ops::click_select(&mut state.scene.animation, &hit, shift);
            } else if p.y > top {
                self.click_channel(state, rows, p, rect, top);
            }
        }
    }

    fn click_channel(
        &mut self,
        state: &mut EditorState,
        rows: &[Row],
        p: Pos2,
        rect: Rect,
        top: f32,
    ) {
        let i = ((p.y - top + self.scroll) / ROW_H).floor();
        let Some(row) = (i >= 0.0).then(|| rows.get(i as usize)).flatten() else {
            return;
        };
        let arrow = p.x < rect.left() + row.indent + 16.0;
        match row.kind {
            RowKind::Object(o) if arrow => toggle(&mut self.collapsed, (false, o)),
            RowKind::Group(o) if arrow => toggle(&mut self.collapsed, (true, o)),
            RowKind::Object(o) | RowKind::Group(o) => state.select(Some(o)),
            RowKind::Channel(c) => {
                if let Some(curve) = state.scene.animation.curves.get(c) {
                    let o = curve.object;
                    state.select(Some(o));
                }
            }
            RowKind::Summary => {}
        }
    }

    /// The keys of the row under `p` at the diamond nearest to it.
    fn hit(
        &self,
        action: &olvin_editor::olvin_scene::action::Action,
        rows: &[Row],
        p: Pos2,
        rect: Rect,
        keys_rect: Rect,
        top: f32,
    ) -> Option<Vec<KeyRef>> {
        let _ = rect;
        let i = ((p.y - top + self.scroll) / ROW_H).floor();
        let row = rows.get((i >= 0.0).then_some(i as usize)?)?;
        let (frame, _) = ops::frames_of(action, &row.curves)
            .into_iter()
            .filter(|(f, _)| (self.view.px(*f, keys_rect.left()) - p.x).abs() <= 7.0)
            .min_by(|a, b| {
                let da = (self.view.px(a.0, keys_rect.left()) - p.x).abs();
                let db = (self.view.px(b.0, keys_rect.left()) - p.x).abs();
                da.total_cmp(&db)
            })?;
        Some(ops::keys_at(action, &row.curves, frame))
    }

    #[allow(clippy::too_many_arguments)]
    fn paint(
        &self,
        ui: &egui::Ui,
        state: &EditorState,
        rows: &[Row],
        rect: Rect,
        keys_rect: Rect,
        ruler_rect: Rect,
        top: f32,
    ) {
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        let pal = theme::palette(ui.ctx());
        painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);
        let body = Rect::from_min_max(pos2(keys_rect.left(), top), rect.max);
        let action = &state.scene.animation;
        let clip = ui.painter_at(body);
        // Rows.
        for (i, row) in rows.iter().enumerate() {
            let y = top + i as f32 * ROW_H - self.scroll;
            if y + ROW_H < top || y > rect.bottom() {
                continue;
            }
            let band = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), ROW_H));
            let selected_object = match row.kind {
                RowKind::Object(o) | RowKind::Group(o) => state.is_selected(o),
                RowKind::Channel(c) => action
                    .curves
                    .get(c)
                    .is_some_and(|k| state.is_selected(k.object)),
                RowKind::Summary => false,
            };
            let fill = match row.kind {
                RowKind::Summary => visuals.faint_bg_color,
                RowKind::Object(_) if selected_object => pal.info.gamma_multiply(0.35),
                RowKind::Object(_) => pal.ink(20),
                RowKind::Group(_) => pal.ink(14),
                RowKind::Channel(_) if i % 2 == 0 => pal.ink(5),
                RowKind::Channel(_) => pal.ink(9),
            };
            painter.rect_filled(band, 0.0, fill);
            let arrow = match row.kind {
                RowKind::Object(o) => Some(if self.collapsed.contains(&(false, o)) {
                    "▸"
                } else {
                    "▾"
                }),
                RowKind::Group(o) => Some(if self.collapsed.contains(&(true, o)) {
                    "▸"
                } else {
                    "▾"
                }),
                _ => None,
            };
            if let Some(a) = arrow {
                painter.text(
                    pos2(rect.left() + row.indent + 4.0, y + ROW_H / 2.0),
                    Align2::LEFT_CENTER,
                    a,
                    FontId::proportional(12.0),
                    visuals.text_color(),
                );
            }
            painter.text(
                pos2(rect.left() + row.indent + 20.0, y + ROW_H / 2.0),
                Align2::LEFT_CENTER,
                &row.label,
                FontId::proportional(12.5),
                visuals.text_color(),
            );
            // Hold bars: a channel whose neighbouring keys have the same value.
            if let RowKind::Channel(c) = row.kind
                && let Some(curve) = action.curves.get(c)
            {
                for pair in curve.keys.windows(2) {
                    if (pair[0].value - pair[1].value).abs() < 1e-6 {
                        let (x0, x1) = (
                            self.view.px(pair[0].frame, keys_rect.left()),
                            self.view.px(pair[1].frame, keys_rect.left()),
                        );
                        let sel = pair[0].is_selected() && pair[1].is_selected();
                        let color = if sel {
                            pal.selected().gamma_multiply(0.55)
                        } else {
                            pal.muted
                        };
                        clip.rect_filled(
                            Rect::from_min_max(
                                pos2(x0, y + ROW_H / 2.0 - 3.0),
                                pos2(x1, y + ROW_H / 2.0 + 3.0),
                            ),
                            0.0,
                            color,
                        );
                    }
                }
            }
            for (f, sel) in ops::frames_of(action, &row.curves) {
                let x = self.view.px(f, keys_rect.left());
                if x < keys_rect.left() - 8.0 || x > keys_rect.right() + 8.0 {
                    continue;
                }
                let color = if sel { pal.selected() } else { pal.key_idle() };
                view::diamond(&clip, pos2(x, y + ROW_H / 2.0), 5.0, color);
            }
        }
        painter.line_segment(
            [
                pos2(keys_rect.left(), rect.top()),
                pos2(keys_rect.left(), rect.bottom()),
            ],
            (1.0, pal.line),
        );
        if rows.len() == 1 {
            painter.text(
                pos2(keys_rect.left() + 16.0, top + ROW_H + 8.0),
                Align2::LEFT_TOP,
                "No keyframes. Select an object and press I in the viewport,\nor right-click a Transform field in the Inspector.",
                FontId::proportional(12.5),
                ops::hint_color(ui),
            );
        }
        let strip = ui.painter_at(Rect::from_min_max(
            pos2(keys_rect.left(), rect.top()),
            rect.max,
        ));
        view::ruler(
            ui,
            &strip,
            ruler_rect,
            Rect::from_min_max(pos2(keys_rect.left(), top), rect.max),
            &self.view,
            (
                state.scene.animation.frame_start,
                state.scene.animation.frame_end,
            ),
            state.anim_frame,
        );
        if let (Some(start), Some(now)) = (self.box_start, ui.input(|i| i.pointer.hover_pos())) {
            painter.rect_stroke(
                Rect::from_two_pos(start, now),
                0.0,
                (1.0, pal.text),
                egui::StrokeKind::Inside,
            );
        }
    }

    fn interp_popup(&mut self, ctx: &egui::Context, state: &mut EditorState) {
        let Some(pos) = self.interp_menu else {
            return;
        };
        if let Some(choice) = ops::popup_menu(ctx, "interp", pos, &INTERPS) {
            if let Some(i) = choice {
                state.scene.animation.set_interp_selected(i);
            }
            self.interp_menu = None;
        }
    }
}

fn toggle(set: &mut HashSet<(bool, u64)>, key: (bool, u64)) {
    if !set.remove(&key) {
        set.insert(key);
    }
}

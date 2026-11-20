//! Modal G / R / S: starting a transform and running it frame by frame.

use olvin_editor::EditorState;
use olvin_editor::egui::{self, Event, Key, PointerButton, Pos2};
use olvin_editor::glam::Vec2;
use olvin_mesh::PolyMesh;

use super::{Modal, UvEditor, v2};
use crate::select::Topo;
use crate::transform::{self, Kind, NumInput, Params};

impl UvEditor {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_transform(
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

    pub(super) fn run_modal(
        &mut self,
        ui: &egui::Ui,
        state: &mut EditorState,
        id: u64,
        origin: Vec2,
    ) {
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
}

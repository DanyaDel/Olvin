use egui::{Id, Sense, TextEdit};

use crate::{EditorState, NewEntity, Panel, theme};

enum Action {
    Select(Option<u64>),
    Create(NewEntity, Option<u64>),
    Duplicate(u64),
    Delete(u64),
    Reparent(u64, Option<u64>),
    StartRename(u64),
    Rename(u64, String),
}

#[derive(Default)]
pub struct Hierarchy {
    filter: String,
    renaming: Option<(u64, String)>,
}

impl Panel for Hierarchy {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            ui.menu_button("+", |ui| create_menu(ui, None, &mut actions));
            ui.add(TextEdit::singleline(&mut self.filter).hint_text("🔍 Поиск").desired_width(f32::INFINITY));
        });
        ui.separator();

        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            let filter = self.filter.trim().to_lowercase();
            if filter.is_empty() {
                for root in state.children(None) {
                    self.node(ui, state, root, &mut actions);
                }
            } else {
                let found: Vec<u64> = state
                    .scene
                    .entities
                    .iter()
                    .filter(|e| e.name.to_lowercase().contains(&filter))
                    .map(|e| e.id)
                    .collect();
                for id in found {
                    self.row(ui, state, id, &mut actions);
                }
            }

            // The empty area below the tree: drop to un-parent, click to deselect.
            let size = ui.available_size().max(egui::vec2(0.0, 24.0));
            let (rect, rest) = ui.allocate_exact_size(size, Sense::click());
            if let Some(dragged) = rest.dnd_release_payload::<u64>() {
                actions.push(Action::Reparent(*dragged, None));
            }
            if rest.dnd_hover_payload::<u64>().is_some() {
                ui.painter().rect_stroke(rect.shrink(1.0), 3.0, (1.0, theme::ACCENT), egui::StrokeKind::Inside);
            }
            if rest.clicked() {
                actions.push(Action::Select(None));
            }
            rest.context_menu(|ui| create_menu(ui, None, &mut actions));
        });

        for action in actions {
            match action {
                Action::Select(id) => state.selection = id,
                Action::Create(kind, parent) => {
                    state.create(kind, parent);
                }
                Action::Duplicate(id) => {
                    state.duplicate(id);
                }
                Action::Delete(id) => state.delete(id),
                Action::Reparent(id, parent) => {
                    state.reparent(id, parent);
                }
                Action::Rename(id, name) => {
                    if let Some(e) = state.scene.get_mut(id) {
                        e.name = name;
                    }
                }
                Action::StartRename(id) => {
                    let name = state.scene.get(id).map(|e| e.name.clone()).unwrap_or_default();
                    self.renaming = Some((id, name));
                }
            }
        }
    }
}

impl Hierarchy {
    fn node(&mut self, ui: &mut egui::Ui, state: &EditorState, id: u64, actions: &mut Vec<Action>) {
        let children = state.children(Some(id));
        if children.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(ui.spacing().indent);
                self.row(ui, state, id, actions);
            });
            return;
        }
        egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            Id::new(("hierarchy", id)),
            true,
        )
        .show_header(ui, |ui| self.row(ui, state, id, actions))
        .body(|ui| {
            for child in children {
                self.node(ui, state, child, actions);
            }
        });
    }

    fn row(&mut self, ui: &mut egui::Ui, state: &EditorState, id: u64, actions: &mut Vec<Action>) {
        let Some(entity) = state.scene.get(id) else { return };

        if let Some((rid, name)) = &mut self.renaming
            && *rid == id
        {
            let edit = ui.add(TextEdit::singleline(name).desired_width(160.0));
            edit.request_focus();
            if edit.lost_focus() {
                let name = name.trim().to_owned();
                self.renaming = None;
                if !name.is_empty() {
                    actions.push(Action::Rename(id, name));
                }
            }
            return;
        }

        let icon = if entity.camera.is_some() {
            "🎥"
        } else if entity.mesh.is_some() {
            "◼"
        } else {
            "○"
        };
        let selected = state.selection == Some(id);
        let drag = ui.dnd_drag_source(Id::new(("hier-drag", id)), id, |ui| {
            ui.selectable_label(selected, format!("{icon} {}", entity.name))
        });
        let label = drag.inner;
        if label.clicked() {
            actions.push(Action::Select(Some(id)));
        }
        if label.double_clicked() {
            actions.push(Action::StartRename(id));
        }
        label.context_menu(|ui| {
            ui.menu_button("Создать дочерний", |ui| create_menu(ui, Some(id), actions));
            if ui.button("Переименовать").clicked() {
                actions.push(Action::StartRename(id));
            }
            if ui.button("Дублировать  Ctrl+D").clicked() {
                actions.push(Action::Duplicate(id));
            }
            if entity.parent.is_some() && ui.button("Вынести в корень").clicked() {
                actions.push(Action::Reparent(id, None));
            }
            ui.separator();
            if ui.button("Удалить  Del").clicked() {
                actions.push(Action::Delete(id));
            }
        });

        let response = drag.response;
        if let Some(dragged) = response.dnd_release_payload::<u64>()
            && *dragged != id
        {
            actions.push(Action::Reparent(*dragged, Some(id)));
        }
        if response.dnd_hover_payload::<u64>().is_some_and(|d| *d != id) {
            ui.painter().rect_stroke(response.rect, 3.0, (1.5, theme::ACCENT), egui::StrokeKind::Outside);
        }
    }
}

fn create_menu(ui: &mut egui::Ui, parent: Option<u64>, actions: &mut Vec<Action>) {
    for kind in NewEntity::ALL {
        if ui.button(kind.label()).clicked() {
            actions.push(Action::Create(kind, parent));
        }
    }
}

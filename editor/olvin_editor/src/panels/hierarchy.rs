use egui::{Id, Sense, TextEdit};

use crate::ext::CreateItem;
use crate::panels::assets::{AssetDrag, DropTarget, drop_asset, dropped};
use crate::{EditorState, NewEntity, Panel, theme};

/// What a create menu produced.
#[derive(Clone, Copy)]
pub(crate) enum Creation {
    Builtin(NewEntity),
    Element(usize),
}

impl Creation {
    pub(crate) fn apply(self, state: &mut EditorState, parent: Option<u64>) -> u64 {
        match self {
            Creation::Builtin(kind) => state.create(kind, parent),
            Creation::Element(i) => {
                let create = state.ext.create[i].create;
                create(state, parent)
            }
        }
    }
}

/// Built-in objects, then element items grouped by their "Group/Item" path.
pub(crate) fn create_menu(ui: &mut egui::Ui, items: &[CreateItem]) -> Option<Creation> {
    let mut chosen = None;
    for (submenu, kinds) in NewEntity::MENU {
        let mut items = |ui: &mut egui::Ui| {
            for kind in kinds {
                if ui.button(kind.label()).clicked() {
                    chosen = Some(Creation::Builtin(*kind));
                }
            }
        };
        match submenu {
            Some(name) => {
                ui.menu_button(name, items);
            }
            None => items(ui),
        }
    }
    let mut groups: Vec<&str> = Vec::new();
    for item in items {
        if let Some((group, _)) = item.path.split_once('/')
            && !groups.contains(&group)
        {
            groups.push(group);
        }
    }
    if !items.is_empty() {
        ui.separator();
    }
    for (i, item) in items.iter().enumerate() {
        if !item.path.contains('/') && ui.button(item.path).clicked() {
            chosen = Some(Creation::Element(i));
        }
    }
    for group in groups {
        ui.menu_button(group, |ui| {
            for (i, item) in items.iter().enumerate() {
                if let Some((g, name)) = item.path.split_once('/')
                    && g == group
                    && ui.button(name).clicked()
                {
                    chosen = Some(Creation::Element(i));
                }
            }
        });
    }
    chosen
}

enum Action {
    Select(Option<u64>),
    Toggle(u64),
    Range(u64),
    Create(Creation, Option<u64>),
    MoveSibling(u64, i32),
    Duplicate(u64),
    Delete(u64),
    Reparent(u64, Option<u64>),
    StartRename(u64),
    Rename(u64, String),
    AssetDrop(std::path::PathBuf, Option<u64>),
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
            ui.menu_button("+", |ui| create_actions(ui, state, None, &mut actions));
            ui.add(
                TextEdit::singleline(&mut self.filter)
                    .hint_text("🔍 Search")
                    .desired_width(f32::INFINITY),
            );
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
            // Assets first: the object check below discards other payloads.
            if let Some(asset) = dropped(&rest) {
                actions.push(Action::AssetDrop(asset.0.clone(), None));
            }
            if let Some(dragged) = rest.dnd_release_payload::<u64>() {
                actions.push(Action::Reparent(*dragged, None));
            }
            if rest.dnd_hover_payload::<u64>().is_some()
                || rest.dnd_hover_payload::<AssetDrag>().is_some()
            {
                ui.painter().rect_stroke(
                    rect.shrink(1.0),
                    3.0,
                    (1.0, theme::palette(ui.ctx()).accent),
                    egui::StrokeKind::Inside,
                );
            }
            if rest.clicked() {
                actions.push(Action::Select(None));
            }
            rest.context_menu(|ui| create_actions(ui, state, None, &mut actions));
        });

        for action in actions {
            match action {
                Action::Select(id) => state.select(id),
                Action::Toggle(id) => state.toggle_selected(id),
                Action::Range(id) => {
                    let order: Vec<u64> = state
                        .scene
                        .hierarchy_order()
                        .into_iter()
                        .map(|i| state.scene.entities[i].id)
                        .collect();
                    state.select_range(id, &order);
                }
                Action::Create(creation, parent) => {
                    creation.apply(state, parent);
                }
                Action::MoveSibling(id, dir) => {
                    state.move_sibling(id, dir);
                }
                Action::Duplicate(id) => {
                    state.duplicate(id);
                }
                Action::Delete(id) => state.delete(id),
                Action::Reparent(id, parent) => {
                    state.reparent(id, parent);
                }
                Action::AssetDrop(path, row) => {
                    drop_asset(state, &path, DropTarget::Hierarchy(row));
                }
                Action::Rename(id, name) => {
                    if let Some(e) = state.scene.get_mut(id) {
                        e.name = name;
                    }
                }
                Action::StartRename(id) => {
                    let name = state
                        .scene
                        .get(id)
                        .map(|e| e.name.clone())
                        .unwrap_or_default();
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
        let Some(entity) = state.scene.get(id) else {
            return;
        };

        if let Some((rid, name)) = &mut self.renaming
            && *rid == id
        {
            let edit = ui.add(TextEdit::singleline(name).desired_width(160.0));
            // Enter or a click elsewhere ends the edit. Ask before focusing:
            // request_focus() undoes "lost focus" within the same frame.
            let done = edit.lost_focus();
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.renaming = None;
            } else if done {
                let name = name.trim().to_owned();
                self.renaming = None;
                if !name.is_empty() {
                    actions.push(Action::Rename(id, name));
                }
            } else {
                edit.request_focus();
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
        let selected = state.is_selected(id);
        let text = if state.selection == Some(id) && state.selected_ids().len() > 1 {
            egui::RichText::new(format!("{icon} {}", entity.name)).strong()
        } else {
            egui::RichText::new(format!("{icon} {}", entity.name))
        };
        // One widget for clicks and drags: a drag source wrapping the label
        // would swallow the clicks.
        let label = ui.add(egui::Button::selectable(selected, text).sense(Sense::click_and_drag()));
        if label.dragged() {
            label.dnd_set_drag_payload(id);
            if let Some(p) = ui.ctx().pointer_interact_pos() {
                let painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    Id::new("hier-drag"),
                ));
                painter.text(
                    p + egui::vec2(12.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &entity.name,
                    egui::FontId::proportional(13.0),
                    theme::palette(ui.ctx()).selected(),
                );
            }
        }
        if label.clicked() {
            let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            actions.push(match (ctrl, shift) {
                (true, _) => Action::Toggle(id),
                (_, true) => Action::Range(id),
                _ => Action::Select(Some(id)),
            });
        }
        if label.double_clicked() {
            actions.push(Action::StartRename(id));
        }
        label.context_menu(|ui| {
            ui.menu_button("Create child", |ui| {
                create_actions(ui, state, Some(id), actions)
            });
            if ui.button("Rename").clicked() {
                actions.push(Action::StartRename(id));
            }
            if ui.button("Duplicate  Ctrl+D").clicked() {
                actions.push(Action::Duplicate(id));
            }
            if ui
                .button("Move Up")
                .on_hover_text("Earlier in order (CSG brushes apply top to bottom)")
                .clicked()
            {
                actions.push(Action::MoveSibling(id, -1));
            }
            if ui.button("Move Down").clicked() {
                actions.push(Action::MoveSibling(id, 1));
            }
            if entity.parent.is_some() && ui.button("Unparent").clicked() {
                actions.push(Action::Reparent(id, None));
            }
            ui.separator();
            if ui.button("Delete  Del").clicked() {
                actions.push(Action::Delete(id));
            }
        });

        let response = label;
        if let Some(asset) = dropped(&response) {
            actions.push(Action::AssetDrop(asset.0.clone(), Some(id)));
        }
        if let Some(dragged) = response.dnd_release_payload::<u64>()
            && *dragged != id
        {
            actions.push(Action::Reparent(*dragged, Some(id)));
        }
        if response
            .dnd_hover_payload::<u64>()
            .is_some_and(|d| *d != id)
            || response.dnd_hover_payload::<AssetDrag>().is_some()
        {
            ui.painter().rect_stroke(
                response.rect,
                3.0,
                (1.5, theme::palette(ui.ctx()).accent),
                egui::StrokeKind::Outside,
            );
        }
    }
}

fn create_actions(
    ui: &mut egui::Ui,
    state: &EditorState,
    parent: Option<u64>,
    actions: &mut Vec<Action>,
) {
    if let Some(creation) = create_menu(ui, &state.ext.create) {
        actions.push(Action::Create(creation, parent));
    }
}

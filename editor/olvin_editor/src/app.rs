use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use egui::{Id, Key, KeyboardShortcut, Modifiers, RichText, TextEdit, ViewportCommand};
use egui_dock::{DockArea, DockState, NodePath, TabViewer};

use crate::workspace::Layout;
use crate::{EditorState, LogLevel, NewEntity, Panel, Registry, Tab, theme};

/// Waiting for "save changes?" before doing something destructive.
enum Confirm {
    Open(PathBuf),
    NewScene,
    Quit,
}

enum WsAction {
    Activate(usize),
    Close(usize),
    StartRename(usize),
    Duplicate(usize),
    Move(usize, usize),
    Add(Option<usize>),
}

pub struct EditorApp {
    registry: Registry,
    layout: Layout,
    panels: HashMap<u32, Box<dyn Panel>>,
    state: EditorState,
    renaming: Option<(usize, String)>,
    save_as: Option<String>,
    confirm: Option<Confirm>,
    allow_close: bool,
    title: String,
}

impl EditorApp {
    pub fn new(registry: Registry, project: Option<PathBuf>) -> Self {
        let layout = Layout::load(&registry);
        let mut state = EditorState::new(project);
        if !registry.plugins().is_empty() {
            state.info(format!("установленные элементы: {}", registry.plugins().join(", ")));
        }
        Self {
            registry,
            layout,
            panels: HashMap::new(),
            state,
            renaming: None,
            save_as: None,
            confirm: None,
            allow_close: false,
            title: String::new(),
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let shortcut = |m, k| KeyboardShortcut::new(m, k);
        let (save, new) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::S)),
                i.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::N)),
            )
        });
        if save {
            self.save();
        }
        if new {
            self.request(Confirm::NewScene);
        }
        if typing {
            return; // Text fields keep their own undo, delete, etc.
        }
        let (redo, undo, duplicate, delete) = ctx.input_mut(|i| {
            let redo = i.consume_shortcut(&shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))
                || i.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::Y));
            (
                redo,
                i.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::Z)),
                i.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::D)),
                i.key_pressed(Key::Delete) || i.key_pressed(Key::X) && i.modifiers.is_none(),
            )
        });
        if redo {
            self.state.redo();
        }
        if undo {
            self.state.undo();
        }
        if let Some(sel) = self.state.selection {
            if duplicate {
                self.state.duplicate(sel);
            }
            if delete {
                self.state.delete(sel);
            }
        }
    }

    fn save(&mut self) {
        if self.state.scene_path.is_some() {
            self.state.save();
        } else {
            self.save_as = Some(String::from("scenes/main.scene.ron"));
        }
    }

    fn request(&mut self, action: Confirm) {
        if self.state.is_dirty() {
            self.confirm = Some(action);
        } else {
            self.perform(action);
        }
    }

    fn perform(&mut self, action: Confirm) {
        match action {
            Confirm::Open(path) => self.state.open(&path),
            Confirm::NewScene => self.state.new_scene(),
            Confirm::Quit => self.allow_close = true,
        }
    }

    fn menus(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Файл", |ui| {
            if ui.button("Новая сцена        Ctrl+N").clicked() {
                self.request(Confirm::NewScene);
            }
            if ui.button("Сохранить           Ctrl+S").clicked() {
                self.save();
            }
            if ui.button("Сохранить как…").clicked() {
                self.save_as = Some(String::from("scenes/new.scene.ron"));
            }
            ui.weak("Открыть: двойной клик по сцене в «Ассетах»");
            ui.separator();
            if ui.button("Выход").clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            }
        });
        ui.menu_button("Правка", |ui| {
            if ui.add_enabled(self.state.can_undo(), egui::Button::new("Отменить        Ctrl+Z")).clicked() {
                self.state.undo();
            }
            if ui.add_enabled(self.state.can_redo(), egui::Button::new("Повторить      Ctrl+Shift+Z")).clicked() {
                self.state.redo();
            }
            ui.separator();
            let sel = self.state.selection;
            if ui.add_enabled(sel.is_some(), egui::Button::new("Дублировать    Ctrl+D")).clicked() {
                self.state.duplicate(sel.unwrap());
            }
            if ui.add_enabled(sel.is_some(), egui::Button::new("Удалить            Del")).clicked() {
                self.state.delete(sel.unwrap());
            }
        });
        ui.menu_button("Объект", |ui| {
            for kind in NewEntity::ALL {
                if ui.button(kind.label()).clicked() {
                    self.state.create(kind, None);
                }
            }
        });
        ui.menu_button("Окно", |ui| {
            ui.menu_button("Добавить панель", |ui| {
                for kind in self.registry.panels() {
                    if ui.button(kind.title).clicked() {
                        let mut alloc = self.layout.alloc();
                        let tab = alloc.tab(kind.id);
                        self.layout.finish_alloc(alloc);
                        let dock = &mut self.layout.workspaces[self.layout.active].dock;
                        if dock.iter_all_tabs().next().is_none() {
                            *dock = DockState::new(vec![tab]);
                        } else {
                            dock.push_to_focused_leaf(tab);
                        }
                    }
                }
            });
            ui.menu_button("Новая рабочая область", |ui| self.new_workspace_menu(ui));
            if ui.button("Сбросить раскладку").clicked() {
                self.layout = Layout::default_for(&self.registry);
                self.panels.clear();
            }
            ui.separator();
            let plugins = self.registry.plugins();
            ui.weak(if plugins.is_empty() {
                "Доп. элементов нет. Ставь через хаб: olvin-hub editor add …".to_owned()
            } else {
                format!("Элементы: {}", plugins.join(", "))
            });
        });
    }

    fn new_workspace_menu(&mut self, ui: &mut egui::Ui) {
        let mut add = None;
        for (i, t) in self.registry.templates().iter().enumerate() {
            if ui.button(t.name).clicked() {
                add = Some(Some(i));
            }
        }
        if ui.button("Пустая").clicked() {
            add = Some(None);
        }
        ui.separator();
        if ui.button("Копия текущей").clicked() {
            self.layout.duplicate(self.layout.active);
        }
        if let Some(template) = add {
            self.apply_ws(WsAction::Add(template));
        }
    }

    /// Browser-like workspace tabs.
    fn workspace_tabs(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let count = self.layout.workspaces.len();
        for i in 0..count {
            if let Some((ri, name)) = &mut self.renaming
                && *ri == i
            {
                let edit = ui.add(TextEdit::singleline(name).desired_width(120.0));
                edit.request_focus();
                if edit.lost_focus() {
                    if !name.trim().is_empty() {
                        self.layout.workspaces[i].name = name.trim().to_owned();
                    }
                    self.renaming = None;
                }
                continue;
            }
            let active = self.layout.active == i;
            let name = &self.layout.workspaces[i].name;
            let text = if active { RichText::new(name).strong() } else { RichText::new(name) };
            let tab = ui.selectable_label(active, text).on_hover_text("Двойной клик — переименовать, СКМ — закрыть");
            if tab.clicked() {
                action = Some(WsAction::Activate(i));
            }
            if tab.double_clicked() {
                action = Some(WsAction::StartRename(i));
            }
            if tab.middle_clicked() {
                action = Some(WsAction::Close(i));
            }
            tab.context_menu(|ui| {
                if ui.button("Переименовать").clicked() {
                    action = Some(WsAction::StartRename(i));
                }
                if ui.button("Дублировать").clicked() {
                    action = Some(WsAction::Duplicate(i));
                }
                if i > 0 && ui.button("◀ Сдвинуть влево").clicked() {
                    action = Some(WsAction::Move(i, i - 1));
                }
                if i + 1 < count && ui.button("Сдвинуть вправо ▶").clicked() {
                    action = Some(WsAction::Move(i, i + 1));
                }
                ui.separator();
                if ui.add_enabled(count > 1, egui::Button::new("Закрыть")).clicked() {
                    action = Some(WsAction::Close(i));
                }
            });
            if active && count > 1 && ui.small_button("×").on_hover_text("Закрыть рабочую область").clicked() {
                action = Some(WsAction::Close(i));
            }
        }
        ui.menu_button("+", |ui| self.new_workspace_menu(ui)).response.on_hover_text("Новая рабочая область");
        if let Some(action) = action {
            self.apply_ws(action);
        }
    }

    fn apply_ws(&mut self, action: WsAction) {
        match action {
            WsAction::Activate(i) => self.layout.active = i,
            WsAction::Close(i) => self.layout.close(i),
            WsAction::StartRename(i) => self.renaming = Some((i, self.layout.workspaces[i].name.clone())),
            WsAction::Duplicate(i) => self.layout.duplicate(i),
            WsAction::Move(from, to) => self.layout.move_workspace(from, to),
            WsAction::Add(Some(t)) => {
                let template = self.registry.templates()[t];
                self.layout.add_workspace(template.name.to_owned(), template.build);
            }
            WsAction::Add(None) => {
                self.layout.add_workspace("Новая".to_owned(), |_| DockState::new(Vec::new()));
                self.renaming = Some((self.layout.active, "Новая".to_owned()));
            }
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let name = self
                .state
                .scene_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "без имени".into());
            ui.label(name);
            if self.state.is_dirty() {
                ui.colored_label(theme::SELECTED, "•").on_hover_text("Есть несохранённые изменения");
            }
            ui.separator();
            ui.weak(format!("объектов: {}", self.state.scene.entities.len()));
            if let Some(e) = self.state.selection.and_then(|id| self.state.scene.get(id)) {
                ui.separator();
                ui.label(format!("выбран: {}", e.name));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(line) = self.state.log.last() {
                    let color = match line.level {
                        LogLevel::Info => ui.visuals().weak_text_color(),
                        LogLevel::Warn => ui.visuals().warn_fg_color,
                        LogLevel::Error => ui.visuals().error_fg_color,
                    };
                    ui.add(egui::Label::new(RichText::new(&line.text).color(color)).truncate());
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(path) = &mut self.save_as {
            let mut close = false;
            let mut target = None;
            egui::Modal::new(Id::new("save-as")).show(ctx, |ui| {
                ui.heading("Сохранить сцену как");
                ui.label("Путь внутри assets/ (или абсолютный):");
                let edit = ui.add(TextEdit::singleline(path).desired_width(360.0));
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.horizontal(|ui| {
                    if ui.button("Сохранить").clicked() || enter {
                        target = Some(path.clone());
                    }
                    if ui.button("Отмена").clicked() {
                        close = true;
                    }
                });
            });
            if let Some(path) = target {
                let mut path = PathBuf::from(path.trim());
                if path.is_relative()
                    && let Some(assets) = self.state.assets_dir()
                {
                    path = assets.join(path);
                }
                if !path.to_string_lossy().ends_with(".scene.ron") {
                    path.set_extension("scene.ron");
                }
                close = self.state.save_as(&path);
            }
            if close {
                self.save_as = None;
            }
        }

        if let Some(confirm) = &self.confirm {
            let what = match confirm {
                Confirm::Open(_) => "открыть другую сцену",
                Confirm::NewScene => "создать новую сцену",
                Confirm::Quit => "выйти",
            };
            let mut choice = None;
            egui::Modal::new(Id::new("confirm")).show(ctx, |ui| {
                ui.heading("Несохранённые изменения");
                ui.label(format!("Сохранить сцену, прежде чем {what}?"));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Сохранить").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Не сохранять").clicked() {
                        choice = Some(2);
                    }
                    if ui.button("Отмена").clicked() {
                        choice = Some(0);
                    }
                });
            });
            if let Some(choice) = choice {
                let action = self.confirm.take().unwrap();
                let proceed = match choice {
                    1 => self.state.save(),
                    2 => true,
                    _ => false,
                };
                if proceed {
                    let quit = matches!(action, Confirm::Quit);
                    self.perform(action);
                    if quit {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                }
            }
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let scene = self
            .state
            .scene_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "без имени".into());
        let project = self
            .state
            .project
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| format!(" — {}", n.to_string_lossy()))
            .unwrap_or_default();
        let dirty = if self.state.is_dirty() { "• " } else { "" };
        let title = format!("{dirty}{scene}{project} — Olvin Editor");
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

struct Viewer<'a> {
    registry: &'a Registry,
    panels: &'a mut HashMap<u32, Box<dyn Panel>>,
    state: &'a mut EditorState,
    added: Vec<(NodePath, &'static str)>,
}

impl TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match self.registry.panel(&tab.kind) {
            Some(kind) => kind.title.into(),
            None => format!("⚠ {}", tab.kind).into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match self.registry.panel(&tab.kind) {
            Some(kind) => self.panels.entry(tab.id).or_insert_with(kind.factory).ui(ui, self.state),
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label(format!(
                        "Элемент «{}» не установлен.\nПоставь его через хаб: olvin-hub editor add …",
                        tab.kind
                    ))
                });
            }
        }
    }

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(("olvin-tab", tab.id))
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }

    fn add_popup(&mut self, ui: &mut egui::Ui, path: NodePath) {
        ui.set_min_width(170.0);
        for kind in self.registry.panels() {
            if ui.button(kind.title).clicked() {
                self.added.push((path, kind.id));
            }
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if ctx.input(|i| i.viewport().close_requested()) && self.state.is_dirty() && !self.allow_close {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Quit);
        }
        if let Some(path) = self.state.open_request.take() {
            self.request(Confirm::Open(path));
        }
        self.shortcuts(&ctx);

        egui::Panel::top("top").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.label(RichText::new("Olvin").strong().color(theme::SELECTED));
                ui.separator();
                self.menus(ui);
                ui.separator();
                self.workspace_tabs(ui);
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));

        let active = self.layout.active;
        let mut added = Vec::new();
        egui::CentralPanel::default_margins().frame(egui::Frame::NONE).show(ui, |ui| {
            let mut viewer = Viewer {
                registry: &self.registry,
                panels: &mut self.panels,
                state: &mut self.state,
                added: Vec::new(),
            };
            let dock = &mut self.layout.workspaces[active].dock;
            if dock.iter_all_tabs().next().is_none() {
                ui.centered_and_justified(|ui| {
                    ui.weak("Пустая рабочая область. Окно → Добавить панель")
                });
            }
            DockArea::new(dock)
                .id(Id::new(("olvin-dock", active)))
                .style(egui_dock::Style::from_egui(ui.style()))
                .show_add_buttons(true)
                .show_add_popup(true)
                .show_leaf_collapse_buttons(false)
                .show_inside(ui, &mut viewer);
            added = viewer.added;
        });
        for (path, kind) in added {
            let mut alloc = self.layout.alloc();
            let tab = alloc.tab(kind);
            self.layout.finish_alloc(alloc);
            let dock = &mut self.layout.workspaces[active].dock;
            dock.set_focused_node_and_surface(path);
            dock.push_to_focused_leaf(tab);
        }

        self.dialogs(&ctx);
        self.update_title(&ctx);

        // Forget panel instances whose tabs were closed.
        let live: HashSet<u32> = self.layout.all_tab_ids().collect();
        self.panels.retain(|id, _| live.contains(id));

        let interacting = ctx.input(|i| i.pointer.any_down()) || ctx.egui_wants_keyboard_input();
        self.state.end_frame(interacting);
    }

    fn on_exit(&mut self) {
        if let Err(err) = self.layout.save() {
            eprintln!("olvin-editor: раскладка не сохранена: {err}");
        }
    }
}

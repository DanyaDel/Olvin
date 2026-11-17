use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use egui::{Id, Key, KeyboardShortcut, Modifiers, RichText, TextEdit, ViewportCommand};
use egui_dock::{DockArea, DockState, NodePath, TabViewer};

use crate::workspace::Layout;
use crate::panels::hierarchy::create_menu;
use crate::{EditorState, LogLevel, Panel, Registry, Tab, theme};

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
    tinted: bool,
}

impl EditorApp {
    pub fn new(registry: Registry, project: Option<PathBuf>, gpu: Option<eframe::egui_wgpu::RenderState>) -> Self {
        let layout = Layout::load(&registry);
        let mut state = EditorState::new(project);
        state.ext = registry.instantiate_extensions();
        state.gpu = gpu;
        if !registry.plugins().is_empty() {
            state.info(format!("installed elements: {}", registry.plugins().join(", ")));
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
            tinted: false,
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
        if self.state.tool_owns_keys {
            return;
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
        ui.menu_button("File", |ui| {
            if ui.button("New Scene        Ctrl+N").clicked() {
                self.request(Confirm::NewScene);
            }
            if ui.button("Save                    Ctrl+S").clicked() {
                self.save();
            }
            if ui.button("Save As…").clicked() {
                self.save_as = Some(String::from("scenes/new.scene.ron"));
            }
            ui.weak("Open: double-click a scene in Assets");
            ui.separator();
            if ui.button("Quit").clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            }
        });
        ui.menu_button("Edit", |ui| {
            if ui.add_enabled(self.state.can_undo(), egui::Button::new("Undo            Ctrl+Z")).clicked() {
                self.state.undo();
            }
            if ui.add_enabled(self.state.can_redo(), egui::Button::new("Redo            Ctrl+Shift+Z")).clicked() {
                self.state.redo();
            }
            ui.separator();
            let sel = self.state.selection;
            if ui.add_enabled(sel.is_some(), egui::Button::new("Duplicate     Ctrl+D")).clicked() {
                self.state.duplicate(sel.unwrap());
            }
            if ui.add_enabled(sel.is_some(), egui::Button::new("Delete           Del")).clicked() {
                self.state.delete(sel.unwrap());
            }
        });
        ui.menu_button("GameObject", |ui| {
            if let Some(creation) = create_menu(ui, &self.state.ext.create) {
                creation.apply(&mut self.state, None);
            }
        });
        ui.menu_button("Window", |ui| {
            ui.menu_button("Add Panel", |ui| {
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
            ui.menu_button("New Workspace", |ui| self.new_workspace_menu(ui));
            if ui.button("Reset Layout").clicked() {
                self.layout = Layout::default_for(&self.registry);
                self.panels.clear();
            }
            ui.separator();
            let plugins = self.registry.plugins();
            ui.weak(if plugins.is_empty() {
                "No extra elements. Install them from the hub".to_owned()
            } else {
                format!("Elements: {}", plugins.join(", "))
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
        if ui.button("Empty").clicked() {
            add = Some(None);
        }
        ui.separator();
        if ui.button("Duplicate Current").clicked() {
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
            let tab = ui.selectable_label(active, text).on_hover_text("Double-click to rename, middle-click to close");
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
                if ui.button("Rename").clicked() {
                    action = Some(WsAction::StartRename(i));
                }
                if ui.button("Duplicate").clicked() {
                    action = Some(WsAction::Duplicate(i));
                }
                if i > 0 && ui.button("◀ Move Left").clicked() {
                    action = Some(WsAction::Move(i, i - 1));
                }
                if i + 1 < count && ui.button("Move Right ▶").clicked() {
                    action = Some(WsAction::Move(i, i + 1));
                }
                ui.separator();
                if ui.add_enabled(count > 1, egui::Button::new("Close")).clicked() {
                    action = Some(WsAction::Close(i));
                }
            });
            if active && count > 1 && ui.small_button("×").on_hover_text("Close workspace").clicked() {
                action = Some(WsAction::Close(i));
            }
        }
        ui.menu_button("+", |ui| self.new_workspace_menu(ui)).response.on_hover_text("New Workspace");
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
                self.layout.add_workspace("New".to_owned(), |_| DockState::new(Vec::new()));
                self.renaming = Some((self.layout.active, "New".to_owned()));
            }
        }
    }

    fn toggle_play(&mut self) {
        if self.state.is_playing() {
            self.state.stop_play();
        } else {
            self.state.start_play();
            crate::engine::reset_play_gpu(&self.state);
        }
    }

    /// ▶ ⏸ ⏭, like Unity's toolbar.
    fn play_controls(&mut self, ui: &mut egui::Ui) {
        let playing = self.state.is_playing();
        let paused = self.state.play.as_ref().is_some_and(|p| p.paused);
        // Right-to-left layout: added in reverse.
        if ui.add_enabled(playing, egui::Button::new("⏭")).on_hover_text("Step one frame").clicked()
            && let Some(p) = &mut self.state.play
        {
            p.paused = true;
            p.step_once();
        }
        if ui.add_enabled(playing, egui::Button::selectable(paused, "⏸")).on_hover_text("Pause").clicked()
            && let Some(p) = &mut self.state.play
        {
            p.paused = !p.paused;
        }
        let play = egui::Button::selectable(playing, RichText::new("▶").color(if playing { theme::ACCENT } else { ui.visuals().text_color() }));
        if ui.add(play).on_hover_text(if playing { "Stop (Ctrl+P)" } else { "Play (Ctrl+P)" }).clicked() {
            self.toggle_play();
        }
    }

    /// Unity tints the whole editor while playing, so edits that will be lost are obvious.
    fn tint(&mut self, ctx: &egui::Context) {
        let playing = self.state.is_playing();
        if playing == self.tinted {
            return;
        }
        self.tinted = playing;
        theme::apply(ctx);
        if playing {
            ctx.all_styles_mut(|style| {
                let v = &mut style.visuals;
                let tint = |c: egui::Color32| egui::Color32::from_rgb(c.r().saturating_sub(4), c.g().saturating_sub(2), c.b().saturating_add(14));
                v.panel_fill = tint(v.panel_fill);
                v.window_fill = tint(v.window_fill);
                v.extreme_bg_color = tint(v.extreme_bg_color);
            });
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
                .unwrap_or_else(|| "untitled".into());
            ui.label(name);
            if self.state.is_dirty() {
                ui.colored_label(theme::SELECTED, "•").on_hover_text("Unsaved changes");
            }
            ui.separator();
            ui.weak(format!("{} objects", self.state.scene.entities.len()));
            if let Some(e) = self.state.selection.and_then(|id| self.state.scene.get(id)) {
                ui.separator();
                ui.label(format!("selected: {}", e.name));
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
                ui.heading("Save Scene As");
                ui.label("Path inside assets/ (or absolute):");
                let edit = ui.add(TextEdit::singleline(path).desired_width(360.0));
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() || enter {
                        target = Some(path.clone());
                    }
                    if ui.button("Cancel").clicked() {
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
                Confirm::Open(_) => "opening another scene",
                Confirm::NewScene => "creating a new scene",
                Confirm::Quit => "quitting",
            };
            let mut choice = None;
            egui::Modal::new(Id::new("confirm")).show(ctx, |ui| {
                ui.heading("Unsaved Changes");
                ui.label(format!("Save the scene before {what}?"));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Don't Save").clicked() {
                        choice = Some(2);
                    }
                    if ui.button("Cancel").clicked() {
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
            .unwrap_or_else(|| "untitled".into());
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
                        "Element \"{}\" is not installed.\nAdd it in the hub (Editor → Elements).",
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
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::P))) {
            self.toggle_play();
        }
        if let Some(play) = &mut self.state.play {
            play.tick();
            ctx.request_repaint();
        }
        self.tint(&ctx);
        self.state.tool_owns_keys = false;

        egui::Panel::top("top").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.label(RichText::new("Olvin").strong().color(theme::SELECTED));
                ui.separator();
                self.menus(ui);
                ui.separator();
                self.workspace_tabs(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.play_controls(ui));
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
                    ui.weak("Empty workspace. Window → Add Panel")
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
            eprintln!("olvin-editor: layout not saved: {err}");
        }
    }
}

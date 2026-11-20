use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use egui::{Id, Key, KeyboardShortcut, Modifiers, RichText, TextEdit, ViewportCommand};
use egui_dock::{DockArea, DockState, NodePath, TabViewer};

use crate::panels::hierarchy::create_menu;
use crate::theme::{self, anim};
use crate::workspace::Layout;
use crate::{EditorState, LogLevel, Panel, Registry, Tab};

/// A top-bar menu whose title fades on hover (STYLE.md §7). Popups fade in
/// through egui's `animation_time`, set by the theme.
fn menu<R>(
    ui: &mut egui::Ui,
    title: impl Into<RichText>,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::Response {
    let button = anim::button(ui, title, false);
    egui::containers::menu::MenuButton::from_button(button)
        .ui(ui, content)
        .0
}

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
    pub fn new(
        registry: Registry,
        project: Option<PathBuf>,
        gpu: Option<eframe::egui_wgpu::RenderState>,
    ) -> Self {
        let layout = Layout::load(&registry);
        let mut state = EditorState::new(project);
        state.prefs = crate::prefs::Preferences::load();
        state.ext = registry.instantiate_extensions();
        state.gpu = gpu;
        if !registry.plugins().is_empty() {
            state.info(format!(
                "installed elements: {}",
                registry.plugins().join(", ")
            ));
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

    /// The scene, selection and preferences, for tools that drive the editor
    /// without a window (headless screenshots).
    pub fn state_mut(&mut self) -> &mut EditorState {
        &mut self.state
    }

    /// Default workspaces instead of the user's saved layout.
    pub fn reset_layout(&mut self) {
        self.layout = Layout::default_for(&self.registry);
        self.panels.clear();
    }

    /// Switches to the workspace called `name`; false if there is none.
    pub fn show_workspace(&mut self, name: &str) -> bool {
        match self.layout.workspaces.iter().position(|w| w.name == name) {
            Some(i) => {
                self.layout.active = i;
                true
            }
            None => false,
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
        let selected = self.state.selected_ids();
        if duplicate && !selected.is_empty() {
            let copies: Vec<u64> = selected
                .iter()
                .filter_map(|id| self.state.duplicate(*id))
                .collect();
            self.state.multi = copies.clone();
            self.state.selection = copies.last().copied();
        }
        if delete {
            for id in selected {
                self.state.delete(id);
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
        menu(ui, theme::menu_title("File"), |ui| {
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
        menu(ui, theme::menu_title("Edit"), |ui| {
            if ui.button("Settings…        Ctrl+,").clicked() {
                self.state.open_settings(crate::settings::Page::default());
            }
            ui.separator();
            if ui
                .add_enabled(
                    self.state.can_undo(),
                    egui::Button::new("Undo            Ctrl+Z"),
                )
                .clicked()
            {
                self.state.undo();
            }
            if ui
                .add_enabled(
                    self.state.can_redo(),
                    egui::Button::new("Redo            Ctrl+Shift+Z"),
                )
                .clicked()
            {
                self.state.redo();
            }
            ui.separator();
            let sel = self.state.selection;
            if ui
                .add_enabled(sel.is_some(), egui::Button::new("Duplicate     Ctrl+D"))
                .clicked()
                && let Some(id) = sel
            {
                self.state.duplicate(id);
            }
            if ui
                .add_enabled(sel.is_some(), egui::Button::new("Delete           Del"))
                .clicked()
                && let Some(id) = sel
            {
                self.state.delete(id);
            }
        });
        menu(ui, theme::menu_title("GameObject"), |ui| {
            if let Some(creation) = create_menu(ui, &self.state.ext.create) {
                creation.apply(&mut self.state, None);
            }
        });
        menu(ui, theme::menu_title("Window"), |ui| {
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
            let text = if active {
                RichText::new(name).strong()
            } else {
                RichText::new(name)
            };
            let tab = ui
                .add(anim::button(ui, text, active))
                .on_hover_text("Double-click to rename, middle-click to close");
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
                if ui
                    .add_enabled(count > 1, egui::Button::new("Close"))
                    .clicked()
                {
                    action = Some(WsAction::Close(i));
                }
            });
            if active
                && count > 1
                && ui
                    .small_button("×")
                    .on_hover_text("Close workspace")
                    .clicked()
            {
                action = Some(WsAction::Close(i));
            }
        }
        menu(ui, "+", |ui| self.new_workspace_menu(ui)).on_hover_text("New Workspace");
        if let Some(action) = action {
            self.apply_ws(action);
        }
    }

    fn apply_ws(&mut self, action: WsAction) {
        match action {
            WsAction::Activate(i) => self.layout.active = i,
            WsAction::Close(i) => self.layout.close(i),
            WsAction::StartRename(i) => {
                self.renaming = Some((i, self.layout.workspaces[i].name.clone()))
            }
            WsAction::Duplicate(i) => self.layout.duplicate(i),
            WsAction::Move(from, to) => self.layout.move_workspace(from, to),
            WsAction::Add(Some(t)) => {
                let template = self.registry.templates()[t];
                self.layout
                    .add_workspace(template.name.to_owned(), template.build);
            }
            WsAction::Add(None) => {
                self.layout
                    .add_workspace("New".to_owned(), |_| DockState::new(Vec::new()));
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
        egui::Frame::new()
            .fill(theme::palette(ui.ctx()).surface)
            .corner_radius(theme::RADIUS_SM)
            .inner_margin(egui::Margin::symmetric(4, 2))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                // Selected icons ease to the accent; hover fades in.
                let icon = |ui: &egui::Ui, text: &str, on: bool| {
                    anim::button(ui, RichText::new(text).size(15.0), on)
                        .min_size(egui::vec2(34.0, 24.0))
                };
                let play = ui.add(icon(ui, "▶", playing)).on_hover_text(if playing {
                    "Stop (Ctrl+P)"
                } else {
                    "Play (Ctrl+P)"
                });
                if play.clicked() {
                    self.toggle_play();
                }
                if ui
                    .add_enabled(playing, icon(ui, "⏸", paused))
                    .on_hover_text("Pause")
                    .clicked()
                    && let Some(p) = &mut self.state.play
                {
                    p.paused = !p.paused;
                }
                if ui
                    .add_enabled(playing, icon(ui, "⏭", false))
                    .on_hover_text("Step one frame")
                    .clicked()
                    && let Some(p) = &mut self.state.play
                {
                    p.paused = true;
                    p.step_once();
                }
            });
    }

    /// Follows the theme and motion settings from preferences.
    fn sync_theme(&mut self, ctx: &egui::Context) {
        let prefs = &self.state.prefs;
        if theme::reduce_motion(ctx) != prefs.reduce_motion {
            theme::set_reduce_motion(ctx, prefs.reduce_motion);
        }
        if theme::kind(ctx) != prefs.theme {
            theme::apply_kind(ctx, prefs.theme);
            // The play tint was applied on top of the old visuals.
            self.tinted = false;
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
                let tint = |c: egui::Color32| {
                    egui::Color32::from_rgb(
                        c.r().saturating_sub(4),
                        c.g().saturating_sub(2),
                        c.b().saturating_add(14),
                    )
                };
                v.panel_fill = tint(v.panel_fill);
                v.window_fill = tint(v.window_fill);
                v.extreme_bg_color = tint(v.extreme_bg_color);
            });
        }
    }

    /// "● PLAYING" with a slowly breathing dot (STYLE.md §6 status), or PAUSED.
    fn play_status(&self, ui: &mut egui::Ui) {
        let Some(play) = &self.state.play else {
            return;
        };
        let p = theme::palette(ui.ctx());
        let (color, label) = if play.paused {
            (p.muted, "Paused")
        } else {
            let breath = anim::pulse(ui.ctx(), 1.8);
            (p.accent.gamma_multiply(0.55 + 0.45 * breath), "Playing")
        };
        let (dot, _) = ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
        ui.painter().circle_filled(dot.center(), 3.0, color);
        ui.label(theme::caps(label).color(if play.paused { p.muted } else { p.accent }));
        ui.separator();
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.play_status(ui);
            let name = self
                .state
                .scene_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "untitled".into());
            ui.label(name);
            if self.state.is_dirty() {
                ui.colored_label(theme::palette(ui.ctx()).warn, "•")
                    .on_hover_text("Unsaved changes");
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
                    // A new message fades in where the old one was; no movement.
                    let key = (self.state.log.len(), &line.text);
                    let shown = anim::appear(ui.ctx(), Id::new("olvin-status-message"), key);
                    let color = color.gamma_multiply(shown);
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
            if let Some(choice) = choice
                && let Some(action) = self.confirm.take()
            {
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
    ctx: egui::Context,
    /// Tab id → (front tab of its leaf, front tab of the focused leaf).
    tab_states: HashMap<u32, (bool, bool)>,
}

/// Which tabs are in front, and which one is focused, for the tab easing.
fn tab_states(dock: &DockState<Tab>) -> HashMap<u32, (bool, bool)> {
    let focused = dock.focused_leaf();
    let mut states = HashMap::new();
    for (path, leaf) in dock.iter_leaves() {
        for (i, tab) in leaf.tabs().iter().enumerate() {
            let front = i == leaf.active.0;
            states.insert(tab.id, (front, front && Some(path) == focused));
        }
    }
    states
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
            Some(kind) => self
                .panels
                .entry(tab.id)
                .or_insert_with(kind.factory)
                .ui(ui, self.state),
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

    /// Eases the tab's fill and text between inactive, front and focused.
    fn tab_style_override(
        &self,
        tab: &Tab,
        global: &egui_dock::TabStyle,
    ) -> Option<egui_dock::TabStyle> {
        let (front, focused) = self.tab_states.get(&tab.id).copied().unwrap_or_default();
        let id = Id::new(("olvin-tab-ease", tab.id));
        let front_t = anim::hover_t(&self.ctx, id.with("front"), front);
        let focused_t = anim::hover_t(&self.ctx, id.with("focused"), focused);
        Some(theme::eased_tab_style(global, front_t, focused_t))
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
        self.frame(ui);
    }

    fn on_exit(&mut self) {
        if let Err(err) = self.layout.save() {
            eprintln!("olvin-editor: layout not saved: {err}");
        }
    }
}

impl EditorApp {
    /// One editor frame: shortcuts, top bar, status bar, dock, windows.
    fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        if ctx.input(|i| i.viewport().close_requested())
            && self.state.is_dirty()
            && !self.allow_close
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Quit);
        }
        if let Some(path) = self.state.open_request.take() {
            self.request(Confirm::Open(path));
        }
        self.shortcuts(&ctx);
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::P)))
        {
            self.toggle_play();
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma))
        }) {
            self.state.settings_page = match self.state.settings_page {
                Some(_) => None,
                None => Some(crate::settings::Page::default()),
            };
        }
        if ctx.zoom_factor() != self.state.prefs.ui_scale && !ctx.input(|i| i.pointer.any_down()) {
            ctx.set_zoom_factor(self.state.prefs.ui_scale);
        }
        if let Some(play) = &mut self.state.play {
            play.tick();
            ctx.request_repaint();
        }
        self.sync_theme(&ctx);
        self.tint(&ctx);
        let palette = theme::palette(&ctx);
        self.state.keys_owned_last_frame = self.state.tool_owns_keys;
        self.state.tool_owns_keys = false;

        let bar = egui::Frame::new()
            .fill(palette.bg)
            .inner_margin(egui::Margin::symmetric(10, 6));
        egui::Panel::top("top").frame(bar).show(ui, |ui| {
            let mut tabs_end = 0.0;
            egui::MenuBar::new().ui(ui, |ui| {
                // The wordmark, painted from the logo geometry (STYLE.md §9).
                ui.add_space(2.0);
                theme::logo::wordmark(ui, 16.0).on_hover_text("Olvin · by NLPA Collab");
                ui.add_space(10.0);
                self.menus(ui);
                ui.add_space(10.0);
                self.workspace_tabs(ui);
                tabs_end = ui.cursor().left();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let open = self.state.settings_page.is_some();
                    if ui
                        .add(anim::button(ui, RichText::new("⚙").size(16.0), open))
                        .on_hover_text("Settings (Ctrl+,)")
                        .clicked()
                    {
                        self.state.settings_page = if open {
                            None
                        } else {
                            Some(crate::settings::Page::default())
                        };
                    }
                });
            });
            // Play controls centred over the window, like Unity; pushed right
            // when the workspace tabs reach the centre so they never overlap.
            let mut center = egui::Rect::from_center_size(
                ui.max_rect().center(),
                egui::vec2(124.0, ui.max_rect().height()),
            );
            let min_left = tabs_end + 16.0;
            if center.left() < min_left {
                center = center.translate(egui::vec2(min_left - center.left(), 0.0));
            }
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(center)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| self.play_controls(ui),
            );
        });
        let status = egui::Frame::new()
            .fill(palette.footer)
            .inner_margin(egui::Margin::symmetric(12, 4));
        egui::Panel::bottom("status")
            .frame(status)
            .show(ui, |ui| self.status_bar(ui));

        let active = self.layout.active;
        let mut added = Vec::new();
        egui::CentralPanel::default_margins()
            .frame(egui::Frame::new().fill(palette.bg))
            .show(ui, |ui| {
                let dock = &mut self.layout.workspaces[active].dock;
                let mut viewer = Viewer {
                    registry: &self.registry,
                    panels: &mut self.panels,
                    state: &mut self.state,
                    added: Vec::new(),
                    ctx: ui.ctx().clone(),
                    tab_states: tab_states(dock),
                };
                if dock.iter_all_tabs().next().is_none() {
                    ui.centered_and_justified(|ui| ui.weak("Empty workspace. Window → Add Panel"));
                }
                DockArea::new(dock)
                    .id(Id::new(("olvin-dock", active)))
                    .style(theme::dock_style(ui.style(), &palette))
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

        crate::settings::window(&ctx, &mut self.state);
        self.dialogs(&ctx);
        self.update_title(&ctx);

        // Forget panel instances whose tabs were closed.
        let live: HashSet<u32> = self.layout.all_tab_ids().collect();
        self.panels.retain(|id, _| live.contains(id));

        let interacting = ctx.input(|i| i.pointer.any_down())
            || ctx.egui_wants_keyboard_input()
            || self.state.busy;
        self.state.end_frame(interacting);
        self.state.busy = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole chrome (top bar, menus, dock, status bar) under every theme,
    /// stopped and playing, without a GPU.
    #[test]
    fn chrome_draws_headless_in_every_theme() {
        let mut registry = Registry::default();
        crate::panels::register(&mut registry);
        let mut app = EditorApp::new(registry, None, None);
        // Deterministic: not the user's saved layout and preferences.
        app.layout = Layout::default_for(&app.registry);
        app.state.prefs = crate::prefs::Preferences::default();

        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let mut time = 0.0;
        let mut frame = |app: &mut EditorApp| {
            time += 0.05;
            let input = egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 860.0),
                )),
                ..Default::default()
            };
            ctx.run_ui(input, |ui| app.frame(ui)).textures_delta.clear();
        };
        for kind in theme::ThemeKind::ALL {
            app.state.prefs.theme = kind;
            for _ in 0..3 {
                frame(&mut app);
            }
            assert_eq!(theme::kind(&ctx), kind);
            assert_eq!(
                ctx.global_style().visuals.panel_fill,
                kind.palette().surface
            );
        }
        app.state.prefs.reduce_motion = true;
        app.toggle_play();
        for _ in 0..3 {
            frame(&mut app);
        }
        assert!(theme::reduce_motion(&ctx));
        app.toggle_play();
        frame(&mut app);
    }
}

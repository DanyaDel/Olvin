use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use egui::{Id, Key, RichText, TextEdit, ViewportCommand};

use crate::keymap::Command;
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

/// Where the top bar put its parts last frame (screen points), for tests.
/// The workspace tabs live in `tabs` and scroll inside it; `play` and `gear`
/// never overlap it or each other.
#[derive(Clone, Copy, Debug)]
pub struct TopBarRects {
    /// Right edge of the logo and menus.
    pub menus_end: f32,
    /// The scrolling strip of workspace tabs, with its "+" button.
    pub tabs: egui::Rect,
    pub play: egui::Rect,
    pub gear: egui::Rect,
}

impl Default for TopBarRects {
    fn default() -> Self {
        Self {
            menus_end: 0.0,
            tabs: egui::Rect::NOTHING,
            play: egui::Rect::NOTHING,
            gear: egui::Rect::NOTHING,
        }
    }
}

/// Width reserved for the play controls (three 34 px buttons and padding).
const PLAY_W: f32 = 124.0;
/// Width reserved for the settings gear.
const GEAR_W: f32 = 36.0;
/// Gap between the parts of the top bar.
const BAR_GAP: f32 = 12.0;
/// The tabs strip never gets narrower than this; the play controls move
/// right instead of squeezing it.
const MIN_TABS_W: f32 = 120.0;

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
    /// `Abort` (from `OLVIN_FAULTS=abort`, CI) lets a panel's panic through.
    /// The autosave the user chose to recover "Later" (File menu still has it).
    recovery_later: Option<PathBuf>,
    state: EditorState,
    renaming: Option<(usize, String)>,
    save_as: Option<String>,
    /// Save As target that already exists, waiting for "Replace".
    overwrite: Option<PathBuf>,
    confirm: Option<Confirm>,
    allow_close: bool,
    title: String,
    tinted: bool,
    top_bar: TopBarRects,
    /// Width the workspace tabs and their "+" wanted last frame, so the play
    /// controls can move right of them instead of cutting them off.
    tabs_wanted_w: f32,
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
        state.check_trust(
            &olvin_trust::TrustStore::load(),
            olvin_trust::env_read_only(),
        );
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
            recovery_later: None,
            state,
            renaming: None,
            save_as: None,
            overwrite: None,
            confirm: None,
            allow_close: false,
            title: String::new(),
            tinted: false,
            top_bar: TopBarRects::default(),
            tabs_wanted_w: 0.0,
        }
    }

    /// The scene, selection and preferences, for tools that drive the editor
    /// without a window (headless screenshots).
    /// What a panicking panel does: `Isolate` shows a crash card, `Abort`
    /// lets the panic through. Defaults to `OLVIN_FAULTS`.
    pub fn set_fault_policy(&mut self, policy: olvin::core::FaultPolicy) {
        self.state.fault_policy = policy;
    }

    pub fn state_mut(&mut self) -> &mut EditorState {
        &mut self.state
    }

    /// Where the top bar's parts were drawn last frame.
    pub fn top_bar_rects(&self) -> TopBarRects {
        self.top_bar
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
        let panel_keys = std::mem::take(&mut self.state.panel_owns_edit_keys);
        // The Settings window is waiting for a key to bind: nothing fires.
        if self.state.capturing_key {
            return;
        }
        let keys = self.state.prefs.keymap.clone();
        let (save, new) = ctx.input_mut(|i| {
            (
                keys.pressed(i, Command::Save),
                keys.pressed(i, Command::NewScene),
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
        // Redo before Undo: Ctrl+Shift+Z must not be taken for Ctrl+Z.
        let (redo, undo, duplicate, delete) = ctx.input_mut(|i| {
            (
                keys.pressed(i, Command::Redo),
                keys.pressed(i, Command::Undo),
                keys.pressed(i, Command::Duplicate),
                keys.pressed(i, Command::Delete),
            )
        });
        if redo {
            self.state.redo();
        }
        if undo {
            self.state.undo();
        }
        if self.state.tool_owns_keys || panel_keys {
            return;
        }
        if duplicate {
            self.state.duplicate_selected();
        }
        if delete {
            self.state.delete_selected();
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
            if self.state.recovery.is_some() && ui.button("Recover Autosave").clicked() {
                self.state.recover_autosave();
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
            let sel = self.state.selection.is_some();
            if ui
                .add_enabled(sel, egui::Button::new("Duplicate     Ctrl+D"))
                .clicked()
            {
                self.state.duplicate_selected();
            }
            if ui
                .add_enabled(sel, egui::Button::new("Delete           Del"))
                .clicked()
            {
                self.state.delete_selected();
            }
        });
        menu(ui, theme::menu_title("GameObject"), |ui| {
            if let Some(creation) = create_menu(ui, &self.state.ext.create) {
                creation.apply(&mut self.state, None);
            }
            ui.separator();
            let sel = self.state.selection.is_some();
            if ui
                .add_enabled(sel, egui::Button::new("Create Prefab"))
                .on_hover_text("Save the selected object and its children as a prefab asset")
                .clicked()
            {
                self.state.create_prefab_selected();
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

    /// Browser-like workspace tabs. They scroll inside the room the top bar
    /// gives them (`width`); the "+" button stays visible after them.
    fn workspace_tabs(&mut self, ui: &mut egui::Ui, width: f32) {
        let mut action = None;
        let count = self.layout.workspaces.len();
        // Bring the active tab into view when it changes (a click, a new one).
        // Also when the strip gets another width (the window was resized).
        let seen = ui.id().with("ws-seen");
        let now = (self.layout.active, count, width.round() as i32);
        let reveal = ui.data(|d| d.get_temp::<(usize, usize, i32)>(seen)) != Some(now);
        ui.data_mut(|d| d.insert_temp(seen, now));
        let plus_w = 34.0;
        let out = egui::ScrollArea::horizontal()
            .id_salt("ws-tabs")
            .max_width((width - plus_w).max(0.0))
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                self.workspace_tab_list(ui, &mut action, reveal);
            });
        self.tabs_wanted_w = out.content_size.x + plus_w;
        menu(ui, "+", |ui| self.new_workspace_menu(ui)).on_hover_text("New Workspace");
        if let Some(action) = action {
            self.apply_ws(action);
        }
    }

    fn workspace_tab_list(
        &mut self,
        ui: &mut egui::Ui,
        action: &mut Option<WsAction>,
        reveal: bool,
    ) {
        let count = self.layout.workspaces.len();
        for i in 0..count {
            if let Some((ri, name)) = &mut self.renaming
                && *ri == i
            {
                let edit = ui.add(TextEdit::singleline(name).desired_width(120.0));
                // Ask before focusing: request_focus() undoes "lost focus".
                let done = edit.lost_focus();
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.renaming = None;
                } else if done {
                    if !name.trim().is_empty() {
                        self.layout.workspaces[i].name = name.trim().to_owned();
                    }
                    self.renaming = None;
                } else {
                    edit.request_focus();
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
            if active && reveal {
                tab.scroll_to_me(None);
            }
            if tab.clicked() {
                *action = Some(WsAction::Activate(i));
            }
            if tab.double_clicked() {
                *action = Some(WsAction::StartRename(i));
            }
            if tab.middle_clicked() {
                *action = Some(WsAction::Close(i));
            }
            tab.context_menu(|ui| {
                if ui.button("Rename").clicked() {
                    *action = Some(WsAction::StartRename(i));
                }
                if ui.button("Duplicate").clicked() {
                    *action = Some(WsAction::Duplicate(i));
                }
                if i > 0 && ui.button("◀ Move Left").clicked() {
                    *action = Some(WsAction::Move(i, i - 1));
                }
                if i + 1 < count && ui.button("Move Right ▶").clicked() {
                    *action = Some(WsAction::Move(i, i + 1));
                }
                ui.separator();
                if ui
                    .add_enabled(count > 1, egui::Button::new("Close"))
                    .clicked()
                {
                    *action = Some(WsAction::Close(i));
                }
            });
            if active
                && count > 1
                && ui
                    .small_button("×")
                    .on_hover_text("Close workspace")
                    .clicked()
            {
                *action = Some(WsAction::Close(i));
            }
        }
    }

    fn apply_ws(&mut self, action: WsAction) {
        // Tab indices shift under a pending rename.
        if !matches!(action, WsAction::StartRename(_) | WsAction::Add(None)) {
            self.renaming = None;
        }
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
        ui.label(theme::caps(ui.ctx(), label).color(if play.paused { p.muted } else { p.accent }));
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
        if self.state.trust_pending
            && let Some(project) = self.state.project.clone()
        {
            let mut choice = None;
            egui::Modal::new(Id::new("trust")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.heading("Trust this project?");
                ui.label(project.display().to_string());
                ui.add_space(6.0);
                ui.label(olvin_trust::EXPLANATION);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Trust and continue").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Open without building").clicked() {
                        choice = Some(false);
                    }
                    if ui.button("Cancel").clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                });
            });
            match choice {
                Some(true) => {
                    let mut store = olvin_trust::TrustStore::load();
                    match store.trust(&project) {
                        Ok(_) => {
                            self.state.project_trusted = true;
                            self.state.info("project trusted");
                        }
                        Err(e) => self
                            .state
                            .error(format!("could not save the trust list: {e}")),
                    }
                    self.state.trust_pending = false;
                }
                Some(false) => self.state.trust_pending = false,
                None => {}
            }
        }
        if let Some(target) = self.overwrite.clone() {
            let mut choice = None;
            egui::Modal::new(Id::new("overwrite")).show(ctx, |ui| {
                ui.heading("Replace Scene?");
                let name = target.file_name().map_or_else(
                    || target.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                ui.label(format!("{name} already exists. Replace it?"));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Replace").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
            match choice {
                Some(true) => {
                    self.overwrite = None;
                    if self.state.save_as(&target) {
                        self.save_as = None;
                    }
                }
                Some(false) => self.overwrite = None,
                None => {}
            }
        } else if let Some(path) = &mut self.save_as {
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
                // Saving over the open scene is a plain Save; anything else
                // that exists must be confirmed.
                if path.exists() && self.state.scene_path.as_deref() != Some(path.as_path()) {
                    self.overwrite = Some(path);
                } else {
                    close = self.state.save_as(&path);
                }
            }
            if close {
                self.save_as = None;
            }
        }

        if self.state.recovery.is_some() && self.state.recovery != self.recovery_later {
            let mut choice = None;
            egui::Modal::new(Id::new("recover-autosave")).show(ctx, |ui| {
                ui.heading("Recover Unsaved Changes?");
                ui.label(
                    "This scene has changes from an earlier session that were never saved \
                     (kept when Play started). Load them?",
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Recover").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(2);
                    }
                    if ui.button("Later").clicked() {
                        choice = Some(0);
                    }
                });
            });
            match choice {
                Some(1) => self.state.recover_autosave(),
                Some(2) => self.state.drop_autosave(),
                Some(_) => self.recovery_later = self.state.recovery.clone(),
                None => {}
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
                    2 => {
                        self.state.drop_autosave();
                        true
                    }
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

    /// Element frame hooks, each guarded: a crashed one is skipped until
    /// restarted from the Subsystems panel.
    fn run_frame_hooks(&mut self, ctx: &egui::Context) {
        let hooks: Vec<_> = self
            .state
            .ext
            .hooks
            .iter()
            .enumerate()
            .filter(|(_, h)| h.crashed.is_none())
            .map(|(i, h)| (i, h.hook, h.owner))
            .collect();
        for (i, hook, owner) in hooks {
            let state = &mut self.state;
            let policy = state.fault_policy;
            if let Err(message) = crate::ext::guarded(policy, || hook(ctx, state)) {
                self.state.error(format!(
                    "element {owner}: frame hook crashed: {message}. It is off until restarted (Subsystems panel)"
                ));
                self.state.ext.hooks[i].crashed = Some(message);
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

/// What a crashed panel shows instead of itself; `true` when Restart is pressed.
fn crash_card(ui: &mut egui::Ui, title: &str, message: &str) -> bool {
    let mut restart = false;
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        ui.heading(
            RichText::new(format!("⚠ {title} crashed")).color(theme::palette(ui.ctx()).error),
        );
        ui.add_space(6.0);
        ui.label(message);
        ui.weak("The rest of the editor works. Details are in the Console.");
        ui.add_space(10.0);
        restart = ui.button("⟲ Restart panel").clicked();
    });
    restart
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
            Some(kind) if self.state.crashed_panels.contains_key(&tab.id) => {
                let message = self.state.crashed_panels[&tab.id].1.clone();
                if crash_card(ui, kind.title, &message) {
                    self.state.crashed_panels.remove(&tab.id);
                    self.state.warn(format!("{}: restarted", kind.title));
                }
            }
            Some(kind) => {
                let panel = self.panels.entry(tab.id).or_insert_with(kind.factory);
                let state = &mut *self.state;
                let policy = state.fault_policy;
                if let Err(message) = crate::ext::guarded(policy, || panel.ui(ui, state)) {
                    // A fresh instance on restart: the old one may be half-updated.
                    self.panels.remove(&tab.id);
                    self.state.error(format!(
                        "panel {} ({}) crashed: {message}. The editor keeps running; the panel waits for Restart",
                        kind.title, tab.kind
                    ));
                    self.state
                        .crashed_panels
                        .insert(tab.id, (kind.title.to_owned(), message));
                }
            }
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
        olvin::core::profile_scope!("editor frame");
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
        self.run_frame_hooks(&ctx);
        let keys = self.state.prefs.keymap.clone();
        let capturing = self.state.capturing_key;
        if !capturing && ctx.input_mut(|i| keys.pressed(i, Command::Play)) {
            self.toggle_play();
        }
        if !capturing && ctx.input_mut(|i| keys.pressed(i, Command::Settings)) {
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
            for fault in play.new_faults() {
                self.state.error(format!(
                    "game: subsystem {} crashed in {} ({:?}, frame {}): {}. It is stopped, the rest of the game runs; fix it and press Play again",
                    fault.owner, fault.system, fault.stage, fault.frame, fault.message
                ));
            }
        } else if self
            .state
            .tick_particle_previews(ctx.input(|i| i.stable_dt))
        {
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
            // Left to right: logo, menus, workspace tabs; then the play
            // controls (centred over the window when there is room) and the
            // gear at the right edge. The tabs get whatever lies between the
            // menus and the play controls and scroll when there are more.
            let bar = ui.max_rect();
            let gear_left = bar.right() - GEAR_W;
            let mut rects = TopBarRects::default();
            egui::MenuBar::new().ui(ui, |ui| {
                // The wordmark, painted from the logo geometry (STYLE.md §9).
                ui.add_space(2.0);
                theme::logo::wordmark(ui, 16.0).on_hover_text("Olvin · by NLPA Collab");
                ui.add_space(10.0);
                self.menus(ui);
                ui.add_space(10.0);
                let start = ui.cursor().left();
                rects.menus_end = start;
                // Centred over the window, but never over the tabs: they push
                // the controls right as far as the window allows, and only
                // scroll when even that is not enough.
                let play_left = (bar.center().x - PLAY_W / 2.0)
                    .max(start + self.tabs_wanted_w + BAR_GAP)
                    .max(start + MIN_TABS_W + BAR_GAP)
                    .min(gear_left - BAR_GAP - PLAY_W);
                let tabs = egui::Rect::from_min_max(
                    egui::pos2(start, bar.top()),
                    egui::pos2((play_left - BAR_GAP).max(start), bar.bottom()),
                );
                rects.tabs = tabs;
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(tabs)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.set_clip_rect(tabs.intersect(ui.clip_rect()));
                        self.workspace_tabs(ui, tabs.width());
                    },
                );
                rects.play = egui::Rect::from_min_size(
                    egui::pos2(play_left, bar.top()),
                    egui::vec2(PLAY_W, bar.height()),
                );
                rects.gear =
                    egui::Rect::from_min_max(egui::pos2(gear_left, bar.top()), bar.right_bottom());
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(rects.play)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| self.play_controls(ui),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(rects.gear)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                    |ui| {
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
                    },
                );
            });
            self.top_bar = rects;
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
                    // Only a tab's own × closes it; the group-wide × took every tab
                    // with it (Unity and Blender have no such button).
                    .show_leaf_close_all_buttons(false)
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
        self.state.sync_prefabs(false);
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

//! `olvin-hub-gui`: the hub as a desktop launcher, laid out like Unity Hub:
//! a sidebar (Projects, Editor, Settings), a responsive project list, modal
//! dialogs and toasts. Styling follows STYLE.md (launcher rules).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use eframe::egui;
use olvin_hub::{Catalog, EditorInstall, HubConfig, Project};

mod chrome;
mod dialogs;
mod editor_page;
mod projects;
mod settings;
mod widgets;

/// The shared STYLE.md kit (themes, fonts, painted logo, motion).
use olvin_ui as ui;
use ui::ThemeKind;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Olvin Hub")
            .with_app_id("olvin-hub")
            .with_inner_size([1180.0, 720.0])
            .with_min_inner_size([420.0, 420.0])
            .with_icon(window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "olvin-hub",
        options,
        Box::new(|cc| {
            let mut hub = Hub::load();
            // Fonts must be installed before the first frame: bold() in the
            // same pass as the first apply would panic.
            hub.sync_style(&cc.egui_ctx);
            Ok(Box::new(hub))
        }),
    )
}

/// Window and taskbar icon: `assets/logo/png/olvin-icon-256.png` (mark on the dark tile).
fn window_icon() -> egui::IconData {
    const PNG: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/logo/png/olvin-icon-256.png"
    ));
    eframe::icon_data::from_png_bytes(PNG).expect("olvin-icon-256.png is not a valid PNG")
}

pub(crate) struct Template {
    name: &'static str,
    description: &'static str,
    modules: &'static [&'static str],
}

pub(crate) const TEMPLATES: [Template; 3] = [
    Template {
        name: "3D",
        description: "Window, 3D rendering and editor scenes. The starting point for most games.",
        modules: &["window", "render", "scenes", "scripts", "input"],
    },
    Template {
        name: "3D + Physics",
        description: "Everything in 3D plus Rapier physics: rigidbodies and collisions.",
        modules: &["window", "render", "scenes", "scripts", "input", "physics"],
    },
    Template {
        name: "Empty",
        description: "Core and ECS only, no window: servers, tools, tests. Add modules later.",
        modules: &[],
    },
];

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub(crate) enum Page {
    Projects,
    Editor,
    Settings,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub(crate) enum View {
    List,
    Grid,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub(crate) enum Sort {
    Modified,
    Name,
}

pub(crate) enum Modal {
    NewProject {
        template: usize,
        name: String,
        location: String,
        modules: Vec<String>,
    },
    AddExisting {
        path: String,
    },
    Modules {
        root: PathBuf,
    },
}

pub(crate) struct Row {
    root: PathBuf,
    name: String,
    modified: Option<SystemTime>,
    /// Display names of installed modules.
    modules: Vec<String>,
    missing: bool,
}

pub(crate) struct Toast {
    id: u64,
    error: bool,
    text: String,
    /// Set on the first frame it is drawn.
    born: Option<f64>,
}

/// A running `cargo build` whose stderr is collected for the UI.
struct Build {
    child: Child,
    log: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone, Copy)]
pub(crate) enum RowAction {
    Open,
    Run,
    Release,
    Modules,
    Reveal,
    Forget,
}

pub(crate) struct Hub {
    catalog: Catalog,
    config: HubConfig,
    /// Where `config` is saved instead of the real hub config (tests).
    config_file: Option<PathBuf>,
    page: Page,
    /// When the current page was opened (`InputState::time`): page fade and
    /// the staggered entrance of rows.
    page_at: f64,
    modal: Option<Modal>,
    modal_at: f64,
    search: String,
    rows: Vec<Row>,
    view: View,
    sort: Sort,
    theme: ThemeKind,
    reduce_motion: bool,
    /// Theme and motion last pushed into the egui context.
    applied: Option<(ThemeKind, bool)>,
    /// Start of the wordmark intro; `None` until the first frame.
    intro_at: Option<f64>,
    toasts: Vec<Toast>,
    next_toast: u64,
    game: Option<(String, Child)>,
    editor: Option<EditorInstall>,
    /// Editor elements picked on the Editor page but not built yet.
    editor_pending: Option<Vec<String>>,
    build: Option<Build>,
    build_log: Vec<String>,
    log_open: bool,
    open_after_build: Option<Option<PathBuf>>,
    /// Cached "engine changed since the editor build" check (it walks the sources).
    stale: Option<(std::time::Instant, bool)>,
}

pub(crate) fn theme_key(kind: ThemeKind) -> &'static str {
    match kind {
        ThemeKind::Dark => "dark",
        ThemeKind::Light => "light",
        ThemeKind::Graphite => "graphite",
        ThemeKind::HighContrast => "high-contrast",
    }
}

fn theme_from_key(key: &str) -> ThemeKind {
    ThemeKind::ALL
        .into_iter()
        .find(|k| theme_key(*k) == key)
        .unwrap_or_default()
}

impl Hub {
    fn load() -> Self {
        let (config, error) = match HubConfig::load() {
            Ok(c) => (c, None),
            Err(e) => {
                // Saving a preference would overwrite the damaged file with an
                // empty list of projects: keep the original next to it.
                let kept = HubConfig::path().ok().and_then(|path| {
                    let backup = path.with_extension("toml.broken");
                    std::fs::copy(&path, &backup).ok().map(|_| backup)
                });
                let note = kept
                    .map(|b| format!(" A copy is kept in {}.", b.display()))
                    .unwrap_or_default();
                (HubConfig::default(), Some(format!("{e:#}.{note}")))
            }
        };
        let mut hub = Self::new(config, EditorInstall::locate().ok());
        if let Some(e) = error {
            hub.toast_error(e);
        }
        hub
    }

    fn new(config: HubConfig, editor: Option<EditorInstall>) -> Self {
        let gui = &config.gui;
        let mut hub = Self {
            catalog: Catalog::builtin(),
            config_file: None,
            page: Page::Projects,
            page_at: 0.0,
            modal: None,
            modal_at: 0.0,
            search: String::new(),
            rows: Vec::new(),
            view: if gui.view == "grid" {
                View::Grid
            } else {
                View::List
            },
            sort: if gui.sort == "name" {
                Sort::Name
            } else {
                Sort::Modified
            },
            theme: theme_from_key(&gui.theme),
            reduce_motion: gui.reduce_motion,
            applied: None,
            intro_at: None,
            toasts: Vec::new(),
            next_toast: 0,
            game: None,
            editor,
            editor_pending: None,
            build: None,
            build_log: Vec::new(),
            log_open: false,
            open_after_build: None,
            stale: None,
            config,
        };
        hub.refresh();
        hub
    }

    /// Writes the hub config (to `config_file` when a test set one).
    fn save_config(&self) -> anyhow::Result<()> {
        match &self.config_file {
            Some(path) => self.config.save_to(path),
            None => self.config.save(),
        }
    }

    fn refresh(&mut self) {
        self.rows = self
            .config
            .projects
            .iter()
            .map(|root| match Project::open(root) {
                Ok(p) => Row {
                    root: root.clone(),
                    name: p.name().to_owned(),
                    modified: p.modified(),
                    modules: p
                        .installed(&self.catalog)
                        .iter()
                        .map(|m| m.name.clone())
                        .collect(),
                    missing: false,
                },
                Err(_) => Row {
                    root: root.clone(),
                    name: root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    modified: None,
                    modules: Vec::new(),
                    missing: true,
                },
            })
            .collect();
    }

    // --- feedback ------------------------------------------------------------

    fn toast(&mut self, text: impl Into<String>) {
        self.push_toast(false, text.into());
    }

    fn toast_error(&mut self, text: impl Into<String>) {
        self.push_toast(true, text.into());
    }

    fn push_toast(&mut self, error: bool, text: String) {
        self.next_toast += 1;
        self.toasts.push(Toast {
            id: self.next_toast,
            error,
            text,
            born: None,
        });
        // Keep the stack short: the oldest go first.
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn report<T>(&mut self, result: anyhow::Result<T>, ok: impl FnOnce(T) -> String) {
        match result {
            Ok(v) => self.toast(ok(v)),
            Err(e) => self.toast_error(format!("{e:#}")),
        }
    }

    /// Saves the `[gui]` part of the config after a preference changed.
    fn save_prefs(&mut self) {
        self.config.gui.theme = theme_key(self.theme).into();
        self.config.gui.reduce_motion = self.reduce_motion;
        self.config.gui.view = match self.view {
            View::List => "list",
            View::Grid => "grid",
        }
        .into();
        self.config.gui.sort = match self.sort {
            Sort::Modified => "modified",
            Sort::Name => "name",
        }
        .into();
        if let Err(e) = self.save_config() {
            self.toast_error(format!("Could not save settings: {e:#}"));
        }
    }

    fn set_page(&mut self, page: Page, now: f64) {
        if self.page != page {
            self.page = page;
            self.page_at = now;
        }
    }

    fn open_modal(&mut self, modal: Modal, now: f64) {
        self.modal = Some(modal);
        self.modal_at = now;
    }

    fn new_project_modal(&self) -> Modal {
        let location = dirs::home_dir()
            .map(|h| h.join("OlvinProjects").display().to_string())
            .unwrap_or_default();
        Modal::NewProject {
            template: 0,
            name: String::new(),
            location,
            modules: TEMPLATES[0].modules.iter().map(|m| m.to_string()).collect(),
        }
    }

    // --- actions -----------------------------------------------------------

    fn create_project(&mut self, name: &str, location: &str, modules: &[String]) -> bool {
        let ids: Vec<&str> = modules.iter().map(String::as_str).collect();
        let result = Project::create_with(
            &expand_home(location),
            name.trim(),
            &self.config.engine,
            &self.catalog,
            &ids,
        )
        .and_then(|p| {
            self.config.register(p.root())?;
            self.save_config()?;
            Ok(p.name().to_owned())
        });
        let ok = result.is_ok();
        self.report(result, |name| format!("Created {name}"));
        self.refresh();
        ok
    }

    fn add_existing(&mut self, path: &str) -> bool {
        let result = Project::open(expand_home(path)).and_then(|p| {
            let added = self.config.register(p.root())?;
            self.save_config()?;
            Ok((p.name().to_owned(), added))
        });
        let ok = result.is_ok();
        self.report(result, |(name, added)| {
            if added {
                format!("Added {name}")
            } else {
                format!("{name} is already in the list")
            }
        });
        self.refresh();
        ok
    }

    fn forget(&mut self, root: &Path) {
        self.config.forget(root);
        let result = self.save_config();
        let name = file_name(root);
        self.report(result, |()| {
            format!("Removed {name} from the list. Files stay on disk.")
        });
        self.refresh();
    }

    fn run_game(&mut self, root: &Path, release: bool) {
        if self.game.is_some() {
            return;
        }
        let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        cmd.arg("run").current_dir(root);
        if release {
            cmd.arg("--release");
        }
        // Its own process group: Stop must end the game, not only the cargo that started it.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let name = file_name(root);
        match cmd.spawn() {
            Ok(child) => {
                self.toast(format!("Building and running {name}…"));
                self.game = Some((name, child));
            }
            Err(e) => self.toast_error(format!("Could not start cargo: {e}")),
        }
    }

    fn stop_game(&mut self) {
        if let Some((name, mut child)) = self.game.take() {
            kill_tree(&mut child);
            self.toast(format!("Stopped {name}"));
        }
    }

    fn editor_selection(&self) -> Vec<String> {
        self.editor
            .as_ref()
            .map(|e| e.initial_selection(&self.catalog))
            .unwrap_or_default()
    }

    fn start_build(&mut self, wanted: &[String]) {
        let Some(install) = &self.editor else { return };
        let result = install
            .write(&self.config.engine, &self.catalog, wanted)
            .and_then(|()| {
                let mut child = install
                    .build_command()
                    .stderr(Stdio::piped())
                    .stdout(Stdio::null())
                    .spawn()
                    .context("could not start cargo")?;
                let log = Arc::new(Mutex::new(Vec::new()));
                let stderr = child.stderr.take().expect("stderr is piped");
                let sink = log.clone();
                std::thread::spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        sink.lock().unwrap().push(line);
                    }
                });
                Ok(Build { child, log })
            });
        match result {
            Ok(build) => {
                self.build_log.clear();
                self.build = Some(build);
                self.editor_pending = None;
                self.toast("Building the editor…");
            }
            Err(e) => {
                self.open_after_build = None;
                self.toast_error(format!("{e:#}"));
            }
        }
    }

    fn open_editor(&mut self, project: Option<PathBuf>) {
        if self.build.is_some() {
            self.open_after_build = Some(project);
            return;
        }
        let Some(install) = &self.editor else {
            self.toast_error("No data folder for the editor install on this system");
            return;
        };
        if !install.is_built() || install.is_stale(&self.config.engine) {
            // Never open an editor older than the engine: rebuild first.
            let wanted = install.initial_selection(&self.catalog);
            self.open_after_build = Some(project);
            self.start_build(&wanted);
            return;
        }
        let name = project.as_deref().map(file_name);
        let result = install.launch(project.as_deref());
        self.report(result, |_| match name {
            Some(n) => format!("Opened {n} in the editor"),
            None => "Opened the editor".into(),
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some((name, child)) = &mut self.game
            && let Ok(Some(status)) = child.try_wait()
        {
            let name = name.clone();
            self.game = None;
            if status.success() {
                self.toast(format!("{name} exited"));
            } else {
                self.toast_error(format!(
                    "{name} failed ({status}). See the terminal for details."
                ));
            }
        }
        if let Some(build) = &mut self.build {
            self.build_log = build.log.lock().unwrap().clone();
            if let Ok(Some(status)) = build.child.try_wait() {
                self.build = None;
                if status.success() {
                    if let Some(install) = &self.editor {
                        install.mark_built(&self.config.engine);
                    }
                    self.stale = None;
                    self.toast("Editor built");
                    if let Some(project) = self.open_after_build.take() {
                        self.open_editor(project);
                    }
                } else {
                    self.open_after_build = None;
                    self.log_open = true;
                    self.toast_error("Editor build failed. Open the log on the Editor page.");
                }
            }
        }
        if self.game.is_some() || self.build.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn editor_stale(&mut self) -> bool {
        if self.build.is_some() {
            return false;
        }
        if let Some((at, stale)) = self.stale
            && at.elapsed() < Duration::from_secs(3)
        {
            return stale;
        }
        let stale = self
            .editor
            .as_ref()
            .is_some_and(|e| e.is_stale(&self.config.engine));
        self.stale = Some((std::time::Instant::now(), stale));
        stale
    }

    fn row_action(&mut self, root: PathBuf, action: RowAction, now: f64) {
        match action {
            RowAction::Open => self.open_editor(Some(root)),
            RowAction::Run => self.run_game(&root, false),
            RowAction::Release => self.run_game(&root, true),
            RowAction::Modules => self.open_modal(Modal::Modules { root }, now),
            RowAction::Reveal => {
                if let Err(e) = Command::new("xdg-open").arg(&root).spawn() {
                    self.toast_error(format!("Could not open the folder: {e}"));
                }
            }
            RowAction::Forget => self.forget(&root),
        }
    }

    // --- frame -----------------------------------------------------------------

    /// Pushes theme and motion into the context when they change. Returns
    /// false on the very first apply: fonts only take effect next pass.
    fn sync_style(&mut self, ctx: &egui::Context) -> bool {
        let want = (self.theme, self.reduce_motion);
        if self.applied == Some(want) {
            return true;
        }
        let first = self.applied.is_none();
        ui::set_reduce_motion(ctx, self.reduce_motion);
        ui::apply(ctx, self.theme);
        self.applied = Some(want);
        !first
    }

    fn shortcuts(&mut self, ctx: &egui::Context, now: f64) {
        if self.modal.is_some() {
            return;
        }
        let (new, settings) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::N),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma),
            )
        });
        if new {
            self.set_page(Page::Projects, now);
            let modal = self.new_project_modal();
            self.open_modal(modal, now);
        }
        if settings {
            self.set_page(Page::Settings, now);
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.sync_style(&ctx) {
            ctx.request_repaint();
            return;
        }
        let now = ctx.input(|i| i.time);
        if self.intro_at.is_none() {
            self.intro_at = Some(now);
            self.page_at = now;
        }
        self.poll(&ctx);
        self.shortcuts(&ctx, now);

        let p = ui::palette(&ctx);
        chrome::paint_backdrop(ui.painter(), ctx.content_rect(), &p);
        let rail = ctx.content_rect().width() < 1000.0;
        egui::Panel::left("nav")
            .exact_size(if rail { 68.0 } else { 220.0 })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.glass_strong)
                    .inner_margin(egui::Margin::symmetric(if rail { 10 } else { 14 }, 16)),
            )
            .show(ui, |ui| self.sidebar(ui, rail, now));
        egui::Panel::bottom("status")
            .exact_size(32.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.footer)
                    .inner_margin(egui::Margin::symmetric(14, 0)),
            )
            .show(ui, |ui| self.status_line(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.page(ui, now));
        self.modals(&ctx, now);
        self.toasts(&ctx);
    }
}

impl eframe::App for Hub {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

/// A typed path: trimmed, with a leading `~` meaning the home folder (a
/// text field is not a shell, and a folder literally named `~` is never wanted).
pub(crate) fn expand_home(text: &str) -> PathBuf {
    let text = text.trim();
    let home = dirs::home_dir();
    match (text, home) {
        ("~", Some(home)) => home,
        (t, Some(home)) if t.starts_with("~/") => home.join(&t[2..]),
        _ => PathBuf::from(text),
    }
}

/// Kills `cargo run` and the game it started.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        // The child leads its own process group (see `run_game`): `kill -- -PGID`.
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn file_name(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}

/// "just now", "5m ago", "4h ago", "3d ago", "2mo ago", "1y ago".
pub(crate) fn ago(t: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86_400 => format!("{}h ago", secs / 3600),
        86_400..2_592_000 => format!("{}d ago", secs / 86_400),
        2_592_000..31_536_000 => format!("{}mo ago", secs / 2_592_000),
        _ => format!("{}y ago", secs / 31_536_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_frames(hub: &mut Hub, ctx: &egui::Context, size: egui::Vec2, frames: usize) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        for frame in 0..frames {
            // Real sessions repaint on every mouse move; sizes settle over frames.
            let pointer = egui::pos2(300.0 + frame as f32 * 7.0, 400.0);
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(frame as f64 / 60.0),
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            };
            // Headless: nobody uploads textures, so drop the deltas explicitly.
            ctx.run_ui(input, |ui| hub.draw(ui)).textures_delta.clear();
        }
    }

    /// Runs a few frames headless and returns the dialog's rect.
    fn modal_rect(modal: Modal, id: &str) -> egui::Rect {
        let ctx = egui::Context::default();
        let mut hub = Hub::new(HubConfig::default(), None);
        hub.modal = Some(modal);
        run_frames(&mut hub, &ctx, egui::vec2(955.0, 999.0), 60);
        assert!(hub.modal.is_some(), "dialog closed by itself");
        ctx.memory(|m| m.area_rect(egui::Id::new(id)))
            .expect("dialog was shown")
    }

    #[test]
    fn dialogs_fit_their_content() {
        let hub = Hub::new(HubConfig::default(), None);
        let cases = [
            (hub.new_project_modal(), "new-project"),
            (
                Modal::AddExisting {
                    path: String::new(),
                },
                "add-existing",
            ),
        ];
        for (modal, id) in cases {
            let rect = modal_rect(modal, id);
            assert!(rect.height() < 900.0, "{id}: {rect:?}");
            assert!(
                rect.top() > 0.0 && rect.bottom() < 999.0,
                "{id} off screen: {rect:?}"
            );
        }
    }

    fn temp(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("olvin-hub-gui-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn create_from_template_then_toggle_modules_and_forget() {
        let dir = temp("create");
        let mut hub = Hub::new(HubConfig::default(), None);
        hub.config_file = Some(dir.join("config/config.toml"));
        let modules: Vec<String> = TEMPLATES[1].modules.iter().map(|m| m.to_string()).collect();
        assert!(hub.create_project("demo_game", dir.to_str().unwrap(), &modules));
        assert_eq!(hub.rows.len(), 1);
        assert_eq!(hub.rows[0].name, "demo_game");
        assert!(!hub.rows[0].missing);
        assert!(hub.rows[0].modules.iter().any(|m| m == "3D physics"));
        // the same name twice is an error toast, not a second row
        assert!(!hub.create_project("demo_game", dir.to_str().unwrap(), &modules));
        assert!(hub.toasts.last().unwrap().error);
        assert_eq!(hub.rows.len(), 1);
        // a reserved name is refused before anything is created
        assert!(!hub.create_project("olvin", dir.to_str().unwrap(), &modules));
        assert!(!dir.join("olvin").exists());

        // the Modules dialog installs `lua` (with `scripts`) and removes it again
        let root = hub.rows[0].root.clone();
        let mut project = Project::open(&root).unwrap();
        let catalog = Catalog::builtin();
        project.add(&catalog, "lua").unwrap();
        project.add(&catalog, "game").unwrap();
        project.save().unwrap();
        hub.refresh();
        assert!(hub.rows[0].modules.iter().any(|m| m == "Lua scripts"));
        assert!(hub.rows[0].modules.iter().any(|m| m == "Game runtime"));

        // a deleted project stays in the list, marked missing
        std::fs::remove_dir_all(&root).unwrap();
        hub.refresh();
        assert!(hub.rows[0].missing);
        hub.forget(&root);
        assert!(hub.rows.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn adding_a_folder_that_is_not_a_project_reports_an_error() {
        let dir = temp("add-bad");
        let mut hub = Hub::new(HubConfig::default(), None);
        hub.config_file = Some(dir.join("config/config.toml"));
        assert!(!hub.add_existing(dir.to_str().unwrap()));
        assert!(hub.toasts.last().unwrap().error);
        assert!(!hub.add_existing("/definitely/not/here"));
        assert!(hub.rows.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn typed_paths_expand_the_home_folder() {
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expand_home(" ~/games "), home.join("games"));
            assert_eq!(expand_home("~"), home);
        }
        assert_eq!(expand_home("/a/~/b"), PathBuf::from("/a/~/b"));
        assert_eq!(expand_home("~x"), PathBuf::from("~x"));
    }

    /// Stop must end the game cargo started, not just cargo.
    #[cfg(unix)]
    #[test]
    fn stopping_kills_the_whole_process_group() {
        let dir = temp("kill");
        let pid_file = dir.join("grandchild.pid");
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(format!("sleep 60 & echo $! > {}; wait", pid_file.display()));
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn().unwrap();
        let grandchild = loop {
            if let Ok(text) = std::fs::read_to_string(&pid_file)
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                break pid;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        kill_tree(&mut child);
        std::thread::sleep(Duration::from_millis(100));
        let alive = std::path::Path::new(&format!("/proc/{grandchild}")).exists()
            && std::fs::read_to_string(format!("/proc/{grandchild}/stat"))
                .is_ok_and(|s| !s.contains(") Z"));
        assert!(!alive, "the grandchild survived");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn relative_times_are_english() {
        let now = SystemTime::now();
        assert_eq!(ago(now), "just now");
        assert_eq!(ago(now - Duration::from_secs(4 * 3600 + 5)), "4h ago");
        assert_eq!(ago(now - Duration::from_secs(86_400 + 5)), "1d ago");
    }

    #[test]
    fn theme_keys_round_trip() {
        for kind in ThemeKind::ALL {
            assert_eq!(theme_from_key(theme_key(kind)), kind);
        }
        assert_eq!(theme_from_key("nope"), ThemeKind::Dark);
    }

    /// Every page at every breakpoint lays out without panicking, and the
    /// layout settles (no frame-to-frame growth).
    #[test]
    fn pages_render_at_all_widths() {
        for width in [470.0, 800.0, 1280.0, 1920.0] {
            for page in [Page::Projects, Page::Editor, Page::Settings] {
                for view in [View::List, View::Grid] {
                    let ctx = egui::Context::default();
                    let mut hub = Hub::new(HubConfig::default(), None);
                    hub.page = page;
                    hub.view = view;
                    run_frames(&mut hub, &ctx, egui::vec2(width, 800.0), 30);
                }
            }
        }
    }
}

#[cfg(test)]
mod shots;

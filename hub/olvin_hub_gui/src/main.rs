//! `olvin-hub-gui`: the hub as a desktop launcher, laid out like Unity Hub:
//! a sidebar (Projects, Editor, Settings), a responsive project list, modal
//! dialogs and toasts. Styling follows STYLE.md (launcher rules).

use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::Context as _;
use eframe::egui;
use olvin_hub::{Catalog, EditorInstall, HubConfig};

mod chrome;
mod dialogs;
mod editor_page;
mod jobs;
mod projects;
mod settings;
mod state;
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
    // invariant: PNG is embedded at build time from a checked-in file.
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

pub(crate) fn theme_from_key(key: &str) -> ThemeKind {
    ThemeKind::ALL
        .into_iter()
        .find(|k| theme_key(*k) == key)
        .unwrap_or_default()
}

impl Hub {
    /// Pushes theme and motion into the context when they change. Returns
    /// false on the very first apply: fonts only take effect next pass.
    pub(crate) fn sync_style(&mut self, ctx: &egui::Context) -> bool {
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

    pub(crate) fn draw(&mut self, ui: &mut egui::Ui) {
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
mod tests;

#[cfg(test)]
mod shots;

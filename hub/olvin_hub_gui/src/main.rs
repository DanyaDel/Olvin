//! `olvin-hub-gui`: the hub as a desktop app, laid out like Unity Hub:
//! a sidebar (Projects, Editor), a project table, modal dialogs.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, UiBuilder, vec2};
use olvin_hub::{Catalog, EditorInstall, HubConfig, Project};

const ACCENT: Color32 = Color32::from_rgb(56, 132, 255);
const ORANGE: Color32 = Color32::from_rgb(255, 160, 51);
const ROW_HEIGHT: f32 = 56.0;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Olvin Hub")
            .with_app_id("olvin-hub")
            .with_inner_size([1100.0, 680.0])
            .with_min_inner_size([760.0, 460.0]),
        ..Default::default()
    };
    eframe::run_native(
        "olvin-hub",
        options,
        Box::new(|cc| {
            style(&cc.egui_ctx);
            Ok(Box::new(Hub::load()))
        }),
    )
}

fn style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    let gray = Color32::from_gray;
    v.panel_fill = gray(32);
    v.window_fill = gray(40);
    v.extreme_bg_color = gray(24);
    v.faint_bg_color = gray(38);
    v.selection.bg_fill = ACCENT;
    v.window_corner_radius = CornerRadius::same(8);
    for (w, fill) in [(&mut v.widgets.inactive, 52), (&mut v.widgets.hovered, 64), (&mut v.widgets.active, 76)] {
        w.bg_fill = gray(fill);
        w.weak_bg_fill = gray(fill);
        w.corner_radius = CornerRadius::same(4);
    }
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.button_padding = vec2(10.0, 5.0);
        s.spacing.item_spacing = vec2(8.0, 6.0);
    });
}

struct Template {
    name: &'static str,
    description: &'static str,
    modules: &'static [&'static str],
}

const TEMPLATES: [Template; 3] = [
    Template {
        name: "3D",
        description: "Окно, 3D-рендер и загрузка сцен из редактора. Стартовая точка для большинства игр.",
        modules: &["window", "render", "scenes"],
    },
    Template {
        name: "3D + физика",
        description: "То же, что 3D, плюс физика Rapier: твёрдые тела и коллизии.",
        modules: &["window", "render", "scenes", "physics"],
    },
    Template {
        name: "Пустой",
        description: "Только ядро и ECS, без окна: сервер, инструмент, тесты. Модули можно добавить позже.",
        modules: &[],
    },
];

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Projects,
    Editor,
}

enum Modal {
    NewProject { template: usize, name: String, location: String },
    AddExisting { path: String },
    Modules { root: PathBuf },
    EditorElements { wanted: Vec<String> },
}

enum Status {
    Info(String),
    Error(String),
}

struct Row {
    root: PathBuf,
    name: String,
    modified: Option<SystemTime>,
    modules: Vec<String>,
    missing: bool,
}

/// A running `cargo build` whose stderr is collected for the UI.
struct Build {
    child: Child,
    log: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone, Copy)]
enum RowAction {
    Open,
    Run,
    Release,
    Modules,
    Reveal,
    Forget,
}

struct Hub {
    catalog: Catalog,
    config: HubConfig,
    page: Page,
    modal: Option<Modal>,
    search: String,
    rows: Vec<Row>,
    status: Option<Status>,
    game: Option<(String, Child)>,
    editor: Option<EditorInstall>,
    build: Option<Build>,
    build_log: Vec<String>,
    open_after_build: Option<Option<PathBuf>>,
}

impl Hub {
    fn load() -> Self {
        let (config, status) = match HubConfig::load() {
            Ok(c) => (c, None),
            Err(e) => (HubConfig::default(), Some(Status::Error(format!("{e:#}")))),
        };
        let mut hub = Self::new(config, EditorInstall::locate().ok());
        hub.status = status;
        hub
    }

    fn new(config: HubConfig, editor: Option<EditorInstall>) -> Self {
        let mut hub = Self {
            catalog: Catalog::builtin(),
            config,
            page: Page::Projects,
            modal: None,
            search: String::new(),
            rows: Vec::new(),
            status: None,
            game: None,
            editor,
            build: None,
            build_log: Vec::new(),
            open_after_build: None,
        };
        hub.refresh();
        hub
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
                    modules: p.installed(&self.catalog).iter().map(|m| m.name.clone()).collect(),
                    missing: false,
                },
                Err(_) => Row {
                    root: root.clone(),
                    name: root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    modified: None,
                    modules: Vec::new(),
                    missing: true,
                },
            })
            .collect();
        self.rows.sort_by(|a, b| b.modified.cmp(&a.modified));
    }

    fn report<T>(&mut self, result: anyhow::Result<T>, ok: impl FnOnce(T) -> String) {
        self.status = Some(match result {
            Ok(v) => Status::Info(ok(v)),
            Err(e) => Status::Error(format!("{e:#}")),
        });
    }

    // --- actions -----------------------------------------------------------

    fn create_project(&mut self, template: usize, name: &str, location: &str) -> bool {
        let t = &TEMPLATES[template];
        let result = Project::create_with(
            &PathBuf::from(location.trim()),
            name.trim(),
            &self.config.engine,
            &self.catalog,
            t.modules,
        )
        .and_then(|p| {
            self.config.register(p.root())?;
            self.config.save()?;
            Ok(p.root().to_path_buf())
        });
        let ok = result.is_ok();
        self.report(result, |root| format!("создан {}", root.display()));
        self.refresh();
        ok
    }

    fn add_existing(&mut self, path: &str) -> bool {
        let result = Project::open(path.trim()).and_then(|p| {
            let added = self.config.register(p.root())?;
            self.config.save()?;
            Ok((p.name().to_owned(), added))
        });
        let ok = result.is_ok();
        self.report(result, |(name, added)| if added { format!("добавлен {name}") } else { format!("{name} уже в списке") });
        self.refresh();
        ok
    }

    fn forget(&mut self, root: &Path) {
        self.config.forget(root);
        let result = self.config.save();
        self.report(result, |()| format!("{} убран из списка (файлы на месте)", root.display()));
        self.refresh();
    }

    fn run_game(&mut self, root: &Path, release: bool) {
        if self.game.is_some() {
            return;
        }
        let mut cmd = Command::new("cargo");
        cmd.arg("run").current_dir(root);
        if release {
            cmd.arg("--release");
        }
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match cmd.spawn() {
            Ok(child) => {
                self.status = Some(Status::Info(format!("{name}: сборка и запуск…")));
                self.game = Some((name, child));
            }
            Err(e) => self.status = Some(Status::Error(format!("cargo: {e}"))),
        }
    }

    fn editor_up_to_date(&self, wanted: &[String]) -> bool {
        self.editor.as_ref().is_some_and(|e| e.is_built() && e.installed(&self.catalog) == wanted)
    }

    fn start_build(&mut self, wanted: &[String]) {
        let Some(install) = &self.editor else { return };
        let result = install.write(&self.config.engine, &self.catalog, wanted).and_then(|()| {
            let mut child = install.build_command().stderr(Stdio::piped()).stdout(Stdio::null()).spawn()?;
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
                self.status = Some(Status::Info("сборка редактора…".into()));
            }
            Err(e) => {
                self.open_after_build = None;
                self.status = Some(Status::Error(format!("{e:#}")));
            }
        }
    }

    fn open_editor(&mut self, project: Option<PathBuf>) {
        if self.build.is_some() {
            self.open_after_build = Some(project);
            return;
        }
        let Some(install) = &self.editor else { return };
        if !install.is_built() {
            let wanted = install.initial_selection(&self.catalog);
            self.open_after_build = Some(project);
            self.start_build(&wanted);
            return;
        }
        let name = project.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        let result = install.launch(project.as_deref());
        self.report(result, |_| match name {
            Some(n) => format!("{n} открыт в редакторе"),
            None => "редактор открыт".into(),
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some((name, child)) = &mut self.game
            && let Ok(Some(status)) = child.try_wait()
        {
            self.status = Some(if status.success() {
                Status::Info(format!("{name} завершилась"))
            } else {
                Status::Error(format!("{name}: ошибка ({status}), подробности в терминале"))
            });
            self.game = None;
        }
        if let Some(build) = &mut self.build {
            self.build_log = build.log.lock().unwrap().clone();
            if let Ok(Some(status)) = build.child.try_wait() {
                self.build = None;
                if status.success() {
                    self.status = Some(Status::Info("редактор собран".into()));
                    if let Some(project) = self.open_after_build.take() {
                        self.open_editor(project);
                    }
                } else {
                    self.open_after_build = None;
                    self.status = Some(Status::Error("сборка редактора не удалась, см. журнал на странице «Редактор»".into()));
                }
            }
        }
        if self.game.is_some() || self.build.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    // --- pages ---------------------------------------------------------------

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Olvin").size(20.0).strong().color(ORANGE));
            ui.label(RichText::new("Hub").size(20.0));
        });
        ui.add_space(16.0);
        for (page, label) in [(Page::Projects, "📁  Проекты"), (Page::Editor, "✏  Редактор")] {
            let selected = self.page == page;
            let button = egui::Button::selectable(selected, RichText::new(label).size(15.0))
                .min_size(vec2(ui.available_width(), 34.0));
            if ui.add(button).clicked() {
                self.page = page;
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(6.0);
            ui.small(format!("движок: {}", self.config.engine)).on_hover_text("olvin-hub engine --path/--git");
            if self.build.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.small("сборка редактора");
                });
            }
        });
    }

    fn projects_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Проекты").size(24.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(accent_button("Новый проект")).clicked() {
                    let location =
                        dirs::home_dir().map(|h| h.join("OlvinProjects").display().to_string()).unwrap_or_default();
                    self.modal = Some(Modal::NewProject { template: 0, name: String::new(), location });
                }
                if ui.button("Добавить").on_hover_text("Добавить существующий проект с диска").clicked() {
                    self.modal = Some(Modal::AddExisting { path: String::new() });
                }
            });
        });
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("🔍 Поиск").desired_width(280.0));
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            let w = ui.available_width();
            ui.add_sized([w * 0.45, 18.0], egui::Label::new(RichText::new("НАЗВАНИЕ").small().weak()));
            ui.add_sized([w * 0.17, 18.0], egui::Label::new(RichText::new("ИЗМЕНЁН").small().weak()));
            ui.label(RichText::new("МОДУЛИ").small().weak());
        });
        ui.separator();

        let filter = self.search.trim().to_lowercase();
        let mut action: Option<(usize, RowAction)> = None;
        let running = self.game.is_some();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if self.rows.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Проектов пока нет").size(16.0));
                    ui.weak("Создай новый или добавь существующий с диска");
                });
            }
            for (i, row) in self.rows.iter().enumerate() {
                if !filter.is_empty() && !row.name.to_lowercase().contains(&filter) {
                    continue;
                }
                if let Some(a) = project_row(ui, row, running) {
                    action = Some((i, a));
                }
            }
        });

        if let Some((i, a)) = action {
            let root = self.rows[i].root.clone();
            match a {
                RowAction::Open => self.open_editor(Some(root)),
                RowAction::Run => self.run_game(&root, false),
                RowAction::Release => self.run_game(&root, true),
                RowAction::Modules => self.modal = Some(Modal::Modules { root }),
                RowAction::Reveal => {
                    let _ = Command::new("xdg-open").arg(&root).spawn();
                }
                RowAction::Forget => self.forget(&root),
            }
        }
    }

    fn editor_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.heading(RichText::new("Редактор").size(24.0));
        ui.weak("Редактор ставится отдельно; его элементы собираются вместе с ним.");
        ui.add_space(12.0);

        let Some(install) = &self.editor else {
            ui.colored_label(ui.visuals().error_fg_color, "Нет папки данных для установки редактора");
            return;
        };
        let built = install.is_built();
        let installed = install.installed(&self.catalog);
        let root = install.root().display().to_string();
        let building = self.build.is_some();

        let (mut open, mut elements, mut rebuild) = (false, false, false);
        egui::Frame::group(ui.style()).fill(ui.visuals().faint_bg_color).inner_margin(14).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Olvin Editor").size(18.0).strong());
                ui.label(RichText::new(env!("CARGO_PKG_VERSION")).weak());
                let (text, color) = match (building, built) {
                    (true, _) => ("сборка…", ORANGE),
                    (false, true) => ("установлен", Color32::from_rgb(110, 200, 120)),
                    (false, false) => ("не установлен", Color32::from_gray(150)),
                };
                chip(ui, text, color);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if built {
                        open = ui.add_enabled(!building, accent_button("Открыть")).clicked();
                    } else {
                        rebuild = ui.add_enabled(!building, accent_button("Установить")).clicked();
                    }
                    elements = ui.add_enabled(!building, egui::Button::new("Элементы…")).clicked();
                    if built && ui.add_enabled(!building, egui::Button::new("Пересобрать")).clicked() {
                        rebuild = true;
                    }
                });
            });
            ui.weak(&root);
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                chip(ui, "Сцена · Иерархия · Инспектор · Ассеты", Color32::from_gray(160));
                for m in self.catalog.editor_modules.iter().filter(|m| installed.contains(&m.id)) {
                    chip(ui, &m.name, ACCENT);
                }
            });
        });

        if building || !self.build_log.is_empty() {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if building {
                    ui.spinner();
                }
                let last = self.build_log.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
                ui.monospace(last.trim());
            });
            egui::CollapsingHeader::new("Журнал сборки").show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(260.0).stick_to_bottom(true).show(ui, |ui| {
                    for line in &self.build_log {
                        ui.monospace(line);
                    }
                });
            });
        }

        let selection = || self.editor.as_ref().map(|e| e.initial_selection(&self.catalog)).unwrap_or_default();
        if elements {
            self.modal = Some(Modal::EditorElements { wanted: selection() });
        }
        if rebuild {
            let wanted = selection();
            self.start_build(&wanted);
        }
        if open {
            self.open_editor(None);
        }
    }

    // --- modals --------------------------------------------------------------

    fn modals(&mut self, ctx: &egui::Context) {
        let Some(mut modal) = self.modal.take() else { return };
        let mut keep = true;
        match &mut modal {
            Modal::NewProject { template, name, location } => {
                let mut create = false;
                let response = egui::Modal::new(egui::Id::new("new-project")).show(ctx, |ui| {
                    ui.set_width(640.0);
                    ui.heading("Новый проект");
                    ui.add_space(8.0);
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(190.0);
                            ui.label(RichText::new("ШАБЛОНЫ").small().weak());
                            for (i, t) in TEMPLATES.iter().enumerate() {
                                let b = egui::Button::selectable(*template == i, RichText::new(t.name).size(15.0))
                                    .min_size(vec2(190.0, 40.0));
                                if ui.add(b).clicked() {
                                    *template = i;
                                }
                            }
                        });
                        // Not `ui.separator()`: in a horizontal layout it takes the
                        // available height, which in a modal comes from the previous
                        // frame's size, and the dialog grows every frame.
                        let (line, _) = ui.allocate_exact_size(vec2(9.0, 3.0 * 46.0), Sense::hover());
                        ui.painter().vline(line.center().x, line.y_range(), ui.visuals().widgets.noninteractive.bg_stroke);
                        ui.vertical(|ui| {
                            let t = &TEMPLATES[*template];
                            ui.label(RichText::new(t.name).size(18.0).strong());
                            ui.label(t.description);
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                for id in t.modules {
                                    if let Some(m) = self.catalog.get(id) {
                                        chip(ui, &m.name, ACCENT);
                                    }
                                }
                                if t.modules.is_empty() {
                                    chip(ui, "ядро + ECS", Color32::from_gray(160));
                                }
                            });
                            ui.add_space(14.0);
                            ui.label("Название проекта");
                            ui.add(egui::TextEdit::singleline(name).hint_text("my_game").desired_width(f32::INFINITY));
                            ui.label("Расположение");
                            ui.add(egui::TextEdit::singleline(location).desired_width(f32::INFINITY));
                            if !name.trim().is_empty() {
                                ui.weak(format!("{}/{}", location.trim_end_matches('/'), name.trim()));
                            }
                        });
                    });
                    ui.add_space(12.0);
                    buttons_row(ui, |ui| {
                        create = ui.add_enabled(!name.trim().is_empty(), accent_button("Создать проект")).clicked();
                        if ui.button("Отмена").clicked() {
                            keep = false;
                        }
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if create && self.create_project(*template, name, location) {
                    keep = false;
                }
            }
            Modal::AddExisting { path } => {
                let mut add = false;
                let response = egui::Modal::new(egui::Id::new("add-existing")).show(ctx, |ui| {
                    ui.set_width(480.0);
                    ui.heading("Добавить проект");
                    ui.label("Папка с Cargo.toml игры на Olvin:");
                    ui.add(egui::TextEdit::singleline(path).hint_text("/home/…/my_game").desired_width(f32::INFINITY));
                    ui.add_space(8.0);
                    buttons_row(ui, |ui| {
                        add = ui.add_enabled(!path.trim().is_empty(), accent_button("Добавить")).clicked();
                        if ui.button("Отмена").clicked() {
                            keep = false;
                        }
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if add && self.add_existing(path) {
                    keep = false;
                }
            }
            Modal::Modules { root } => {
                let Ok(mut project) = Project::open(&*root) else {
                    self.status = Some(Status::Error(format!("{} не открывается", root.display())));
                    return;
                };
                let mut toggled = None;
                let response = egui::Modal::new(egui::Id::new("modules")).show(ctx, |ui| {
                    ui.set_width(660.0);
                    ui.heading(format!("Модули — {}", project.name()));
                    ui.weak("Правки сразу пишутся в Cargo.toml проекта.");
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        for category in self.catalog.categories() {
                            ui.add_space(4.0);
                            ui.label(RichText::new(category).strong());
                            for m in self.catalog.modules.iter().filter(|m| m.category == category) {
                                let mut on = project.is_installed(m);
                                ui.horizontal(|ui| {
                                    if ui.checkbox(&mut on, &m.name).changed() {
                                        toggled = Some((m.id.clone(), on));
                                    }
                                    ui.weak(&m.description);
                                });
                                if !m.requires.is_empty() {
                                    ui.indent(&m.id, |ui| ui.small(format!("требует: {}", m.requires.join(", "))));
                                }
                            }
                        }
                    });
                    ui.add_space(8.0);
                    buttons_row(ui, |ui| {
                        if ui.add(accent_button("Готово")).clicked() {
                            keep = false;
                        }
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if let Some((id, on)) = toggled {
                    let result = if on {
                        project.add(&self.catalog, &id).map(|added| format!("+ {}", added.join(", ")))
                    } else {
                        project.remove(&self.catalog, &id, false).map(|_| format!("- {id}"))
                    }
                    .and_then(|msg| project.save().map(|()| msg));
                    self.report(result, |m| m);
                    self.refresh();
                }
            }
            Modal::EditorElements { wanted } => {
                let mut apply = false;
                let response = egui::Modal::new(egui::Id::new("editor-elements")).show(ctx, |ui| {
                    ui.set_width(580.0);
                    ui.heading("Элементы редактора");
                    ui.weak("База (всегда): сцена, иерархия, инспектор, ассеты.");
                    ui.add_space(6.0);
                    for m in &self.catalog.editor_modules {
                        let mut on = wanted.contains(&m.id);
                        if ui.checkbox(&mut on, RichText::new(&m.name).strong()).changed() {
                            if on {
                                wanted.push(m.id.clone());
                            } else {
                                wanted.retain(|id| *id != m.id);
                            }
                        }
                        ui.indent(("desc", &m.id), |ui| ui.weak(&m.description));
                    }
                    *wanted = self
                        .catalog
                        .editor_modules
                        .iter()
                        .filter(|m| wanted.contains(&m.id))
                        .map(|m| m.id.clone())
                        .collect();
                    ui.add_space(10.0);
                    buttons_row(ui, |ui| {
                        let same = self.editor_up_to_date(wanted);
                        apply = ui.add_enabled(!same, accent_button(if same { "Без изменений" } else { "Установить" })).clicked();
                        if ui.button("Отмена").clicked() {
                            keep = false;
                        }
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if apply {
                    let w = wanted.clone();
                    self.start_build(&w);
                    keep = false;
                }
            }
        }
        if keep {
            self.modal = Some(modal);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match &self.status {
                Some(Status::Info(m)) => ui.label(m),
                Some(Status::Error(m)) => ui.colored_label(ui.visuals().error_fg_color, m),
                None => ui.weak("готово"),
            };
            if let Some((name, _)) = &self.game {
                let name = name.clone();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("■ Остановить").clicked()
                        && let Some((_, mut child)) = self.game.take()
                    {
                        let _ = child.kill();
                        self.status = Some(Status::Info(format!("{name} остановлена")));
                    }
                    ui.spinner();
                    ui.label(format!("{name} запущена"));
                });
            }
        });
    }
}

/// One table row; a click anywhere outside the menu opens the project.
fn project_row(ui: &mut egui::Ui, row: &Row, running: bool) -> Option<RowAction> {
    let mut action = None;
    let bg = ui.painter().add(egui::Shape::Noop);
    let response = ui
        .scope_builder(UiBuilder::new().sense(Sense::click()), |ui| {
            ui.set_min_height(ROW_HEIGHT);
            ui.horizontal(|ui| {
                let w = ui.available_width();
                let cell = |ui: &mut egui::Ui, width: f32, add: &mut dyn FnMut(&mut egui::Ui)| {
                    ui.allocate_ui_with_layout(vec2(width, ROW_HEIGHT), egui::Layout::left_to_right(egui::Align::Center), add);
                };
                cell(ui, w * 0.45, &mut |ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
                    let fill = if row.missing { Color32::from_gray(70) } else { ACCENT.gamma_multiply(0.8) };
                    ui.painter().rect_filled(rect, 6.0, fill);
                    let letter = row.name.chars().next().unwrap_or('?').to_uppercase().to_string();
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(17.0), Color32::WHITE);
                    ui.vertical(|ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new(&row.name).strong().size(15.0));
                        let path = row.root.display().to_string();
                        if row.missing {
                            ui.colored_label(ui.visuals().error_fg_color, format!("{path} — не найден"));
                        } else {
                            ui.add(egui::Label::new(RichText::new(path).small().weak()).truncate());
                        }
                    });
                });
                cell(ui, w * 0.17, &mut |ui| {
                    ui.weak(row.modified.map(ago).unwrap_or_else(|| "—".into()));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button(RichText::new("•••").strong(), |ui| {
                        let items = [
                            ("✏ Открыть в редакторе", RowAction::Open, true),
                            ("▶ Запустить", RowAction::Run, !running),
                            ("▶ Запустить (Release)", RowAction::Release, !running),
                            ("🧩 Модули…", RowAction::Modules, true),
                            ("📂 Показать в папке", RowAction::Reveal, true),
                        ];
                        for (label, a, enabled) in items {
                            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                                action = Some(a);
                            }
                        }
                        ui.separator();
                        if ui.button("Убрать из списка").clicked() {
                            action = Some(RowAction::Forget);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for m in row.modules.iter().take(4) {
                            chip(ui, m, Color32::from_gray(150));
                        }
                        if row.modules.len() > 4 {
                            ui.weak(format!("+{}", row.modules.len() - 4));
                        }
                    });
                });
            });
        })
        .response;
    if response.hovered() {
        ui.painter().set(bg, egui::Shape::rect_filled(response.rect, 6.0, Color32::from_gray(44)));
    }
    if response.clicked() && action.is_none() && !row.missing {
        action = Some(RowAction::Open);
    }
    action
}

/// Dialog buttons, right-aligned. The row is bounded to one line: a bare
/// right-to-left layout in a modal would claim the whole screen height.
fn buttons_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.7)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(8, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(color));
        });
}

fn accent_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE).strong()).fill(ACCENT)
}

fn ago(t: SystemTime) -> String {
    let secs = SystemTime::now().duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..60 => "только что".into(),
        60..3600 => format!("{} мин назад", secs / 60),
        3600..86_400 => format!("{} ч назад", secs / 3600),
        _ => format!("{} дн назад", secs / 86_400),
    }
}

impl eframe::App for Hub {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

impl Hub {
    fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        egui::Panel::left("nav")
            .exact_size(190.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(Color32::from_gray(26)).inner_margin(12))
            .show(ui, |ui| self.sidebar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default_margins()
            .frame(egui::Frame::new().fill(Color32::from_gray(32)).inner_margin(egui::Margin::symmetric(24, 8)))
            .show(ui, |ui| match self.page {
                Page::Projects => self.projects_page(ui),
                Page::Editor => self.editor_page(ui),
            });
        self.modals(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a few frames headless and returns the dialog's rect.
    fn modal_rect(modal: Modal, id: &str) -> egui::Rect {
        let ctx = egui::Context::default();
        style(&ctx);
        let mut hub = Hub::new(HubConfig::default(), None);
        hub.modal = Some(modal);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(955.0, 999.0));
        for frame in 0..40 {
            // Real sessions repaint on every mouse move; sizes settle over frames.
            let pointer = egui::pos2(300.0 + frame as f32 * 7.0, 400.0);
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| hub.draw(ui));
        }
        assert!(hub.modal.is_some(), "dialog closed by itself");
        ctx.memory(|m| m.area_rect(egui::Id::new(id))).expect("dialog was shown")
    }

    #[test]
    fn dialogs_fit_their_content() {
        let cases = [
            (Modal::NewProject { template: 0, name: String::new(), location: "/tmp".into() }, "new-project"),
            (Modal::AddExisting { path: String::new() }, "add-existing"),
            (Modal::EditorElements { wanted: Vec::new() }, "editor-elements"),
        ];
        for (modal, id) in cases {
            let rect = modal_rect(modal, id);
            assert!(rect.height() < 600.0, "{id}: {rect:?}");
            assert!(rect.top() > 0.0 && rect.bottom() < 999.0, "{id} off screen: {rect:?}");
        }
    }
}

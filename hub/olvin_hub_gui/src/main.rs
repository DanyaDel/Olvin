//! `olvin-hub-gui`: desktop front-end of the hub. Projects on the left,
//! the module catalog of the selected project on the right.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

use eframe::egui;
use olvin_hub::{Catalog, EditorInstall, HubConfig, Project};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Olvin Hub")
            .with_inner_size([940.0, 620.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("olvin-hub", options, Box::new(|_| Ok(Box::new(HubApp::load()))))
}

enum Status {
    Info(String),
    Error(String),
}

struct HubApp {
    catalog: Catalog,
    config: HubConfig,
    selected: Option<usize>,
    project: Option<Project>,
    new_name: String,
    new_dir: String,
    existing_path: String,
    status: Option<Status>,
    game: Option<Child>,
    editor: Option<EditorInstall>,
    /// Editor elements ticked in the UI (catalog order); applied by rebuilding.
    editor_wanted: Vec<String>,
    editor_build: Option<Child>,
    /// After the build finishes, open the editor (with this project).
    open_after_build: Option<Option<PathBuf>>,
}

impl HubApp {
    fn load() -> Self {
        let (config, status) = match HubConfig::load() {
            Ok(config) => (config, None),
            Err(err) => (HubConfig::default(), Some(Status::Error(format!("{err:#}")))),
        };
        let new_dir = dirs::home_dir()
            .map(|h| h.join("OlvinProjects").display().to_string())
            .unwrap_or_default();
        let mut app = Self {
            catalog: Catalog::builtin(),
            config,
            selected: None,
            project: None,
            new_name: String::new(),
            new_dir,
            existing_path: String::new(),
            status,
            game: None,
            editor: None,
            editor_wanted: Vec::new(),
            editor_build: None,
            open_after_build: None,
        };
        match EditorInstall::locate() {
            Ok(install) => {
                app.editor_wanted = install.initial_selection(&app.catalog);
                app.editor = Some(install);
            }
            Err(err) => app.status = Some(Status::Error(format!("{err:#}"))),
        }
        if !app.config.projects.is_empty() {
            app.select(0);
        }
        app
    }

    fn report<T>(&mut self, result: anyhow::Result<T>, ok: impl FnOnce(T) -> String) {
        self.status = Some(match result {
            Ok(value) => Status::Info(ok(value)),
            Err(err) => Status::Error(format!("{err:#}")),
        });
    }

    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        match Project::open(&self.config.projects[index]) {
            Ok(project) => self.project = Some(project),
            Err(err) => {
                self.project = None;
                self.status = Some(Status::Error(format!("{err:#}")));
            }
        }
    }

    fn create(&mut self) {
        let parent = PathBuf::from(self.new_dir.trim());
        let result = Project::create(&parent, self.new_name.trim(), &self.config.engine, &self.catalog)
            .and_then(|project| {
                self.config.register(project.root())?;
                self.config.save()?;
                Ok(project.root().display().to_string())
            });
        let created = result.is_ok();
        self.report(result, |root| format!("создан {root}"));
        if created {
            self.new_name.clear();
            self.select(self.config.projects.len() - 1);
        }
    }

    fn register_existing(&mut self) {
        let result = Project::open(self.existing_path.trim()).and_then(|project| {
            self.config.register(project.root())?;
            self.config.save()?;
            Ok(project.root().to_path_buf())
        });
        if let Ok(root) = &result
            && let Some(i) = self.config.projects.iter().position(|p| p == root)
        {
            self.existing_path.clear();
            self.select(i);
        }
        self.report(result, |root| format!("добавлен {}", root.display()));
    }

    fn forget_selected(&mut self) {
        if let Some(i) = self.selected.take() {
            let root = self.config.projects.remove(i);
            self.project = None;
            let result = self.config.save();
            self.report(result, |()| format!("забыт {} (файлы на месте)", root.display()));
        }
    }

    fn toggle(&mut self, id: &str, install: bool) {
        let Some(project) = &mut self.project else { return };
        let result = if install {
            project.add(&self.catalog, id).map(|added| format!("+ {}", added.join(", ")))
        } else {
            project.remove(&self.catalog, id, false).map(|_| format!("- {id}"))
        };
        let result = result.and_then(|msg| project.save().map(|()| msg));
        if result.is_err() {
            // Drop any half-applied edit by re-reading the file.
            if let Some(i) = self.selected {
                self.select(i);
            }
        }
        self.report(result, |msg| msg);
    }

    fn run_game(&mut self, release: bool) {
        let Some(project) = &self.project else { return };
        let mut cmd = Command::new("cargo");
        cmd.arg("run").current_dir(project.root());
        if release {
            cmd.arg("--release");
        }
        let result = cmd.spawn().map_err(anyhow::Error::from);
        let name = project.name().to_owned();
        match result {
            Ok(child) => {
                self.game = Some(child);
                self.status = Some(Status::Info(format!("{name}: сборка и запуск…")));
            }
            Err(err) => self.status = Some(Status::Error(format!("cargo: {err}"))),
        }
    }

    fn poll_game(&mut self) {
        if let Some(child) = &mut self.game
            && let Ok(Some(status)) = child.try_wait()
        {
            self.game = None;
            self.status = Some(if status.success() {
                Status::Info("игра завершилась".into())
            } else {
                Status::Error(format!("игра или сборка завершились с ошибкой ({status}), см. терминал"))
            });
        }
    }

    fn editor_up_to_date(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| e.is_built() && e.installed(&self.catalog) == self.editor_wanted)
    }

    fn build_editor(&mut self) {
        let Some(install) = &self.editor else { return };
        let result = install
            .write(&self.config.engine, &self.catalog, &self.editor_wanted)
            .and_then(|()| Ok(install.build_command().spawn()?));
        match result {
            Ok(child) => {
                self.editor_build = Some(child);
                self.status = Some(Status::Info("сборка редактора… (первая занимает несколько минут)".into()));
            }
            Err(err) => {
                self.open_after_build = None;
                self.status = Some(Status::Error(format!("{err:#}")));
            }
        }
    }

    fn open_editor(&mut self, project: Option<PathBuf>) {
        if self.editor_build.is_some() {
            self.open_after_build = Some(project);
            return;
        }
        if !self.editor_up_to_date() {
            self.open_after_build = Some(project);
            self.build_editor();
            return;
        }
        let Some(install) = &self.editor else { return };
        let result = install.launch(project.as_deref());
        self.report(result, |_| "редактор открыт".into());
    }

    fn poll_editor_build(&mut self) {
        if let Some(child) = &mut self.editor_build
            && let Ok(Some(status)) = child.try_wait()
        {
            self.editor_build = None;
            if status.success() {
                self.status = Some(Status::Info("редактор собран".into()));
                if let Some(project) = self.open_after_build.take() {
                    self.open_editor(project);
                }
            } else {
                self.open_after_build = None;
                self.status = Some(Status::Error(format!("сборка редактора не удалась ({status}), см. терминал")));
            }
        }
    }

    fn editor_section(&mut self, ui: &mut egui::Ui) {
        ui.strong("Редактор");
        let Some(install) = &self.editor else {
            ui.weak("недоступен");
            return;
        };
        let built = install.is_built();
        let building = self.editor_build.is_some();
        ui.horizontal(|ui| {
            if building {
                ui.spinner();
                ui.weak("сборка…");
            } else if built {
                ui.weak("установлен");
            } else {
                ui.weak("не установлен");
            }
        });
        ui.weak("База: сцена, иерархия, инспектор, ассеты");
        let mut changed = false;
        for m in &self.catalog.editor_modules {
            let mut on = self.editor_wanted.contains(&m.id);
            if ui.add_enabled(!building, egui::Checkbox::new(&mut on, &m.name)).on_hover_text(&m.description).changed() {
                changed = true;
                if on {
                    self.editor_wanted.push(m.id.clone());
                } else {
                    self.editor_wanted.retain(|id| *id != m.id);
                }
            }
        }
        if changed {
            let wanted = std::mem::take(&mut self.editor_wanted);
            self.editor_wanted =
                self.catalog.editor_modules.iter().filter(|m| wanted.contains(&m.id)).map(|m| m.id.clone()).collect();
        }
        let label = if built { "Применить (пересобрать)" } else { "Установить" };
        let up_to_date = self.editor_up_to_date();
        ui.horizontal(|ui| {
            if ui.add_enabled(!building && !up_to_date, egui::Button::new(label)).clicked() {
                self.build_editor();
            }
            if ui.add_enabled(!building, egui::Button::new("Открыть")).on_hover_text("Без проекта").clicked() {
                self.open_editor(None);
            }
        });
    }

    fn projects_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Проекты");
        ui.add_space(4.0);
        let mut clicked = None;
        egui::ScrollArea::vertical().max_height(ui.available_height() * 0.5).show(ui, |ui| {
            if self.config.projects.is_empty() {
                ui.weak("пока пусто");
            }
            for (i, root) in self.config.projects.iter().enumerate() {
                let name = root.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                if ui
                    .selectable_label(self.selected == Some(i), name.as_ref())
                    .on_hover_text(root.display().to_string())
                    .clicked()
                {
                    clicked = Some(i);
                }
            }
        });
        if let Some(i) = clicked {
            self.select(i);
        }

        ui.separator();
        ui.strong("Новый проект");
        ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("имя, например my_game"));
        ui.add(egui::TextEdit::singleline(&mut self.new_dir).hint_text("папка"));
        if ui.add_enabled(!self.new_name.trim().is_empty(), egui::Button::new("Создать")).clicked() {
            self.create();
        }

        ui.separator();
        ui.strong("Существующий проект");
        ui.add(egui::TextEdit::singleline(&mut self.existing_path).hint_text("путь к папке"));
        if ui
            .add_enabled(!self.existing_path.trim().is_empty(), egui::Button::new("Добавить"))
            .clicked()
        {
            self.register_existing();
        }

        ui.separator();
        self.editor_section(ui);

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.small(format!("движок: {}", self.config.engine));
        });
    }

    fn project_panel(&mut self, ui: &mut egui::Ui) {
        let Some(project) = &self.project else {
            ui.centered_and_justified(|ui| ui.weak("Выбери проект слева или создай новый"));
            return;
        };
        let installed: Vec<bool> =
            self.catalog.modules.iter().map(|m| project.is_installed(m)).collect();
        ui.heading(project.name());
        ui.weak(project.root().display().to_string());
        ui.add_space(6.0);

        let running = self.game.is_some();
        let (mut run, mut run_release, mut stop, mut forget) = (false, false, false, false);
        let mut edit = false;
        ui.horizontal(|ui| {
            edit = ui
                .add_enabled(self.editor.is_some(), egui::Button::new("✏ Открыть в редакторе"))
                .on_hover_text("Соберёт редактор, если нужно")
                .clicked();
            run = ui.add_enabled(!running, egui::Button::new("▶ Запустить")).clicked();
            run_release = ui.add_enabled(!running, egui::Button::new("▶ Release")).clicked();
            if running {
                stop = ui.button("■ Остановить").clicked();
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                forget = ui.button("Забыть проект").on_hover_text("Убрать из списка, файлы не удаляются").clicked();
            });
        });
        ui.separator();

        let mut toggled = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for category in self.catalog.categories() {
                ui.add_space(6.0);
                ui.strong(category);
                for (m, &on) in self.catalog.modules.iter().zip(&installed) {
                    if m.category != category {
                        continue;
                    }
                    let mut checked = on;
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut checked, &m.name).changed() {
                            toggled = Some((m.id.clone(), checked));
                        }
                        ui.weak(&m.description);
                    });
                    if !m.requires.is_empty() {
                        ui.indent(&m.id, |ui| ui.small(format!("требует: {}", m.requires.join(", "))));
                    }
                }
            }
        });

        if let Some((id, install)) = toggled {
            self.toggle(&id, install);
        }
        if run || run_release {
            self.run_game(run_release);
        }
        if stop && let Some(mut child) = self.game.take() {
            let _ = child.kill();
            self.status = Some(Status::Info("остановлено".into()));
        }
        if forget {
            self.forget_selected();
        }
        if edit {
            let root = self.project.as_ref().map(|p| p.root().to_path_buf());
            self.open_editor(root);
        }
    }
}

impl eframe::App for HubApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_game();
        self.poll_editor_build();
        egui::Panel::bottom("status").show(ui, |ui| match &self.status {
            Some(Status::Info(msg)) => ui.label(msg),
            Some(Status::Error(msg)) => ui.colored_label(ui.visuals().error_fg_color, msg),
            None => ui.weak("готово"),
        });
        egui::Panel::left("projects")
            .default_size(260.0)
            .resizable(true)
            .show(ui, |ui| self.projects_panel(ui));
        egui::CentralPanel::default_margins().show(ui, |ui| self.project_panel(ui));
        if self.game.is_some() || self.editor_build.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(300));
        }
    }
}

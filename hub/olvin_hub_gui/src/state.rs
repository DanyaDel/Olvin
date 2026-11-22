//! Hub state: construction from the saved config, persistence, the project
//! list, toasts, page and dialog switching.

use olvin_hub::{Catalog, EditorInstall, HubConfig, Project};

use crate::*;

impl Hub {
    pub(crate) fn load() -> Self {
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

    pub(crate) fn new(config: HubConfig, editor: Option<EditorInstall>) -> Self {
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
    pub(crate) fn save_config(&self) -> anyhow::Result<()> {
        match &self.config_file {
            Some(path) => self.config.save_to(path),
            None => self.config.save(),
        }
    }

    pub(crate) fn refresh(&mut self) {
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

    pub(crate) fn toast(&mut self, text: impl Into<String>) {
        self.push_toast(false, text.into());
    }

    pub(crate) fn toast_error(&mut self, text: impl Into<String>) {
        self.push_toast(true, text.into());
    }

    pub(crate) fn push_toast(&mut self, error: bool, text: String) {
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

    pub(crate) fn report<T>(&mut self, result: anyhow::Result<T>, ok: impl FnOnce(T) -> String) {
        match result {
            Ok(v) => self.toast(ok(v)),
            Err(e) => self.toast_error(format!("{e:#}")),
        }
    }

    /// Saves the `[gui]` part of the config after a preference changed.
    pub(crate) fn save_prefs(&mut self) {
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

    pub(crate) fn set_page(&mut self, page: Page, now: f64) {
        if self.page != page {
            self.page = page;
            self.page_at = now;
        }
    }

    pub(crate) fn open_modal(&mut self, modal: Modal, now: f64) {
        self.modal = Some(modal);
        self.modal_at = now;
    }

    pub(crate) fn new_project_modal(&self) -> Modal {
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
}

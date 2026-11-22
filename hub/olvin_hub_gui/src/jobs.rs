//! Background work: project creation, running games, the editor build and
//! the per-frame poll of child processes.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use olvin_hub::Project;

use crate::*;

impl Hub {
    // --- actions -----------------------------------------------------------

    pub(crate) fn create_project(
        &mut self,
        name: &str,
        location: &str,
        modules: &[String],
    ) -> bool {
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

    pub(crate) fn add_existing(&mut self, path: &str) -> bool {
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

    pub(crate) fn forget(&mut self, root: &Path) {
        self.config.forget(root);
        let result = self.save_config();
        let name = file_name(root);
        self.report(result, |()| {
            format!("Removed {name} from the list. Files stay on disk.")
        });
        self.refresh();
    }

    pub(crate) fn run_game(&mut self, root: &Path, release: bool) {
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

    pub(crate) fn stop_game(&mut self) {
        if let Some((name, mut child)) = self.game.take() {
            kill_tree(&mut child);
            self.toast(format!("Stopped {name}"));
        }
    }

    pub(crate) fn editor_selection(&self) -> Vec<String> {
        self.editor
            .as_ref()
            .map(|e| e.initial_selection(&self.catalog))
            .unwrap_or_default()
    }

    pub(crate) fn start_build(&mut self, wanted: &[String]) {
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

    pub(crate) fn open_editor(&mut self, project: Option<PathBuf>) {
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

    pub(crate) fn poll(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn editor_stale(&mut self) -> bool {
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

    pub(crate) fn row_action(&mut self, root: PathBuf, action: RowAction, now: f64) {
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
}

/// Kills `cargo run` and the game it started.
pub(crate) fn kill_tree(child: &mut Child) {
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

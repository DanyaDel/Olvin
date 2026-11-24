//! State shared by all IDE panels (thread-local, like other elements'
//! `Shared`): the project tree, open documents, the cargo job and its
//! output/problems, pending commands. `Ide::frame` runs once per egui pass,
//! from whichever IDE panel is drawn first.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use olvin_editor::EditorState;
use olvin_editor::egui::{self, Key, Modifiers};

use crate::diagnostics::{self, CargoMessage, Diagnostic, Severity};
use crate::document::{Conflict, DiskEvent, Document};
use crate::files::{self, Entry};
use crate::job::{Job, JobKind, JobMsg, Stream};
use crate::keys::{self, Captured, KeyRouter};
use crate::{dialogs, palette, shader};

thread_local! {
    static IDE: RefCell<Ide> = RefCell::new(Ide::default());
}

/// Runs `f` with the shared IDE state. Don't nest.
pub fn with<R>(f: impl FnOnce(&mut Ide) -> R) -> R {
    IDE.with_borrow_mut(f)
}

pub const POLL: Duration = Duration::from_secs(1);
pub const SHADER_DEBOUNCE: Duration = Duration::from_millis(300);
const OUTPUT_LIMIT: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutKind {
    Normal,
    Error,
    Warning,
    /// Messages from the IDE itself ("Build finished in 2.1 s").
    Info,
    Success,
}

#[derive(Clone, Debug)]
pub struct OutputLine {
    pub text: String,
    pub kind: OutKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    QuickOpen,
    CommandPalette,
    GoToLine,
    Save,
    SaveAll,
    CloseTab,
    NextTab,
    PrevTab,
    Find,
    Replace,
    ToggleComment,
    Check,
    Build,
    Run,
    Stop,
    ClearOutput,
    NewFile,
    NewFolder,
    NewSurfaceShader,
    ValidateShader,
    RevealActive,
    RefreshExplorer,
    CollapseAll,
}

impl Command {
    pub const ALL: [Command; 23] = [
        Self::QuickOpen,
        Self::CommandPalette,
        Self::GoToLine,
        Self::Save,
        Self::SaveAll,
        Self::CloseTab,
        Self::NextTab,
        Self::PrevTab,
        Self::Find,
        Self::Replace,
        Self::ToggleComment,
        Self::Check,
        Self::Build,
        Self::Run,
        Self::Stop,
        Self::ClearOutput,
        Self::NewFile,
        Self::NewFolder,
        Self::NewSurfaceShader,
        Self::ValidateShader,
        Self::RevealActive,
        Self::RefreshExplorer,
        Self::CollapseAll,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::QuickOpen => "Go to File…",
            Self::CommandPalette => "Show All Commands",
            Self::GoToLine => "Go to Line…",
            Self::Save => "File: Save",
            Self::SaveAll => "File: Save All",
            Self::CloseTab => "File: Close Tab",
            Self::NextTab => "View: Next Tab",
            Self::PrevTab => "View: Previous Tab",
            Self::Find => "Edit: Find",
            Self::Replace => "Edit: Replace",
            Self::ToggleComment => "Edit: Toggle Line Comment",
            Self::Check => "Cargo: Check",
            Self::Build => "Cargo: Build",
            Self::Run => "Cargo: Run Game",
            Self::Stop => "Cargo: Stop",
            Self::ClearOutput => "Output: Clear",
            Self::NewFile => "File: New File…",
            Self::NewFolder => "File: New Folder…",
            Self::NewSurfaceShader => "Shader: New Surface Shader…",
            Self::ValidateShader => "Shader: Validate Now",
            Self::RevealActive => "Explorer: Reveal Active File",
            Self::RefreshExplorer => "Explorer: Refresh",
            Self::CollapseAll => "Explorer: Collapse All",
        }
    }

    pub fn keys(self) -> &'static str {
        match self {
            Self::QuickOpen => "Ctrl+E",
            Self::CommandPalette => "Ctrl+Shift+P",
            Self::GoToLine => "Ctrl+G",
            Self::Save => "Ctrl+S",
            Self::SaveAll => "Ctrl+Shift+S",
            Self::CloseTab => "Ctrl+W",
            Self::NextTab => "Ctrl+Tab",
            Self::PrevTab => "Ctrl+Shift+Tab",
            Self::Find => "Ctrl+F",
            Self::Replace => "Ctrl+H",
            Self::ToggleComment => "Ctrl+/",
            Self::Build => "Ctrl+Shift+B",
            Self::NewFile => "Ctrl+N",
            _ => "",
        }
    }
}

/// Things only the code editor panel can do (it owns the cursor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorRequest {
    Find,
    Replace,
    ToggleComment,
    Focus,
}

/// A problem to draw in the editor (from cargo or the shader check).
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub line: usize,
    pub column: usize,
    /// Chars; `usize::MAX` = to the end of the line.
    pub len: usize,
    pub severity: Severity,
    pub message: String,
}

#[derive(Default)]
pub struct Ide {
    pub root: Option<PathBuf>,
    pub entries: Vec<Entry>,
    pub expanded: HashSet<PathBuf>,
    /// Explorer selection (target folder for "New File").
    pub selected: Option<PathBuf>,
    /// The explorer should scroll to the active file.
    pub reveal: bool,
    last_poll: Option<Instant>,

    pub docs: Vec<Document>,
    pub active: Option<usize>,

    pub job: Option<Job>,
    pub last_job: Option<(JobKind, bool)>,
    pub output: Vec<OutputLine>,
    pub diagnostics: Vec<Diagnostic>,

    pub requests: VecDeque<EditorRequest>,
    pub dialog: Option<dialogs::Dialog>,
    pub palette: Option<palette::Palette>,
    /// Passes until the keyboard goes back to the code (a dialog closed).
    pub refocus_in: u8,
    last_pass: Option<u64>,
}

impl Ide {
    // --- per-frame ----------------------------------------------------------

    /// Once per pass: keys, polling, background job, dialogs and palette.
    pub fn frame(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let ctx = ui.ctx().clone();
        let pass = ctx.cumulative_pass_nr();
        if self.last_pass == Some(pass) {
            return;
        }
        self.last_pass = Some(pass);

        if self.refocus_in > 0 {
            self.refocus_in -= 1;
            if self.refocus_in == 0 && self.active.is_some() {
                self.requests.push_back(EditorRequest::Focus);
            }
        }
        if self.root != state.project {
            self.set_root(state.project.clone());
        }

        let router = ctx.plugin_or_default::<KeyRouter>();
        let captured = {
            let mut r = router.lock();
            r.ide_visible = true;
            r.take()
        };
        for c in captured {
            let cmd = match c {
                Captured::Save => Command::Save,
                Captured::SaveAll => Command::SaveAll,
                Captured::NewFile => Command::NewFile,
                Captured::Palette => Command::CommandPalette,
                Captured::Build => Command::Build,
            };
            self.run(cmd, state);
        }
        if ctx.input_mut(|i| keys::consume_exact(i, Modifiers::COMMAND, Key::E)) > 0 {
            self.run(Command::QuickOpen, state);
        }

        let due = self.last_poll.is_none_or(|t| t.elapsed() >= POLL);
        if due {
            self.last_poll = Some(Instant::now());
            self.poll_disk(state);
        }
        self.poll_job(state);
        for doc in &mut self.docs {
            if doc.tick_shader(SHADER_DEBOUNCE) {
                ctx.request_repaint();
            }
            if doc.shader_pending() {
                ctx.request_repaint_after(SHADER_DEBOUNCE);
            }
        }

        palette::show(self, &ctx, state);
        dialogs::show(self, &ctx, state);

        if self.job.as_ref().is_some_and(Job::running) {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            ctx.request_repaint_after(POLL);
        }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>) {
        self.root = root;
        self.entries = self.root.as_deref().map(files::scan).unwrap_or_default();
        self.expanded.clear();
        if let Some(root) = &self.root {
            self.expanded.insert(root.join("src"));
        }
        self.last_poll = Some(Instant::now());
    }

    pub fn rescan(&mut self) {
        if let Some(root) = &self.root {
            let fresh = files::scan(root);
            if files::changed(&self.entries, &fresh) {
                self.entries = fresh;
            }
        }
    }

    fn poll_disk(&mut self, state: &mut EditorState) {
        self.rescan();
        for doc in &mut self.docs {
            match doc.check_disk() {
                DiskEvent::None => {}
                DiskEvent::Reloaded => {
                    state.info(format!("reloaded {} (changed on disk)", doc.name()))
                }
                DiskEvent::Conflict(Conflict::Changed) => state.warn(format!(
                    "{} changed on disk and has unsaved edits",
                    doc.name()
                )),
                DiskEvent::Conflict(Conflict::Deleted) => {
                    state.warn(format!("{} was deleted on disk", doc.name()))
                }
            }
        }
    }

    // --- documents ------------------------------------------------------------

    pub fn active_doc(&mut self) -> Option<&mut Document> {
        self.active.and_then(|i| self.docs.get_mut(i))
    }

    pub fn active_path(&self) -> Option<&Path> {
        self.active
            .and_then(|i| self.docs.get(i))
            .map(|d| d.path.as_path())
    }

    pub fn find_doc(&self, path: &Path) -> Option<usize> {
        self.docs.iter().position(|d| same_file(&d.path, path))
    }

    /// Opens (or switches to) a file. Returns its tab index.
    pub fn open(&mut self, path: &Path, state: &mut EditorState) -> Option<usize> {
        if let Some(i) = self.find_doc(path) {
            self.active = Some(i);
            return Some(i);
        }
        if !files::is_text(path) {
            state.warn(format!("{} is not a text file", path.display()));
            return None;
        }
        match Document::open(path) {
            Ok(doc) => {
                if let Some(why) = doc.read_only {
                    state.info(format!("{}: {why}", doc.name()));
                }
                self.docs.push(doc);
                let i = self.docs.len() - 1;
                self.active = Some(i);
                self.requests.push_back(EditorRequest::Focus);
                Some(i)
            }
            Err(err) => {
                state.error(format!("could not open {}: {err}", path.display()));
                None
            }
        }
    }

    /// Opens a file at a 1-based line and column.
    pub fn open_at(&mut self, path: &Path, line: usize, column: usize, state: &mut EditorState) {
        if let Some(i) = self.open(path, state) {
            let doc = &mut self.docs[i];
            doc.goto = Some(crate::text_ops::line_col_to_char(&doc.text, line, column));
            self.requests.push_back(EditorRequest::Focus);
        }
    }

    /// Closes a tab; asks first if it has unsaved edits (unless `force`).
    pub fn close(&mut self, i: usize, force: bool) {
        let Some(doc) = self.docs.get(i) else {
            return;
        };
        if doc.dirty && !force {
            self.dialog = Some(dialogs::Dialog::CloseDirty {
                path: doc.path.clone(),
            });
            return;
        }
        self.docs.remove(i);
        self.active = match self.active {
            _ if self.docs.is_empty() => None,
            Some(a) if a > i => Some(a - 1),
            Some(a) if a == i => Some(i.min(self.docs.len() - 1)),
            other => other,
        };
    }

    pub fn save(&mut self, i: usize, state: &mut EditorState) -> bool {
        let Some(doc) = self.docs.get_mut(i) else {
            return false;
        };
        match doc.save() {
            Ok(()) => {
                let name = self.rel(&self.docs[i].path);
                state.info(format!("saved {name}"));
                self.rescan();
                true
            }
            Err(err) => {
                state.error(format!("could not save {}: {err}", doc.name()));
                false
            }
        }
    }

    pub fn save_all(&mut self, state: &mut EditorState) {
        for i in 0..self.docs.len() {
            if self.docs[i].dirty && self.docs[i].read_only.is_none() {
                self.save(i, state);
            }
        }
    }

    pub fn cycle_tab(&mut self, back: bool) {
        let n = self.docs.len();
        if n == 0 {
            return;
        }
        let a = self.active.unwrap_or(0);
        self.active = Some(if back { (a + n - 1) % n } else { (a + 1) % n });
        self.requests.push_back(EditorRequest::Focus);
    }

    /// A path relative to the project for display.
    pub fn rel(&self, path: &Path) -> String {
        self.root
            .as_ref()
            .and_then(|r| path.strip_prefix(r).ok())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// Where "New File" puts things: the selected folder, the active
    /// file's folder, or the project root.
    pub fn target_dir(&self) -> Option<PathBuf> {
        let from_selection = self.selected.as_ref().map(|p| {
            if p.is_dir() {
                p.clone()
            } else {
                p.parent().map(Path::to_path_buf).unwrap_or_default()
            }
        });
        from_selection
            .or_else(|| {
                self.active_path()
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
            })
            .or_else(|| self.root.clone())
    }

    /// Expands the folders above the active file and scrolls to it.
    pub fn reveal_active(&mut self) {
        let (Some(root), Some(path)) =
            (self.root.clone(), self.active_path().map(Path::to_path_buf))
        else {
            return;
        };
        for dir in files::ancestors(&root, &path) {
            self.expanded.insert(dir);
        }
        self.selected = Some(path);
        self.reveal = true;
    }

    /// Updates open tabs after a file or folder was renamed.
    pub fn renamed(&mut self, from: &Path, to: &Path) {
        for doc in &mut self.docs {
            if let Ok(rest) = doc.path.strip_prefix(from) {
                doc.path = if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                };
                doc.disk_mtime = files::mtime(&doc.path);
            }
        }
        if self.expanded.remove(from) {
            self.expanded.insert(to.to_path_buf());
        }
    }

    /// Closes tabs of a deleted file or folder (edits are lost).
    pub fn deleted(&mut self, path: &Path) {
        while let Some(i) = self.docs.iter().position(|d| d.path.starts_with(path)) {
            self.close(i, true);
        }
    }

    // --- problems ---------------------------------------------------------------

    /// Cargo problems and shader errors for one file, for the editor.
    pub fn markers(&self, doc: &Document) -> Vec<Marker> {
        let mut out: Vec<Marker> = self
            .diagnostics
            .iter()
            .filter(|d| same_file(&d.path, &doc.path))
            .map(|d| Marker {
                line: d.line,
                column: d.column,
                len: if d.end_line == d.line {
                    d.end_column.saturating_sub(d.column).max(1)
                } else {
                    usize::MAX
                },
                severity: d.severity,
                message: d.summary(),
            })
            .collect();
        if let Some(errors) = &doc.shader {
            out.extend(errors.iter().map(|e| Marker {
                line: e.line,
                column: e.column,
                len: e.len,
                severity: Severity::Error,
                message: e.message.clone(),
            }));
        }
        out
    }

    pub fn counts(&self) -> (usize, usize) {
        let errors = self
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let warnings = self
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count();
        (errors, warnings)
    }

    // --- cargo --------------------------------------------------------------------

    pub fn job_running(&self) -> bool {
        self.job.as_ref().is_some_and(Job::running)
    }

    pub fn start_job(&mut self, kind: JobKind, state: &mut EditorState) {
        if !state.project_trusted {
            state.error(
                "project is not trusted: cargo would run its build.rs and proc-macros. Trust it first (olvin-hub trust, or reopen it in the editor)",
            );
            return;
        }
        let Some(root) = self.root.clone() else {
            state.error("no project open: nothing to build");
            return;
        };
        if !root.join("Cargo.toml").is_file() {
            state.error(format!("no Cargo.toml in {}", root.display()));
            return;
        }
        if let Some(job) = &mut self.job {
            job.kill();
        }
        self.save_all(state);
        self.output.clear();
        self.diagnostics.clear();
        self.say(
            format!(
                "> cargo {} (in {})",
                kind.label().to_lowercase(),
                root.display()
            ),
            OutKind::Info,
        );
        match Job::spawn(kind, kind.command(&root)) {
            Ok(job) => self.job = Some(job),
            Err(err) => {
                self.say(format!("could not start cargo: {err}"), OutKind::Error);
                state.error(format!("could not start cargo: {err}"));
            }
        }
    }

    pub fn stop_job(&mut self) {
        if let Some(job) = &mut self.job
            && job.running()
        {
            job.kill();
            self.say("stopping…", OutKind::Info);
        }
    }

    pub fn say(&mut self, text: impl Into<String>, kind: OutKind) {
        if self.output.len() >= OUTPUT_LIMIT {
            self.output.drain(..OUTPUT_LIMIT / 10);
        }
        self.output.push(OutputLine {
            text: text.into(),
            kind,
        });
    }

    fn poll_job(&mut self, state: &mut EditorState) {
        let Some(job) = &mut self.job else {
            return;
        };
        let kind = job.kind;
        let msgs = job.poll(2000);
        let finished = job.finished;
        let root = self.root.clone().unwrap_or_default();
        for msg in msgs {
            match msg {
                JobMsg::Line(Stream::Stderr, line) => {
                    let text = diagnostics::strip_ansi(&line);
                    let kind = classify(&text);
                    self.say(text, kind);
                }
                JobMsg::Line(Stream::Stdout, line) => match diagnostics::parse_line(&line, &root) {
                    CargoMessage::Diagnostic {
                        diagnostic,
                        rendered,
                    } => {
                        let level = diagnostic.as_ref().map(|d| d.severity);
                        if let Some(text) = rendered {
                            for l in text.trim_end().lines() {
                                let k = match level {
                                    Some(Severity::Error) => OutKind::Error,
                                    Some(Severity::Warning) => OutKind::Warning,
                                    _ => classify(l),
                                };
                                self.say(l, k);
                            }
                        }
                        if let Some(d) = diagnostic
                            && !self.diagnostics.contains(&d)
                        {
                            self.diagnostics.push(d);
                        }
                    }
                    CargoMessage::Text(text) => self.say(text, OutKind::Normal),
                    CargoMessage::Artifact
                    | CargoMessage::Other
                    | CargoMessage::Finished { .. } => {}
                },
                JobMsg::Exit(code) => {
                    let secs = finished.map_or(0.0, |f| f.1.as_secs_f32());
                    let ok = code == Some(0);
                    let (errors, warnings) = self.counts();
                    let what = kind.label();
                    let text = match code {
                        Some(0) => format!(
                            "{what} finished in {secs:.1} s ({errors} errors, {warnings} warnings)"
                        ),
                        Some(c) => format!(
                            "{what} failed with exit code {c} after {secs:.1} s ({errors} errors, {warnings} warnings)"
                        ),
                        None => format!("{what} stopped after {secs:.1} s"),
                    };
                    self.say(
                        text.clone(),
                        if ok { OutKind::Success } else { OutKind::Error },
                    );
                    if ok {
                        state.info(text);
                    } else {
                        state.warn(text);
                    }
                    self.last_job = Some((kind, ok));
                    self.diagnostics.sort_by(|a, b| {
                        (a.severity, &a.path, a.line, a.column)
                            .partial_cmp(&(b.severity, &b.path, b.line, b.column))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
            }
        }
    }

    // --- commands -------------------------------------------------------------------

    pub fn run(&mut self, cmd: Command, state: &mut EditorState) {
        match cmd {
            Command::QuickOpen => self.palette = Some(palette::Palette::new(palette::Mode::Files)),
            Command::CommandPalette => {
                self.palette = Some(palette::Palette::new(palette::Mode::Commands))
            }
            Command::GoToLine => {
                if self.active.is_some() {
                    self.palette = Some(palette::Palette::new(palette::Mode::Line));
                }
            }
            Command::Save => {
                if let Some(i) = self.active {
                    self.save(i, state);
                }
            }
            Command::SaveAll => self.save_all(state),
            Command::CloseTab => {
                if let Some(i) = self.active {
                    self.close(i, false);
                }
            }
            Command::NextTab => self.cycle_tab(false),
            Command::PrevTab => self.cycle_tab(true),
            Command::Find => self.requests.push_back(EditorRequest::Find),
            Command::Replace => self.requests.push_back(EditorRequest::Replace),
            Command::ToggleComment => self.requests.push_back(EditorRequest::ToggleComment),
            Command::Check => self.start_job(JobKind::Check, state),
            Command::Build => self.start_job(JobKind::Build, state),
            Command::Run => self.start_job(JobKind::Run, state),
            Command::Stop => self.stop_job(),
            Command::ClearOutput => self.output.clear(),
            Command::NewFile | Command::NewFolder => {
                if let Some(dir) = self.target_dir() {
                    self.dialog = Some(dialogs::Dialog::New {
                        dir,
                        folder: cmd == Command::NewFolder,
                        name: String::new(),
                        error: None,
                    });
                } else {
                    state.error("no project open");
                }
            }
            Command::NewSurfaceShader => {
                if self.root.is_some() {
                    self.dialog = Some(dialogs::Dialog::NewShader {
                        name: String::from("my_surface"),
                        error: None,
                    });
                } else {
                    state.error("no project open");
                }
            }
            Command::ValidateShader => {
                if let Some(doc) = self.active_doc() {
                    doc.validate_shader();
                    let msg = match &doc.shader {
                        None => {
                            format!("{} is not a surface shader (no `fn surface(`)", doc.name())
                        }
                        Some(e) if e.is_empty() => format!("{}: surface shader OK", doc.name()),
                        Some(e) => format!("{}: {} shader error(s)", doc.name(), e.len()),
                    };
                    state.info(msg);
                }
            }
            Command::RevealActive => self.reveal_active(),
            Command::RefreshExplorer => self.rescan(),
            Command::CollapseAll => self.expanded.clear(),
        }
    }

    /// Creates `assets/shaders/<name>.wgsl` from the contract template and opens it.
    pub fn new_surface_shader(
        &mut self,
        name: &str,
        state: &mut EditorState,
    ) -> Result<(), String> {
        files::valid_name(name).map_err(str::to_owned)?;
        let root = self.root.clone().ok_or("no project open")?;
        let file = if name.ends_with(".wgsl") {
            name.to_owned()
        } else {
            format!("{name}.wgsl")
        };
        let path = root.join("assets/shaders").join(file);
        create_file(&path, shader::template())?;
        state.info(format!("created {}", self.rel(&path)));
        self.rescan();
        self.open(&path, state);
        self.reveal_active();
        Ok(())
    }
}

/// Creates a new file (and its folders); fails if it exists.
pub fn create_file(path: &Path, contents: &str) -> Result<(), String> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => "already exists".to_owned(),
            _ => e.to_string(),
        })?;
    f.write_all(contents.as_bytes()).map_err(|e| e.to_string())
}

fn classify(line: &str) -> OutKind {
    let l = line.trim_start();
    if l.starts_with("error") {
        OutKind::Error
    } else if l.starts_with("warning") {
        OutKind::Warning
    } else if l.starts_with("Finished") {
        OutKind::Success
    } else {
        OutKind::Normal
    }
}

pub fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || (a.file_name() == b.file_name()
            && a.canonicalize()
                .ok()
                .is_some_and(|ca| b.canonicalize().ok() == Some(ca)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_table() {
        let labels: HashSet<&str> = Command::ALL.iter().map(|c| c.label()).collect();
        assert_eq!(labels.len(), Command::ALL.len(), "labels are unique");
        assert_eq!(Command::QuickOpen.keys(), "Ctrl+E");
        assert_eq!(classify("error[E0308]: x"), OutKind::Error);
        assert_eq!(classify("   Finished `dev` profile"), OutKind::Success);
    }

    /// T1: an untrusted project never gets a cargo job, and says why.
    #[test]
    fn untrusted_projects_do_not_start_cargo() {
        let dir = std::env::temp_dir().join(format!("olvin_ide_trust_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let mut state = EditorState::new(Some(dir.clone()));
        state.check_trust(&olvin_trust::TrustStore::in_memory(), false);
        assert!(!state.project_trusted && state.trust_pending);
        let mut ide = Ide::default();
        ide.set_root(Some(dir.clone()));
        ide.start_job(JobKind::Check, &mut state);
        assert!(!ide.job_running());
        assert!(state.log.last().unwrap().text.contains("not trusted"));

        // Read-only from the hub: untrusted, but no question.
        let mut state = EditorState::new(Some(dir.clone()));
        state.check_trust(&olvin_trust::TrustStore::in_memory(), true);
        assert!(!state.project_trusted && !state.trust_pending);
        // A trusted one is untouched.
        let mut store = olvin_trust::TrustStore::in_memory();
        store.trust(&dir).unwrap();
        let mut state = EditorState::new(Some(dir.clone()));
        state.check_trust(&store, false);
        assert!(state.project_trusted && !state.trust_pending);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tabs_close_and_cycle() {
        let dir = std::env::temp_dir().join(format!("olvin_ide_tabs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = EditorState::new(None);
        let mut ide = Ide::default();
        ide.set_root(Some(dir.clone()));
        for n in ["a.rs", "b.rs", "c.rs"] {
            std::fs::write(dir.join(n), n).unwrap();
            ide.open(&dir.join(n), &mut state);
        }
        assert_eq!(ide.active, Some(2));
        ide.cycle_tab(false);
        assert_eq!(ide.active, Some(0));
        ide.cycle_tab(true);
        assert_eq!(ide.active, Some(2));
        assert_eq!(ide.open(&dir.join("b.rs"), &mut state), Some(1));

        ide.docs[1].text.push('!');
        ide.docs[1].edited();
        ide.close(1, false);
        assert!(matches!(
            ide.dialog,
            Some(dialogs::Dialog::CloseDirty { .. })
        ));
        assert_eq!(ide.docs.len(), 3);
        ide.close(1, true);
        assert_eq!(ide.docs.len(), 2);
        assert_eq!(ide.active, Some(1));

        std::fs::rename(dir.join("c.rs"), dir.join("d.rs")).unwrap();
        ide.renamed(&dir.join("c.rs"), &dir.join("d.rs"));
        assert_eq!(ide.docs[1].path, dir.join("d.rs"));
        ide.deleted(&dir);
        assert!(ide.docs.is_empty() && ide.active.is_none());

        ide.new_surface_shader("water", &mut state).unwrap();
        let shader = dir.join("assets/shaders/water.wgsl");
        assert!(shader.is_file());
        assert_eq!(ide.active_path(), Some(shader.as_path()));
        assert_eq!(ide.docs[0].shader, Some(vec![]));
        assert!(ide.expanded.contains(&dir.join("assets/shaders")));
        assert!(ide.new_surface_shader("water", &mut state).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

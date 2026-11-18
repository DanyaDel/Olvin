//! Hot reload: watches `assets/` recursively and reports debounced changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

use notify::event::{EventKind, ModifyKind, RenameMode};
use notify::{RecursiveMode, Watcher};

use crate::meta::is_ignored_name;
use crate::root::relative_to;

/// Paths are relative to `assets/`, forward slashes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AssetEvent {
    Changed(String),
    Created(String),
    Removed(String),
    Moved { from: String, to: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Created,
    Changed,
    Removed,
}

/// File watcher for `assets/`. Call [`poll`](Self::poll) once per frame: it
/// never blocks and returns only events quiet for the debounce interval.
/// `.meta`, hidden and temp files are ignored.
pub struct AssetWatcher {
    root: PathBuf,
    rx: Receiver<notify::Result<notify::Event>>,
    _watcher: notify::RecommendedWatcher,
    debounce: Duration,
    pending: HashMap<String, (Pending, Instant)>,
    moves: Vec<(String, String, Instant)>,
    /// Half of a rename (From) waiting for its To, by tracker cookie.
    rename_from: HashMap<usize, (String, Instant)>,
}

impl AssetWatcher {
    /// Starts watching (fails e.g. when inotify is unavailable or out of watches).
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root: PathBuf = root.into();
        let root = root.canonicalize().unwrap_or(root);
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .map_err(|e| e.to_string())?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| format!("{}: {e}", root.display()))?;
        Ok(Self {
            root,
            rx,
            _watcher: watcher,
            debounce: Duration::from_millis(100),
            pending: HashMap::new(),
            moves: Vec::new(),
            rename_from: HashMap::new(),
        })
    }

    /// Quiet time before a change is reported (default 100 ms).
    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Moves first, then other events sorted by path.
    pub fn poll(&mut self) -> Vec<AssetEvent> {
        let now = Instant::now();
        loop {
            match self.rx.try_recv() {
                Ok(Ok(event)) => self.handle(event, now),
                Ok(Err(e)) => log::warn!("asset watcher: {e}"),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        // A From whose To never came: the file left the watched tree.
        let stale: Vec<usize> = self
            .rename_from
            .iter()
            .filter(|(_, (_, t))| now.duration_since(*t) >= self.debounce)
            .map(|(k, _)| *k)
            .collect();
        for k in stale {
            if let Some((rel, t)) = self.rename_from.remove(&k) {
                self.mark(rel, Pending::Removed, t);
            }
        }

        let ready = |t: &Instant| now.duration_since(*t) >= self.debounce;
        let mut out = Vec::new();
        // Keep later events of a path behind its move.
        let busy = |p: &str, pending: &HashMap<String, (Pending, Instant)>| {
            pending.get(p).is_some_and(|(_, t)| !ready(t))
        };
        let mut i = 0;
        while i < self.moves.len() {
            let (_, to, t) = &self.moves[i];
            if ready(t) && !busy(to, &self.pending) {
                let (from, to, _) = self.moves.remove(i);
                out.push(AssetEvent::Moved { from, to });
            } else {
                i += 1;
            }
        }
        let mut done: Vec<(String, Pending)> = self
            .pending
            .iter()
            .filter(|(_, (_, t))| ready(t))
            .map(|(p, (k, _))| (p.clone(), *k))
            .collect();
        done.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, kind) in done {
            self.pending.remove(&path);
            out.push(match kind {
                Pending::Created => AssetEvent::Created(path),
                Pending::Changed => AssetEvent::Changed(path),
                Pending::Removed => AssetEvent::Removed(path),
            });
        }
        out
    }

    fn rel(&self, path: &Path) -> Option<String> {
        let rel = relative_to(&self.root, path)?;
        if rel.is_empty() || rel.split('/').any(is_ignored_name) {
            return None;
        }
        Some(rel)
    }

    fn handle(&mut self, event: notify::Event, now: Instant) {
        match event.kind {
            EventKind::Create(_) => {
                for p in &event.paths {
                    if let Some(rel) = self.rel(p) {
                        self.mark(rel, Pending::Created, now);
                    }
                }
            }
            EventKind::Remove(_) => {
                for p in &event.paths {
                    if let Some(rel) = self.rel(p) {
                        self.mark(rel, Pending::Removed, now);
                    }
                }
            }
            EventKind::Modify(ModifyKind::Name(mode)) => match (mode, event.paths.as_slice()) {
                (RenameMode::Both, [from, to]) => self.rename(self.rel(from), self.rel(to), now),
                (RenameMode::From, [from]) => match (event.attrs.tracker(), self.rel(from)) {
                    (Some(cookie), Some(rel)) => {
                        self.rename_from.insert(cookie, (rel, now));
                    }
                    (None, Some(rel)) => self.mark(rel, Pending::Removed, now),
                    _ => {}
                },
                (RenameMode::To, [to]) => {
                    let from = event
                        .attrs
                        .tracker()
                        .and_then(|c| self.rename_from.remove(&c))
                        .map(|(rel, _)| rel);
                    let to = self.rel(to);
                    match from {
                        Some(from) => self.rename(Some(from), to, now),
                        None => {
                            if let Some(to) = to {
                                self.mark(to, Pending::Created, now);
                            }
                        }
                    }
                }
                _ => {
                    // Unknown rename shape: report by existence.
                    for p in &event.paths {
                        if let Some(rel) = self.rel(p) {
                            let kind = if p.exists() {
                                Pending::Created
                            } else {
                                Pending::Removed
                            };
                            self.mark(rel, kind, now);
                        }
                    }
                }
            },
            EventKind::Modify(ModifyKind::Metadata(_)) | EventKind::Access(_) => {}
            EventKind::Modify(_) | EventKind::Any | EventKind::Other => {
                for p in &event.paths {
                    if p.is_file()
                        && let Some(rel) = self.rel(p)
                    {
                        self.mark(rel, Pending::Changed, now);
                    }
                }
            }
        }
    }

    fn rename(&mut self, from: Option<String>, to: Option<String>, now: Instant) {
        match (from, to) {
            (Some(from), Some(to)) => {
                // inotify reports From, To and then Both for the same rename.
                if self.moves.iter().any(|(f, t, _)| *f == from && *t == to) {
                    return;
                }
                // Writes to the old name that weren't reported yet belong to the new one.
                if let Some((k, _)) = self.pending.remove(&from) {
                    if k == Pending::Created {
                        self.mark(to, Pending::Created, now);
                        return;
                    }
                    if k == Pending::Changed {
                        self.mark(to.clone(), Pending::Changed, now);
                    }
                }
                self.moves.push((from, to, now));
            }
            // Atomic save: temp file renamed over the asset.
            (None, Some(to)) => self.mark(to, Pending::Changed, now),
            (Some(from), None) => self.mark(from, Pending::Removed, now),
            (None, None) => {}
        }
    }

    /// Coalesces with what's already pending for the path.
    fn mark(&mut self, rel: String, kind: Pending, now: Instant) {
        use Pending::*;
        let merged = match (self.pending.get(&rel).map(|(k, _)| *k), kind) {
            (None, k) => Some(k),
            (Some(Created), Changed) => Some(Created),
            (Some(Created), Removed) => None,
            (Some(Removed), Created) => Some(Changed),
            (Some(Removed), Changed) => Some(Changed),
            (Some(Changed), Created) => Some(Changed),
            (Some(_), k) => Some(k),
        };
        match merged {
            Some(k) => {
                self.pending.insert(rel, (k, now));
            }
            None => {
                self.pending.remove(&rel);
            }
        }
    }
}

//! An open file: text, saved state, disk watching, shader checks.

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use crate::files;
use crate::lang::Lang;
use crate::shader::{self, ShaderError};

/// Bigger files open read-only and without highlighting.
pub const BIG_FILE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// Changed on disk while it has unsaved edits here.
    Changed,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskEvent {
    None,
    Reloaded,
    Conflict(Conflict),
}

pub struct Document {
    pub path: PathBuf,
    pub text: String,
    saved: String,
    pub dirty: bool,
    pub lang: Lang,
    pub read_only: Option<&'static str>,
    /// The file used `\r\n`; it's edited with `\n` and saved back with `\r\n`.
    crlf: bool,
    pub disk_mtime: Option<SystemTime>,
    pub conflict: Option<Conflict>,
    /// Surface shader errors (empty = fine); `None` when not a surface shader.
    pub shader: Option<Vec<ShaderError>>,
    shader_pending: Option<Instant>,
    /// Char index to put the cursor at and scroll to (set by "go to").
    pub goto: Option<usize>,
    /// Bumped when the text is replaced from outside the editor widget.
    pub generation: u64,
}

impl Document {
    pub fn open(path: &Path) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            return Err("is a folder".into());
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let mut read_only = (meta.len() > BIG_FILE).then_some("Large file: opened read-only");
        let text = match String::from_utf8(bytes) {
            Ok(t) => t,
            Err(e) => {
                read_only = Some("Not UTF-8 text: opened read-only");
                String::from_utf8_lossy(e.as_bytes()).into_owned()
            }
        };
        let crlf = text.contains("\r\n");
        let text = if crlf {
            text.replace("\r\n", "\n")
        } else {
            text
        };
        let lang = if read_only.is_some() {
            Lang::Plain
        } else {
            Lang::from_path(path)
        };
        let mut doc = Self {
            path: path.to_path_buf(),
            saved: text.clone(),
            text,
            dirty: false,
            lang,
            read_only,
            crlf,
            disk_mtime: meta.modified().ok(),
            conflict: None,
            shader: None,
            shader_pending: None,
            goto: None,
            generation: 0,
        };
        doc.validate_shader();
        Ok(doc)
    }

    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn is_surface_shader(&self) -> bool {
        self.lang == Lang::Wgsl && shader::is_surface_shader(&self.text)
    }

    /// Call after the text changed in the editor.
    pub fn edited(&mut self) {
        self.dirty = self.text != self.saved;
        if self.lang == Lang::Wgsl {
            self.shader_pending = Some(Instant::now());
        }
    }

    /// Replaces the whole text (find/replace, reload).
    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.generation += 1;
        self.edited();
    }

    pub fn save(&mut self) -> Result<(), String> {
        if let Some(why) = self.read_only {
            return Err(why.into());
        }
        let data = if self.crlf {
            self.text.replace('\n', "\r\n")
        } else {
            self.text.clone()
        };
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&self.path, data).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.saved = self.text.clone();
        self.dirty = false;
        self.conflict = None;
        self.disk_mtime = files::mtime(&self.path);
        Ok(())
    }

    /// Re-reads the file, dropping unsaved edits.
    pub fn reload(&mut self) -> Result<(), String> {
        let fresh = Self::open(&self.path)?;
        self.saved = fresh.saved;
        self.text = fresh.text;
        self.crlf = fresh.crlf;
        self.read_only = fresh.read_only;
        self.disk_mtime = fresh.disk_mtime;
        self.dirty = false;
        self.conflict = None;
        self.shader = fresh.shader;
        self.generation += 1;
        Ok(())
    }

    /// Keeps the edits here; the next save overwrites the disk version.
    pub fn keep_mine(&mut self) {
        self.conflict = None;
        self.disk_mtime = files::mtime(&self.path);
    }

    /// Looks at the file on disk: reloads if it changed and there are no
    /// edits here, otherwise reports a conflict.
    pub fn check_disk(&mut self) -> DiskEvent {
        let now = files::mtime(&self.path);
        if now.is_none() && self.path.exists() {
            return DiskEvent::None;
        }
        if now == self.disk_mtime || self.conflict.is_some() {
            return DiskEvent::None;
        }
        if now.is_none() {
            self.conflict = Some(Conflict::Deleted);
            self.disk_mtime = None;
            return DiskEvent::Conflict(Conflict::Deleted);
        }
        // Same content (touched, or saved by us through another path)?
        if std::fs::read_to_string(&self.path).is_ok_and(|t| t.replace("\r\n", "\n") == self.saved)
        {
            self.disk_mtime = now;
            return DiskEvent::None;
        }
        if self.dirty {
            self.conflict = Some(Conflict::Changed);
            DiskEvent::Conflict(Conflict::Changed)
        } else if self.reload().is_ok() {
            DiskEvent::Reloaded
        } else {
            DiskEvent::None
        }
    }

    /// Runs the debounced shader check if it's due. Returns true if it ran.
    pub fn tick_shader(&mut self, debounce: std::time::Duration) -> bool {
        match self.shader_pending {
            Some(t) if t.elapsed() >= debounce => {
                self.validate_shader();
                true
            }
            _ => false,
        }
    }

    pub fn validate_shader(&mut self) {
        self.shader_pending = None;
        self.shader = self
            .is_surface_shader()
            .then(|| shader::validate(&self.text));
    }

    pub fn shader_pending(&self) -> bool {
        self.shader_pending.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn edits_saves_and_watches_disk() {
        let dir = std::env::temp_dir().join(format!("olvin_ide_doc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.rs");
        std::fs::write(&path, "fn a() {}\r\n").unwrap();

        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.text, "fn a() {}\n");
        assert_eq!(doc.lang, Lang::Rust);
        doc.text.push_str("// x\n");
        doc.edited();
        assert!(doc.dirty);
        doc.save().unwrap();
        assert!(!doc.dirty);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "fn a() {}\r\n// x\r\n"
        );

        // Outside change, no local edits: reloaded.
        std::fs::write(&path, "fn b() {}\n").unwrap();
        doc.disk_mtime = Some(SystemTime::UNIX_EPOCH); // mtime granularity
        assert_eq!(doc.check_disk(), DiskEvent::Reloaded);
        assert_eq!(doc.text, "fn b() {}\n");

        // Outside change with local edits: conflict.
        doc.text.push('x');
        doc.edited();
        std::fs::write(&path, "fn c() {}\n").unwrap();
        doc.disk_mtime = Some(SystemTime::UNIX_EPOCH);
        assert_eq!(doc.check_disk(), DiskEvent::Conflict(Conflict::Changed));
        assert_eq!(doc.check_disk(), DiskEvent::None);
        doc.keep_mine();
        assert!(doc.conflict.is_none() && doc.dirty);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(doc.check_disk(), DiskEvent::Conflict(Conflict::Deleted));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shader_check_is_debounced() {
        let dir = std::env::temp_dir().join(format!("olvin_ide_wgsl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.wgsl");
        std::fs::write(&path, shader::template()).unwrap();
        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.shader, Some(vec![]));
        doc.text = doc.text.replace("return out;", "return 1.0;");
        doc.edited();
        assert!(!doc.tick_shader(Duration::from_secs(60)));
        assert!(doc.tick_shader(Duration::ZERO));
        assert_eq!(doc.shader.as_ref().map(Vec::len), Some(1));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

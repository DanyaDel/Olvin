//! The project tree: a flat, sorted list of files and folders (without
//! `target/` and dotfiles), rescanned cheaply by polling mtimes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use olvin_editor::egui::Color32;

/// More than this and the scan stops (a huge assets folder stays usable).
pub const MAX_ENTRIES: usize = 20_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    /// Path relative to the project root, `/`-separated.
    pub rel: String,
    pub depth: usize,
    pub is_dir: bool,
    pub mtime: Option<SystemTime>,
}

impl Entry {
    pub fn name(&self) -> &str {
        self.rel.rsplit('/').next().unwrap_or(&self.rel)
    }
}

pub fn hidden(name: &str) -> bool {
    name.starts_with('.') || name == "target"
}

/// Walks `root` depth-first: folders first, then files, by name.
pub fn scan(root: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    walk(root, root, 0, &mut out);
    out
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<Entry>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<(bool, String, PathBuf, Option<SystemTime>)> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if hidden(&name) {
                return None;
            }
            let meta = std::fs::metadata(e.path()).ok()?;
            Some((meta.is_dir(), name, e.path(), meta.modified().ok()))
        })
        .collect();
    items.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    for (is_dir, _, path, mtime) in items {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        out.push(Entry {
            path: path.clone(),
            rel,
            depth,
            is_dir,
            mtime,
        });
        if is_dir {
            walk(root, &path, depth + 1, out);
        }
    }
}

pub fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Whether two scans differ (added/removed/modified entries).
pub fn changed(old: &[Entry], new: &[Entry]) -> bool {
    old.len() != new.len()
        || old
            .iter()
            .zip(new)
            .any(|(a, b)| a.path != b.path || a.mtime != b.mtime || a.is_dir != b.is_dir)
}

/// Rows to draw: entries whose ancestors are all expanded.
pub fn visible<'a>(entries: &'a [Entry], expanded: &HashSet<PathBuf>) -> Vec<&'a Entry> {
    let mut out = Vec::new();
    // Depth below which everything is collapsed.
    let mut hide_below: Option<usize> = None;
    for e in entries {
        if let Some(d) = hide_below {
            if e.depth > d {
                continue;
            }
            hide_below = None;
        }
        out.push(e);
        if e.is_dir && !expanded.contains(&e.path) {
            hide_below = Some(e.depth);
        }
    }
    out
}

/// Every folder between `root` and `path` (to reveal a file).
pub fn ancestors(root: &Path, path: &Path) -> Vec<PathBuf> {
    path.ancestors()
        .skip(1)
        .take_while(|p| p.starts_with(root) && *p != root)
        .map(Path::to_path_buf)
        .collect()
}

/// Icon glyph and colour for a file or folder.
pub fn icon(name: &str, is_dir: bool, open: bool) -> (&'static str, Color32) {
    if is_dir {
        let glyph = if open { "📂" } else { "📁" };
        return (glyph, Color32::from_rgb(220, 180, 90));
    }
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    if lower.ends_with(".scene.ron") {
        return ("🎬", Color32::from_rgb(120, 200, 255));
    }
    if lower.ends_with(".mat.ron") {
        return ("🎨", Color32::from_rgb(230, 130, 200));
    }
    match ext {
        "rs" => ("📜", Color32::from_rgb(222, 120, 70)),
        "wgsl" => ("💎", Color32::from_rgb(86, 212, 190)),
        "ron" => ("⚙", Color32::from_rgb(180, 160, 230)),
        "lua" => ("☾", Color32::from_rgb(90, 140, 230)),
        "toml" | "lock" => ("⚙", Color32::from_rgb(150, 160, 175)),
        "md" | "txt" => ("📝", Color32::from_rgb(140, 180, 255)),
        "png" | "jpg" | "jpeg" | "ktx2" | "dds" | "tga" | "bmp" | "hdr" | "exr" | "webp" => {
            ("🖼", Color32::from_rgb(130, 200, 130))
        }
        "ogg" | "wav" | "mp3" | "flac" => ("🎵", Color32::from_rgb(240, 200, 100)),
        "gltf" | "glb" | "obj" | "fbx" | "blend" => ("📦", Color32::from_rgb(220, 160, 100)),
        _ => ("🗋", Color32::from_rgb(160, 168, 182)),
    }
}

/// Opening a file in the editor makes sense (not an image or a model).
pub fn is_text(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    !matches!(
        ext.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "ktx2"
            | "dds"
            | "tga"
            | "bmp"
            | "hdr"
            | "exr"
            | "webp"
            | "ogg"
            | "wav"
            | "mp3"
            | "flac"
            | "glb"
            | "fbx"
            | "blend"
            | "bin"
            | "so"
            | "exe"
            | "zip"
    )
}

/// Checks a new file or folder name.
pub fn valid_name(name: &str) -> Result<(), &'static str> {
    let name = name.trim();
    if name.is_empty() {
        Err("name is empty")
    } else if name == "."
        || name == ".."
        || name.contains(['\\', ':', '*', '?', '"', '<', '>', '|', '\0'])
    {
        Err("name has characters that aren't allowed")
    } else if name.split('/').any(|part| part.is_empty() || part == "..") {
        Err("bad path")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_filters_and_sorts() {
        let dir = std::env::temp_dir().join(format!("olvin_ide_scan_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["src/b", "assets", "target/debug", ".git"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for f in [
            "Cargo.toml",
            "src/main.rs",
            "src/b/x.rs",
            ".gitignore",
            "target/debug/g",
            "a.md",
        ] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let entries = scan(&dir);
        let rels: Vec<&str> = entries.iter().map(|e| e.rel.as_str()).collect();
        assert_eq!(
            rels,
            [
                "assets",
                "src",
                "src/b",
                "src/b/x.rs",
                "src/main.rs",
                "a.md",
                "Cargo.toml"
            ]
        );
        assert_eq!(entries[3].depth, 2);
        assert_eq!(entries[3].name(), "x.rs");

        let mut expanded = HashSet::new();
        let shown = |ex: &HashSet<PathBuf>| {
            visible(&entries, ex)
                .iter()
                .map(|e| e.rel.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(shown(&expanded), ["assets", "src", "a.md", "Cargo.toml"]);
        expanded.insert(dir.join("src"));
        assert_eq!(
            shown(&expanded),
            [
                "assets",
                "src",
                "src/b",
                "src/main.rs",
                "a.md",
                "Cargo.toml"
            ]
        );
        for a in ancestors(&dir, &dir.join("src/b/x.rs")) {
            expanded.insert(a);
        }
        assert_eq!(shown(&expanded).len(), 7);

        assert!(!changed(&entries, &scan(&dir)));
        std::fs::write(dir.join("src/new.rs"), "").unwrap();
        assert!(changed(&entries, &scan(&dir)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names() {
        assert!(valid_name("main.rs").is_ok());
        assert!(valid_name("sub/dir.rs").is_ok());
        assert!(valid_name("").is_err());
        assert!(valid_name("../x").is_err());
        assert!(valid_name("a:b").is_err());
        assert_eq!(icon("main.rs", false, false).0, "📜");
        assert_eq!(icon("a.mat.ron", false, false).0, "🎨");
        assert!(is_text(Path::new("a.wgsl")));
        assert!(!is_text(Path::new("a.PNG")));
    }
}

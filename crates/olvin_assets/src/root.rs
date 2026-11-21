//! The project's `assets/` folder: paths, kinds, and the GUID index.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::meta::{Guid, Importer, Meta, is_ignored_name, meta_path};
use crate::model::{ModelData, import_gltf, write_materials};
use crate::texture::{ImageData, load_image_sized};

/// What an asset is, from its file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Scene,
    Material,
    Texture,
    Model,
    Shader,
    Script,
    Audio,
    Prefab,
    /// `*.animgraph.ron`: an animation state graph (`olvin_scene::anim::AnimGraphDesc`).
    AnimGraph,
    Other,
}

impl AssetKind {
    /// Kind by (compound) extension: `*.scene.ron`, `*.mat.ron`, `*.prefab.ron`, `*.png`…
    pub fn of(path: impl AsRef<Path>) -> Self {
        let name = path
            .as_ref()
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.ends_with(".scene.ron") {
            return AssetKind::Scene;
        }
        if name.ends_with(".mat.ron") {
            return AssetKind::Material;
        }
        if name.ends_with(".prefab.ron") {
            return AssetKind::Prefab;
        }
        if name.ends_with(".animgraph.ron") {
            return AssetKind::AnimGraph;
        }
        let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
        match ext {
            "png" | "jpg" | "jpeg" | "hdr" | "exr" | "tga" | "bmp" | "webp" => AssetKind::Texture,
            "gltf" | "glb" | "obj" | "fbx" => AssetKind::Model,
            "wgsl" => AssetKind::Shader,
            "rs" | "lua" | "rhai" | "wasm" => AssetKind::Script,
            "wav" | "ogg" | "mp3" | "flac" => AssetKind::Audio,
            _ => AssetKind::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssetEntry {
    /// Relative to `assets/`, forward slashes.
    pub path: String,
    pub kind: AssetKind,
    pub guid: Guid,
}

/// The `assets/` folder of a project with its GUID index (filled by [`AssetRoot::scan`]).
#[derive(Clone, Debug)]
pub struct AssetRoot {
    root: PathBuf,
    by_guid: HashMap<Guid, String>,
    by_path: HashMap<String, Guid>,
}

impl AssetRoot {
    /// Does not touch the disk; call [`scan`](Self::scan) to index.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            by_guid: HashMap::new(),
            by_path: HashMap::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    /// `assets/` + relative path (forward or back slashes).
    pub fn resolve(&self, rel: &str) -> PathBuf {
        let mut p = self.root.clone();
        for part in rel
            .split(['/', '\\'])
            .filter(|s| !s.is_empty() && *s != ".")
        {
            p.push(part);
        }
        p
    }

    /// Path relative to `assets/` with forward slashes, `None` if outside.
    pub fn relative(&self, abs: impl AsRef<Path>) -> Option<String> {
        relative_to(&self.root, abs.as_ref())
    }

    /// Walks `assets/`, creates missing `.meta` files, fixes duplicate GUIDs
    /// (a file copied together with its meta gets a new one) and rebuilds the
    /// index. Sorted by path. Hidden and temp files are skipped.
    pub fn scan(&mut self) -> Vec<AssetEntry> {
        let mut files = Vec::new();
        walk(&self.root, &mut files);
        files.sort();
        self.by_guid.clear();
        self.by_path.clear();
        let mut out = Vec::with_capacity(files.len());
        for abs in files {
            let Some(rel) = self.relative(&abs) else {
                continue;
            };
            let kind = AssetKind::of(&abs);
            let mpath = meta_path(&abs);
            let mut meta = match Meta::load(&mpath) {
                Ok(m) => Some(m),
                Err(e) if mpath.exists() => {
                    log::warn!("unreadable meta, regenerating: {e}");
                    None
                }
                Err(_) => None,
            };
            let mut dirty = false;
            if meta
                .as_ref()
                .is_some_and(|m| self.by_guid.contains_key(&m.guid))
            {
                log::warn!("{rel}: duplicate GUID, assigning a new one");
                if let Some(m) = &mut meta {
                    m.guid = Guid::new_random();
                }
                dirty = true;
            }
            let meta = meta.unwrap_or_else(|| {
                dirty = true;
                Meta::new(kind)
            });
            if dirty && let Err(e) = meta.save(&mpath) {
                log::warn!("{e}");
            }
            self.by_guid.insert(meta.guid, rel.clone());
            self.by_path.insert(rel.clone(), meta.guid);
            out.push(AssetEntry {
                path: rel,
                kind,
                guid: meta.guid,
            });
        }
        out
    }

    /// GUID of an asset: from the index, else from its `.meta` (created if missing).
    pub fn guid_of(&self, rel: &str) -> Option<Guid> {
        if let Some(g) = self.by_path.get(&normalize(rel)) {
            return Some(*g);
        }
        self.meta(rel).ok().map(|m| m.guid)
    }

    /// Current path of an asset by GUID (index from the last scan/move).
    pub fn path_of(&self, guid: Guid) -> Option<&str> {
        self.by_guid.get(&guid).map(String::as_str)
    }

    /// Reads the asset's `.meta`, creating it with defaults when missing.
    pub fn meta(&self, rel: &str) -> Result<Meta, String> {
        let abs = self.resolve(rel);
        if !abs.is_file() {
            return Err(format!("{}: not a file", abs.display()));
        }
        let mpath = meta_path(&abs);
        if let Ok(m) = Meta::load(&mpath) {
            return Ok(m);
        }
        let m = Meta::new(AssetKind::of(&abs));
        m.save(&mpath)?;
        Ok(m)
    }

    /// Saves importer settings (the inspector's "Apply"). The GUID must stay the same.
    pub fn set_importer(&mut self, rel: &str, importer: Importer) -> Result<(), String> {
        let mut m = self.meta(rel)?;
        m.importer = importer;
        m.save(meta_path(self.resolve(rel)))
    }

    /// Renames/moves a file or folder together with its `.meta` file(s); the
    /// GUIDs stay. Fails if the destination exists.
    pub fn move_asset(&mut self, from: &str, to: &str) -> Result<(), String> {
        let (from, to) = (normalize(from), normalize(to));
        let (src, dst) = (self.resolve(&from), self.resolve(&to));
        if !src.exists() {
            return Err(format!("{}: not found", src.display()));
        }
        if dst.exists() {
            return Err(format!("{}: already exists", dst.display()));
        }
        if let Some(dir) = dst.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        if src.is_file() {
            // Make sure the GUID exists before it moves.
            self.meta(&from)?;
        }
        std::fs::rename(&src, &dst)
            .map_err(|e| format!("{} → {}: {e}", src.display(), dst.display()))?;
        let (msrc, mdst) = (meta_path(&src), meta_path(&dst));
        if msrc.exists() {
            std::fs::rename(&msrc, &mdst).map_err(|e| e.to_string())?;
        }
        // Re-key every indexed path under `from`.
        let prefix = format!("{from}/");
        let moved: Vec<(String, Guid)> = self
            .by_path
            .iter()
            .filter(|(p, _)| **p == from || p.starts_with(&prefix))
            .map(|(p, g)| (p.clone(), *g))
            .collect();
        for (old, guid) in moved {
            let new = format!("{to}{}", &old[from.len()..]);
            self.by_path.remove(&old);
            self.by_path.insert(new.clone(), guid);
            self.by_guid.insert(guid, new);
        }
        if let Ok(m) = Meta::load(&mdst) {
            self.by_path.insert(to.clone(), m.guid);
            self.by_guid.insert(m.guid, to);
        }
        Ok(())
    }

    /// Deletes a file or folder (recursively) and its `.meta`.
    pub fn delete_asset(&mut self, rel: &str) -> Result<(), String> {
        let rel = normalize(rel);
        let abs = self.resolve(&rel);
        if abs.is_dir() {
            std::fs::remove_dir_all(&abs)
        } else {
            std::fs::remove_file(&abs)
        }
        .map_err(|e| format!("{}: {e}", abs.display()))?;
        let mpath = meta_path(&abs);
        if mpath.exists() {
            std::fs::remove_file(&mpath).map_err(|e| e.to_string())?;
        }
        let prefix = format!("{rel}/");
        self.by_path.retain(|p, g| {
            let gone = *p == rel || p.starts_with(&prefix);
            if gone {
                self.by_guid.remove(g);
            }
            !gone
        });
        Ok(())
    }

    /// Loads a texture with the settings from its `.meta` (sRGB, max size).
    pub fn load_texture(&self, rel: &str) -> Result<ImageData, String> {
        let settings = self.meta(rel)?.texture().cloned().unwrap_or_default();
        load_image_sized(self.resolve(rel), settings.srgb, settings.max_size)
    }

    /// Imports a glTF/GLB with the settings from its `.meta`. Embedded textures
    /// are extracted next to it (see [`import_gltf`]).
    pub fn import_model(&self, rel: &str) -> Result<ModelData, String> {
        let settings = self.meta(rel)?.model().cloned().unwrap_or_default();
        import_gltf(self.resolve(rel), &self.root, &settings)
    }

    /// Writes the model's materials as `<dir>/<name>.mat.ron`; returns their
    /// assets-relative paths, one per `model.materials`.
    pub fn write_materials(&self, model: &ModelData, dir: &str) -> Result<Vec<String>, String> {
        let written = write_materials(model, self.resolve(dir))?;
        written
            .iter()
            .map(|p| {
                self.relative(p)
                    .ok_or_else(|| format!("{}: outside assets", p.display()))
            })
            .collect()
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let name = entry.file_name();
        if is_ignored_name(&name.to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, out),
            Ok(t) if t.is_file() => out.push(path),
            _ => {}
        }
    }
}

/// `a\b/./c/` → `a/b/c`.
pub(crate) fn normalize(rel: &str) -> String {
    rel.split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// `abs` relative to `root`, forward slashes; tries canonical paths too.
pub(crate) fn relative_to(root: &Path, abs: &Path) -> Option<String> {
    let strip = |root: &Path, abs: &Path| -> Option<String> {
        let rest = abs.strip_prefix(root).ok()?;
        let mut parts = Vec::new();
        for c in rest.components() {
            match c {
                Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
                Component::CurDir => {}
                _ => return None,
            }
        }
        Some(parts.join("/"))
    };
    strip(root, abs).or_else(|| {
        let root = root.canonicalize().ok()?;
        let abs = match abs.canonicalize() {
            Ok(a) => a,
            // Removed files can't be canonicalized: canonicalize the parent.
            Err(_) => abs.parent()?.canonicalize().ok()?.join(abs.file_name()?),
        };
        strip(&root, &abs)
    })
}

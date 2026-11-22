//! A pack in memory (from the embedded folder or from disk) and its checks.

use std::borrow::Cow;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::manifest::{
    MAX_FILE_BYTES, MAX_FILES, MAX_PACK_BYTES, MAX_PREVIEW_BYTES, Manifest, check_extension,
    check_path,
};

/// One file of `assets/`, path relative to the project's `assets/`.
#[derive(Clone, Debug)]
pub struct PackFile {
    pub path: String,
    pub data: Cow<'static, [u8]>,
}

impl PackFile {
    pub fn hash(&self) -> String {
        hash_hex(&self.data)
    }
}

/// Where a pack came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Compiled into the binary from the repo's `library/`.
    Builtin,
    /// A folder on disk (`$OLVIN_LIBRARY`, `<project>/library/`).
    Folder(std::path::PathBuf),
}

#[derive(Clone, Debug)]
pub struct Pack {
    pub manifest: Manifest,
    pub files: Vec<PackFile>,
    /// `preview.png`, if the pack has one.
    pub preview: Option<Cow<'static, [u8]>>,
    pub source: Source,
}

pub fn hash_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

impl Pack {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    pub fn total_bytes(&self) -> usize {
        self.files.iter().map(|f| f.data.len()).sum()
    }

    /// The prefab "Add to scene" instantiates: the first `*.prefab.ron` among
    /// `provides`.
    pub fn headline_prefab(&self) -> Option<&str> {
        self.manifest
            .provides
            .iter()
            .map(String::as_str)
            .find(|p| p.ends_with(".prefab.ron"))
    }

    /// A scene to open to try the pack (the first `*.scene.ron` in `provides`).
    pub fn demo_scene(&self) -> Option<&str> {
        self.manifest
            .provides
            .iter()
            .map(String::as_str)
            .find(|p| p.ends_with(".scene.ron"))
    }

    pub fn file(&self, path: &str) -> Option<&PackFile> {
        self.files.iter().find(|f| f.path == path)
    }

    /// Everything wrong with the pack that makes it unfit to import. `folder` is
    /// its directory name.
    pub fn validate(&self, folder: &str) -> Vec<String> {
        let mut errors = self.manifest.validate(folder);
        if self.files.is_empty() {
            errors.push("assets/ has no files".into());
        }
        if self.files.len() > MAX_FILES {
            errors.push(format!("more than {MAX_FILES} files"));
        }
        if self.total_bytes() > MAX_PACK_BYTES {
            errors.push(format!(
                "assets/ is larger than {} MiB",
                MAX_PACK_BYTES >> 20
            ));
        }
        if let Some(p) = &self.preview
            && p.len() > MAX_PREVIEW_BYTES
        {
            errors.push(format!(
                "preview.png is {} KB, the limit is {}",
                p.len() >> 10,
                MAX_PREVIEW_BYTES >> 10
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for f in &self.files {
            if let Err(e) = check_path(&f.path).and_then(|_| check_extension(&f.path)) {
                errors.push(format!("{}: {e}", f.path));
            }
            if f.data.len() > MAX_FILE_BYTES {
                errors.push(format!(
                    "{}: larger than {} MiB",
                    f.path,
                    MAX_FILE_BYTES >> 20
                ));
            }
            // Case-folded: two names that differ in case collide on some systems.
            if !seen.insert(f.path.to_lowercase()) {
                errors.push(format!("{}: duplicate path", f.path));
            }
        }
        for p in &self.manifest.provides {
            if self.file(p).is_none() {
                errors.push(format!("provides `{p}`: no such file in assets/"));
            }
        }
        errors
    }

    /// Style problems that do not block the import: files outside the pack's own
    /// namespace (`<top>/<pack-id>/...` or `<pack-id>/...`) can collide with
    /// other packs and with the user's files.
    pub fn lint(&self) -> Vec<String> {
        let id = &self.manifest.id;
        self.files
            .iter()
            .filter(|f| {
                let mut parts = f.path.split('/');
                let (a, b) = (parts.next(), parts.next());
                a != Some(id) && b != Some(id)
            })
            .map(|f| {
                format!(
                    "{}: not under `<pack-id>/` or `<folder>/<pack-id>/`",
                    f.path
                )
            })
            .collect()
    }

    /// Reads `<dir>/pack.ron`, `<dir>/assets/**` and `<dir>/preview.png`.
    pub fn from_dir(dir: &Path) -> Result<Pack, String> {
        let text = fs::read_to_string(dir.join("pack.ron"))
            .map_err(|e| format!("{}: pack.ron: {e}", dir.display()))?;
        let manifest = Manifest::parse(&text)?;
        let mut files = Vec::new();
        let assets = dir.join("assets");
        if assets.is_dir() {
            walk(&assets, &assets, &mut files)?;
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let preview = fs::read(dir.join("preview.png")).ok().map(Cow::Owned);
        Ok(Pack {
            manifest,
            files,
            preview,
            source: Source::Folder(dir.to_path_buf()),
        })
    }

    pub(crate) fn from_embedded(dir: &'static include_dir::Dir<'static>) -> Result<Pack, String> {
        let name = dir
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let manifest_file = dir
            .get_file(dir.path().join("pack.ron"))
            .ok_or_else(|| format!("{name}: no pack.ron"))?;
        let text = std::str::from_utf8(manifest_file.contents())
            .map_err(|e| format!("{name}: pack.ron: {e}"))?;
        let manifest = Manifest::parse(text).map_err(|e| format!("{name}: {e}"))?;
        let mut files = Vec::new();
        if let Some(assets) = dir.get_dir(dir.path().join("assets")) {
            embedded_files(assets, assets.path(), &mut files);
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let preview = dir
            .get_file(dir.path().join("preview.png"))
            .map(|f| Cow::Borrowed(f.contents()));
        Ok(Pack {
            manifest,
            files,
            preview,
            source: Source::Builtin,
        })
    }
}

fn embedded_files(dir: &'static include_dir::Dir<'static>, base: &Path, out: &mut Vec<PackFile>) {
    for file in dir.files() {
        let rel = file.path().strip_prefix(base).unwrap_or(file.path());
        out.push(PackFile {
            path: rel.to_string_lossy().replace('\\', "/"),
            data: Cow::Borrowed(file.contents()),
        });
    }
    for sub in dir.dirs() {
        embedded_files(sub, base, out);
    }
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<PackFile>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            walk(base, &path, out)?;
        } else {
            let rel = path
                .strip_prefix(base)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let data = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out.push(PackFile {
                path: rel,
                data: Cow::Owned(data),
            });
        }
    }
    Ok(())
}

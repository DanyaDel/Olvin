//! Files dragged in from the OS file manager. They are copied into the folder
//! the Assets panel shows, and `olvin_assets` gives every new file its `.meta`
//! GUID. Nothing is overwritten: a name that is taken gets `-2`, `-3`, ...

use std::path::{Path, PathBuf};

/// Compound asset extensions: the counter goes before them (`x-2.prefab.ron`).
const COMPOUND: [&str; 5] = [
    ".scene.ron",
    ".mat.ron",
    ".prefab.ron",
    ".animgraph.ron",
    ".look.ron",
];

/// What a drop did, for the console and the Assets panel.
#[derive(Debug, Default)]
pub struct DropReport {
    /// Files and folders copied, as absolute paths.
    pub imported: Vec<PathBuf>,
    /// Copies renamed to avoid overwriting something.
    pub renamed: usize,
    /// Human-readable reasons for everything that was not copied.
    pub skipped: Vec<String>,
}

/// The first name in `dir` that is free: `file`, then `stem-2.ext`, `stem-3.ext`, ...
pub fn free_name(dir: &Path, file: &str) -> String {
    if !dir.join(file).exists() {
        return file.to_owned();
    }
    let lower = file.to_lowercase();
    let split = COMPOUND
        .iter()
        .find(|c| lower.ends_with(*c))
        .map(|c| file.len() - c.len())
        .or_else(|| file.rfind('.').filter(|&i| i > 0))
        .unwrap_or(file.len());
    let (stem, ext) = file.split_at(split);
    (2u32..)
        .map(|n| format!("{stem}-{n}{ext}"))
        .find(|candidate| !dir.join(candidate).exists())
        .unwrap_or_else(|| file.to_owned())
}

/// Copies `sources` into `dir` (which must be inside `assets`). Folders are
/// copied with their contents; symlinks are skipped so a link cannot loop.
pub fn import(sources: &[PathBuf], dir: &Path) -> DropReport {
    let mut report = DropReport::default();
    for src in sources {
        let Some(name) = src.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            report
                .skipped
                .push(format!("{}: no file name", src.display()));
            continue;
        };
        let meta = match std::fs::symlink_metadata(src) {
            Ok(m) => m,
            Err(e) => {
                report.skipped.push(format!("{name}: {e}"));
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            report
                .skipped
                .push(format!("{name}: symlinks are not copied"));
            continue;
        }
        // A folder dropped into itself (or into its own subfolder) would never end.
        if meta.is_dir() && dir.starts_with(src) {
            report
                .skipped
                .push(format!("{name}: cannot copy a folder into itself"));
            continue;
        }
        let target_name = free_name(dir, &name);
        if target_name != name {
            report.renamed += 1;
        }
        let target = dir.join(&target_name);
        let result = if meta.is_dir() {
            copy_dir(src, &target)
        } else if meta.is_file() {
            std::fs::copy(src, &target).map(|_| ())
        } else {
            Err(std::io::Error::other("not a file or folder"))
        };
        match result {
            Ok(()) => report.imported.push(target),
            Err(e) => report.skipped.push(format!("{name}: {e}")),
        }
    }
    report
}

/// Recursive copy of a folder into a folder that does not exist yet.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = to.join(entry.file_name());
        if file_type.is_symlink() {
            continue;
        } else if file_type.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder under the system temp dir (the files are a few bytes).
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("olvin-os-drop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn counter_goes_before_compound_extensions() {
        let dir = scratch("names");
        std::fs::write(dir.join("hero.png"), b"x").unwrap();
        std::fs::write(dir.join("hero-2.png"), b"x").unwrap();
        std::fs::write(dir.join("Player.prefab.ron"), b"x").unwrap();
        assert_eq!(free_name(&dir, "hero.png"), "hero-3.png");
        assert_eq!(free_name(&dir, "Player.prefab.ron"), "Player-2.prefab.ron");
        assert_eq!(free_name(&dir, "rock.png"), "rock.png");
        assert_eq!(free_name(&dir, ".hidden"), ".hidden");
    }

    #[test]
    fn copies_files_and_folders_and_never_overwrites() {
        let src = scratch("src");
        let dst = scratch("dst");
        std::fs::write(src.join("tex.png"), b"one").unwrap();
        std::fs::write(dst.join("tex.png"), b"keep").unwrap();
        let folder = src.join("pack");
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        std::fs::write(folder.join("inner/a.lua"), b"print(1)").unwrap();

        let report = import(&[src.join("tex.png"), folder.clone()], &dst);

        assert_eq!(report.imported.len(), 2, "{:?}", report.skipped);
        assert_eq!(report.renamed, 1);
        assert_eq!(std::fs::read(dst.join("tex.png")).unwrap(), b"keep");
        assert_eq!(std::fs::read(dst.join("tex-2.png")).unwrap(), b"one");
        assert_eq!(
            std::fs::read(dst.join("pack/inner/a.lua")).unwrap(),
            b"print(1)"
        );
    }

    #[test]
    fn refuses_a_folder_into_itself() {
        let src = scratch("loop");
        let report = import(std::slice::from_ref(&src), &src.join("sub"));
        assert!(report.imported.is_empty());
        assert_eq!(report.skipped.len(), 1);
        std::fs::create_dir_all(src.join("sub")).unwrap();
        let report = import(std::slice::from_ref(&src), &src.join("sub"));
        assert!(report.imported.is_empty());
        assert!(report.skipped[0].contains("into itself"));
    }
}

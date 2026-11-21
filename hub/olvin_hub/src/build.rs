//! `olvin-hub build`: builds a game and lays out a folder (or an archive) that
//! can be handed to players: the executable next to a copy of `assets/`.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::Project;

/// Written into a folder the hub made, so a rebuild may replace it but never
/// wipes a folder the user pointed `--out` at by mistake.
const MARKER: &str = ".olvin-build";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveFormat {
    TarGz,
    Zip,
}

impl ArchiveFormat {
    pub fn parse(text: &str) -> Result<Self> {
        match text.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "tar.gz" | "tgz" => Ok(Self::TarGz),
            "zip" => Ok(Self::Zip),
            other => bail!("unknown archive format `{other}`: use tar.gz or zip"),
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::TarGz => "tar.gz",
            Self::Zip => "zip",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BuildOptions {
    /// `cargo build --release` (otherwise the dev profile).
    pub release: bool,
    /// Output folder (default: `<project>/dist/<name>`).
    pub out: Option<PathBuf>,
    /// Also pack the folder next to itself (`<out>.tar.gz` / `<out>.zip`).
    pub archive: Option<ArchiveFormat>,
    /// Cargo target directory (default: cargo's own choice, `CARGO_TARGET_DIR` included).
    pub target_dir: Option<PathBuf>,
}

#[derive(Debug)]
pub struct BuildReport {
    pub out_dir: PathBuf,
    pub binary: PathBuf,
    /// Files copied from `assets/`.
    pub assets: usize,
    pub archive: Option<PathBuf>,
    /// `/dist` was appended to the project's `.gitignore` (see [`Project::build`]).
    pub gitignore_updated: bool,
}

impl Project {
    /// Where [`build`](Self::build) puts the game by default.
    pub fn dist_dir(&self) -> PathBuf {
        self.root().join("dist").join(self.name())
    }

    /// Builds the game with cargo, then copies the executable and `assets/` to
    /// the output folder (and packs it, if asked). Cargo's own output goes to
    /// the terminal. The default folder is inside the project, so a project
    /// `.gitignore` that doesn't know `dist` gets `/dist` added (projects made
    /// before the template had it); no `.gitignore`, nothing is written.
    pub fn build(&self, options: &BuildOptions) -> Result<BuildReport> {
        let exe = self.cargo_build(options)?;
        let out = options.out.clone().unwrap_or_else(|| self.dist_dir());
        prepare_out(&out)?;
        let gitignore_updated = options.out.is_none() && ignore_dist(self.root())?;

        let binary = out.join(exe.file_name().context("the executable has no file name")?);
        fs::copy(&exe, &binary)
            .with_context(|| format!("could not copy {} to {}", exe.display(), binary.display()))?;
        let assets = match self.root().join("assets") {
            dir if dir.is_dir() => copy_tree(&dir, &out.join("assets"))?,
            _ => 0,
        };
        let archive = options
            .archive
            .map(|format| pack(&out, self.name(), format))
            .transpose()?;
        Ok(BuildReport {
            out_dir: out,
            binary,
            assets,
            archive,
            gitignore_updated,
        })
    }

    /// Runs `cargo build` and returns the path of the game's executable.
    fn cargo_build(&self, options: &BuildOptions) -> Result<PathBuf> {
        let manifest = self.root().join("Cargo.toml");
        let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        cmd.args(["build", "--message-format=json-render-diagnostics"])
            .arg("--manifest-path")
            .arg(&manifest);
        if options.release {
            cmd.arg("--release");
        }
        if let Some(dir) = &options.target_dir {
            cmd.arg("--target-dir").arg(dir);
        }
        let mut child = cmd
            .stdout(Stdio::piped())
            .spawn()
            .context("could not start cargo")?;
        let manifest = manifest.canonicalize().unwrap_or(manifest);
        let mut exe = None;
        for line in BufReader::new(child.stdout.take().expect("stdout is piped")).lines() {
            if let Some(path) = artifact_of(&line?, &manifest) {
                exe = Some(path);
            }
        }
        let status = child.wait()?;
        if !status.success() {
            bail!("cargo build failed ({status})");
        }
        exe.context("cargo built no executable: does the project have a binary target?")
    }
}

/// The executable of `manifest`'s package from one line of cargo's JSON output.
fn artifact_of(line: &str, manifest: &Path) -> Option<PathBuf> {
    let msg: serde_json::Value = serde_json::from_str(line).ok()?;
    if msg["reason"] != "compiler-artifact" {
        return None;
    }
    let is_bin = msg["target"]["kind"].as_array()?.iter().any(|k| k == "bin");
    let same = Path::new(msg["manifest_path"].as_str()?)
        .canonicalize()
        .is_ok_and(|p| p == manifest);
    (is_bin && same)
        .then(|| PathBuf::from(msg["executable"].as_str().unwrap_or_default()))
        .filter(|p| !p.as_os_str().is_empty())
}

/// Appends `/dist` to `<root>/.gitignore` unless a line already covers it.
fn ignore_dist(root: &Path) -> Result<bool> {
    let path = root.join(".gitignore");
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(false);
    };
    let covered = text.lines().map(str::trim).any(|l| {
        matches!(
            l,
            "dist" | "dist/" | "/dist" | "/dist/" | "dist/*" | "/dist/*"
        )
    });
    if covered {
        return Ok(false);
    }
    let sep = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    fs::write(&path, format!("{text}{sep}/dist\n"))
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(true)
}

/// An empty folder, or one a previous build made (it is cleared).
fn prepare_out(out: &Path) -> Result<()> {
    if out.exists() {
        let marked = out.join(MARKER).is_file();
        let empty = fs::read_dir(out)?.next().is_none();
        if !marked && !empty {
            bail!(
                "{} exists and was not made by `olvin-hub build`. Pick another --out or empty it",
                out.display()
            );
        }
        if marked {
            fs::remove_dir_all(out)?;
        }
    }
    fs::create_dir_all(out)?;
    fs::write(out.join(MARKER), "made by olvin-hub build\n")?;
    Ok(())
}

fn hidden(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|n| n.starts_with('.') || n.ends_with('~'))
}

/// Copies `from` into `to` without hidden files and editor backups; returns the file count.
fn copy_tree(from: &Path, to: &Path) -> Result<usize> {
    fs::create_dir_all(to)?;
    let mut count = 0;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if hidden(&entry.file_name()) {
            continue;
        }
        let target = to.join(entry.file_name());
        // `is_dir` follows symlinks: a linked folder is copied, not read as a file.
        if entry.path().is_dir() {
            count += copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)
                .with_context(|| format!("could not copy {}", entry.path().display()))?;
            count += 1;
        }
    }
    Ok(count)
}

/// Files of the folder as (path on disk, path inside the archive), sorted.
fn files_of(dir: &Path, prefix: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if hidden(&entry.file_name()) {
            continue;
        }
        let inner = prefix.join(entry.file_name());
        if entry.path().is_dir() {
            files_of(&entry.path(), &inner, out)?;
        } else {
            out.push((entry.path(), inner));
        }
    }
    Ok(())
}

/// Packs the folder as `<folder>.tar.gz` / `.zip`, everything under a top
/// folder named after the game.
fn pack(dir: &Path, name: &str, format: ArchiveFormat) -> Result<PathBuf> {
    let mut files = Vec::new();
    files_of(dir, Path::new(name), &mut files)?;
    // Built from the folder's name: `--out dist/game/` must not give `dist/game/.zip`.
    let folder = dir.file_name().context("the output folder has no name")?;
    let mut file_name = folder.to_owned();
    file_name.push(format!(".{}", format.extension()));
    let path = dir.with_file_name(file_name);
    let file =
        File::create(&path).with_context(|| format!("could not create {}", path.display()))?;
    match format {
        ArchiveFormat::TarGz => {
            let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut tar = tar::Builder::new(gz);
            for (src, inner) in &files {
                tar.append_path_with_name(src, inner)?;
            }
            tar.into_inner()?.finish()?;
        }
        ArchiveFormat::Zip => {
            let mut zip = zip::ZipWriter::new(file);
            for (src, inner) in &files {
                let mut options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    options = options.unix_permissions(fs::metadata(src)?.permissions().mode());
                }
                zip.start_file(inner.to_string_lossy().replace('\\', "/"), options)?;
                io::copy(&mut File::open(src)?, &mut zip)?;
            }
            zip.finish()?;
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn temp_dir(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("olvin-hub-build-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A game that builds offline and in seconds: `olvin` is a stub crate.
    fn tiny_game(dir: &Path) -> Project {
        let root = dir.join("tiny");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("olvin/src")).unwrap();
        fs::write(
            root.join("olvin/Cargo.toml"),
            "[package]\nname = \"olvin\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(root.join("olvin/src/lib.rs"), "").unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nolvin = { path = \"olvin\" }\n\n[workspace]\n",
        )
        .unwrap();
        fs::write(
            root.join("src/main.rs"),
            "use olvin as _;\nfn main() { println!(\"tiny\"); }\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("assets/scenes")).unwrap();
        fs::write(root.join("assets/scenes/main.scene.ron"), "()").unwrap();
        fs::write(root.join("assets/icon.png"), [1, 2, 3]).unwrap();
        fs::write(root.join("assets/.hidden"), "x").unwrap();
        fs::write(root.join("assets/notes.txt~"), "x").unwrap();
        fs::write(root.join(".gitignore"), "/target\n/logs").unwrap();
        Project::open(root).unwrap()
    }

    fn options(dir: &Path) -> BuildOptions {
        BuildOptions {
            target_dir: Some(dir.join("target")),
            ..Default::default()
        }
    }

    #[test]
    fn builds_into_a_folder_with_assets() {
        let dir = temp_dir("folder");
        let game = tiny_game(&dir);
        let report = game
            .build(&BuildOptions {
                release: true,
                ..options(&dir)
            })
            .unwrap();
        assert_eq!(report.out_dir, game.dist_dir());
        assert!(report.gitignore_updated, "an old template's .gitignore");
        assert_eq!(
            fs::read_to_string(game.root().join(".gitignore")).unwrap(),
            "/target\n/logs\n/dist\n"
        );
        assert!(report.binary.is_file());
        assert!(report.binary.starts_with(&report.out_dir));
        assert_eq!(report.assets, 2, "hidden files and backups are left out");
        assert!(
            report
                .out_dir
                .join("assets/scenes/main.scene.ron")
                .is_file()
        );
        assert!(!report.out_dir.join("assets/.hidden").exists());
        assert!(
            dir.join("target/release").is_dir(),
            "release profile was used"
        );
        let run = Command::new(&report.binary).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "tiny");

        // A second build replaces the folder, dropping stale files.
        fs::write(report.out_dir.join("stale.txt"), "x").unwrap();
        assert!(!game.build(&options(&dir)).unwrap().gitignore_updated);
        assert!(!report.out_dir.join("stale.txt").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn archives_and_refuses_foreign_folders() {
        let dir = temp_dir("archive");
        let game = tiny_game(&dir);
        let out = dir.join("out/tiny-linux");
        let report = game
            .build(&BuildOptions {
                out: Some(out.clone()),
                archive: Some(ArchiveFormat::TarGz),
                ..options(&dir)
            })
            .unwrap();
        let tgz = report.archive.unwrap();
        assert_eq!(tgz, dir.join("out/tiny-linux.tar.gz"));
        let mut names = Vec::new();
        let gz = flate2::read::GzDecoder::new(File::open(&tgz).unwrap());
        for entry in tar::Archive::new(gz).entries().unwrap() {
            names.push(entry.unwrap().path().unwrap().display().to_string());
        }
        assert!(names.contains(&"tiny/tiny".to_owned()), "{names:?}");
        assert!(names.contains(&"tiny/assets/scenes/main.scene.ron".to_owned()));
        assert!(names.iter().all(|n| !n.contains(MARKER)));

        let zip_path = game
            .build(&BuildOptions {
                out: Some(out),
                archive: Some(ArchiveFormat::Zip),
                ..options(&dir)
            })
            .unwrap()
            .archive
            .unwrap();
        let mut zip = zip::ZipArchive::new(File::open(zip_path).unwrap()).unwrap();
        let mut text = String::new();
        zip.by_name("tiny/assets/scenes/main.scene.ron")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "()");

        // A folder with someone's files in it is not ours to wipe.
        let mine = dir.join("mine");
        fs::create_dir_all(&mine).unwrap();
        fs::write(mine.join("precious.txt"), "x").unwrap();
        let err = game
            .build(&BuildOptions {
                out: Some(mine.clone()),
                ..options(&dir)
            })
            .unwrap_err();
        assert!(err.to_string().contains("not made by"), "{err}");
        assert!(mine.join("precious.txt").is_file());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn trailing_slash_out_and_linked_assets() {
        let dir = temp_dir("slash");
        let game = tiny_game(&dir);
        #[cfg(unix)]
        {
            fs::create_dir_all(dir.join("shared")).unwrap();
            fs::write(dir.join("shared/a.txt"), "a").unwrap();
            std::os::unix::fs::symlink(dir.join("shared"), game.root().join("assets/linked"))
                .unwrap();
        }
        let mut out = dir.join("out/tiny-win").into_os_string();
        out.push("/");
        let report = game
            .build(&BuildOptions {
                out: Some(PathBuf::from(out)),
                archive: Some(ArchiveFormat::Zip),
                ..options(&dir)
            })
            .unwrap();
        assert_eq!(report.archive.unwrap(), dir.join("out/tiny-win.zip"));
        #[cfg(unix)]
        assert!(report.out_dir.join("assets/linked/a.txt").is_file());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn broken_game_fails_with_cargo_error() {
        let dir = temp_dir("broken");
        let game = tiny_game(&dir);
        fs::write(game.root().join("src/main.rs"), "fn main() { nope }").unwrap();
        let err = game.build(&options(&dir)).unwrap_err();
        assert!(err.to_string().contains("cargo build failed"), "{err}");
        assert!(
            !game.dist_dir().exists(),
            "nothing is laid out after a failed build"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn archive_formats_parse() {
        assert_eq!(
            ArchiveFormat::parse("tar.gz").unwrap(),
            ArchiveFormat::TarGz
        );
        assert_eq!(ArchiveFormat::parse(".ZIP").unwrap(), ArchiveFormat::Zip);
        assert!(ArchiveFormat::parse("rar").is_err());
    }
}

//! `olvin-hub build`: builds a game and lays out a folder (or an archive) that
//! can be handed to players: the executable next to a copy of `assets/`.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use olvin_core::integrity::{self, MANIFEST_FILE, Manifest, PUBKEY_ENV, SIGNATURE_FILE, Signer};

use crate::Project;
use crate::signing::{Signing, resolve_signer};

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

/// The licence of the font the engine bakes into games with the `gui` feature.
const INTER_OFL: &str = include_str!("../../../assets/fonts/Inter-OFL.txt");
/// What the engine ships that is not Olvin's own.
const THIRD_PARTY: &str = include_str!("../../../THIRD_PARTY.md");

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
    /// Release only: also write the debug symbols to `<out's parent>/symbols/`
    /// (never into the shipped folder or archive), so crash addresses stay
    /// readable. Costs a slower build (line tables) and needs `objcopy` on Unix.
    pub keep_symbols: bool,
    /// Release only: leave the executable as cargo links it (no `strip`).
    pub no_strip: bool,
    /// Release only: keep real build paths in the binary (panic locations,
    /// debug info). By default they are rewritten, see [`remap_rules`].
    pub no_remap: bool,
    /// Release only: which key signs `manifest.json` (see [`Signing`]). Without
    /// a key the manifest is written unsigned and the game has no public key
    /// to check against.
    pub signing: Signing,
    /// `cargo build --locked`: the build fails instead of changing `Cargo.lock`
    /// (dependencies are exactly the audited ones). Needs a committed lock file.
    pub locked: bool,
}

#[derive(Debug)]
pub struct BuildReport {
    pub out_dir: PathBuf,
    pub binary: PathBuf,
    /// Files copied from `assets/`.
    pub assets: usize,
    /// Files in `licenses/` (third-party notices; 0 means no folder).
    pub licenses: usize,
    pub archive: Option<PathBuf>,
    /// Separate debug symbols of the shipped executable (`--keep-symbols`). Keep
    /// them private: they are not in `out_dir` and not in the archive.
    pub symbols: Option<PathBuf>,
    /// `manifest.json` (release builds): hashes of every shipped file.
    pub manifest: Option<PathBuf>,
    /// `manifest.json` has a signature and the game has the public key.
    pub signed: bool,
    /// The public key embedded in the game, hex.
    pub public_key: Option<String>,
    /// The build id (start of the executable's SHA-256): names symbol files and crash reports.
    pub build_id: Option<String>,
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
        if options.keep_symbols && !options.release {
            bail!("--keep-symbols needs --release: a debug build is not stripped anyway");
        }
        if options.keep_symbols && options.no_strip {
            bail!("--keep-symbols and --no-strip contradict each other");
        }
        let out = options.out.clone().unwrap_or_else(|| self.dist_dir());
        // Resolved before the long cargo build: a bad key fails now, not after minutes.
        let signer = if options.release {
            resolve_signer(&options.signing, &[self.root(), &out])?
        } else {
            None
        };
        let built = self.cargo_build(options, signer.as_ref())?;
        let exe = built.exe.clone();
        prepare_out(&out)?;
        let gitignore_updated = options.out.is_none() && ignore_dist(self.root())?;

        let binary = out.join(exe.file_name().context("the executable has no file name")?);
        fs::copy(&exe, &binary)
            .with_context(|| format!("could not copy {} to {}", exe.display(), binary.display()))?;
        // Always there, even empty: the engine recognizes a built folder by `assets/`
        // next to the executable (`ProjectPaths`), otherwise logs and saves would
        // land in whatever the working directory is.
        let assets = match self.root().join("assets") {
            dir if dir.is_dir() => copy_tree(&dir, &out.join("assets"))?,
            _ => {
                fs::create_dir_all(out.join("assets"))?;
                0
            }
        };
        let licenses = self.copy_licenses(&out)?;
        let symbols = options
            .keep_symbols
            .then(|| split_symbols(&built, &binary, &out, self.name()))
            .transpose()?;
        let manifest = options
            .release
            .then(|| write_manifest(&out, &binary, signer.as_ref()))
            .transpose()?;
        let archive = options
            .archive
            .map(|format| pack(&out, self.name(), format))
            .transpose()?;
        Ok(BuildReport {
            out_dir: out,
            binary,
            assets,
            licenses,
            archive,
            symbols,
            manifest: manifest.as_ref().map(|(path, _)| path.clone()),
            signed: signer.is_some(),
            public_key: signer.as_ref().map(|s| s.public_key().to_hex()),
            build_id: manifest.map(|(_, id)| id),
            gitignore_updated,
        })
    }

    /// Fills `<out>/licenses/`: the engine's own third-party notices when the
    /// game enables a feature that bakes in a font (`gui` pulls the built-in
    /// Inter, OFL 1.1, whose licence requires the text to travel with it), plus
    /// the project's own `licenses/` folder, if it has one. Returns the number
    /// of files, 0 (and no folder) when nothing is needed.
    fn copy_licenses(&self, out: &Path) -> Result<usize> {
        let dest = out.join("licenses");
        let mut count = 0;
        if self.olvin_features().iter().any(|f| f == "gui") {
            fs::create_dir_all(&dest)?;
            fs::write(dest.join("Inter-OFL.txt"), INTER_OFL)?;
            fs::write(dest.join("THIRD_PARTY.md"), THIRD_PARTY)?;
            count += 2;
        }
        let own = self.root().join("licenses");
        if own.is_dir() {
            count += copy_tree(&own, &dest)?;
        }
        Ok(count)
    }

    /// Runs `cargo build` and returns the game's executable (and the debug files next to it).
    fn cargo_build(&self, options: &BuildOptions, signer: Option<&Signer>) -> Result<Built> {
        let manifest = self.root().join("Cargo.toml");
        let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        cmd.args(["build", "--message-format=json-render-diagnostics"])
            .arg("--manifest-path")
            .arg(&manifest);
        if options.locked {
            cmd.arg("--locked");
        }
        if options.release {
            cmd.arg("--release");
            for (key, value) in release_profile(options) {
                cmd.env(key, value);
            }
            if !options.no_remap {
                let rules = remap_rules(
                    self.root(),
                    std::env::var_os("HOME").map(PathBuf::from),
                    cargo_home(),
                    rustc_sysroot(),
                );
                set_rustflags(&mut cmd, &remap_flags(&rules));
            }
        }
        // The game reads the key with `option_env!` (`integrity_plugin!`): only
        // a signed build has one, and a stray variable must not leak into an unsigned one.
        match signer {
            Some(signer) => cmd.env(PUBKEY_ENV, signer.public_key().to_hex()),
            None => cmd.env_remove(PUBKEY_ENV),
        };
        if let Some(dir) = &options.target_dir {
            cmd.arg("--target-dir").arg(dir);
        }
        let mut child = cmd
            .stdout(Stdio::piped())
            .spawn()
            .context("could not start cargo")?;
        let manifest = manifest.canonicalize().unwrap_or(manifest);
        let mut built = None;
        for line in BufReader::new(
            child
                .stdout
                .take()
                .context("cargo stdout is not captured")?,
        )
        .lines()
        {
            if let Some(artifact) = artifact_of(&line?, &manifest) {
                built = Some(artifact);
            }
        }
        let status = child.wait()?;
        if !status.success() {
            bail!("cargo build failed ({status})");
        }
        built.context("cargo built no executable: does the project have a binary target?")
    }
}

/// Where cargo keeps the registry sources: `$CARGO_HOME`, else `~/.cargo`.
fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
}

/// The active toolchain's root (`rustc --print sysroot`); `None` if rustc does not answer.
fn rustc_sysroot() -> Option<PathBuf> {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let path = text.trim();
    (output.status.success() && !path.is_empty()).then(|| PathBuf::from(path))
}

/// Path prefixes to hide from a release binary, with what replaces them: the
/// home folder (`~`), cargo's home (`/cargo`), the toolchain (`/rustc-sysroot`)
/// and the project (`/project`). Where they are nested, the more specific one
/// comes later: rustc applies the last rule that matches. A path that is only
/// reachable through a symlink is listed in both spellings, and a root-like
/// path (`/`) is never remapped, since it would match everything.
fn remap_rules(
    project: &Path,
    home: Option<PathBuf>,
    cargo_home: Option<PathBuf>,
    sysroot: Option<PathBuf>,
) -> Vec<(PathBuf, &'static str)> {
    let wanted = [
        (home, "~"),
        (cargo_home, "/cargo"),
        (sysroot, "/rustc-sysroot"),
        (Some(project.to_path_buf()), "/project"),
    ];
    let mut rules: Vec<(PathBuf, &'static str)> = Vec::new();
    for (path, to) in wanted {
        let Some(path) = path else { continue };
        let canonical = path.canonicalize().ok();
        for from in std::iter::once(path).chain(canonical) {
            let rooted = from.components().count() < 2;
            if !rooted && !rules.iter().any(|(f, _)| *f == from) {
                rules.push((from, to));
            }
        }
    }
    rules
}

/// `--remap-path-prefix` flags for the rules (paths that are not UTF-8 are skipped).
fn remap_flags(rules: &[(PathBuf, &str)]) -> Vec<String> {
    rules
        .iter()
        .filter_map(|(from, to)| Some(format!("--remap-path-prefix={}={to}", from.to_str()?)))
        .collect()
}

/// Adds `flags` to the build's rustflags without losing the user's own. When
/// `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS` is set it would shadow every other
/// source, so ours are appended to that value; otherwise they go in through
/// `--config build.rustflags`, which cargo joins with the ones from
/// `.cargo/config.toml`. (A `target.<triple>.rustflags` setting still takes
/// precedence over both, which is why the CI check looks at the binary.)
fn set_rustflags(cmd: &mut Command, flags: &[String]) {
    let existing = std::env::var("CARGO_ENCODED_RUSTFLAGS")
        .map(|v| v.split('\x1f').map(str::to_owned).collect::<Vec<_>>())
        .or_else(|_| {
            std::env::var("RUSTFLAGS")
                .map(|v| v.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
        });
    match existing {
        Ok(mut all) => {
            all.extend(flags.iter().cloned());
            cmd.env_remove("RUSTFLAGS")
                .env("CARGO_ENCODED_RUSTFLAGS", all.join("\x1f"));
        }
        Err(_) => {
            let array =
                toml::Value::Array(flags.iter().cloned().map(toml::Value::String).collect());
            cmd.arg("--config").arg(format!("build.rustflags={array}"));
        }
    }
}

/// What cargo made for the game's binary target.
struct Built {
    exe: PathBuf,
    /// Debug files cargo put next to it (`.pdb` on Windows).
    pdb: Option<PathBuf>,
}

/// Profile settings the hub forces on a release build, as cargo env overrides
/// (`CARGO_PROFILE_RELEASE_*` beat the project's `Cargo.toml`).
///
/// Stripped by default: symbol names make a binary far easier to read. With
/// `--keep-symbols` cargo links an unstripped binary with line tables and the
/// hub splits it afterwards (see [`split_symbols`]).
fn release_profile(options: &BuildOptions) -> Vec<(&'static str, &'static str)> {
    if options.keep_symbols {
        vec![
            ("CARGO_PROFILE_RELEASE_STRIP", "none"),
            ("CARGO_PROFILE_RELEASE_DEBUG", "line-tables-only"),
        ]
    } else if options.no_strip {
        Vec::new()
    } else {
        vec![("CARGO_PROFILE_RELEASE_STRIP", "symbols")]
    }
}

/// The game's executable from one line of cargo's JSON output.
fn artifact_of(line: &str, manifest: &Path) -> Option<Built> {
    let msg: serde_json::Value = serde_json::from_str(line).ok()?;
    if msg["reason"] != "compiler-artifact" {
        return None;
    }
    let is_bin = msg["target"]["kind"].as_array()?.iter().any(|k| k == "bin");
    let same = Path::new(msg["manifest_path"].as_str()?)
        .canonicalize()
        .is_ok_and(|p| p == manifest);
    if !(is_bin && same) {
        return None;
    }
    let exe = PathBuf::from(msg["executable"].as_str().unwrap_or_default());
    let pdb = msg["filenames"]
        .as_array()?
        .iter()
        .filter_map(|f| f.as_str())
        .find(|f| f.ends_with(".pdb"))
        .map(PathBuf::from);
    (!exe.as_os_str().is_empty()).then_some(Built { exe, pdb })
}

/// Moves the debug information of a release build out of the shipped
/// executable `binary` (already copied into `out`) into
/// `<out's parent>/symbols/<name>-<build id>.debug`, where the build id is the
/// start of the stripped executable's SHA-256. The symbols folder is a sibling
/// of `out`, so neither the folder nor an archive of it contains them.
///
/// Unix: GNU `objcopy` (or `llvm-objcopy`; `$OBJCOPY` overrides). Windows: the
/// `.pdb` cargo wrote is copied (the executable has no symbols in it). Other
/// platforms are not supported yet and fail loudly rather than ship symbols.
fn split_symbols(built: &Built, binary: &Path, out: &Path, name: &str) -> Result<PathBuf> {
    let dir = out
        .parent()
        .context("the output folder has no parent for symbols/")?
        .join("symbols");
    fs::create_dir_all(&dir)?;
    let partial = dir.join(format!(".{name}.partial"));
    if cfg!(windows) {
        let pdb = built
            .pdb
            .as_ref()
            .context("cargo reported no .pdb: cannot keep symbols")?;
        fs::copy(pdb, &partial)?;
    } else if cfg!(unix) && !cfg!(target_os = "macos") {
        let objcopy = objcopy().context(
            "--keep-symbols needs objcopy (binutils) or llvm-objcopy on PATH, or $OBJCOPY",
        )?;
        run_tool(
            Command::new(&objcopy)
                .arg("--only-keep-debug")
                .arg(&built.exe)
                .arg(&partial),
        )?;
        run_tool(Command::new(&objcopy).arg("--strip-all").arg(binary))?;
    } else {
        bail!("--keep-symbols is not supported on this platform yet");
    }
    let id = build_id(binary)?;
    let ext = if cfg!(windows) { "pdb" } else { "debug" };
    let path = dir.join(format!("{name}-{id}.{ext}"));
    fs::rename(&partial, &path)?;
    Ok(path)
}

fn objcopy() -> Option<PathBuf> {
    if let Some(custom) = std::env::var_os("OBJCOPY") {
        return Some(PathBuf::from(custom));
    }
    ["objcopy", "llvm-objcopy"]
        .into_iter()
        .find(|tool| {
            Command::new(tool)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
        .map(PathBuf::from)
}

fn run_tool(cmd: &mut Command) -> Result<()> {
    let output = cmd.output().context("could not start the strip tool")?;
    if !output.status.success() {
        bail!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// First 16 hex digits of the file's SHA-256: names symbol files so a crash
/// report from a shipped build finds the symbols of exactly that build.
fn build_id(path: &Path) -> Result<String> {
    Ok(integrity::sha256_file(path)?[..16].to_owned())
}

/// Writes `manifest.json` (and `manifest.sig` with a `signer`) into the game
/// folder; returns the manifest's path and the build id.
fn write_manifest(out: &Path, binary: &Path, signer: Option<&Signer>) -> Result<(PathBuf, String)> {
    let exe = binary
        .file_name()
        .and_then(|n| n.to_str())
        .context("the executable's name is not UTF-8")?;
    let manifest = Manifest::build(out, exe, env!("CARGO_PKG_VERSION"))
        .context("could not hash the files of the build")?;
    let json = manifest.to_json();
    let path = out.join(MANIFEST_FILE);
    fs::write(&path, &json)?;
    if let Some(signer) = signer {
        fs::write(
            out.join(SIGNATURE_FILE),
            format!("{}\n", signer.sign_hex(json.as_bytes())),
        )?;
    }
    Ok((path, manifest.build_id))
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
            "[package]\nname = \"olvin\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[features]\ngui = []\n",
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
            // Never pick up a real key from the developer's config folder.
            signing: Signing::Off,
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
        assert!(report.out_dir.join("assets").is_dir());
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

    /// A project without `assets/` still gets the folder in its build, or the
    /// engine would not recognize the build folder as the project root.
    #[test]
    fn build_without_assets_still_has_the_folder() {
        let dir = temp_dir("noassets");
        let game = tiny_game(&dir);
        fs::remove_dir_all(game.root().join("assets")).unwrap();
        let report = game.build(&options(&dir)).unwrap();
        assert_eq!(report.assets, 0);
        assert!(report.out_dir.join("assets").is_dir());
        fs::remove_dir_all(dir).unwrap();
    }

    /// `gui` bakes Inter into the game: its OFL text must travel with the build.
    #[test]
    fn gui_games_get_the_font_licence() {
        let dir = temp_dir("licenses");
        let game = tiny_game(&dir);
        let plain = game.build(&options(&dir)).unwrap();
        assert_eq!(plain.licenses, 0);
        assert!(!plain.out_dir.join("licenses").exists());

        let manifest = game.root().join("Cargo.toml");
        let text = fs::read_to_string(&manifest).unwrap().replace(
            "olvin = { path = \"olvin\" }",
            "olvin = { path = \"olvin\", features = [\"gui\"] }",
        );
        fs::write(&manifest, text).unwrap();
        fs::create_dir_all(game.root().join("licenses")).unwrap();
        fs::write(game.root().join("licenses/MyFont-OFL.txt"), "mine").unwrap();
        let game = Project::open(game.root()).unwrap();
        let report = game.build(&options(&dir)).unwrap();
        assert_eq!(report.licenses, 3);
        let licenses = report.out_dir.join("licenses");
        let inter = fs::read_to_string(licenses.join("Inter-OFL.txt")).unwrap();
        assert!(inter.contains("SIL OPEN FONT LICENSE Version 1.1"));
        assert!(licenses.join("THIRD_PARTY.md").is_file());
        assert!(licenses.join("MyFont-OFL.txt").is_file());
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

    /// A tiny game with a function whose symbol name is easy to look for.
    fn probe_game(dir: &Path) -> Project {
        let game = tiny_game(dir);
        fs::write(
            game.root().join("src/main.rs"),
            "use olvin as _;\n#[inline(never)]\nfn olvin_probe_fn(x: usize) -> usize { x.wrapping_mul(31) }\n\
             fn main() { println!(\"tiny\"); if std::env::args().count() > 99 { println!(\"{}\", olvin_probe_fn(std::env::args().count())); } }\n",
        )
        .unwrap();
        game
    }

    fn contains(path: &Path, needle: &str) -> bool {
        let bytes = fs::read(path).unwrap();
        bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    /// Release builds are stripped by default; `no_strip` leaves the names in.
    #[test]
    fn release_strips_symbols_unless_asked_not_to() {
        let dir = temp_dir("strip");
        let game = probe_game(&dir);
        let stripped = game
            .build(&BuildOptions {
                release: true,
                ..options(&dir)
            })
            .unwrap();
        assert!(!contains(&stripped.binary, "olvin_probe_fn"));
        assert!(stripped.symbols.is_none());
        let kept = game
            .build(&BuildOptions {
                release: true,
                no_strip: true,
                ..options(&dir)
            })
            .unwrap();
        assert!(contains(&kept.binary, "olvin_probe_fn"));
        fs::remove_dir_all(dir).unwrap();
    }

    /// `keep_symbols`: the shipped binary is stripped, the symbols sit beside
    /// the output folder, and an archive of the folder does not hold them.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn keep_symbols_goes_beside_the_folder_not_into_it() {
        let dir = temp_dir("symbols");
        let game = probe_game(&dir);
        let out = dir.join("out/tiny-linux");
        let report = game
            .build(&BuildOptions {
                release: true,
                keep_symbols: true,
                out: Some(out.clone()),
                archive: Some(ArchiveFormat::TarGz),
                ..options(&dir)
            })
            .unwrap();
        let symbols = report.symbols.unwrap();
        assert!(symbols.starts_with(dir.join("out/symbols")), "{symbols:?}");
        assert!(!symbols.starts_with(&out));
        let name = symbols.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("tiny-") && name.ends_with(".debug"),
            "{name}"
        );
        assert!(
            contains(&symbols, "olvin_probe_fn"),
            "names are in the symbols"
        );
        assert!(
            !contains(&report.binary, "olvin_probe_fn"),
            "and not in the game"
        );
        assert!(
            fs::metadata(&symbols).unwrap().len() > fs::metadata(&report.binary).unwrap().len()
        );
        let run = Command::new(&report.binary).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "tiny");
        let gz = flate2::read::GzDecoder::new(File::open(report.archive.unwrap()).unwrap());
        for entry in tar::Archive::new(gz).entries().unwrap() {
            let path = entry.unwrap().path().unwrap().display().to_string();
            assert!(
                !path.contains("symbols") && !path.ends_with(".debug"),
                "{path}"
            );
        }

        // Contradicting or pointless flags are refused before cargo runs.
        let err = game
            .build(&BuildOptions {
                keep_symbols: true,
                ..options(&dir)
            })
            .unwrap_err();
        assert!(err.to_string().contains("--release"), "{err}");
        let err = game
            .build(&BuildOptions {
                release: true,
                keep_symbols: true,
                no_strip: true,
                ..options(&dir)
            })
            .unwrap_err();
        assert!(err.to_string().contains("contradict"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }

    fn run_stdout(binary: &Path) -> String {
        let run = Command::new(binary).output().unwrap();
        String::from_utf8_lossy(&run.stdout).trim().to_owned()
    }

    /// B2: a release build with a key embeds the public key in the game, writes a
    /// signed manifest that verifies, and the archive carries it but not the key.
    #[test]
    fn signed_release_has_a_manifest_that_verifies_and_the_key_stays_out() {
        let dir = temp_dir("signed");
        let game = tiny_game(&dir);
        fs::write(
            game.root().join("src/main.rs"),
            "use olvin as _;\nfn main() { println!(\"{}\", option_env!(\"OLVIN_PUBKEY\").unwrap_or(\"none\")); }\n",
        )
        .unwrap();
        let key = dir.join("keys/release.key");
        let public = crate::signing::generate_key(&key, false, &[game.root()]).unwrap();

        let unsigned = game
            .build(&BuildOptions {
                release: true,
                ..options(&dir)
            })
            .unwrap();
        assert!(!unsigned.signed && unsigned.public_key.is_none());
        assert!(unsigned.manifest.unwrap().is_file());
        assert!(!unsigned.out_dir.join(SIGNATURE_FILE).exists());
        assert_eq!(run_stdout(&unsigned.binary), "none");

        let out = dir.join("out/tiny");
        let report = game
            .build(&BuildOptions {
                release: true,
                signing: Signing::Key(key.clone()),
                out: Some(out.clone()),
                archive: Some(ArchiveFormat::TarGz),
                ..options(&dir)
            })
            .unwrap();
        assert!(report.signed);
        assert_eq!(report.public_key.as_deref(), Some(public.to_hex().as_str()));
        assert_eq!(
            run_stdout(&report.binary),
            public.to_hex(),
            "the game has the key"
        );
        let verified = integrity::verify_dir(&out, Some(&public));
        assert!(verified.ok(), "{:?}", verified.problems);
        assert_eq!(verified.signature, integrity::SignatureState::Valid);
        assert_eq!(verified.build_id, report.build_id);
        // icon.png, scenes/main.scene.ron and the executable.
        assert_eq!(verified.files_checked, 3);

        let mut names = Vec::new();
        let gz = flate2::read::GzDecoder::new(File::open(report.archive.unwrap()).unwrap());
        for entry in tar::Archive::new(gz).entries().unwrap() {
            names.push(entry.unwrap().path().unwrap().display().to_string());
        }
        assert!(
            names.contains(&"tiny/manifest.json".to_owned()),
            "{names:?}"
        );
        assert!(names.contains(&"tiny/manifest.sig".to_owned()));
        assert!(
            names
                .iter()
                .all(|n| !n.contains(".key") && !n.contains("keys"))
        );

        // A patched asset in the shipped folder is found.
        fs::write(out.join("assets/icon.png"), [9, 9, 9]).unwrap();
        let tampered = integrity::verify_dir(&out, Some(&public));
        assert_eq!(
            tampered.problems,
            [integrity::Problem::Modified("assets/icon.png".into())]
        );

        // A key inside the project is refused before cargo even starts.
        let inside = game.root().join("release.key");
        fs::copy(&key, &inside).unwrap();
        let err = game
            .build(&BuildOptions {
                release: true,
                signing: Signing::Key(inside),
                ..options(&dir)
            })
            .unwrap_err();
        assert!(err.to_string().contains("inside"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn remap_rules_put_specific_paths_last() {
        let rules = remap_rules(
            Path::new("/home/alice/dev/game"),
            Some("/home/alice".into()),
            Some("/home/alice/.cargo".into()),
            Some("/home/alice/.rustup/toolchains/stable".into()),
        );
        let flags = remap_flags(&rules);
        assert_eq!(
            flags,
            [
                "--remap-path-prefix=/home/alice=~",
                "--remap-path-prefix=/home/alice/.cargo=/cargo",
                "--remap-path-prefix=/home/alice/.rustup/toolchains/stable=/rustc-sysroot",
                "--remap-path-prefix=/home/alice/dev/game=/project",
            ]
        );
        // `/` as a home (a container) must not rewrite every path.
        let rules = remap_rules(Path::new("/srv/game"), Some("/".into()), None, None);
        assert_eq!(rules, [(PathBuf::from("/srv/game"), "/project")]);
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

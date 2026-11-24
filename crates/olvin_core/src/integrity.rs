//! Signed manifest and start-up integrity check (roadmap B2, E2).
//!
//! `olvin-hub build --release` writes two files next to the game:
//!
//! * `manifest.json`: the path, size and SHA-256 of every shipped file (the
//!   executable included), the engine version and a build id;
//! * `manifest.sig`: an Ed25519 signature (hex) over the exact bytes of
//!   `manifest.json`, made with the developer's release key (`olvin-hub keygen`).
//!
//! The matching public key is compiled into the game (`OLVIN_PUBKEY`, set by the
//! hub for the cargo build; see [`integrity_plugin!`]). [`IntegrityPlugin`]
//! checks the signature and every hash at start and then, per [`Policy`], stops
//! the game or only logs.
//!
//! # What this gives, and what it does not
//!
//! * **Tampered download or copy.** A player, an installer or a launcher that
//!   runs the same check from outside the game (`olvin-hub verify DIR --pubkey
//!   KEY`) finds changed, missing or added files, and cannot be fooled by
//!   someone without the private key. This is real protection, *provided the
//!   verifier is trusted and holds the right public key*.
//! * **Casual tampering of assets and scripts.** Editing a scene, a script or a
//!   shader in a shipped folder makes the in-game check fail. A signal and a
//!   deterrent, nothing more.
//! * **Not protection against a patched game.** The check is code inside the
//!   game, and so is the public key. Someone who edits the executable can
//!   delete the check or swap the key (the manifest then simply gets re-signed
//!   with their own key). A patched binary does not carry a valid signature
//!   *from you*, which an outside verifier will say; the game cannot say it
//!   about itself. Also, an attacker on the developer's machine owns the key.
//!
//! The check streams every file through SHA-256 (the hardware SHA extensions
//! are used where present) on a few threads, so it costs about the time to read
//! the shipped files once. Numbers for the vertical slice are in
//! `docs/adr/0002-build-security.md`.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::secret::SecretBytes;
use crate::{App, AppExit, PathsSource, Plugin, ProjectPaths};

/// The manifest, next to the executable.
pub const MANIFEST_FILE: &str = "manifest.json";
/// Its signature: 64 bytes, hex.
pub const SIGNATURE_FILE: &str = "manifest.sig";
/// Environment variable the hub sets for the cargo build: the public key, hex.
pub const PUBKEY_ENV: &str = "OLVIN_PUBKEY";
/// The manifest and its signature are not part of what they describe.
const IGNORED_TOP_LEVEL: [&str; 2] = [MANIFEST_FILE, SIGNATURE_FILE];
/// Only this folder is checked for files the manifest does not list.
const ASSETS_DIR: &str = "assets";

/// `manifest.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Layout of this file; 1.
    pub format: u32,
    /// Version of the hub (engine) that made the build.
    pub engine_version: String,
    /// First 16 hex digits of the executable's SHA-256; names the symbol file too.
    pub build_id: String,
    /// Sorted by path.
    pub files: Vec<FileEntry>,
}

/// One shipped file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative to the game folder, `/` separated.
    pub path: String,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
}

impl Manifest {
    /// Hashes every non-hidden file under `dir` except the manifest and its
    /// signature. `exe` names the executable inside `dir` (its hash becomes the
    /// build id).
    pub fn build(dir: &Path, exe: &str, engine_version: &str) -> io::Result<Self> {
        let mut paths = Vec::new();
        list_files(dir, Path::new(""), &mut paths)?;
        paths.retain(|p| !IGNORED_TOP_LEVEL.contains(&p.as_str()));
        paths.sort();
        let files = hash_files(dir, &paths)?;
        let build_id = files
            .iter()
            .find(|f| f.path == exe)
            .map(|f| f.sha256[..16].to_owned())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{exe} is not in the folder"),
                )
            })?;
        Ok(Self {
            format: 1,
            engine_version: engine_version.to_owned(),
            build_id,
            files,
        })
    }

    /// The exact bytes that get signed and written.
    pub fn to_json(&self) -> String {
        // invariant: plain strings and numbers always serialize.
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("manifest is damaged: {e}"))?;
        if manifest.format != 1 {
            return Err(format!("unknown manifest format {}", manifest.format));
        }
        Ok(manifest)
    }
}

/// A key that verifies manifests.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PublicKey(VerifyingKey);

impl PublicKey {
    /// 64 hex digits.
    pub fn from_hex(text: &str) -> Result<Self, String> {
        let bytes: [u8; 32] = decode_hex(text.trim())
            .and_then(|v| v.try_into().ok())
            .ok_or("a public key is 64 hex digits")?;
        VerifyingKey::from_bytes(&bytes)
            .map(Self)
            .map_err(|e| format!("not a valid public key: {e}"))
    }

    pub fn to_hex(&self) -> String {
        encode_hex(self.0.as_bytes())
    }

    /// Whether `signature_hex` is this key's signature of exactly `message`.
    pub fn verify(&self, message: &[u8], signature_hex: &str) -> bool {
        let Some(bytes) = decode_hex(signature_hex.trim()) else {
            return false;
        };
        let Ok(bytes) = <[u8; 64]>::try_from(bytes) else {
            return false;
        };
        self.0
            .verify_strict(message, &Signature::from_bytes(&bytes))
            .is_ok()
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.to_hex())
    }
}

/// A signing key. The 32-byte seed is wiped when this is dropped (and the
/// dalek key zeroizes itself); it never prints.
pub struct Signer(SigningKey);

impl Signer {
    /// From the 32-byte seed. Wrong length: `None`.
    pub fn from_seed(seed: &SecretBytes) -> Option<Self> {
        let bytes: &[u8; 32] = seed.expose().try_into().ok()?;
        Some(Self(SigningKey::from_bytes(bytes)))
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.0.verifying_key())
    }

    /// Signature of `message`, hex.
    pub fn sign_hex(&self, message: &[u8]) -> String {
        encode_hex(&self.0.sign(message).to_bytes())
    }
}

impl fmt::Debug for Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Signer([hidden])")
    }
}

/// What a failed check does to the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Policy {
    /// Log the problems and stop the game (the default).
    #[default]
    Enforce,
    /// Log the problems and run on.
    Warn,
}

/// One thing that is wrong with a game folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// No `manifest.json` where the executable is.
    ManifestMissing,
    /// The manifest cannot be read as one (the text says why).
    ManifestInvalid(String),
    /// A public key is embedded but there is no `manifest.sig`.
    SignatureMissing,
    /// `manifest.sig` is not this key's signature of `manifest.json`.
    SignatureInvalid,
    /// The key given to the check is not a key.
    PublicKeyInvalid(String),
    /// Listed in the manifest, not on disk.
    Missing(String),
    /// Size or hash differs from the manifest.
    Modified(String),
    /// A file in `assets/` the manifest does not list.
    Extra(String),
    /// The manifest lists a path that leaves the folder.
    UnsafePath(String),
    /// A file could not be read.
    Unreadable(String, String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManifestMissing => write!(f, "{MANIFEST_FILE} is missing"),
            Self::ManifestInvalid(why) => f.write_str(why),
            Self::SignatureMissing => write!(f, "{SIGNATURE_FILE} is missing"),
            Self::SignatureInvalid => write!(f, "the signature does not match {MANIFEST_FILE}"),
            Self::PublicKeyInvalid(why) => write!(f, "bad public key: {why}"),
            Self::Missing(path) => write!(f, "missing: {path}"),
            Self::Modified(path) => write!(f, "modified: {path}"),
            Self::Extra(path) => write!(f, "not in the manifest: {path}"),
            Self::UnsafePath(path) => write!(f, "manifest path leaves the folder: {path}"),
            Self::Unreadable(path, why) => write!(f, "cannot read {path}: {why}"),
        }
    }
}

/// How the signature stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureState {
    /// Checked against the public key and valid.
    Valid,
    /// No public key to check with: only the hashes were compared.
    NotChecked,
    /// Checked and not valid (or missing); see the problems.
    Failed,
}

/// The outcome of a check; the game keeps it as a resource.
#[derive(Clone, Debug)]
pub struct IntegrityReport {
    pub signature: SignatureState,
    pub build_id: Option<String>,
    /// Files compared with the manifest.
    pub files_checked: usize,
    /// Bytes read for that.
    pub bytes_hashed: u64,
    pub problems: Vec<Problem>,
    pub elapsed: Duration,
    /// The check did not run (a development run: no key embedded, no manifest).
    pub skipped: Option<&'static str>,
}

impl IntegrityReport {
    /// Nothing is wrong (a skipped check is not a failure, but it is not a pass either: see `skipped`).
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }

    fn failed(problem: Problem) -> Self {
        Self {
            signature: SignatureState::Failed,
            problems: vec![problem],
            ..Self::skipped("")
        }
        .not_skipped()
    }

    fn not_skipped(mut self) -> Self {
        self.skipped = None;
        self
    }

    fn skipped(why: &'static str) -> Self {
        Self {
            signature: SignatureState::NotChecked,
            build_id: None,
            files_checked: 0,
            bytes_hashed: 0,
            problems: Vec::new(),
            elapsed: Duration::ZERO,
            skipped: Some(why),
        }
    }
}

/// Checks the game folder `dir` against its manifest. With `key`, the manifest
/// signature must be valid too; without one, only the hashes are compared (and
/// the report says the signature was not checked).
pub fn verify_dir(dir: &Path, key: Option<&PublicKey>) -> IntegrityReport {
    let started = Instant::now();
    let mut report = IntegrityReport {
        signature: SignatureState::NotChecked,
        build_id: None,
        files_checked: 0,
        bytes_hashed: 0,
        problems: Vec::new(),
        elapsed: Duration::ZERO,
        skipped: None,
    };
    let finish = |mut report: IntegrityReport| {
        report.elapsed = started.elapsed();
        report
    };
    let Ok(text) = fs::read(dir.join(MANIFEST_FILE)) else {
        report.problems.push(Problem::ManifestMissing);
        if key.is_some() {
            report.signature = SignatureState::Failed;
        }
        return finish(report);
    };
    if let Some(key) = key {
        match fs::read_to_string(dir.join(SIGNATURE_FILE)) {
            Err(_) => {
                report.signature = SignatureState::Failed;
                report.problems.push(Problem::SignatureMissing);
            }
            Ok(sig) if key.verify(&text, &sig) => report.signature = SignatureState::Valid,
            Ok(_) => {
                report.signature = SignatureState::Failed;
                report.problems.push(Problem::SignatureInvalid);
            }
        }
        // A manifest whose signature failed is not trusted for anything else.
        if report.signature == SignatureState::Failed {
            return finish(report);
        }
    }
    let manifest = match Manifest::parse(&text) {
        Ok(manifest) => manifest,
        Err(why) => {
            report.problems.push(Problem::ManifestInvalid(why));
            return finish(report);
        }
    };
    report.build_id = Some(manifest.build_id.clone());

    let mut wanted = Vec::new();
    for entry in &manifest.files {
        if is_relative_safe(&entry.path) {
            wanted.push(entry);
        } else {
            report
                .problems
                .push(Problem::UnsafePath(entry.path.clone()));
        }
    }
    let results = parallel_map(&wanted, |entry| check_entry(dir, entry));
    for result in results {
        report.files_checked += 1;
        report.bytes_hashed += result.bytes;
        report.problems.extend(result.problem);
    }

    let mut on_disk = Vec::new();
    let _ = list_files(&dir.join(ASSETS_DIR), Path::new(ASSETS_DIR), &mut on_disk);
    for path in on_disk {
        if !manifest.files.iter().any(|f| f.path == path) {
            report.problems.push(Problem::Extra(path));
        }
    }
    finish(report)
}

struct EntryResult {
    bytes: u64,
    problem: Option<Problem>,
}

fn check_entry(dir: &Path, entry: &FileEntry) -> EntryResult {
    let path = dir.join(&entry.path);
    let meta = match fs::metadata(&path) {
        Ok(meta) if meta.is_file() => meta,
        Ok(_) => {
            return EntryResult {
                bytes: 0,
                problem: Some(Problem::Modified(entry.path.clone())),
            };
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return EntryResult {
                bytes: 0,
                problem: Some(Problem::Missing(entry.path.clone())),
            };
        }
        Err(e) => {
            return EntryResult {
                bytes: 0,
                problem: Some(Problem::Unreadable(entry.path.clone(), e.to_string())),
            };
        }
    };
    if meta.len() != entry.size {
        return EntryResult {
            bytes: 0,
            problem: Some(Problem::Modified(entry.path.clone())),
        };
    }
    match sha256_file(&path) {
        Ok(hash) if hash == entry.sha256 => EntryResult {
            bytes: meta.len(),
            problem: None,
        },
        Ok(_) => EntryResult {
            bytes: meta.len(),
            problem: Some(Problem::Modified(entry.path.clone())),
        },
        Err(e) => EntryResult {
            bytes: 0,
            problem: Some(Problem::Unreadable(entry.path.clone(), e.to_string())),
        },
    }
}

/// A manifest path must stay inside the folder: relative, no `..`, no root.
fn is_relative_safe(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Streamed SHA-256 of a file, lower-case hex.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(encode_hex(&hasher.finalize()))
}

fn hash_files(dir: &Path, paths: &[String]) -> io::Result<Vec<FileEntry>> {
    let results = parallel_map(paths, |path| -> io::Result<FileEntry> {
        let full = dir.join(path);
        Ok(FileEntry {
            path: path.clone(),
            size: fs::metadata(&full)?.len(),
            sha256: sha256_file(&full)?,
        })
    });
    results.into_iter().collect()
}

/// `f` over `items` on up to four threads, results in the order of `items`.
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(4)
        .min(items.len());
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let parts: Vec<Vec<(usize, R)>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        done.push((i, f(item)));
                    }
                    done
                })
            })
            .collect();
        workers.into_iter().filter_map(|w| w.join().ok()).collect()
    });
    let mut all: Vec<(usize, R)> = parts.into_iter().flatten().collect();
    all.sort_by_key(|(i, _)| *i);
    all.into_iter().map(|(_, r)| r).collect()
}

/// Relative `/`-separated paths of the non-hidden files under `dir`.
fn list_files(dir: &Path, prefix: &Path, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name
            .to_str()
            .is_none_or(|n| n.starts_with('.') || n.ends_with('~'))
        {
            continue;
        }
        let inner = prefix.join(&name);
        if entry.path().is_dir() {
            list_files(&entry.path(), &inner, out)?;
        } else if let Some(text) = inner.to_str() {
            out.push(text.replace('\\', "/"));
        }
    }
    Ok(())
}

/// Bytes as lower-case hex.
pub fn encode_hex(bytes: &[u8]) -> String {
    use fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Hex text to bytes; `None` on odd length or a non-hex digit.
pub fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// Checks the game's files at start. Add it first, before the plugins that open
/// a window or load assets.
///
/// With no public key (a development build) it does nothing and says so in the
/// log. With one, it expects `manifest.json` and `manifest.sig` next to the
/// executable and applies the [`Policy`] to whatever is wrong. The result is
/// the [`IntegrityReport`] resource.
pub struct IntegrityPlugin {
    key: Result<Option<PublicKey>, String>,
    policy: Policy,
    dir: Option<PathBuf>,
}

impl IntegrityPlugin {
    /// `key`: the public key, hex (`option_env!("OLVIN_PUBKEY")` in the game;
    /// see [`integrity_plugin!`]). `None` makes the plugin inert.
    pub fn new(key: Option<&str>) -> Self {
        Self {
            key: match key {
                None => Ok(None),
                Some(hex) => PublicKey::from_hex(hex).map(Some),
            },
            policy: Policy::Enforce,
            dir: None,
        }
    }

    pub fn policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }

    /// Checks this folder instead of the executable's (tests).
    pub fn dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    fn run(&self, app: &App) -> IntegrityReport {
        let key = match &self.key {
            Ok(None) => return IntegrityReport::skipped("no public key in this build"),
            Ok(Some(key)) => key,
            Err(why) => return IntegrityReport::failed(Problem::PublicKeyInvalid(why.clone())),
        };
        let dir = self.dir.clone().or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Path::to_path_buf))
        });
        let Some(dir) = dir else {
            return IntegrityReport::skipped("the executable's folder is unknown");
        };
        // `cargo run --release` of a project: signed key in the env, no manifest.
        let packaged = app
            .resources
            .get::<ProjectPaths>()
            .is_some_and(|p| p.source == PathsSource::Executable);
        if self.dir.is_none() && !packaged && !dir.join(MANIFEST_FILE).exists() {
            return IntegrityReport::skipped("not a packaged build (run through cargo)");
        }
        verify_dir(&dir, Some(key))
    }
}

impl Plugin for IntegrityPlugin {
    fn build(&self, app: &mut App) {
        let report = self.run(app);
        match report.skipped {
            Some(why) => log::info!("integrity check skipped: {why}"),
            None if report.ok() => log::info!(
                "integrity: {} files, {} KiB, signature {:?}, {:.1} ms, build {}",
                report.files_checked,
                report.bytes_hashed / 1024,
                report.signature,
                report.elapsed.as_secs_f64() * 1000.0,
                report.build_id.as_deref().unwrap_or("?")
            ),
            None => {
                let level = match self.policy {
                    Policy::Enforce => log::Level::Error,
                    Policy::Warn => log::Level::Warn,
                };
                log::log!(
                    level,
                    "integrity check FAILED ({} problems)",
                    report.problems.len()
                );
                for problem in report.problems.iter().take(20) {
                    log::log!(level, "integrity: {problem}");
                }
                if self.policy == Policy::Enforce {
                    eprintln!(
                        "olvin: the game files do not match their signed manifest ({}). \
                         Reinstall the game from its official source.",
                        report
                            .problems
                            .first()
                            .map_or_else(String::new, ToString::to_string)
                    );
                    app.resources.fetch_mut::<AppExit>().requested = true;
                }
            }
        }
        app.insert_resource(report);
    }
}

/// `IntegrityPlugin::new(option_env!("OLVIN_PUBKEY"))`, expanded in *your*
/// crate: `option_env!` must be read where the game is compiled, so that only
/// the game recompiles when the key changes, not the engine.
#[macro_export]
macro_rules! integrity_plugin {
    () => {
        $crate::integrity::IntegrityPlugin::new(option_env!("OLVIN_PUBKEY"))
    };
}

#[cfg(test)]
mod tests;

//! Release signing keys (roadmap B2, E3): `olvin-hub keygen`, loading a key for
//! `olvin-hub build --release`, reading a public key for `olvin-hub verify`.
//!
//! The private key is a 32-byte Ed25519 seed in a small text file
//! (`olvin-ed25519-secret:<64 hex digits>`), created with mode 0600 and refused
//! on load when other users can read it. It never lives inside the project or
//! the build folder: a build refuses a key found there. In memory it is held
//! in [`SecretString`]/[`SecretBytes`] and wiped on drop.
//!
//! For CI the key can come from the environment instead of a file:
//! `OLVIN_SIGNING_KEY` is a path, `OLVIN_SIGNING_KEY_DATA` the file's content
//! (a CI secret). Mind that environment variables are readable by every
//! process of the job, and by `/proc/<pid>/environ` of the same user.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use olvin_core::integrity::{PublicKey, Signer, decode_hex, encode_hex};
use olvin_core::secret::{SecretBytes, SecretString};
use zeroize::Zeroize;

/// Environment: path of the signing key file.
pub const KEY_ENV: &str = "OLVIN_SIGNING_KEY";
/// Environment: the content of the signing key file (a CI secret).
pub const KEY_DATA_ENV: &str = "OLVIN_SIGNING_KEY_DATA";

const SECRET_PREFIX: &str = "olvin-ed25519-secret:";
const PUBLIC_PREFIX: &str = "olvin-ed25519-public:";

/// Which key a build signs with.
#[derive(Clone, Debug, Default)]
pub enum Signing {
    /// `OLVIN_SIGNING_KEY_DATA`, then `OLVIN_SIGNING_KEY`, then the default key
    /// file if it exists; otherwise the build is not signed.
    #[default]
    Auto,
    /// This key file (`--sign-key`).
    Key(PathBuf),
    /// Never sign.
    Off,
}

/// `<config dir>/olvin-hub/keys/release.key`.
pub fn default_key_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("this platform has no config directory")?;
    Ok(dir.join("olvin-hub/keys/release.key"))
}

/// Where the public half of the key at `private` is written: `release.key` -> `release.pub`.
pub fn public_key_path(private: &Path) -> PathBuf {
    private.with_extension("pub")
}

/// Creates a new signing key at `path` (mode 0600, never overwriting unless
/// `force`) and its public key next to it. Returns the public key.
///
/// `forbidden` are folders the key must not be in (the project; refused).
pub fn generate_key(path: &Path, force: bool, forbidden: &[&Path]) -> Result<PublicKey> {
    ensure_outside(path, forbidden)?;
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| anyhow::anyhow!("no randomness from the OS: {e}"))?;
    let secret = SecretBytes::new(seed.to_vec());
    seed.zeroize();
    let signer = Signer::from_seed(&secret).context("the seed has the wrong length")?;
    let line = SecretString::new(format!("{SECRET_PREFIX}{}\n", encode_hex(secret.expose())));
    drop(secret);

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut options = OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow::anyhow!(
                "{} already exists: a new key would make every build signed with the old one \
                 unverifiable. Pick another --out, or --force if you mean it",
                path.display()
            )
        } else {
            anyhow::Error::new(e).context(format!("could not create {}", path.display()))
        }
    })?;
    #[cfg(unix)]
    {
        // An existing file keeps its old mode with `force`: set it explicitly.
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(line.expose().as_bytes())?;
    file.sync_all()?;

    let public = signer.public_key();
    fs::write(
        public_key_path(path),
        format!("{PUBLIC_PREFIX}{}\n", public.to_hex()),
    )
    .with_context(|| format!("could not write {}", public_key_path(path).display()))?;
    Ok(public)
}

/// The signer a build should use, or `None` for an unsigned build.
/// `forbidden`: folders a key file must not be in (the project, the output folder).
pub fn resolve_signer(signing: &Signing, forbidden: &[&Path]) -> Result<Option<Signer>> {
    match signing {
        Signing::Off => Ok(None),
        Signing::Key(path) => load_key_file(path, forbidden).map(Some),
        Signing::Auto => {
            if let Some(data) = std::env::var(KEY_DATA_ENV).ok().filter(|d| !d.is_empty()) {
                return parse_key(&SecretString::new(data))
                    .with_context(|| format!("{KEY_DATA_ENV} is not a signing key"))
                    .map(Some);
            }
            if let Some(path) = std::env::var_os(KEY_ENV).filter(|p| !p.is_empty()) {
                return load_key_file(Path::new(&path), forbidden).map(Some);
            }
            match default_key_path() {
                Ok(path) if path.is_file() => load_key_file(&path, forbidden).map(Some),
                _ => Ok(None),
            }
        }
    }
}

fn load_key_file(path: &Path, forbidden: &[&Path]) -> Result<Signer> {
    ensure_outside(path, forbidden)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .with_context(|| format!("could not read the signing key {}", path.display()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            bail!(
                "{} is readable by other users (mode {:o}). Fix it: chmod 600 {}",
                path.display(),
                mode & 0o777,
                path.display()
            );
        }
    }
    let bytes = fs::read(path)
        .with_context(|| format!("could not read the signing key {}", path.display()))?;
    let text = SecretString::new(
        String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{} is not text", path.display()))?,
    );
    parse_key(&text).with_context(|| format!("{} is not a signing key", path.display()))
}

fn parse_key(text: &SecretString) -> Result<Signer> {
    let hex = text
        .expose()
        .lines()
        .find_map(|l| l.trim().strip_prefix(SECRET_PREFIX))
        .context("no `olvin-ed25519-secret:` line")?;
    let seed = SecretBytes::new(decode_hex(hex.trim()).context("the key is not hex")?);
    Signer::from_seed(&seed).context("a key is 32 bytes (64 hex digits)")
}

/// A key must not sit where it would be committed or shipped.
fn ensure_outside(path: &Path, forbidden: &[&Path]) -> Result<()> {
    // The file may not exist yet: resolve its folder.
    let resolved = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
            let dir = dir.unwrap_or(Path::new("."));
            let name = path.file_name().context("the key path has no file name")?;
            // The folder may not exist either: walk up to one that does.
            let mut existing = dir.to_path_buf();
            let mut tail = PathBuf::new();
            while !existing.exists() {
                let Some(name) = existing.file_name().map(PathBuf::from) else {
                    break;
                };
                tail = name.join(tail);
                if !existing.pop() {
                    break;
                }
            }
            existing
                .canonicalize()
                .unwrap_or(existing)
                .join(tail)
                .join(name)
        }
    };
    for folder in forbidden {
        let folder = folder
            .canonicalize()
            .unwrap_or_else(|_| folder.to_path_buf());
        if resolved.starts_with(&folder) {
            bail!(
                "the signing key {} is inside {}: a key there ends up in the repository or the \
                 archive. Keep it elsewhere (default: {})",
                path.display(),
                folder.display(),
                default_key_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "the hub config folder".into())
            );
        }
    }
    Ok(())
}

/// A public key given as 64 hex digits, or as the path of a `.pub` file.
pub fn read_public_key(arg: &str) -> Result<PublicKey> {
    let text = match fs::read_to_string(arg) {
        Ok(text) => text,
        Err(_) => arg.to_owned(),
    };
    let hex = text
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix(PUBLIC_PREFIX))
        .unwrap_or(text.trim());
    PublicKey::from_hex(hex).map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_core::integrity::PublicKey;

    fn temp_dir(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("olvin-hub-signing-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn keygen_writes_a_private_key_that_signs_and_a_public_one_that_verifies() {
        let dir = temp_dir("keygen");
        let path = dir.join("keys/release.key");
        let public = generate_key(&path, false, &[]).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(SECRET_PREFIX) && text.trim().len() == SECRET_PREFIX.len() + 64);
        // The public file is what `verify --pubkey` takes.
        let pub_path = public_key_path(&path);
        assert_eq!(pub_path, dir.join("keys/release.pub"));
        assert_eq!(read_public_key(pub_path.to_str().unwrap()).unwrap(), public);
        assert_eq!(read_public_key(&public.to_hex()).unwrap(), public);
        assert!(
            !fs::read_to_string(&pub_path)
                .unwrap()
                .contains(text.trim().rsplit(':').next().unwrap())
        );

        let signer = resolve_signer(&Signing::Key(path.clone()), &[])
            .unwrap()
            .unwrap();
        assert_eq!(signer.public_key(), public);
        assert!(public.verify(b"msg", &signer.sign_hex(b"msg")));
        assert!(!public.verify(b"other", &signer.sign_hex(b"msg")));

        // Two keys differ; an existing key is never replaced by accident.
        let err = generate_key(&path, false, &[]).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        let replaced = generate_key(&path, true, &[]).unwrap();
        assert_ne!(replaced, public);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_key_inside_the_project_is_refused_both_ways() {
        let dir = temp_dir("inside");
        let project = dir.join("game");
        fs::create_dir_all(&project).unwrap();
        let err = generate_key(&project.join("keys/release.key"), false, &[&project]).unwrap_err();
        assert!(err.to_string().contains("inside"), "{err}");
        assert!(!project.join("keys").exists(), "nothing was written");

        let outside = dir.join("keys/release.key");
        generate_key(&outside, false, &[&project]).unwrap();
        fs::copy(&outside, project.join("copied.key")).unwrap();
        let err =
            resolve_signer(&Signing::Key(project.join("copied.key")), &[&project]).unwrap_err();
        assert!(err.to_string().contains("inside"), "{err}");
        // `..` does not smuggle it in.
        let sneaky = dir.join("keys/../game/copied.key");
        assert!(resolve_signer(&Signing::Key(sneaky), &[&project]).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_key_other_users_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let path = dir.join("release.key");
        generate_key(&path, false, &[]).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let err = resolve_signer(&Signing::Key(path.clone()), &[]).unwrap_err();
        assert!(err.to_string().contains("chmod 600"), "{err}");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(resolve_signer(&Signing::Key(path), &[]).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn broken_key_data_is_an_error_and_off_is_none() {
        for bad in [
            "",
            "garbage",
            "olvin-ed25519-secret:zz",
            "olvin-ed25519-secret:00ff",
        ] {
            assert!(
                parse_key(&SecretString::new(bad.into())).is_err(),
                "{bad:?}"
            );
        }
        let good = format!("olvin-ed25519-secret:{}\n", "ab".repeat(32));
        assert!(parse_key(&SecretString::new(good)).is_ok());
        assert!(resolve_signer(&Signing::Off, &[]).unwrap().is_none());
        assert!(PublicKey::from_hex("nope").is_err());
        assert!(read_public_key("nope").is_err());
    }
}

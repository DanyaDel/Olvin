use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const ENGINE_GIT: &str = "https://github.com/DanyaDel/Olvin";

/// Where game projects get the engine from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineSource {
    /// Local checkout of the Olvin workspace.
    Path(PathBuf),
    Git(String),
}

impl Default for EngineSource {
    /// The workspace this hub was built from, if it's still there; else GitHub.
    fn default() -> Self {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        match workspace.canonicalize() {
            Ok(path) if path.join("crates/olvin/Cargo.toml").is_file() => Self::Path(path),
            _ => Self::Git(ENGINE_GIT.into()),
        }
    }
}

impl EngineSource {
    pub fn validate(&self) -> Result<()> {
        if let Self::Path(path) = self
            && !path.join("crates/olvin/Cargo.toml").is_file()
        {
            bail!("{} не похоже на Olvin: нет crates/olvin", path.display());
        }
        Ok(())
    }

    /// The `olvin = { ... }` dependency entry, without features.
    pub fn dependency(&self) -> toml_edit::InlineTable {
        let mut dep = toml_edit::InlineTable::new();
        match self {
            Self::Path(path) => {
                dep.insert("path", path.join("crates/olvin").display().to_string().into())
            }
            Self::Git(url) => dep.insert("git", url.as_str().into()),
        };
        dep.insert("default-features", false.into());
        dep
    }
}

impl std::fmt::Display for EngineSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(path) => write!(f, "path {}", path.display()),
            Self::Git(url) => write!(f, "git {url}"),
        }
    }
}

/// Persistent hub state: `<config dir>/olvin-hub/config.toml`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HubConfig {
    pub engine: EngineSource,
    pub projects: Vec<PathBuf>,
}

impl HubConfig {
    pub fn path() -> Result<PathBuf> {
        let dir = dirs::config_dir().context("на этой платформе нет папки конфигов")?;
        Ok(dir.join("olvin-hub/config.toml"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("не удалось прочитать {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("не удалось прочитать {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, toml::to_string_pretty(self)?)
            .with_context(|| format!("не удалось записать {}", path.display()))
    }

    /// Remembers a project; returns false if it was already known.
    pub fn register(&mut self, root: &Path) -> Result<bool> {
        let root = root.canonicalize()?;
        if self.projects.contains(&root) {
            return Ok(false);
        }
        self.projects.push(root);
        Ok(true)
    }

    pub fn forget(&mut self, root: &Path) -> bool {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let before = self.projects.len();
        self.projects.retain(|p| *p != root);
        self.projects.len() != before
    }
}

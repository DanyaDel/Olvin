//! Where the game's files are. Every plugin that reads or writes files
//! (sounds, input maps, logs, scenes) resolves relative paths against
//! [`ProjectPaths`] instead of the working directory, so the game finds its
//! `assets/` whether it is started by `cargo run` from anywhere in a
//! workspace, from the folder `olvin-hub build` made, or by a double click.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Overrides the project folder (the one that holds `assets/`).
pub const PROJECT_DIR_ENV: &str = "OLVIN_PROJECT_DIR";
/// Overrides only the assets folder (default: `<project>/assets`).
pub const ASSETS_DIR_ENV: &str = "OLVIN_ASSETS_DIR";

/// How [`ProjectPaths::detect`] found the project folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathsSource {
    /// Set in code ([`ProjectPaths::new`]).
    Explicit,
    /// [`PROJECT_DIR_ENV`].
    Env,
    /// The executable's folder has `assets/` (the `olvin-hub build` layout).
    Executable,
    /// `CARGO_MANIFEST_DIR`, set by `cargo run` and `cargo test`: the game's crate.
    Manifest,
    /// The working directory (the old behaviour, last resort).
    CurrentDir,
}

/// Resource: the project folder and its `assets/`. `App::new` inserts the
/// detected one; insert your own before adding plugins to override it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectPaths {
    /// Holds `assets/`, `logs/`, `saves/`.
    pub root: PathBuf,
    /// Asset paths (clips, scenes, `input.ron`) are relative to this.
    pub assets: PathBuf,
    pub source: PathsSource,
}

impl Default for ProjectPaths {
    /// Same as [`ProjectPaths::detect`].
    fn default() -> Self {
        Self::detect().clone()
    }
}

impl ProjectPaths {
    /// `root` with `root/assets`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            assets: root.join("assets"),
            root,
            source: PathsSource::Explicit,
        }
    }

    /// Resolved once per process: [`PROJECT_DIR_ENV`], else the executable's
    /// folder if it has `assets/`, else `CARGO_MANIFEST_DIR` (dev runs through
    /// cargo), else the working directory. [`ASSETS_DIR_ENV`] then replaces
    /// the assets folder.
    pub fn detect() -> &'static Self {
        static PATHS: OnceLock<ProjectPaths> = OnceLock::new();
        PATHS.get_or_init(|| {
            let exe = std::env::current_exe().ok();
            let paths = Self::detect_from(
                |key| std::env::var_os(key).map(PathBuf::from),
                exe.as_deref(),
            );
            log::debug!(
                "project: {} ({:?}), assets: {}",
                paths.root.display(),
                paths.source,
                paths.assets.display()
            );
            paths
        })
    }

    /// [`detect`](Self::detect) with the environment and executable given
    /// (tests, tools that look at another process).
    pub fn detect_from(env: impl Fn(&str) -> Option<PathBuf>, exe: Option<&Path>) -> Self {
        let exe_dir = exe
            .and_then(Path::parent)
            .filter(|dir| dir.join("assets").is_dir());
        let (root, source) = if let Some(dir) = env(PROJECT_DIR_ENV) {
            (dir, PathsSource::Env)
        } else if let Some(dir) = exe_dir {
            (dir.to_path_buf(), PathsSource::Executable)
        } else if let Some(dir) = env("CARGO_MANIFEST_DIR") {
            (dir, PathsSource::Manifest)
        } else {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            (cwd, PathsSource::CurrentDir)
        };
        let assets = env(ASSETS_DIR_ENV)
            .map(|a| root.join(a))
            .unwrap_or_else(|| root.join("assets"));
        Self {
            root,
            assets,
            source,
        }
    }

    /// `path` inside `assets/`; absolute paths stay as they are.
    pub fn asset(&self, path: impl AsRef<Path>) -> PathBuf {
        self.assets.join(path)
    }

    /// `path` inside the project folder (`logs/game.log`); absolute paths stay.
    pub fn resolve(&self, path: impl AsRef<Path>) -> PathBuf {
        self.root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<PathBuf> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| PathBuf::from(v))
        }
    }

    #[test]
    fn detection_order() {
        let dir = std::env::temp_dir().join(format!("olvin-paths-{}", std::process::id()));
        let built = dir.join("dist/game");
        std::fs::create_dir_all(built.join("assets")).unwrap();
        let exe = built.join("game");
        let bare_exe = dir.join("target/debug/game");

        let p = ProjectPaths::detect_from(env(&[(PROJECT_DIR_ENV, "/proj")]), Some(&exe));
        assert_eq!(
            (p.root.as_path(), p.source),
            (Path::new("/proj"), PathsSource::Env)
        );
        assert_eq!(p.assets, Path::new("/proj/assets"));

        let p = ProjectPaths::detect_from(env(&[("CARGO_MANIFEST_DIR", "/crate")]), Some(&exe));
        assert_eq!((p.root, p.source), (built.clone(), PathsSource::Executable));

        // A cargo target folder has no assets/: the crate wins.
        let p =
            ProjectPaths::detect_from(env(&[("CARGO_MANIFEST_DIR", "/crate")]), Some(&bare_exe));
        assert_eq!(
            (p.root.as_path(), p.source),
            (Path::new("/crate"), PathsSource::Manifest)
        );

        let p = ProjectPaths::detect_from(env(&[]), Some(&bare_exe));
        assert_eq!(p.source, PathsSource::CurrentDir);
        assert_eq!(p.root, std::env::current_dir().unwrap());

        // Relative assets override is inside the project, absolute replaces it.
        let p = ProjectPaths::detect_from(
            env(&[(PROJECT_DIR_ENV, "/proj"), (ASSETS_DIR_ENV, "data")]),
            None,
        );
        assert_eq!(p.asset("a.wav"), Path::new("/proj/data/a.wav"));
        let p = ProjectPaths::detect_from(env(&[(ASSETS_DIR_ENV, "/elsewhere")]), Some(&exe));
        assert_eq!(p.assets, Path::new("/elsewhere"));
        assert_eq!(p.resolve("logs/x.log"), built.join("logs/x.log"));
        assert_eq!(p.asset("/abs.wav"), Path::new("/abs.wav"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn app_has_detected_paths() {
        let app = crate::App::new();
        let paths = app.resources.fetch::<ProjectPaths>();
        assert_eq!(*paths, *ProjectPaths::detect());
        // `cargo test` sets the manifest dir of this crate.
        assert!(paths.root.ends_with("olvin_core") || paths.source == PathsSource::Env);
    }
}

//! Scene loading on a worker thread: the file is read and parsed (with migrations)
//! off the main thread, the game polls for the result and spawns it when ready.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use olvin_core::{App, Plugin, Stage};
use olvin_scene::file::SceneFile;

/// Identifies one load request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoadHandle(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadStatus {
    Pending,
    Ready,
    Failed,
    /// Unknown handle, or the result was already taken.
    Gone,
}

type LoadResult = Result<SceneFile, String>;

/// Resource: queue of background scene loads. Results are drained from the worker
/// channel by a system in `Stage::First`, or by [`SceneLoader::poll`].
pub struct SceneLoader {
    next_id: u64,
    pending: HashMap<u64, PathBuf>,
    done: HashMap<u64, (PathBuf, LoadResult)>,
    tx: Sender<(u64, LoadResult)>,
    rx: Receiver<(u64, LoadResult)>,
}

impl Default for SceneLoader {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            next_id: 0,
            pending: HashMap::new(),
            done: HashMap::new(),
            tx,
            rx,
        }
    }
}

impl SceneLoader {
    /// Starts loading `path` and returns at once.
    pub fn request(&mut self, path: impl AsRef<Path>) -> LoadHandle {
        let path = path.as_ref().to_path_buf();
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, path.clone());
        let tx = self.tx.clone();
        thread::spawn(move || {
            // Приёмник мог исчезнуть вместе с загрузчиком — тогда результат не нужен.
            let _ = tx.send((id, SceneFile::load(&path)));
        });
        LoadHandle(id)
    }

    /// Moves finished loads from the worker channel into the ready set.
    pub fn poll(&mut self) {
        while let Ok((id, result)) = self.rx.try_recv() {
            if let Some(path) = self.pending.remove(&id) {
                if let Err(e) = &result {
                    log::warn!("scene load failed: {e}");
                }
                self.done.insert(id, (path, result));
            }
        }
    }

    pub fn status(&self, handle: LoadHandle) -> LoadStatus {
        match self.done.get(&handle.0) {
            Some((_, Ok(_))) => LoadStatus::Ready,
            Some((_, Err(_))) => LoadStatus::Failed,
            None if self.pending.contains_key(&handle.0) => LoadStatus::Pending,
            None => LoadStatus::Gone,
        }
    }

    /// True while any request is still running.
    pub fn is_loading(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Takes the finished scene (or its error). `None` while pending or when gone.
    pub fn take(&mut self, handle: LoadHandle) -> Option<LoadResult> {
        self.done.remove(&handle.0).map(|(_, r)| r)
    }

    /// [`SceneLoader::take`] plus the path the scene was loaded from (asset
    /// paths inside the scene resolve against its `assets/` folder).
    pub fn take_with_path(&mut self, handle: LoadHandle) -> Option<(PathBuf, LoadResult)> {
        self.done.remove(&handle.0)
    }

    /// The path a request was made for, while it is pending or not yet taken.
    pub fn path(&self, handle: LoadHandle) -> Option<&Path> {
        self.pending
            .get(&handle.0)
            .or_else(|| self.done.get(&handle.0).map(|(p, _)| p))
            .map(PathBuf::as_path)
    }
}

/// Inserts [`SceneLoader`] and polls it every frame.
pub struct SceneLoaderPlugin;

impl Plugin for SceneLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SceneLoader>();
        app.add_system(Stage::First, |res| res.fetch_mut::<SceneLoader>().poll());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{State, StateAppExt};
    use std::time::{Duration, Instant};

    fn scene_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("olvin_game_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait(app: &mut App, mut done: impl FnMut(&App) -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !done(app) {
            assert!(Instant::now() < end, "загрузка не завершилась");
            app.update();
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn loads_in_background_and_reports_errors() {
        let dir = scene_dir("load");
        let good = dir.join("a.scene.ron");
        let mut scene = SceneFile::default();
        scene
            .entities
            .push(olvin_scene::file::EntityDesc::new(1, "A"));
        scene.save(&good).unwrap();
        let broken = dir.join("b.scene.ron");
        std::fs::write(&broken, "(entities: [").unwrap();

        let mut app = App::new();
        app.add_plugin(SceneLoaderPlugin);
        let (ok, bad, missing) = {
            let mut loader = app.resources.fetch_mut::<SceneLoader>();
            (
                loader.request(&good),
                loader.request(&broken),
                loader.request(dir.join("nope.scene.ron")),
            )
        };
        assert_ne!(ok, bad);
        wait(&mut app, |a| {
            !a.resources.fetch::<SceneLoader>().is_loading()
        });

        let mut loader = app.resources.fetch_mut::<SceneLoader>();
        assert_eq!(loader.status(ok), LoadStatus::Ready);
        assert_eq!(loader.status(bad), LoadStatus::Failed);
        assert_eq!(loader.path(ok), Some(good.as_path()));
        assert_eq!(loader.take(ok).unwrap().unwrap(), scene);
        assert!(
            loader
                .take(bad)
                .unwrap()
                .unwrap_err()
                .contains("b.scene.ron")
        );
        assert!(loader.take(missing).unwrap().is_err());
        assert_eq!(loader.status(ok), LoadStatus::Gone);
        assert!(loader.take(ok).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_screen_switches_to_playing_when_the_scene_is_ready() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        enum Mode {
            Loading,
            Playing,
        }
        let dir = scene_dir("flow");
        let path = dir.join("level.scene.ron");
        SceneFile::default().save(&path).unwrap();

        let mut app = App::new();
        app.add_plugin(SceneLoaderPlugin).add_state(Mode::Loading);
        let handle = std::rc::Rc::new(std::cell::Cell::new(None));
        let h = handle.clone();
        app.on_enter(Mode::Loading, move |res| {
            h.set(Some(res.fetch_mut::<SceneLoader>().request(&path)));
        });
        let h = handle.clone();
        app.add_system_in_state(Stage::Update, Mode::Loading, move |res| {
            let handle = h.get().unwrap();
            if res.fetch::<SceneLoader>().status(handle) == LoadStatus::Ready {
                res.fetch_mut::<State<Mode>>().set(Mode::Playing);
            }
        });
        wait(&mut app, |a| {
            a.resources.fetch::<State<Mode>>().is(Mode::Playing)
        });
        let scene = app
            .resources
            .fetch_mut::<SceneLoader>()
            .take(handle.get().unwrap());
        assert!(scene.unwrap().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

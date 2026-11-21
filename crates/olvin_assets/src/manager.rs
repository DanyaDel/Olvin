//! Runtime asset manager: reference-counted handles, background loading,
//! dependencies (model → material → texture), unloading of unused assets and
//! hot reload on top of [`AssetWatcher`].
//!
//! Pure data, like the rest of the crate: the manager decodes files into
//! [`ImageData`], [`MaterialDesc`] and [`ModelData`]; uploading to the GPU is
//! up to the consumer, which watches [`AssetManager::take_changes`].

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use olvin_scene::anim::AnimGraphDesc;
use olvin_scene::material::MaterialDesc;

use crate::root::normalize;
use crate::{AssetEvent, AssetKind, AssetRoot, AssetWatcher, ImageData, ModelData};
use crate::{import_gltf, load_image_sized};

/// What an asset asks the manager to load together with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dependency {
    Texture(String),
    Material(String),
}

/// A type the manager can load. Implemented for [`ImageData`] (textures),
/// [`MaterialDesc`] (`*.mat.ron`), [`ModelData`] (glTF) and [`AnimGraphDesc`]
/// (`*.animgraph.ron`).
pub trait Asset: Any + Send + Sync + Sized {
    /// What the file is, for [`AssetChange`].
    const KIND: AssetKind;

    /// Runs on the calling thread: reads the `.meta` and returns the heavy
    /// part (file reading, decoding) as a closure for a worker thread.
    fn prepare(root: &AssetRoot, rel: &str) -> Result<Job<Self>, String>;

    /// What has to be loaded for this asset to be complete.
    fn dependencies(&self) -> Vec<Dependency>;
}

/// The part of loading that runs on a worker thread.
pub type Job<T> = Box<dyn FnOnce() -> Result<T, String> + Send>;

impl Asset for ImageData {
    const KIND: AssetKind = AssetKind::Texture;

    fn prepare(root: &AssetRoot, rel: &str) -> Result<Job<Self>, String> {
        let settings = root.meta(rel)?.texture().cloned().unwrap_or_default();
        let path = root.resolve(rel);
        Ok(Box::new(move || {
            load_image_sized(path, settings.srgb, settings.max_size)
        }))
    }

    fn dependencies(&self) -> Vec<Dependency> {
        Vec::new()
    }
}

impl Asset for MaterialDesc {
    const KIND: AssetKind = AssetKind::Material;

    fn prepare(root: &AssetRoot, rel: &str) -> Result<Job<Self>, String> {
        let path = root.resolve(rel);
        Ok(Box::new(move || MaterialDesc::load(path)))
    }

    fn dependencies(&self) -> Vec<Dependency> {
        textures_of(self)
    }
}

impl Asset for ModelData {
    const KIND: AssetKind = AssetKind::Model;

    fn prepare(root: &AssetRoot, rel: &str) -> Result<Job<Self>, String> {
        let settings = root.meta(rel)?.model().cloned().unwrap_or_default();
        let path = root.resolve(rel);
        let assets = root.path().to_owned();
        Ok(Box::new(move || import_gltf(path, &assets, &settings)))
    }

    /// The model's materials are inline, so its textures hang straight off it.
    fn dependencies(&self) -> Vec<Dependency> {
        let mut out = Vec::new();
        for dep in self.materials.iter().flat_map(textures_of) {
            if !out.contains(&dep) {
                out.push(dep);
            }
        }
        out
    }
}

impl Asset for AnimGraphDesc {
    const KIND: AssetKind = AssetKind::AnimGraph;

    fn prepare(root: &AssetRoot, rel: &str) -> Result<Job<Self>, String> {
        let path = root.resolve(rel);
        Ok(Box::new(move || AnimGraphDesc::load(path)))
    }

    /// Clips are named, they live in the model the graph is applied to.
    fn dependencies(&self) -> Vec<Dependency> {
        Vec::new()
    }
}

fn textures_of(material: &MaterialDesc) -> Vec<Dependency> {
    let mut out = Vec::new();
    for tex in material.textures() {
        let dep = Dependency::Texture(normalize(tex));
        if !out.contains(&dep) {
            out.push(dep);
        }
    }
    out
}

struct HandleInner {
    path: String,
    ty: TypeId,
    kind: AssetKind,
}

/// A counted reference to an asset. Clone it freely; once the last clone (and
/// the assets that depend on it) is gone, the asset becomes unloadable.
pub struct Handle<T> {
    inner: Arc<HandleInner>,
    _ty: PhantomData<fn() -> T>,
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _ty: PhantomData,
        }
    }
}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl<T> Eq for Handle<T> {}

impl<T> std::fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Handle({})", self.inner.path)
    }
}

impl<T> Handle<T> {
    /// Relative to `assets/`, forward slashes.
    pub fn path(&self) -> &str {
        &self.inner.path
    }

    /// Forgets the type, e.g. to keep different assets in one list.
    pub fn untyped(&self) -> AnyHandle {
        AnyHandle(self.inner.clone())
    }
}

/// A [`Handle`] without its type.
#[derive(Clone)]
pub struct AnyHandle(Arc<HandleInner>);

impl AnyHandle {
    pub fn path(&self) -> &str {
        &self.0.path
    }

    pub fn kind(&self) -> AssetKind {
        self.0.kind
    }

    /// `None` if the handle is of another asset type.
    pub fn typed<T: Asset>(&self) -> Option<Handle<T>> {
        (self.0.ty == TypeId::of::<T>()).then(|| Handle {
            inner: self.0.clone(),
            _ty: PhantomData,
        })
    }
}

impl std::fmt::Debug for AnyHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AnyHandle({:?}, {})", self.0.kind, self.0.path)
    }
}

/// Where an asset is in its life.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadState {
    Loading,
    Ready,
    /// The last (re)load failed. During a hot reload the previous data stays
    /// readable, so `Failed` with data is possible: see [`AssetManager::get`].
    Failed(String),
}

/// An asset got new data (first load or hot reload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetChange {
    pub path: String,
    pub kind: AssetKind,
    /// 1 for the first load, +1 per reload.
    pub version: u32,
}

type Key = (TypeId, String);

struct Loaded {
    data: Box<dyn Any + Send + Sync>,
    deps: Vec<Dependency>,
}

struct Done {
    key: Key,
    generation: u64,
    result: Result<Loaded, String>,
}

struct Entry {
    weak: Weak<HandleInner>,
    kind: AssetKind,
    data: Option<Arc<dyn Any + Send + Sync>>,
    /// Strong handles to the dependencies keep them alive as long as we are.
    deps: Vec<AnyHandle>,
    version: u32,
    /// Unique per request, so stale results of a superseded load are ignored.
    generation: u64,
    loading: bool,
    error: Option<String>,
    /// Starts the load again (hot reload); built when the asset first loads.
    reload: Reloader,
}

type Reloader = Box<dyn Fn(&AssetRoot) -> Result<Job<Loaded>, String> + Send + Sync>;

type Task = Box<dyn FnOnce() + Send>;

/// Loads assets in the background and tracks who uses them. Drive it with
/// [`update`](Self::update) once per frame (the [`AssetsPlugin`](crate::AssetsPlugin)
/// does when given a root).
pub struct AssetManager {
    root: AssetRoot,
    entries: HashMap<Key, Entry>,
    tasks: Option<Sender<Task>>,
    results: Receiver<Done>,
    result_tx: Sender<Done>,
    workers: Vec<std::thread::JoinHandle<()>>,
    watcher: Option<AssetWatcher>,
    changes: Vec<AssetChange>,
    auto_unload: bool,
    next_generation: u64,
}

/// Worker threads when the machine says nothing sensible.
const DEFAULT_WORKERS: usize = 2;

impl AssetManager {
    /// Manager for the `assets/` folder at `root` (not scanned: metas are
    /// created lazily, when an asset is first loaded).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_workers(root, default_workers())
    }

    /// Like [`new`](Self::new) with an explicit number of loader threads (at least 1).
    pub fn with_workers(root: impl Into<PathBuf>, workers: usize) -> Self {
        let (task_tx, task_rx) = channel::<Task>();
        let task_rx = Arc::new(Mutex::new(task_rx));
        let (result_tx, results) = channel();
        let workers = (0..workers.max(1))
            .map(|i| {
                let rx = task_rx.clone();
                std::thread::Builder::new()
                    .name(format!("olvin-assets-{i}"))
                    .spawn(move || {
                        loop {
                            // The lock is held only while waiting for a task.
                            let task = rx.lock().ok().and_then(|rx| rx.recv().ok());
                            match task {
                                Some(task) => task(),
                                None => break,
                            }
                        }
                    })
                    .expect("spawn asset loader thread")
            })
            .collect();
        Self {
            root: AssetRoot::new(root),
            entries: HashMap::new(),
            tasks: Some(task_tx),
            results,
            result_tx,
            workers,
            watcher: None,
            changes: Vec::new(),
            auto_unload: true,
            next_generation: 0,
        }
    }

    /// Reloads changed files on [`update`](Self::update). Fails when the
    /// watcher can't start (no inotify, folder missing).
    pub fn enable_hot_reload(&mut self) -> Result<(), String> {
        self.watcher = Some(AssetWatcher::new(self.root.path())?);
        Ok(())
    }

    pub fn hot_reload_enabled(&self) -> bool {
        self.watcher.is_some()
    }

    pub fn root(&self) -> &AssetRoot {
        &self.root
    }

    /// With `true` (the default) [`update`](Self::update) unloads assets
    /// nobody holds a handle to; with `false` only [`unload_unused`](Self::unload_unused) does.
    pub fn set_auto_unload(&mut self, on: bool) {
        self.auto_unload = on;
    }

    /// Handle to the asset at `rel`, starting a background load if it isn't
    /// loaded yet. Asking again for a loaded or loading path shares the work.
    pub fn load<T: Asset>(&mut self, rel: &str) -> Handle<T> {
        let rel = normalize(rel);
        let key = (TypeId::of::<T>(), rel.clone());
        if let Some(entry) = self.entries.get_mut(&key) {
            // Dropped but not swept yet: the cached data lives on.
            if let Some(inner) = entry.weak.upgrade() {
                return Handle {
                    inner,
                    _ty: PhantomData,
                };
            }
            let inner = Arc::new(HandleInner {
                path: rel,
                ty: key.0,
                kind: T::KIND,
            });
            entry.weak = Arc::downgrade(&inner);
            return Handle {
                inner,
                _ty: PhantomData,
            };
        }
        let inner = Arc::new(HandleInner {
            path: rel.clone(),
            ty: key.0,
            kind: T::KIND,
        });
        let reload: Reloader = Box::new({
            let rel = rel.clone();
            move |root| {
                let job = T::prepare(root, &rel)?;
                Ok(Box::new(move || {
                    let data = job()?;
                    let deps = data.dependencies();
                    Ok(Loaded {
                        data: Box::new(data),
                        deps,
                    })
                }))
            }
        });
        let entry = Entry {
            weak: Arc::downgrade(&inner),
            kind: T::KIND,
            data: None,
            deps: Vec::new(),
            version: 0,
            generation: 0,
            loading: false,
            error: None,
            reload,
        };
        self.entries.insert(key.clone(), entry);
        self.start(&key);
        Handle {
            inner,
            _ty: PhantomData,
        }
    }

    /// The loaded data, shared. `None` while loading for the first time or
    /// after a failed first load; after a failed reload it is the old data.
    pub fn get<T: Asset>(&self, handle: &Handle<T>) -> Option<Arc<T>> {
        let entry = self
            .entries
            .get(&(TypeId::of::<T>(), handle.inner.path.clone()))?;
        entry.data.clone()?.downcast::<T>().ok()
    }

    pub fn state<T: Asset>(&self, handle: &Handle<T>) -> LoadState {
        match self
            .entries
            .get(&(TypeId::of::<T>(), handle.inner.path.clone()))
        {
            Some(e) if e.loading => LoadState::Loading,
            Some(e) => match &e.error {
                Some(err) => LoadState::Failed(err.clone()),
                None if e.data.is_some() => LoadState::Ready,
                None => LoadState::Loading,
            },
            None => LoadState::Failed("unloaded".into()),
        }
    }

    /// The asset and everything it depends on has data.
    pub fn is_fully_loaded<T: Asset>(&self, handle: &Handle<T>) -> bool {
        self.fully_loaded(&handle.untyped(), TypeId::of::<T>())
    }

    fn fully_loaded(&self, handle: &AnyHandle, ty: TypeId) -> bool {
        let Some(e) = self.entries.get(&(ty, handle.0.path.clone())) else {
            return false;
        };
        !e.loading && e.data.is_some() && e.deps.iter().all(|d| self.fully_loaded(d, d.0.ty))
    }

    /// Reloads since the first load: 0 while nothing has loaded yet.
    pub fn version<T: Asset>(&self, handle: &Handle<T>) -> u32 {
        self.entries
            .get(&(TypeId::of::<T>(), handle.inner.path.clone()))
            .map_or(0, |e| e.version)
    }

    /// Dependencies of a loaded asset (textures of a material, …).
    pub fn dependencies<T: Asset>(&self, handle: &Handle<T>) -> Vec<AnyHandle> {
        self.entries
            .get(&(TypeId::of::<T>(), handle.inner.path.clone()))
            .map(|e| e.deps.clone())
            .unwrap_or_default()
    }

    /// Live handles to the asset: every clone, plus one per loaded asset
    /// that depends on it. The manager itself holds none.
    pub fn ref_count<T>(&self, handle: &Handle<T>) -> usize {
        Arc::strong_count(&handle.inner)
    }

    /// Assets currently held in memory (loading, loaded or failed).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the asset at `rel` of type `T` is held in memory.
    pub fn contains<T: Asset>(&self, rel: &str) -> bool {
        self.entries
            .contains_key(&(TypeId::of::<T>(), normalize(rel)))
    }

    /// Loads still running.
    pub fn pending(&self) -> usize {
        self.entries.values().filter(|e| e.loading).count()
    }

    /// New data since the last call: for uploading to the GPU or refreshing editors.
    pub fn take_changes(&mut self) -> Vec<AssetChange> {
        std::mem::take(&mut self.changes)
    }

    /// Re-reads one asset from disk (what hot reload does for changed files).
    pub fn reload<T: Asset>(&mut self, handle: &Handle<T>) {
        let key = (TypeId::of::<T>(), handle.inner.path.clone());
        self.start(&key);
    }

    /// Drops every asset without a handle, together with dependencies that
    /// become unused by that. Returns how many were unloaded.
    pub fn unload_unused(&mut self) -> usize {
        let mut total = 0;
        loop {
            let dead: Vec<Key> = self
                .entries
                .iter()
                .filter(|(_, e)| e.weak.strong_count() == 0)
                .map(|(k, _)| k.clone())
                .collect();
            if dead.is_empty() {
                return total;
            }
            total += dead.len();
            // Removing an entry drops its dependency handles: the next round
            // finds the ones only it was keeping alive.
            for key in dead {
                self.entries.remove(&key);
            }
        }
    }

    /// One frame: hot-reload events, finished loads, unloading.
    pub fn update(&mut self) {
        if let Some(watcher) = &mut self.watcher {
            for event in watcher.poll() {
                match event {
                    AssetEvent::Changed(p) | AssetEvent::Created(p) => self.reload_path(&p),
                    AssetEvent::Moved { to, .. } => self.reload_path(&to),
                    AssetEvent::Removed(p) => log::info!("asset removed, keeping loaded data: {p}"),
                }
            }
        }
        while let Ok(done) = self.results.try_recv() {
            self.finish(done);
        }
        if self.auto_unload {
            self.unload_unused();
        }
    }

    /// Pumps [`update`](Self::update) until nothing is loading or `timeout`
    /// passes. Returns whether everything finished (for tools and tests).
    pub fn wait_idle(&mut self, timeout: Duration) -> bool {
        let start = Instant::now();
        loop {
            self.update();
            if self.pending() == 0 {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn reload_path(&mut self, rel: &str) {
        let keys: Vec<Key> = self
            .entries
            .iter()
            .filter(|((_, p), e)| p == rel && e.weak.strong_count() > 0)
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys {
            self.start(&key);
        }
    }

    fn start(&mut self, key: &Key) {
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        self.next_generation += 1;
        let generation = self.next_generation;
        entry.generation = generation;
        match (entry.reload)(&self.root) {
            Ok(job) => {
                entry.loading = true;
                let tx = self.result_tx.clone();
                let key = key.clone();
                let task: Task = Box::new(move || {
                    let result = job();
                    let _ = tx.send(Done {
                        key,
                        generation,
                        result,
                    });
                });
                if let Some(tasks) = &self.tasks {
                    let _ = tasks.send(task);
                }
            }
            Err(err) => {
                entry.loading = false;
                log::warn!("asset {}: {err}", key.1);
                entry.error = Some(err);
            }
        }
    }

    fn finish(&mut self, done: Done) {
        let Some(entry) = self.entries.get_mut(&done.key) else {
            return;
        };
        if entry.generation != done.generation {
            return;
        }
        entry.loading = false;
        match done.result {
            Ok(loaded) => {
                entry.data = Some(Arc::from(loaded.data));
                entry.error = None;
                entry.version += 1;
                self.changes.push(AssetChange {
                    path: done.key.1.clone(),
                    kind: entry.kind,
                    version: entry.version,
                });
                let deps: Vec<AnyHandle> = loaded
                    .deps
                    .iter()
                    .map(|d| match d {
                        Dependency::Texture(p) => self.load::<ImageData>(p).untyped(),
                        Dependency::Material(p) => self.load::<MaterialDesc>(p).untyped(),
                    })
                    .collect();
                if let Some(entry) = self.entries.get_mut(&done.key) {
                    entry.deps = deps;
                }
            }
            Err(err) => {
                log::warn!("asset {}: {err}", done.key.1);
                entry.error = Some(err);
            }
        }
    }
}

impl Drop for AssetManager {
    fn drop(&mut self) {
        // Closing the channel ends the workers once they finish their task.
        self.tasks = None;
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map_or(DEFAULT_WORKERS, |n| n.get().div_ceil(2))
        .clamp(1, 4)
}

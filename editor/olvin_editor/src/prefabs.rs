//! Prefab assets in the editor: reading `*.prefab.ron` under `assets/` (cached,
//! so the instances in the open scene and the asset they were expanded from
//! stay comparable), writing them, and keeping them in the undo history the
//! way materials are.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use olvin_scene::file::SceneFile;

/// How often the files of the cached prefabs are checked for outside edits.
pub const POLL: Duration = Duration::from_millis(400);

/// Content (file text) of the prefabs written by the editor, by path: what an
/// undo step remembers (the files on disk always hold the current content).
pub type PrefabEdits = BTreeMap<String, String>;

type Signature = Option<(SystemTime, u64)>;

struct Cached {
    scene: Result<SceneFile, String>,
    sig: Signature,
}

/// The prefab files the open scene uses.
#[derive(Default)]
pub struct PrefabLibrary {
    /// The project's `assets/`.
    root: Option<PathBuf>,
    cache: HashMap<String, Cached>,
    /// Current content of every prefab written this session.
    edits: PrefabEdits,
    /// Content of those prefabs before their first write (undo past it).
    originals: PrefabEdits,
}

impl PrefabLibrary {
    pub fn set_root(&mut self, root: Option<PathBuf>) {
        if root != self.root {
            *self = Self {
                root,
                ..Default::default()
            };
        }
    }

    fn signature(&self, rel: &str) -> Signature {
        let meta = std::fs::metadata(self.root.as_ref()?.join(rel)).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    }

    fn read(&self, rel: &str) -> Cached {
        let scene = match &self.root {
            Some(root) => SceneFile::load(root.join(rel)),
            None => Err("no project".into()),
        };
        Cached {
            scene,
            sig: self.signature(rel),
        }
    }

    /// The prefab at `rel`; read once, then the cached content (until
    /// [`PrefabLibrary::reload`]).
    pub fn get(&mut self, rel: &str) -> Result<SceneFile, String> {
        if !self.cache.contains_key(rel) {
            let fresh = self.read(rel);
            self.cache.insert(rel.to_owned(), fresh);
        }
        self.cache[rel].scene.clone()
    }

    /// A loader for `olvin_scene::prefab` over the cache.
    pub fn loader(&mut self) -> impl FnMut(&str) -> Result<SceneFile, String> + '_ {
        |rel| self.get(rel)
    }

    /// What is cached now, as a loader that never reads the disk: the
    /// prefabs instances were expanded from, before a reload.
    pub fn snapshot(&self) -> HashMap<String, SceneFile> {
        self.cache
            .iter()
            .filter_map(|(k, c)| Some((k.clone(), c.scene.as_ref().ok()?.clone())))
            .collect()
    }

    /// Cached prefabs whose file changed (or appeared, or vanished) since they were read.
    pub fn changed(&self) -> Vec<String> {
        let mut list: Vec<String> = self
            .cache
            .iter()
            .filter(|(rel, c)| self.signature(rel) != c.sig)
            .map(|(rel, _)| rel.clone())
            .collect();
        list.sort();
        list
    }

    pub fn reload(&mut self, rel: &str) {
        let fresh = self.read(rel);
        self.cache.insert(rel.to_owned(), fresh);
    }

    /// Writes a prefab (new or changed) and caches it as it is now.
    pub fn write(&mut self, rel: &str, scene: &SceneFile) -> Result<(), String> {
        let path = self.root.clone().ok_or("no project")?.join(rel);
        if !self.edits.contains_key(rel)
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            self.originals.insert(rel.to_owned(), text);
        }
        scene.save(&path)?;
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        self.edits.insert(rel.to_owned(), text);
        let sig = self.signature(rel);
        self.cache.insert(
            rel.to_owned(),
            Cached {
                scene: Ok(scene.clone()),
                sig,
            },
        );
        Ok(())
    }

    /// The written prefabs, for the undo history.
    pub fn edits(&self) -> &PrefabEdits {
        &self.edits
    }

    /// Rewrites the prefabs that differ from `target` (an undo step's edits);
    /// one that was not written yet goes back to its original content.
    /// Returns the prefabs whose file changed.
    pub fn restore(&mut self, target: &PrefabEdits) -> Result<Vec<String>, String> {
        let root = self.root.clone().ok_or("no project")?;
        let keys: Vec<String> = self.edits.keys().chain(target.keys()).cloned().collect();
        let (mut result, mut changed) = (Ok(()), Vec::new());
        for rel in keys {
            let Some(want) = target.get(&rel).or_else(|| self.originals.get(&rel)) else {
                // Created by the editor and not part of this step: the file stays.
                self.edits.remove(&rel);
                continue;
            };
            let current = self.edits.get(&rel).or_else(|| self.originals.get(&rel));
            if current != Some(want) {
                if let Err(e) = std::fs::write(root.join(&rel), want) {
                    result = Err(format!("{rel}: {e}"));
                    continue;
                }
                let fresh = self.read(&rel);
                self.cache.insert(rel.clone(), fresh);
                changed.push(rel.clone());
            }
            if target.contains_key(&rel) {
                self.edits.insert(rel, want.clone());
            } else {
                self.edits.remove(&rel);
            }
        }
        result.map(|()| changed)
    }

    /// Forgets cached content and edits of the prefabs `gone` matches (moved or deleted).
    pub fn forget(&mut self, gone: &dyn Fn(&String) -> bool) {
        self.cache.retain(|k, _| !gone(k));
        self.edits.retain(|k, _| !gone(k));
        self.originals.retain(|k, _| !gone(k));
    }

    /// `prefabs/<name>.prefab.ron` with a name no file has yet.
    pub fn unique_path(&self, name: &str) -> Option<String> {
        let root = self.root.as_ref()?;
        let base: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let base = if base.trim_matches('_').is_empty() {
            "Prefab".to_owned()
        } else {
            base
        };
        let mut rel = format!("prefabs/{base}.prefab.ron");
        for n in 2.. {
            if !root.join(&rel).exists() {
                break;
            }
            rel = format!("prefabs/{base}_{n}.prefab.ron");
        }
        Some(rel)
    }
}

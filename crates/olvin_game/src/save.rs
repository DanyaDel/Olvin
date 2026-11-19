//! Save games: a layer separate from scene files. A scene describes a level as the
//! editor authored it; a save holds the *runtime* state of a play session. Components
//! and resources are serialized through a registry of types (`name` → serde functions),
//! and files live in slots (`<dir>/<slot>.save.ron`).
//!
//! Only entities that carry [`SaveId`] are saved. References between entities inside
//! components (e.g. `Parent`) are not remapped: store `SaveId`s instead of `Entity`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use olvin_core::{App, Plugin, Resources};
use olvin_ecs::{Component, Entity, World};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The version of the save format this build writes.
pub const SAVE_VERSION: u32 = 1;

/// Upgrades a save one version up: `SAVE_MIGRATIONS[n]` takes version `n + 1` to `n + 2`.
/// The chain is empty while the format is at its first version.
pub const SAVE_MIGRATIONS: &[fn(&mut SaveData)] = &[];

/// Stable key of a saved entity: the same id means the same object after loading.
/// Pick ids yourself (a level object's scene id, or a counter for spawned ones).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SaveId(pub u64);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveMeta {
    /// Shown in the slot list.
    #[serde(default)]
    pub title: String,
    /// The scene the session was in, so the game can load it before restoring.
    #[serde(default)]
    pub scene: String,
    #[serde(default)]
    pub playtime_secs: f64,
    /// Unix time of saving, filled in by [`SaveRegistry::capture`].
    #[serde(default)]
    pub saved_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedEntity {
    pub id: u64,
    /// Registered component name → RON text of its value.
    pub components: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveData {
    pub version: u32,
    #[serde(default)]
    pub meta: SaveMeta,
    #[serde(default)]
    pub entities: Vec<SavedEntity>,
    /// Registered resource name → RON text of its value.
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
}

impl Default for SaveData {
    fn default() -> Self {
        Self {
            version: SAVE_VERSION,
            meta: SaveMeta::default(),
            entities: Vec::new(),
            resources: BTreeMap::new(),
        }
    }
}

impl SaveData {
    /// Reads a save of any supported version and migrates it to [`SAVE_VERSION`].
    pub fn parse(text: &str) -> Result<Self, SaveError> {
        Self::parse_with(text, SAVE_VERSION, SAVE_MIGRATIONS)
    }

    /// [`SaveData::parse`] with an explicit target version and migration chain.
    pub fn parse_with(
        text: &str,
        target: u32,
        migrations: &[fn(&mut SaveData)],
    ) -> Result<Self, SaveError> {
        #[derive(Deserialize)]
        struct Header {
            version: u32,
        }
        let header: Header = ron::from_str(text).map_err(|e| SaveError::Parse(e.to_string()))?;
        if header.version > target || header.version == 0 {
            return Err(SaveError::Version {
                found: header.version,
                supported: target,
            });
        }
        let mut data: SaveData =
            ron::from_str(text).map_err(|e| SaveError::Parse(e.to_string()))?;
        while data.version < target {
            let Some(step) = migrations.get(data.version as usize - 1) else {
                break;
            };
            step(&mut data);
            data.version += 1;
        }
        Ok(data)
    }

    pub fn to_text(&self) -> Result<String, SaveError> {
        let config = ron::ser::PrettyConfig::new().struct_names(false);
        ron::ser::to_string_pretty(self, config).map_err(|e| SaveError::Parse(e.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SaveError {
    Io(String),
    Parse(String),
    /// The save is from a newer build (or has no valid version).
    Version {
        found: u32,
        supported: u32,
    },
    /// A slot name that is empty or could escape the save directory.
    BadSlotName(String),
    NoSuchSlot(String),
    /// A component or resource value that failed to (de)serialize.
    Value {
        name: String,
        error: String,
    },
    /// Two live entities share one [`SaveId`].
    DuplicateId(u64),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) | Self::Parse(e) => f.write_str(e),
            Self::Version { found, supported } => write!(
                f,
                "save version {found} is not supported (this build reads up to {supported})"
            ),
            Self::BadSlotName(n) => write!(f, "bad slot name {n:?}"),
            Self::NoSuchSlot(n) => write!(f, "no save slot {n:?}"),
            Self::Value { name, error } => write!(f, "`{name}`: {error}"),
            Self::DuplicateId(id) => write!(f, "two entities share SaveId({id})"),
        }
    }
}

impl std::error::Error for SaveError {}

/// Applies a parsed value to an entity / the resources; built before anything is
/// changed, so a broken save never leaves the world half-restored.
type EntityApply = Box<dyn FnOnce(&mut World, Entity)>;
type ResourceApply = Box<dyn FnOnce(&Resources)>;

struct ComponentEntry {
    name: String,
    save: fn(&World, Entity) -> Option<Result<String, String>>,
    prepare: fn(&str) -> Result<EntityApply, String>,
    remove: fn(&mut World, Entity),
}

struct ResourceEntry {
    name: String,
    save: fn(&Resources) -> Option<Result<String, String>>,
    prepare: fn(&str) -> Result<ResourceApply, String>,
}

/// Resource: which component and resource types are saved, under which names.
/// Names are the file format: renaming a type in code must not rename it here.
#[derive(Default)]
pub struct SaveRegistry {
    components: Vec<ComponentEntry>,
    resources: Vec<ResourceEntry>,
}

impl SaveRegistry {
    /// Saves component `T` of entities that have a [`SaveId`].
    pub fn register_component<T>(&mut self, name: &str) -> &mut Self
    where
        T: Component + Serialize + DeserializeOwned,
    {
        assert!(
            !self.components.iter().any(|c| c.name == name),
            "save component `{name}` is registered twice"
        );
        self.components.push(ComponentEntry {
            name: name.to_string(),
            save: |world, e| {
                let c = world.get::<&T>(e).ok()?;
                Some(ron::to_string(&*c).map_err(|e| e.to_string()))
            },
            prepare: |text| {
                let value: T = ron::from_str(text).map_err(|e| e.to_string())?;
                Ok(Box::new(move |world, e| {
                    let _ = world.insert_one(e, value);
                }))
            },
            remove: |world, e| {
                let _ = world.remove_one::<T>(e);
            },
        });
        self
    }

    /// Saves the resource `T` (global game state: score, quest flags...).
    pub fn register_resource<T>(&mut self, name: &str) -> &mut Self
    where
        T: 'static + Serialize + DeserializeOwned,
    {
        assert!(
            !self.resources.iter().any(|r| r.name == name),
            "save resource `{name}` is registered twice"
        );
        self.resources.push(ResourceEntry {
            name: name.to_string(),
            save: |res| {
                let r = res.get::<T>()?;
                Some(ron::to_string(&*r).map_err(|e| e.to_string()))
            },
            prepare: |text| {
                let value: T = ron::from_str(text).map_err(|e| e.to_string())?;
                Ok(Box::new(move |res| {
                    *res.fetch_mut::<T>() = value;
                }))
            },
        });
        self
    }

    /// Snapshots every [`SaveId`] entity (sorted by id) and every registered resource
    /// that is present. `meta.saved_at` is set to now.
    pub fn capture(&self, res: &Resources, mut meta: SaveMeta) -> Result<SaveData, SaveError> {
        meta.saved_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let world = res.fetch::<World>();
        let mut ids: Vec<(u64, Entity)> = world
            .query::<(Entity, &SaveId)>()
            .iter()
            .map(|(e, id)| (id.0, e))
            .collect();
        ids.sort_unstable();
        if let Some(w) = ids.windows(2).find(|w| w[0].0 == w[1].0) {
            return Err(SaveError::DuplicateId(w[0].0));
        }
        let mut data = SaveData {
            meta,
            ..Default::default()
        };
        for (id, e) in ids {
            let mut components = BTreeMap::new();
            for c in &self.components {
                if let Some(text) = (c.save)(&world, e) {
                    let text = text.map_err(|error| SaveError::Value {
                        name: c.name.clone(),
                        error,
                    })?;
                    components.insert(c.name.clone(), text);
                }
            }
            data.entities.push(SavedEntity { id, components });
        }
        for r in &self.resources {
            if let Some(text) = (r.save)(res) {
                let text = text.map_err(|error| SaveError::Value {
                    name: r.name.clone(),
                    error,
                })?;
                data.resources.insert(r.name.clone(), text);
            }
        }
        Ok(data)
    }

    /// Makes the world match the save: [`SaveId`] entities are updated or created, those
    /// missing from the save are despawned, other entities stay. Components that are
    /// registered but absent from a saved entity are removed. Everything is parsed
    /// first, so on error nothing changes. Names unknown to the registry are skipped
    /// with a warning (a save from a build with more types).
    pub fn restore(&self, res: &Resources, data: &SaveData) -> Result<(), SaveError> {
        let mut plan: Vec<(u64, Vec<EntityApply>, Vec<usize>)> = Vec::new();
        for saved in &data.entities {
            let mut applies = Vec::new();
            let mut present = Vec::new();
            for (name, text) in &saved.components {
                let Some(i) = self.components.iter().position(|c| &c.name == name) else {
                    log::warn!("save: unknown component `{name}` skipped");
                    continue;
                };
                let apply =
                    (self.components[i].prepare)(text).map_err(|error| SaveError::Value {
                        name: name.clone(),
                        error,
                    })?;
                applies.push(apply);
                present.push(i);
            }
            plan.push((saved.id, applies, present));
        }
        let mut resource_applies = Vec::new();
        for (name, text) in &data.resources {
            let Some(r) = self.resources.iter().find(|r| &r.name == name) else {
                log::warn!("save: unknown resource `{name}` skipped");
                continue;
            };
            resource_applies.push((r.prepare)(text).map_err(|error| SaveError::Value {
                name: name.clone(),
                error,
            })?);
        }

        let mut world = res.fetch_mut::<World>();
        let existing: BTreeMap<u64, Entity> = world
            .query::<(Entity, &SaveId)>()
            .iter()
            .map(|(e, id)| (id.0, e))
            .collect();
        let keep: Vec<u64> = plan.iter().map(|p| p.0).collect();
        for (id, e) in &existing {
            if !keep.contains(id) {
                let _ = world.despawn(*e);
            }
        }
        for (id, applies, present) in plan {
            let e = match existing.get(&id) {
                Some(e) => *e,
                None => world.spawn((SaveId(id),)),
            };
            for (i, c) in self.components.iter().enumerate() {
                if !present.contains(&i) {
                    (c.remove)(&mut world, e);
                }
            }
            for apply in applies {
                apply(&mut world, e);
            }
        }
        drop(world);
        for apply in resource_applies {
            apply(res);
        }
        Ok(())
    }
}

/// One entry of [`SaveSlots::list`].
#[derive(Clone, Debug, PartialEq)]
pub struct SlotInfo {
    pub name: String,
    pub meta: SaveMeta,
}

/// Resource: the directory with save files, one `<slot>.save.ron` each.
#[derive(Clone, Debug)]
pub struct SaveSlots {
    dir: PathBuf,
}

const EXT: &str = ".save.ron";

impl SaveSlots {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, slot: &str) -> Result<PathBuf, SaveError> {
        let ok = !slot.is_empty()
            && !slot.starts_with('.')
            && slot
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' ' | '.'));
        if !ok {
            return Err(SaveError::BadSlotName(slot.to_string()));
        }
        Ok(self.dir.join(format!("{slot}{EXT}")))
    }

    pub fn exists(&self, slot: &str) -> bool {
        self.path(slot).is_ok_and(|p| p.is_file())
    }

    /// Writes the slot through a temp file and a rename, so a crash mid-write
    /// leaves the previous save intact.
    pub fn save(&self, slot: &str, data: &SaveData) -> Result<(), SaveError> {
        let path = self.path(slot)?;
        let text = data.to_text()?;
        std::fs::create_dir_all(&self.dir).map_err(|e| SaveError::Io(e.to_string()))?;
        let tmp = path.with_extension("ron.tmp");
        std::fs::write(&tmp, text).map_err(|e| SaveError::Io(format!("{}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path).map_err(|e| SaveError::Io(format!("{}: {e}", path.display())))
    }

    pub fn load(&self, slot: &str) -> Result<SaveData, SaveError> {
        let path = self.path(slot)?;
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SaveError::NoSuchSlot(slot.to_string()));
            }
            Err(e) => return Err(SaveError::Io(format!("{}: {e}", path.display()))),
        };
        SaveData::parse(&text)
    }

    pub fn delete(&self, slot: &str) -> Result<(), SaveError> {
        let path = self.path(slot)?;
        std::fs::remove_file(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => SaveError::NoSuchSlot(slot.to_string()),
            _ => SaveError::Io(format!("{}: {e}", path.display())),
        })
    }

    /// Slots newest first. Files that fail to parse are skipped (and logged), so one
    /// corrupted save does not hide the others.
    pub fn list(&self) -> Vec<SlotInfo> {
        let Ok(dir) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out: Vec<SlotInfo> = dir
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file = entry.file_name().into_string().ok()?;
                let name = file.strip_suffix(EXT)?.to_string();
                match self.load(&name) {
                    Ok(data) => Some(SlotInfo {
                        name,
                        meta: data.meta,
                    }),
                    Err(e) => {
                        log::warn!("save slot `{name}` skipped: {e}");
                        None
                    }
                }
            })
            .collect();
        out.sort_by(|a, b| {
            b.meta
                .saved_at
                .cmp(&a.meta.saved_at)
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }
}

/// Inserts [`SaveRegistry`] and [`SaveSlots`] for `dir`.
pub struct SavePlugin {
    pub dir: PathBuf,
}

impl SavePlugin {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<World>();
        app.init_resource::<SaveRegistry>();
        app.insert_resource(SaveSlots::new(self.dir.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Health(f32);
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Inventory {
        items: Vec<String>,
        gold: u32,
    }
    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Progress {
        level: u32,
        flags: BTreeMap<String, bool>,
    }
    /// Не зарегистрирован: в сохранение не попадает.
    struct Cache;

    fn registry() -> SaveRegistry {
        let mut r = SaveRegistry::default();
        r.register_component::<Health>("health")
            .register_component::<Inventory>("inventory")
            .register_resource::<Progress>("progress");
        r
    }

    fn game() -> App {
        let mut app = App::new();
        app.add_plugin(SavePlugin::new(temp_dir("unused")));
        *app.resources.fetch_mut::<SaveRegistry>() = registry();
        app.insert_resource(Progress {
            level: 3,
            flags: BTreeMap::from([("door".to_string(), true)]),
        });
        {
            let mut world = app.resources.fetch_mut::<World>();
            world.spawn((
                SaveId(10),
                Health(75.5),
                Inventory {
                    items: vec!["key".into(), "lamp".into()],
                    gold: 12,
                },
                Cache,
            ));
            world.spawn((SaveId(11), Health(10.0)));
            world.spawn((Health(999.0),)); // без SaveId — не сохраняется
        }
        app
    }

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("olvin_save_{name}_{}", std::process::id()))
    }

    fn capture(app: &App) -> SaveData {
        let registry = app.resources.fetch::<SaveRegistry>();
        registry
            .capture(
                &app.resources,
                SaveMeta {
                    title: "Тест".into(),
                    scene: "level1.scene.ron".into(),
                    playtime_secs: 12.5,
                    saved_at: 0,
                },
            )
            .unwrap()
    }

    fn restore(app: &App, data: &SaveData) -> Result<(), SaveError> {
        app.resources
            .fetch::<SaveRegistry>()
            .restore(&app.resources, data)
    }

    #[test]
    fn capture_saves_only_tagged_entities_and_registered_types() {
        let app = game();
        let data = capture(&app);
        assert_eq!(data.version, SAVE_VERSION);
        assert!(data.meta.saved_at > 0);
        assert_eq!(data.entities.len(), 2);
        assert_eq!(data.entities[0].id, 10);
        assert_eq!(
            data.entities[0].components.len(),
            2,
            "Cache не зарегистрирован"
        );
        assert_eq!(
            data.entities[1].components.keys().collect::<Vec<_>>(),
            ["health"]
        );
        assert!(data.resources.contains_key("progress"));
    }

    #[test]
    fn restore_rewrites_updates_spawns_and_despawns() {
        let app = game();
        let data = capture(&app);

        // Игра ушла вперёд: правки, новый объект, потеря сохранённого, чужой объект.
        {
            let world = app.resources.fetch::<World>();
            let e = world
                .query::<(Entity, &SaveId)>()
                .iter()
                .find(|(_, id)| id.0 == 10)
                .unwrap()
                .0;
            world.get::<&mut Health>(e).unwrap().0 = 1.0;
            world.get::<&mut Inventory>(e).unwrap().gold = 0;
        }
        {
            let mut world = app.resources.fetch_mut::<World>();
            world.spawn((SaveId(99), Health(5.0)));
            let gone = world
                .query::<(Entity, &SaveId)>()
                .iter()
                .find(|(_, id)| id.0 == 11)
                .unwrap()
                .0;
            world.despawn(gone).unwrap();
        }
        app.resources.fetch_mut::<Progress>().level = 9;

        restore(&app, &data).unwrap();

        let world = app.resources.fetch::<World>();
        let mut found = BTreeMap::new();
        for (id, h, inv) in world
            .query::<(&SaveId, &Health, Option<&Inventory>)>()
            .iter()
        {
            found.insert(id.0, (h.clone(), inv.cloned()));
        }
        assert_eq!(found.len(), 2, "объект 99 удалён, 11 возвращён: {found:?}");
        assert_eq!(found[&10].0, Health(75.5));
        assert_eq!(found[&10].1.as_ref().unwrap().gold, 12);
        assert_eq!(found[&11], (Health(10.0), None));
        // Объект без SaveId не тронут.
        assert_eq!(world.query::<&Health>().iter().count(), 3);
        assert_eq!(app.resources.fetch::<Progress>().level, 3);
    }

    #[test]
    fn restore_removes_components_missing_from_the_save() {
        let app = game();
        let mut data = capture(&app);
        data.entities[0].components.remove("inventory");
        restore(&app, &data).unwrap();
        let world = app.resources.fetch::<World>();
        assert_eq!(world.query::<&Inventory>().iter().count(), 0);
    }

    #[test]
    fn broken_value_changes_nothing() {
        let app = game();
        let mut data = capture(&app);
        data.resources
            .insert("progress".into(), "(level: ???".into());
        data.entities[0]
            .components
            .insert("health".into(), "{{{".into());
        data.entities.pop();
        let err = restore(&app, &data).unwrap_err();
        assert!(matches!(err, SaveError::Value { .. }), "{err}");
        let world = app.resources.fetch::<World>();
        assert_eq!(
            world.query::<&SaveId>().iter().count(),
            2,
            "ничего не удалено"
        );
        assert_eq!(app.resources.fetch::<Progress>().level, 3);
    }

    #[test]
    fn unknown_names_are_skipped_and_duplicate_ids_rejected() {
        let app = game();
        let mut data = capture(&app);
        data.entities[0]
            .components
            .insert("from_the_future".into(), "(1)".into());
        data.resources.insert("future_res".into(), "()".into());
        restore(&app, &data).unwrap();

        app.resources.fetch_mut::<World>().spawn((SaveId(10),));
        let registry = app.resources.fetch::<SaveRegistry>();
        let err = registry
            .capture(&app.resources, SaveMeta::default())
            .unwrap_err();
        assert_eq!(err, SaveError::DuplicateId(10));
    }

    #[test]
    fn slots_save_load_list_delete() {
        let dir = temp_dir("slots");
        let _ = std::fs::remove_dir_all(&dir);
        let slots = SaveSlots::new(&dir);
        assert!(slots.list().is_empty());
        let app = game();
        let mut data = capture(&app);
        slots.save("autosave", &data).unwrap();
        data.meta.title = "Позже".into();
        data.meta.saved_at += 100;
        slots.save("slot 1", &data).unwrap();

        assert!(slots.exists("autosave"));
        let list = slots.list();
        assert_eq!(
            list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["slot 1", "autosave"],
            "новые первыми"
        );
        assert_eq!(list[0].meta.title, "Позже");
        assert_eq!(slots.load("slot 1").unwrap(), data);
        // Сохранение поверх старого заменяет файл, временных файлов не остаётся.
        slots.save("slot 1", &data).unwrap();
        assert!(
            !std::fs::read_dir(&dir)
                .unwrap()
                .any(|e| { e.unwrap().file_name().to_string_lossy().ends_with(".tmp") })
        );

        // Битый файл не прячет остальные слоты.
        std::fs::write(dir.join("rotten.save.ron"), "(((").unwrap();
        assert_eq!(slots.list().len(), 2);
        assert!(matches!(slots.load("rotten"), Err(SaveError::Parse(_))));

        slots.delete("autosave").unwrap();
        assert_eq!(
            slots.load("autosave"),
            Err(SaveError::NoSuchSlot("autosave".into()))
        );
        assert_eq!(
            slots.delete("autosave"),
            Err(SaveError::NoSuchSlot("autosave".into()))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn slot_names_cannot_escape_the_directory() {
        let slots = SaveSlots::new(temp_dir("names"));
        for bad in ["", "../x", "a/b", ".hidden", "a\\b"] {
            assert_eq!(
                slots.save(bad, &SaveData::default()),
                Err(SaveError::BadSlotName(bad.into())),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn full_cycle_through_disk_restores_a_fresh_world() {
        let dir = temp_dir("cycle");
        let slots = SaveSlots::new(&dir);
        slots.save("a", &capture(&game())).unwrap();

        // «Новая сессия»: пустой мир, значения по умолчанию.
        let mut fresh = App::new();
        fresh.add_plugin(SavePlugin::new(&dir));
        *fresh.resources.fetch_mut::<SaveRegistry>() = registry();
        fresh.insert_resource(Progress::default());
        let data = fresh.resources.fetch::<SaveSlots>().load("a").unwrap();
        assert_eq!(data.meta.scene, "level1.scene.ron");
        restore(&fresh, &data).unwrap();
        assert_eq!(capture(&fresh).entities, data.entities);
        assert!(fresh.resources.fetch::<Progress>().flags["door"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn versions_future_rejected_and_chain_applied() {
        let err = SaveData::parse("(version: 7)").unwrap_err();
        assert_eq!(
            err,
            SaveError::Version {
                found: 7,
                supported: SAVE_VERSION
            }
        );
        assert!(SaveData::parse("(version: 0)").is_err());
        assert!(
            SaveData::parse("(entities: [])").is_err(),
            "версия обязательна"
        );

        fn to2(d: &mut SaveData) {
            d.meta.title.push_str("+2");
        }
        fn to3(d: &mut SaveData) {
            d.meta.title.push_str("+3");
        }
        let chain: &[fn(&mut SaveData)] = &[to2, to3];
        let d = SaveData::parse_with("(version: 1)", 3, chain).unwrap();
        assert_eq!((d.version, d.meta.title.as_str()), (3, "+2+3"));
        let d = SaveData::parse_with("(version: 2)", 3, chain).unwrap();
        assert_eq!(d.meta.title, "+3");
        assert_eq!(SAVE_MIGRATIONS.len() as u32, SAVE_VERSION - 1);
    }

    #[test]
    fn v1_fixture_loads() {
        let text = include_str!("../tests/fixtures/v1.save.ron");
        let data = SaveData::parse(text).unwrap();
        assert_eq!(data.meta.title, "Старое сохранение");
        let app = game();
        restore(&app, &data).unwrap();
        assert_eq!(app.resources.fetch::<Progress>().level, 5);
        let world = app.resources.fetch::<World>();
        let hp: Vec<f32> = world.query::<&Health>().iter().map(|h| h.0).collect();
        assert!(hp.contains(&40.0));
    }
}

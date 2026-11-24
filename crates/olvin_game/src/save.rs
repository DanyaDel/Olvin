//! Save games: a layer separate from scene files. A scene describes a level as the
//! editor authored it; a save holds the *runtime* state of a play session. Components
//! and resources are serialized through a registry of types (`name` → serde functions),
//! and files live in slots (`<dir>/<slot>.save.ron`).
//!
//! Saved entities are the ones with a [`SaveId`] and the level objects spawned from a
//! scene (keyed by their `SceneId`, a separate id space; see
//! [`SaveRegistry::scene_objects`]). References between entities inside components
//! (e.g. `Parent`) are not remapped: store `SaveId`s instead of `Entity`.
//!
//! Engine state comes with [`SaveRegistry::register_engine`] (on in [`SavePlugin`]):
//! `Transform`, and with the `physics` / `script` features body velocities, the
//! character motor and behaviour states. A restored `Transform` is enough for
//! physics: it teleports bodies whose `Transform` was changed at its next step.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use olvin_core::{App, Plugin, Resources};
use olvin_ecs::{Component, Entity, World};
use olvin_scene::{SceneId, Transform};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::save_sign::{SaveSigner, SaveStatus, strip_signature};

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
    /// `id` is a `SceneId` (a level object), not a [`SaveId`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scene: bool,
    /// Registered component name → RON text of its value.
    pub components: BTreeMap<String, String>,
}

/// How a saved entity is found again: its [`SaveId`], or the `SceneId` of a level object.
type Key = (bool, u64);

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
    /// The save's signature does not match its text (`SavePolicy::Enforce`
    /// or `Strict`), or it has no signature (`Strict`).
    Tampered(String),
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
            Self::Tampered(slot) => write!(
                f,
                "save slot `{slot}` was edited (the signature does not match, or is missing)"
            ),
        }
    }
}

impl std::error::Error for SaveError {}

/// Applies a parsed value to an entity / the resources; built before anything is
/// changed, so a broken save never leaves the world half-restored.
type EntityApply = Box<dyn FnOnce(&mut World, Entity)>;
type ExternalApply = Box<dyn FnOnce(&Resources, Entity)>;
type ExternalSave = Box<dyn Fn(&Resources, Entity) -> Option<Result<String, String>>>;
type ExternalPrepare = Box<dyn Fn(&str) -> Result<ExternalApply, String>>;
type ResourceApply = Box<dyn FnOnce(&Resources)>;

struct ComponentEntry {
    name: String,
    save: fn(&World, Entity) -> Option<Result<String, String>>,
    prepare: fn(&str) -> Result<EntityApply, String>,
    remove: fn(&mut World, Entity),
}

/// Per-entity state kept outside the ECS (a physics body, script instances).
struct ExternalEntry {
    name: String,
    save: ExternalSave,
    prepare: ExternalPrepare,
}

struct ResourceEntry {
    name: String,
    save: fn(&Resources) -> Option<Result<String, String>>,
    prepare: fn(&str) -> Result<ResourceApply, String>,
}

/// Resource: which component and resource types are saved, under which names.
/// Names are the file format: renaming a type in code must not rename it here.
pub struct SaveRegistry {
    components: Vec<ComponentEntry>,
    externals: Vec<ExternalEntry>,
    resources: Vec<ResourceEntry>,
    /// Save level objects (entities with a `SceneId` and no [`SaveId`]) under
    /// their scene id. On by default; off, only [`SaveId`] entities are saved
    /// and restoring leaves scene objects alone.
    pub scene_objects: bool,
}

impl Default for SaveRegistry {
    fn default() -> Self {
        Self {
            components: Vec::new(),
            externals: Vec::new(),
            resources: Vec::new(),
            scene_objects: true,
        }
    }
}

impl SaveRegistry {
    fn has_entity_name(&self, name: &str) -> bool {
        self.components.iter().any(|c| c.name == name)
            || self.externals.iter().any(|c| c.name == name)
    }

    /// Saves component `T` of saved entities ([`SaveId`] or scene objects).
    pub fn register_component<T>(&mut self, name: &str) -> &mut Self
    where
        T: Component + Serialize + DeserializeOwned,
    {
        assert!(
            !self.has_entity_name(name),
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

    /// Saves per-entity state that does not live in an ECS component: `read`
    /// returns it at capture (`None`: nothing for this entity), `write` puts it
    /// back after all components are restored (the world is not borrowed then).
    /// For physics bodies, script instances and the like.
    pub fn register_external<T>(
        &mut self,
        name: &str,
        read: fn(&Resources, Entity) -> Option<T>,
        write: fn(&Resources, Entity, T),
    ) -> &mut Self
    where
        T: Serialize + DeserializeOwned + 'static,
    {
        assert!(
            !self.has_entity_name(name),
            "save component `{name}` is registered twice"
        );
        self.externals.push(ExternalEntry {
            name: name.to_string(),
            save: Box::new(move |res, e| {
                let value = read(res, e)?;
                Some(ron::to_string(&value).map_err(|e| e.to_string()))
            }),
            prepare: Box::new(move |text| {
                let value: T = ron::from_str(text).map_err(|e| e.to_string())?;
                Ok(Box::new(move |res, e| write(res, e, value)))
            }),
        });
        self
    }

    /// Built-in engine state, each under a fixed name: `transform`; with the
    /// `physics` feature `motor` (`CharacterMotor`) and `velocity` (linear and
    /// angular velocity of dynamic bodies, set back right away; the restored
    /// `Transform` teleports the body at the next physics step); with `script`
    /// `scripts` ([`olvin_script::Behaviour::save_state`] of every instance).
    /// Names already registered are skipped, so calling it twice is fine.
    pub fn register_engine(&mut self) -> &mut Self {
        if !self.has_entity_name("transform") {
            self.register_component::<Transform>("transform");
        }
        #[cfg(feature = "physics")]
        {
            use olvin_physics::{CharacterMotor, Physics};
            if !self.has_entity_name("motor") {
                self.register_component::<CharacterMotor>("motor");
            }
            if !self.has_entity_name("velocity") {
                self.register_external::<BodyVelocity>(
                    "velocity",
                    |res, e| {
                        let world = res.fetch::<World>();
                        let physics = res.get::<Physics>()?;
                        Some(BodyVelocity {
                            linear: physics.velocity(&world, e)?,
                            angular: physics.angular_velocity(&world, e)?,
                        })
                    },
                    |res, e, v| {
                        let world = res.fetch::<World>();
                        if let Some(mut physics) = res.get_mut::<Physics>() {
                            physics.set_velocity(&world, e, v.linear);
                            physics.set_angular_velocity(&world, e, v.angular);
                        }
                    },
                );
            }
        }
        #[cfg(feature = "script")]
        {
            use olvin_script::{ScriptRuntime, ScriptState};
            if !self.has_entity_name("scripts") {
                self.register_external::<ScriptState>(
                    "scripts",
                    |res, e| {
                        let states = res.get::<ScriptRuntime>()?.save_states(e);
                        (!states.is_empty()).then_some(states)
                    },
                    |res, e, states| {
                        if let Some(mut rt) = res.get_mut::<ScriptRuntime>() {
                            rt.load_states(e, states);
                        }
                    },
                );
            }
        }
        self
    }

    /// Saved entities by key, sorted; two entities with one key are an error.
    fn keyed(&self, world: &World) -> Result<Vec<(Key, Entity)>, SaveError> {
        let keys = self.keys(world);
        if let Some(w) = keys.windows(2).find(|w| w[0].0 == w[1].0) {
            return Err(SaveError::DuplicateId(w[0].0.1));
        }
        Ok(keys)
    }

    fn keys(&self, world: &World) -> Vec<(Key, Entity)> {
        let mut keys: Vec<(Key, Entity)> = world
            .query::<(Entity, &SaveId)>()
            .iter()
            .map(|(e, id)| ((false, id.0), e))
            .collect();
        if self.scene_objects {
            keys.extend(
                world
                    .query::<(Entity, &SceneId)>()
                    .without::<&SaveId>()
                    .iter()
                    .map(|(e, id)| ((true, id.0), e)),
            );
        }
        keys.sort_unstable();
        keys
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

    /// Snapshots every saved entity ([`SaveId`] ones by id, then scene objects by
    /// scene id) and every registered resource that is present. `meta.saved_at`
    /// is set to now.
    pub fn capture(&self, res: &Resources, mut meta: SaveMeta) -> Result<SaveData, SaveError> {
        meta.saved_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let world = res.fetch::<World>();
        let keys = self.keyed(&world)?;
        let mut data = SaveData {
            meta,
            ..Default::default()
        };
        for ((scene, id), e) in keys {
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
            for c in &self.externals {
                if let Some(text) = (c.save)(res, e) {
                    let text = text.map_err(|error| SaveError::Value {
                        name: c.name.clone(),
                        error,
                    })?;
                    components.insert(c.name.clone(), text);
                }
            }
            data.entities.push(SavedEntity {
                id,
                scene,
                components,
            });
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

    /// Makes the world match the save: saved entities are updated, [`SaveId`] ones
    /// missing from the world are created (a missing scene object is skipped with a
    /// warning: load the scene first), saved entities missing from the save are
    /// despawned, other entities stay. Components that are registered but absent
    /// from a saved entity are removed. External state ([`Self::register_external`])
    /// is written after the components. Everything is parsed first, so on error
    /// nothing changes. Names unknown to the registry are skipped with a warning (a
    /// save from a build with more types).
    pub fn restore(&self, res: &Resources, data: &SaveData) -> Result<(), SaveError> {
        type Plan = (Key, Vec<EntityApply>, Vec<usize>, Vec<ExternalApply>);
        let mut plan: Vec<Plan> = Vec::new();
        for saved in &data.entities {
            let mut applies = Vec::new();
            let mut present = Vec::new();
            let mut externals = Vec::new();
            for (name, text) in &saved.components {
                let value = |error| SaveError::Value {
                    name: name.clone(),
                    error,
                };
                if let Some(i) = self.components.iter().position(|c| &c.name == name) {
                    applies.push((self.components[i].prepare)(text).map_err(value)?);
                    present.push(i);
                } else if let Some(x) = self.externals.iter().find(|c| &c.name == name) {
                    externals.push((x.prepare)(text).map_err(value)?);
                } else {
                    log::warn!("save: unknown component `{name}` skipped");
                }
            }
            plan.push(((saved.scene, saved.id), applies, present, externals));
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
        // Duplicates in the world are not an error here: the last one wins.
        let existing: BTreeMap<Key, Entity> = self.keys(&world).into_iter().collect();
        let keep: Vec<Key> = plan.iter().map(|p| p.0).collect();
        for (key, e) in &existing {
            if !keep.contains(key) {
                let _ = world.despawn(*e);
            }
        }
        let mut later: Vec<(Entity, Vec<ExternalApply>)> = Vec::new();
        for (key, applies, present, externals) in plan {
            let e = match (existing.get(&key), key) {
                (Some(e), _) => *e,
                (None, (false, id)) => world.spawn((SaveId(id),)),
                (None, (true, id)) => {
                    log::warn!("save: scene object {id} is not in the world, skipped");
                    continue;
                }
            };
            for (i, c) in self.components.iter().enumerate() {
                if !present.contains(&i) {
                    (c.remove)(&mut world, e);
                }
            }
            for apply in applies {
                apply(&mut world, e);
            }
            later.push((e, externals));
        }
        drop(world);
        for (e, externals) in later {
            for apply in externals {
                apply(res, e);
            }
        }
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
    /// Signs what it writes and checks what it reads, when a key is set.
    signer: Option<SaveSigner>,
}

const EXT: &str = ".save.ron";

impl SaveSlots {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            signer: None,
        }
    }

    /// Signs saves with `signer` and checks them on load (see [`SaveSigner`]:
    /// a signal and a deterrent, not protection).
    pub fn with_signing(mut self, signer: SaveSigner) -> Self {
        self.signer = Some(signer);
        self
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
        let mut text = data.to_text()?;
        if let Some(signed) = self.signer.as_ref().and_then(|s| s.sign(&text)) {
            text = signed;
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| SaveError::Io(e.to_string()))?;
        let tmp = path.with_extension("ron.tmp");
        std::fs::write(&tmp, text).map_err(|e| SaveError::Io(format!("{}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path).map_err(|e| SaveError::Io(format!("{}: {e}", path.display())))
    }

    pub fn load(&self, slot: &str) -> Result<SaveData, SaveError> {
        self.load_checked(slot).map(|(data, _)| data)
    }

    /// Like [`SaveSlots::load`], and says how the save relates to its
    /// signature, so a game can react to [`SaveStatus::Tampered`].
    pub fn load_checked(&self, slot: &str) -> Result<(SaveData, SaveStatus), SaveError> {
        let path = self.path(slot)?;
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SaveError::NoSuchSlot(slot.to_string()));
            }
            Err(e) => return Err(SaveError::Io(format!("{}: {e}", path.display()))),
        };
        let status = self
            .signer
            .as_ref()
            .map_or(SaveStatus::Unchecked, |s| s.check(&text));
        if let Some(signer) = &self.signer {
            if !signer.allows(status) {
                return Err(SaveError::Tampered(slot.to_string()));
            }
            if status == SaveStatus::Tampered {
                log::warn!("save slot `{slot}`: the signature does not match, the file was edited");
            }
        }
        Ok((SaveData::parse(strip_signature(&text))?, status))
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

/// Linear and angular velocity of a dynamic body (`velocity` in saves).
#[cfg(feature = "physics")]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyVelocity {
    pub linear: glam::Vec3,
    pub angular: glam::Vec3,
}

/// Inserts [`SaveRegistry`] and [`SaveSlots`] for `dir`.
pub struct SavePlugin {
    pub dir: PathBuf,
    /// Sign and check saves ([`SavePlugin::signed`]).
    pub signer: Option<SaveSigner>,
    /// Register engine state ([`SaveRegistry::register_engine`]). On in [`SavePlugin::new`].
    pub engine: bool,
}

impl SavePlugin {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            signer: None,
            engine: true,
        }
    }

    /// Signs saves with `signer` and checks them on load.
    pub fn signed(mut self, signer: SaveSigner) -> Self {
        self.signer = Some(signer);
        self
    }

    /// Only the types the game registers itself.
    pub fn without_engine(mut self) -> Self {
        self.engine = false;
        self
    }
}

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<World>();
        app.init_resource::<SaveRegistry>();
        if self.engine {
            app.resources.fetch_mut::<SaveRegistry>().register_engine();
        }
        let mut slots = SaveSlots::new(self.dir.clone());
        if let Some(signer) = &self.signer {
            slots = slots.with_signing(signer.clone());
        }
        app.insert_resource(slots);
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

    #[test]
    fn scene_objects_are_saved_by_scene_id() {
        let app = game();
        let (lamp, door) = {
            let mut world = app.resources.fetch_mut::<World>();
            // Scene id 10 is not SaveId 10: separate id spaces.
            let lamp = world.spawn((SceneId(10), Health(3.0)));
            let door = world.spawn((SceneId(11), Health(4.0)));
            (lamp, door)
        };
        let mut data = capture(&app);
        let scene: Vec<u64> = data
            .entities
            .iter()
            .filter(|e| e.scene)
            .map(|e| e.id)
            .collect();
        assert_eq!(scene, [10, 11]);
        assert!(data.to_text().unwrap().contains("scene: true"));

        app.resources
            .fetch::<World>()
            .get::<&mut Health>(lamp)
            .unwrap()
            .0 = 0.0;
        // The door is gone from the save, an unknown scene object appears in it.
        data.entities.retain(|e| !(e.scene && e.id == 11));
        data.entities.push(SavedEntity {
            id: 99,
            scene: true,
            components: BTreeMap::new(),
        });
        restore(&app, &data).unwrap();
        let world = app.resources.fetch::<World>();
        assert_eq!(world.get::<&Health>(lamp).unwrap().0, 3.0);
        assert!(!world.contains(door), "missing from the save: despawned");
        assert_eq!(
            world.query::<&SceneId>().iter().count(),
            1,
            "a scene object is not created from a save"
        );
    }

    #[test]
    fn external_state_is_written_after_components() {
        #[derive(Default)]
        struct Outside(Vec<(Entity, u32, bool)>);
        let mut app = game();
        app.insert_resource(Outside::default());
        app.resources
            .fetch_mut::<SaveRegistry>()
            .register_external::<u32>(
                "outside",
                |_, _| Some(7),
                |res, e, v| {
                    let has_health = res.fetch::<World>().get::<&Health>(e).is_ok();
                    res.fetch_mut::<Outside>().0.push((e, v, has_health));
                },
            );
        let data = capture(&app);
        assert_eq!(data.entities[0].components["outside"], "7");
        restore(&app, &data).unwrap();
        let outside = app.resources.fetch::<Outside>();
        assert_eq!(outside.0.len(), 2);
        assert!(outside.0.iter().all(|&(_, v, h)| v == 7 && h));
    }

    /// Through the public API only: the engine registrations of `SavePlugin`,
    /// physics following the restored `Transform`s.
    #[cfg(feature = "physics")]
    #[test]
    fn round_trip_with_physics() {
        use glam::Vec3;
        use olvin_core::Time;
        use olvin_physics::{CharacterMotor, Physics, PhysicsPlugin};
        use olvin_scene::file::{
            CharacterControllerDesc, ColliderDesc, ColliderShape, RigidbodyDesc,
        };

        let mut app = App::new();
        app.insert_resource(Time::with_fixed_delta(1.0 / 60.0));
        app.add_plugin(PhysicsPlugin::default())
            .add_plugin(SavePlugin::new(temp_dir("physics")));
        let (crate_, hero) = {
            let mut world = app.resources.fetch_mut::<World>();
            world.spawn((
                Transform::from_xyz(0.0, -0.5, 0.0).with_scale(Vec3::new(40.0, 1.0, 40.0)),
                ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
            ));
            let crate_ = world.spawn((
                SceneId(5),
                Transform::from_xyz(0.0, 0.5, 0.0),
                ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
                RigidbodyDesc::default(),
            ));
            let hero = world.spawn((
                SaveId(1),
                Transform::from_xyz(5.0, 1.0, 0.0),
                CharacterControllerDesc::default(),
            ));
            (crate_, hero)
        };
        let run = |app: &mut App, n: usize| {
            for _ in 0..n {
                app.update();
            }
        };
        let pos = |app: &App, e: Entity| {
            app.resources
                .fetch::<World>()
                .get::<&Transform>(e)
                .unwrap()
                .translation
        };
        let velocity = |app: &App, e: Entity| {
            let world = app.resources.fetch::<World>();
            app.resources
                .fetch::<Physics>()
                .velocity(&world, e)
                .unwrap()
        };
        run(&mut app, 30);
        {
            let world = app.resources.fetch::<World>();
            let mut physics = app.resources.fetch_mut::<Physics>();
            physics.set_velocity(&world, crate_, Vec3::new(0.0, 4.0, 0.0));
            world.get::<&mut CharacterMotor>(hero).unwrap().velocity.y = 3.0;
        }
        run(&mut app, 10);
        let data = capture(&app);
        let (saved_crate, saved_v, saved_hero) =
            (pos(&app, crate_), velocity(&app, crate_), pos(&app, hero));
        assert!(saved_v.y > 1.0, "saved in flight: {saved_v}");

        // Somewhere else, at rest.
        app.resources
            .fetch::<World>()
            .get::<&mut Transform>(hero)
            .unwrap()
            .translation = Vec3::new(-8.0, 1.0, 3.0);
        run(&mut app, 120);
        assert!(pos(&app, crate_).distance(saved_crate) > 0.3);
        assert!(pos(&app, hero).distance(saved_hero) > 5.0);

        restore(&app, &data).unwrap();
        assert!((velocity(&app, crate_) - saved_v).length() < 1e-4);
        run(&mut app, 1);
        let dt = 1.0 / 60.0;
        let crate_now = pos(&app, crate_);
        assert!(
            crate_now.distance(saved_crate + saved_v * dt) < 0.02,
            "{crate_now} vs {saved_crate}"
        );
        assert!(
            pos(&app, hero).distance(saved_hero) < 0.1,
            "{} vs {saved_hero}",
            pos(&app, hero)
        );
        let world = app.resources.fetch::<World>();
        assert!(
            world.get::<&CharacterMotor>(hero).unwrap().velocity.y > 1.0,
            "mid-jump"
        );
    }

    fn signed_slots(dir: &std::path::Path, policy: crate::SavePolicy) -> SaveSlots {
        let key = olvin_core::secret::SecretBytes::new(b"slot test key".to_vec());
        SaveSlots::new(dir).with_signing(SaveSigner::new(key).with_policy(policy))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("olvin-save-sign-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> SaveData {
        SaveData::parse(r#"(version: 1, meta: (title: "t", scene: "s", playtime_secs: 1.0, saved_at: 5), entities: [], resources: {})"#)
            .expect("sample save parses")
    }

    #[test]
    fn signed_saves_round_trip_and_edits_are_caught() {
        let dir = scratch("edit");
        let slots = signed_slots(&dir, crate::SavePolicy::Enforce);
        slots.save("one", &sample()).unwrap();
        let file = dir.join("one.save.ron");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("// olvin-save-hmac: "),
            "signature line written"
        );
        assert_eq!(slots.load_checked("one").unwrap().1, SaveStatus::Valid);

        // An edit with a text editor: the title changes, the signature stays.
        std::fs::write(&file, text.replace("\"t\"", "\"cheat\"")).unwrap();
        assert_eq!(
            slots.load("one"),
            Err(SaveError::Tampered("one".into())),
            "Enforce refuses an edited save"
        );
        // Warn lets it through and says so.
        let warn = signed_slots(&dir, crate::SavePolicy::Warn);
        assert_eq!(warn.load_checked("one").unwrap().1, SaveStatus::Tampered);
    }

    #[test]
    fn unsigned_saves_load_until_strict_and_stripping_the_line_does_not_bypass_strict() {
        let dir = scratch("unsigned");
        // A save from before signing was turned on.
        SaveSlots::new(&dir).save("old", &sample()).unwrap();
        let enforce = signed_slots(&dir, crate::SavePolicy::Enforce);
        assert_eq!(enforce.load_checked("old").unwrap().1, SaveStatus::Unsigned);
        let strict = signed_slots(&dir, crate::SavePolicy::Strict);
        assert_eq!(strict.load("old"), Err(SaveError::Tampered("old".into())));

        // A signed save with its signature line removed is just unsigned.
        strict.save("new", &sample()).unwrap();
        let file = dir.join("new.save.ron");
        let text = std::fs::read_to_string(&file).unwrap();
        let cut = text.rfind("\n// olvin-save-hmac").unwrap();
        std::fs::write(&file, &text[..cut]).unwrap();
        assert_eq!(strict.load("new"), Err(SaveError::Tampered("new".into())));
    }

    #[test]
    fn without_a_key_nothing_changes() {
        let dir = scratch("nokey");
        let slots = SaveSlots::new(&dir);
        slots.save("plain", &sample()).unwrap();
        let text = std::fs::read_to_string(dir.join("plain.save.ron")).unwrap();
        assert!(!text.contains("olvin-save-hmac"));
        assert_eq!(
            slots.load_checked("plain").unwrap().1,
            SaveStatus::Unchecked
        );
    }
}

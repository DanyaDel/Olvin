//! Behaviours written in Lua 5.4 (feature `lua`).
//!
//! A script is a `*.lua` file that returns a table with the same callbacks as
//! [`Behaviour`](crate::Behaviour); the file name is the script name scenes use:
//!
//! ```lua
//! -- assets/scripts/Spinner.lua
//! return {
//!     params = { { name = "speed", kind = "float", default = 90 } },
//!     start = function(self, ctx) self.angle = 0 end,
//!     update = function(self, ctx)
//!         self.angle = self.angle + self.params.speed * ctx:dt()
//!         ctx:rotate_euler_deg(0, self.params.speed * ctx:dt(), 0)
//!     end,
//! }
//! ```
//!
//! Callbacks: `start`, `update`, `fixed_update`, `on_collision_enter/exit`,
//! `on_trigger_enter/exit` (with the other entity's id), `on_destroy`,
//! `on_message(self, ctx, name, value, from)`; the others get
//! `(self, ctx[, other])`. `self` is a table private to the entity, with
//! `self.params` filled from the editor parameters (typed by `params`).
//! Callbacks are read from the script's table when it (re)loads, so setting
//! `self.update = …` at run time does not replace them.
//!
//! The `ctx` API is listed in `ARCHITECTURE.md` (olvin_script → Lua). In short:
//! time, this entity's and other entities' `Transform`, lookups, messages
//! (`ctx:send`, `on_message`) and shared variables (`ctx:var`/`ctx:set_var`,
//! [`ScriptVars`](crate::ScriptVars)), physics (feature `physics`), input
//! (`input`) and sound (`audio`). Vectors are `vec3(x, y, z)` tables with
//! arithmetic (`a + b`, `v * 2`, `v:length()`); every vector argument also
//! takes three numbers, and every vector getter has a `*_xyz` twin that
//! returns three numbers without making a table: the fast path for hot code.
//!
//! Scripts in subfolders of the scripts directory are named by their path
//! (`scripts/fps-controller/player.lua` → `fps-controller/player`).
//!
//! Hot reload: [`LuaScripts::poll`] re-reads changed files. The new callbacks
//! replace the old ones in place, while every entity keeps its own `self`
//! table, so state survives (`start` does not run again). A file that fails to
//! load leaves the running version alone and logs the error.
//!
//! Sandbox: only `table`, `string`, `math`, `utf8` and `coroutine` are loaded
//! (no `io`, `os`, `debug`, `package`; `dofile`/`loadfile`/`require` are
//! removed), memory and instructions per call are limited ([`LuaLimits`]).
//! Sound paths must stay inside the assets folder; messages, variables and
//! sounds started from Lua are capped. A script that errors is skipped, with
//! the error in the log, until its file is reloaded; the frame goes on.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use glam::Vec3;
use mlua::{
    AnyUserData, Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value, VmState,
};
use olvin_core::{App, Plugin, Stage};
use olvin_ecs::Entity;

use crate::{
    Behaviour, ParamKind, ParamSpec, Params, ScriptCtx, ScriptMessage, ScriptPlugin, ScriptRegistry,
};

mod api;
#[cfg(feature = "audio")]
mod audio;
#[cfg(feature = "net")]
mod net;
#[cfg(feature = "net")]
pub use net::LuaNetPlugin;

use api::{CtxHandle, CtxSlot, entity_id, from_script_value, vec3_of};

/// How much a script may use per callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LuaLimits {
    /// Bytes the whole Lua state may hold. 0 means no limit.
    pub memory: usize,
    /// Lua VM instructions per callback. 0 means no limit.
    pub instructions: u32,
}

impl Default for LuaLimits {
    fn default() -> Self {
        Self {
            memory: 64 << 20,
            instructions: 5_000_000,
        }
    }
}

/// The hook runs every this many instructions.
const HOOK_STEP: u32 = 1000;

/// The callbacks a script table may define, in [`Cb`] order.
const CALLBACKS: [&str; 10] = [
    "start",
    "update",
    "fixed_update",
    "on_collision_enter",
    "on_collision_exit",
    "on_trigger_enter",
    "on_trigger_exit",
    "on_destroy",
    "on_message",
    "on_net_message",
];

#[derive(Clone, Copy)]
enum Cb {
    Start,
    Update,
    FixedUpdate,
    CollisionEnter,
    CollisionExit,
    TriggerEnter,
    TriggerExit,
    Destroy,
    Message,
    NetMessage,
}

impl Cb {
    fn name(self) -> &'static str {
        CALLBACKS[self as usize]
    }
}

/// A script's callbacks, looked up once per (re)load instead of by name on
/// every call. Shared by the script's instances; a reload refills it.
#[derive(Default)]
struct Callbacks([Option<Function>; CALLBACKS.len()]);

impl Callbacks {
    fn read(table: &Table) -> mlua::Result<Self> {
        let mut out = Self::default();
        for (slot, name) in out.0.iter_mut().zip(CALLBACKS) {
            *slot = match table.raw_get::<Value>(name)? {
                Value::Nil => None,
                Value::Function(f) => Some(f),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "`{name}` must be a function, not {}",
                        other.type_name()
                    )));
                }
            };
        }
        Ok(out)
    }

    fn get(&self, cb: Cb) -> Option<&Function> {
        self.0[cb as usize].as_ref()
    }
}

/// Lua code run once per state: the vector type and the vector API of `ctx`.
const PRELUDE: &str = include_str!("prelude.lua");

// --- Loaded scripts ------------------------------------------------------

struct Entry {
    path: Option<PathBuf>,
    modified: Option<SystemTime>,
    /// The script table of the current version; instances read fields through `__index`.
    proto: Table,
    callbacks: Rc<RefCell<Callbacks>>,
    /// Bumped on every successful reload; clears the "failed" state of instances.
    generation: Rc<Cell<u32>>,
}

struct Shared {
    lua: Lua,
    slot: Rc<CtxSlot>,
    ctx: AnyUserData,
    /// The prelude's `vec3` constructor.
    vec3: Function,
    /// Instructions run in the current callback, counted by the hook.
    spent: Rc<Cell<u32>>,
    limits: LuaLimits,
    entries: RefCell<BTreeMap<String, Entry>>,
}

/// Lua scripts of a game: the interpreter and what was loaded into it. A
/// resource; [`LuaPlugin`] sets it up.
pub struct LuaScripts {
    shared: Rc<Shared>,
    dirs: Vec<PathBuf>,
    last_poll: Option<Instant>,
    /// Minimum time between disk checks in [`LuaScripts::poll`].
    pub poll_interval: Duration,
}

impl LuaScripts {
    pub fn new() -> mlua::Result<Self> {
        Self::with_limits(LuaLimits::default())
    }

    pub fn with_limits(limits: LuaLimits) -> mlua::Result<Self> {
        let libs = StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        let globals = lua.globals();
        for name in ["dofile", "loadfile", "require"] {
            globals.set(name, Value::Nil)?;
        }
        globals.set(
            "print",
            lua.create_function(|_, args: MultiValue| {
                let mut line = String::new();
                for (i, v) in args.iter().enumerate() {
                    if i > 0 {
                        line.push('\t');
                    }
                    let _ = match v.to_string() {
                        Ok(s) => write!(line, "{s}"),
                        Err(_) => write!(line, "<{}>", v.type_name()),
                    };
                }
                log::info!(target: "script", "{line}");
                Ok(())
            })?,
        )?;

        let slot = Rc::new(CtxSlot::default());
        let ctx = lua.create_userdata(CtxHandle {
            slot: slot.clone(),
            vec3: None,
        })?;
        // Only Rust methods, so `__index` is the plain methods table: the
        // prelude adds the vector API to it in Lua.
        let methods: Table = ctx.metatable()?.get("__index")?;
        let vec3: Function = lua.load(PRELUDE).set_name("=olvin prelude").call(methods)?;
        ctx.borrow_mut::<CtxHandle>()?.vec3 = Some(vec3.clone());

        if limits.memory > 0 {
            lua.set_memory_limit(limits.memory)?;
        }
        let spent = Rc::new(Cell::new(0u32));
        if limits.instructions > 0 {
            let (counter, max) = (spent.clone(), limits.instructions);
            lua.set_hook(
                HookTriggers::new().every_nth_instruction(HOOK_STEP),
                move |_, _| {
                    let total = counter.get().saturating_add(HOOK_STEP);
                    counter.set(total);
                    if total > max {
                        Err(mlua::Error::runtime(format!(
                            "script ran more than {max} instructions in one callback"
                        )))
                    } else {
                        Ok(VmState::Continue)
                    }
                },
            )?;
        }
        Ok(Self {
            shared: Rc::new(Shared {
                lua,
                slot,
                ctx,
                vec3,
                spent,
                limits,
                entries: RefCell::default(),
            }),
            dirs: Vec::new(),
            last_poll: None,
            poll_interval: Duration::from_millis(250),
        })
    }

    pub fn limits(&self) -> LuaLimits {
        self.shared.limits
    }

    /// Bytes the Lua state holds right now.
    pub fn used_memory(&self) -> usize {
        self.shared.lua.used_memory()
    }

    /// Names of the loaded scripts.
    pub fn names(&self) -> Vec<String> {
        self.shared.entries.borrow().keys().cloned().collect()
    }

    /// Loads (or replaces) the script `name` from Lua source. Not watched on disk.
    pub fn load_source(
        &mut self,
        registry: &mut ScriptRegistry,
        name: &str,
        source: &str,
    ) -> Result<(), String> {
        self.install(registry, name, None, None, source, name)
    }

    /// Loads `path` as a script named after the file (`Spinner.lua` → `Spinner`).
    pub fn load_file(&mut self, registry: &mut ScriptRegistry, path: &Path) -> Result<(), String> {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("{}: no file name", path.display()))?
            .to_owned();
        self.load_file_as(registry, path, &name)
    }

    fn load_file_as(
        &mut self,
        registry: &mut ScriptRegistry,
        path: &Path,
        name: &str,
    ) -> Result<(), String> {
        let source = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let modified = fs::metadata(path).and_then(|m| m.modified()).ok();
        let chunk = path.display().to_string();
        self.install(
            registry,
            name,
            Some(path.to_owned()),
            modified,
            &source,
            &chunk,
        )
    }

    /// Loads every `*.lua` in `dir` and its subfolders and watches the folder
    /// for new files. A file in `dir` itself is named after the file
    /// (`Spinner`), one in a subfolder by its path (`fps-controller/player`, as
    /// library packs ship them). Files that fail to load are logged and skipped.
    pub fn load_dir(&mut self, registry: &mut ScriptRegistry, dir: &Path) {
        if !self.dirs.iter().any(|d| d == dir) {
            self.dirs.push(dir.to_owned());
        }
        for path in lua_files(dir) {
            let name = script_name(dir, &path);
            if let Err(e) = self.load_file_as(registry, &path, &name) {
                log::error!("lua: {e}");
            }
        }
    }

    /// Checks the loaded files and watched folders on disk (at most once per
    /// `poll_interval`); reloads what changed and loads new files. Returns the
    /// names that were (re)loaded. Errors are logged, the old version stays.
    pub fn poll(&mut self, registry: &mut ScriptRegistry) -> Vec<String> {
        if let Some(last) = self.last_poll
            && last.elapsed() < self.poll_interval
        {
            return Vec::new();
        }
        self.last_poll = Some(Instant::now());
        self.check_disk(registry)
    }

    /// Like [`poll`](Self::poll) without the interval.
    pub fn check_disk(&mut self, registry: &mut ScriptRegistry) -> Vec<String> {
        let changed: Vec<PathBuf> = self
            .shared
            .entries
            .borrow()
            .values()
            .filter_map(|e| {
                let path = e.path.as_ref()?;
                let now = fs::metadata(path).and_then(|m| m.modified()).ok()?;
                (Some(now) != e.modified).then(|| path.clone())
            })
            .collect();
        let known: Vec<PathBuf> = self
            .shared
            .entries
            .borrow()
            .values()
            .filter_map(|e| e.path.clone())
            .collect();
        let named = |entries: &BTreeMap<String, Entry>, path: &PathBuf| {
            entries
                .iter()
                .find(|(_, e)| e.path.as_ref() == Some(path))
                .map(|(name, _)| name.clone())
        };
        let mut jobs: Vec<(String, PathBuf)> = {
            let entries = self.shared.entries.borrow();
            changed
                .into_iter()
                .filter_map(|p| Some((named(&entries, &p)?, p)))
                .collect()
        };
        for dir in &self.dirs {
            for path in lua_files(dir) {
                if !known.contains(&path) {
                    jobs.push((script_name(dir, &path), path));
                }
            }
        }
        let mut reloaded = Vec::new();
        for (name, path) in jobs {
            match self.load_file_as(registry, &path, &name) {
                Ok(()) => {
                    log::info!("lua: reloaded `{name}`");
                    reloaded.push(name);
                }
                Err(e) => {
                    // Remember the stamp so a broken file is reported once, not every poll.
                    if let Some(entry) = self.shared.entries.borrow_mut().get_mut(&name) {
                        entry.modified = fs::metadata(&path).and_then(|m| m.modified()).ok();
                    }
                    log::error!("lua: {e}; keeping the previous version");
                }
            }
        }
        reloaded
    }

    fn install(
        &mut self,
        registry: &mut ScriptRegistry,
        name: &str,
        path: Option<PathBuf>,
        modified: Option<SystemTime>,
        source: &str,
        chunk: &str,
    ) -> Result<(), String> {
        let lua = &self.shared.lua;
        self.shared.spent.set(0);
        let table: Table = lua
            .load(source)
            .set_name(format!("@{chunk}"))
            .eval()
            .map_err(|e| format!("{chunk}: {e}"))?;
        let schema = read_schema(&table).map_err(|e| format!("{chunk}: {e}"))?;
        let callbacks = Callbacks::read(&table).map_err(|e| format!("{chunk}: {e}"))?;

        let mut entries = self.shared.entries.borrow_mut();
        let (proto, shared_callbacks, generation) = match entries.get(name) {
            Some(old) => {
                // Same table, new callbacks: instances keep pointing at it.
                old.proto.clear().map_err(|e| e.to_string())?;
                old.generation.set(old.generation.get() + 1);
                *old.callbacks.borrow_mut() = callbacks;
                (
                    old.proto.clone(),
                    old.callbacks.clone(),
                    old.generation.clone(),
                )
            }
            None => (
                lua.create_table().map_err(|e| e.to_string())?,
                Rc::new(RefCell::new(callbacks)),
                Rc::new(Cell::new(0)),
            ),
        };
        for pair in table.pairs::<Value, Value>() {
            let (k, v) = pair.map_err(|e| e.to_string())?;
            proto.raw_set(k, v).map_err(|e| e.to_string())?;
        }
        entries.insert(
            name.to_owned(),
            Entry {
                path,
                modified,
                proto: proto.clone(),
                callbacks: shared_callbacks.clone(),
                generation: generation.clone(),
            },
        );
        drop(entries);

        let shared = self.shared.clone();
        let script = name.to_owned();
        let specs = schema.clone();
        registry.register_fn(name, schema, move |params| {
            Box::new(LuaBehaviour::new(
                shared.clone(),
                &script,
                &proto,
                &specs,
                shared_callbacks.clone(),
                generation.clone(),
                params,
            ))
        });
        Ok(())
    }
}

/// Every `*.lua` under `dir`, subfolders included, in a stable order.
fn lua_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "lua") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    files
}

/// The name scenes use: the file stem for a file directly in `dir`, else the
/// path under `dir` without `.lua`, with `/` (`fps-controller/player`).
fn script_name(dir: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(dir).unwrap_or(path).with_extension("");
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// `params = { { name = "speed", kind = "float", default = 1, doc = "…" }, … }`
fn read_schema(table: &Table) -> mlua::Result<Vec<ParamSpec>> {
    let Some(list) = table.get::<Option<Table>>("params")? else {
        return Ok(Vec::new());
    };
    let mut specs = Vec::new();
    for item in list.sequence_values::<Table>() {
        let item = item?;
        let name: String = item.get("name")?;
        let kind = match item
            .get::<Option<String>>("kind")?
            .unwrap_or_else(|| "string".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "float" | "number" => ParamKind::Float,
            "int" | "integer" => ParamKind::Int,
            "bool" | "boolean" => ParamKind::Bool,
            "vec3" => ParamKind::Vec3,
            "string" => ParamKind::String,
            "entity" => ParamKind::Entity,
            other => {
                return Err(mlua::Error::runtime(format!(
                    "param `{name}`: unknown kind `{other}`"
                )));
            }
        };
        let default = match item.get::<Value>("default")? {
            Value::Nil => String::new(),
            Value::Table(t) => crate::format_vec3(vec3_of(&t)?),
            other => other.to_string()?,
        };
        let doc = item.get::<Option<String>>("doc")?.unwrap_or_default();
        specs.push(ParamSpec::new(&name, kind, default, &doc));
    }
    Ok(specs)
}

// --- The behaviour -------------------------------------------------------

struct LuaBehaviour {
    shared: Rc<Shared>,
    script: String,
    /// The entity's private `self`; `None` if it could not be built.
    state: Option<Table>,
    callbacks: Rc<RefCell<Callbacks>>,
    generation: Rc<Cell<u32>>,
    /// Generation at which a callback failed; skipped until the next reload.
    failed_at: Option<u32>,
}

impl LuaBehaviour {
    fn new(
        shared: Rc<Shared>,
        script: &str,
        proto: &Table,
        specs: &[ParamSpec],
        callbacks: Rc<RefCell<Callbacks>>,
        generation: Rc<Cell<u32>>,
        params: &Params,
    ) -> Self {
        let state = Self::make_state(&shared, proto, specs, params)
            .map_err(|e| log::error!("lua script `{script}`: could not create its state: {e}"))
            .ok();
        Self {
            shared,
            script: script.to_owned(),
            state,
            callbacks,
            generation,
            failed_at: None,
        }
    }

    fn make_state(
        shared: &Shared,
        proto: &Table,
        specs: &[ParamSpec],
        params: &Params,
    ) -> mlua::Result<Table> {
        let lua = &shared.lua;
        let state = lua.create_table()?;
        let typed = lua.create_table()?;
        for spec in specs {
            let raw = params.raw(&spec.name).unwrap_or(&spec.default);
            let value = match spec.kind {
                ParamKind::Float => Value::Number(
                    params
                        .f32(&spec.name, spec.default.parse().unwrap_or(0.0))
                        .into(),
                ),
                ParamKind::Int => Value::Integer(
                    params
                        .i32(&spec.name, spec.default.parse().unwrap_or(0))
                        .into(),
                ),
                ParamKind::Bool => Value::Boolean(params.bool(
                    &spec.name,
                    crate::parse_bool(&spec.default).unwrap_or(false),
                )),
                ParamKind::Vec3 => {
                    let fallback = crate::parse_vec3(&spec.default).unwrap_or(Vec3::ZERO);
                    let v = params.vec3(&spec.name, fallback);
                    shared.vec3.call((v.x, v.y, v.z))?
                }
                ParamKind::String => Value::String(lua.create_string(raw)?),
                ParamKind::Entity => params
                    .entity(&spec.name)
                    .map_or(Value::Nil, |e| Value::Integer(entity_id(e) as i64)),
            };
            typed.set(spec.name.as_str(), value)?;
        }
        state.set("params", typed)?;
        let meta = lua.create_table()?;
        meta.set("__index", proto)?;
        state.set_metatable(Some(meta))?;
        Ok(state)
    }

    /// Calls the callback `cb` with `(self, ctx, args...)` when the script defines it.
    fn call(&mut self, ctx: &mut ScriptCtx<'_>, cb: Cb, args: impl mlua::IntoLuaMulti) {
        let Some(state) = &self.state else { return };
        if self.failed_at == Some(self.generation.get()) {
            return;
        }
        let callbacks = self.callbacks.borrow();
        let Some(f) = callbacks.get(cb) else { return };
        let shared = &*self.shared;
        let result = {
            let _lent = shared.slot.lend(ctx);
            shared.spent.set(0);
            f.call::<()>((state, &shared.ctx, args))
        };
        if let Err(e) = result {
            log::error!(
                target: "script",
                "lua script `{}` failed in {}: {e}; skipped until it is reloaded",
                self.script,
                cb.name()
            );
            self.failed_at = Some(self.generation.get());
        }
    }

    fn call_with(&mut self, ctx: &mut ScriptCtx<'_>, cb: Cb, other: Entity) {
        self.call(ctx, cb, entity_id(other));
    }
}

impl Behaviour for LuaBehaviour {
    fn start(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.call(ctx, Cb::Start, ());
    }
    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.call(ctx, Cb::Update, ());
    }
    fn fixed_update(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.call(ctx, Cb::FixedUpdate, ());
    }
    fn on_collision_enter(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.call_with(ctx, Cb::CollisionEnter, other);
    }
    fn on_collision_exit(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.call_with(ctx, Cb::CollisionExit, other);
    }
    fn on_trigger_enter(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.call_with(ctx, Cb::TriggerEnter, other);
    }
    fn on_trigger_exit(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.call_with(ctx, Cb::TriggerExit, other);
    }
    fn on_destroy(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.call(ctx, Cb::Destroy, ());
    }
    fn on_message(&mut self, ctx: &mut ScriptCtx<'_>, message: &ScriptMessage) {
        // A message that came over the network is for `on_net_message` only.
        #[cfg(feature = "net")]
        if let Some((from, name)) = net::split_net_name(&message.name) {
            let shared = self.shared.clone();
            let lua = &shared.lua;
            if let Ok(args) = (|| {
                Ok::<_, mlua::Error>((
                    lua.create_string(name)?,
                    from_script_value(lua, &shared.vec3, &message.value)?,
                    from,
                ))
            })() {
                self.call(ctx, Cb::NetMessage, args);
            }
            return;
        }
        if self.callbacks.borrow().get(Cb::Message).is_none() {
            return;
        }
        let shared = self.shared.clone();
        let lua = &shared.lua;
        let args = (|| {
            Ok::<_, mlua::Error>((
                lua.create_string(&message.name)?,
                from_script_value(lua, &shared.vec3, &message.value)?,
                message.from.map(entity_id),
            ))
        })();
        match args {
            Ok(args) => self.call(ctx, Cb::Message, args),
            Err(e) => log::error!(
                target: "script",
                "lua script `{}`: message `{}` could not be passed: {e}",
                self.script,
                message.name
            ),
        }
    }
}

// --- Plugin --------------------------------------------------------------

/// Loads `*.lua` from `dir` into the [`ScriptRegistry`] and, with `hot_reload`,
/// re-reads changed files while the game runs. Adds [`ScriptPlugin`] if it is
/// not there yet.
pub struct LuaPlugin {
    pub dir: PathBuf,
    pub hot_reload: bool,
    pub limits: LuaLimits,
}

impl LuaPlugin {
    /// Scripts from `dir`, hot reload on in debug builds.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            hot_reload: cfg!(debug_assertions),
            limits: LuaLimits::default(),
        }
    }
}

impl Plugin for LuaPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(ScriptPlugin::default());
        let mut scripts = match LuaScripts::with_limits(self.limits) {
            Ok(s) => s,
            Err(e) => {
                log::error!("lua: could not start the interpreter: {e}");
                return;
            }
        };
        scripts.load_dir(&mut app.resources.fetch_mut::<ScriptRegistry>(), &self.dir);
        app.insert_resource(scripts);
        if self.hot_reload {
            app.add_system(Stage::PreUpdate, |res| {
                let (Some(mut scripts), Some(mut registry)) =
                    (res.get_mut::<LuaScripts>(), res.get_mut::<ScriptRegistry>())
                else {
                    return;
                };
                scripts.poll(&mut registry);
            });
        }
    }
}

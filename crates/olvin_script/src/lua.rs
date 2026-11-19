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
//!         ctx:rotate_euler_deg(vec3(0, self.params.speed * ctx:dt(), 0))
//!     end,
//! }
//! ```
//!
//! Callbacks: `start`, `update`, `fixed_update`, `on_collision_enter/exit`,
//! `on_trigger_enter/exit` (with the other entity's id), `on_destroy`; each gets
//! `(self, ctx[, other])`. `self` is a table private to the entity, with
//! `self.params` filled from the editor parameters (typed by `params`).
//!
//! Hot reload: [`LuaScripts::poll`] re-reads changed files. The new callbacks
//! replace the old ones in place, while every entity keeps its own `self`
//! table, so state survives (`start` does not run again). A file that fails to
//! load leaves the running version alone and logs the error.
//!
//! Sandbox: only `table`, `string`, `math`, `utf8` and `coroutine` are loaded
//! (no `io`, `os`, `debug`, `package`; `dofile`/`loadfile`/`require` are
//! removed), memory and instructions per call are limited ([`LuaLimits`]). A
//! script that errors is skipped, with the error in the log, until its file
//! is reloaded; the frame goes on.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use glam::{EulerRot, Quat, Vec3};
use mlua::{
    AnyUserData, Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, UserData,
    UserDataMethods, Value, VmState,
};
use olvin_core::{App, Plugin, Stage};
use olvin_ecs::Entity;

use crate::{Behaviour, ParamKind, ParamSpec, Params, ScriptCtx, ScriptPlugin, ScriptRegistry};

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

fn entity_id(e: Entity) -> u64 {
    e.to_bits().get()
}

fn entity_from(id: u64) -> Option<Entity> {
    Entity::from_bits(id)
}

// --- The context handle --------------------------------------------------

/// Where the current callback's `ScriptCtx` is lent to Lua. Null outside callbacks.
#[derive(Default)]
struct CtxSlot {
    ptr: Cell<Option<std::ptr::NonNull<ScriptCtx<'static>>>>,
}

impl CtxSlot {
    fn with<R>(&self, f: impl FnOnce(&mut ScriptCtx<'_>) -> R) -> mlua::Result<R> {
        let Some(mut ptr) = self.ptr.get() else {
            return Err(mlua::Error::runtime(
                "ctx is only valid inside the callback it was passed to",
            ));
        };
        // SAFETY: the pointer is set by `LuaBehaviour::call` from a live
        // `&mut ScriptCtx` and cleared (by a guard) before that call returns.
        // The callback does not touch the original reference meanwhile, and
        // `f` finishes before Lua runs again, so the borrows never overlap.
        Ok(f(unsafe { ptr.as_mut() }))
    }
}

/// Clears the lent context when the callback ends, even on unwind.
struct Lent<'a>(&'a CtxSlot);

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        self.0.ptr.set(None);
    }
}

struct Vec3Arg(Vec3);

impl mlua::FromLua for Vec3Arg {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        let Value::Table(t) = value else {
            return Err(mlua::Error::runtime(
                "expected a vector: vec3(x, y, z) or {x=, y=, z=}",
            ));
        };
        vec3_of(&t).map(Self)
    }
}

/// `{x=, y=, z=}` or `{x, y, z}`; missing axes are 0.
fn vec3_of(t: &Table) -> mlua::Result<Vec3> {
    let axis = |name: &str, index: i64| -> mlua::Result<f32> {
        let v: Value = t.get(name)?;
        let v = if v.is_nil() { t.get(index)? } else { v };
        Ok(match v {
            Value::Integer(i) => i as f32,
            Value::Number(n) => n as f32,
            _ => 0.0,
        })
    };
    Ok(Vec3::new(axis("x", 1)?, axis("y", 2)?, axis("z", 3)?))
}

fn vec3_table(lua: &Lua, v: Vec3) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 3)?;
    t.set("x", v.x)?;
    t.set("y", v.y)?;
    t.set("z", v.z)?;
    Ok(t)
}

struct CtxHandle(Rc<CtxSlot>);

impl UserData for CtxHandle {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // Identity and time.
        m.add_method("entity", |_, h, ()| h.0.with(|c| entity_id(c.entity())));
        m.add_method("name", |_, h, ()| h.0.with(|c| c.name()));
        m.add_method("dt", |_, h, ()| h.0.with(|c| c.dt()));
        m.add_method("fixed_dt", |_, h, ()| h.0.with(|c| c.fixed_dt()));
        m.add_method("elapsed", |_, h, ()| h.0.with(|c| c.elapsed()));
        m.add_method("frame", |_, h, ()| h.0.with(|c| c.time().frame));
        m.add_method("log", |_, h, msg: String| h.0.with(|c| c.log(msg)));
        m.add_method("warn", |_, h, msg: String| h.0.with(|c| c.warn(msg)));

        // Transform of this entity.
        m.add_method("position", |lua, h, ()| {
            vec3_table(lua, h.0.with(|c| c.position())?)
        });
        m.add_method("world_position", |lua, h, ()| {
            vec3_table(lua, h.0.with(|c| c.world_position())?)
        });
        m.add_method("set_position", |_, h, v: Vec3Arg| {
            h.0.with(|c| c.set_position(v.0))
        });
        m.add_method("translate", |_, h, v: Vec3Arg| {
            h.0.with(|c| c.translate(v.0))
        });
        m.add_method("translate_local", |_, h, v: Vec3Arg| {
            h.0.with(|c| c.translate_local(v.0))
        });
        m.add_method("rotate_euler_deg", |_, h, v: Vec3Arg| {
            h.0.with(|c| c.rotate_euler_deg(v.0))
        });
        m.add_method("rotation_euler_deg", |lua, h, ()| {
            let (y, x, z) = h.0.with(|c| c.rotation().to_euler(EulerRot::YXZ))?;
            vec3_table(lua, Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI))
        });
        m.add_method("set_rotation_euler_deg", |_, h, v: Vec3Arg| {
            h.0.with(|c| {
                let r = v.0 * (std::f32::consts::PI / 180.0);
                c.set_rotation(Quat::from_euler(EulerRot::YXZ, r.y, r.x, r.z));
            })
        });
        m.add_method("scale", |lua, h, ()| {
            vec3_table(lua, h.0.with(|c| c.transform().scale)?)
        });
        m.add_method("set_scale", |_, h, v: Vec3Arg| {
            h.0.with(|c| c.with_transform(|t| t.scale = v.0))
        });
        m.add_method(
            "look_at",
            |_, h, (target, up): (Vec3Arg, Option<Vec3Arg>)| {
                h.0.with(|c| c.look_at(target.0, up.map_or(Vec3::Y, |u| u.0)))
            },
        );
        m.add_method("forward", |lua, h, ()| {
            vec3_table(lua, h.0.with(|c| c.forward())?)
        });
        m.add_method("right", |lua, h, ()| {
            vec3_table(lua, h.0.with(|c| c.right())?)
        });
        m.add_method("up", |lua, h, ()| vec3_table(lua, h.0.with(|c| c.up())?));

        // Other entities.
        m.add_method("find_by_name", |_, h, name: String| {
            h.0.with(|c| c.find_by_name(&name).map(entity_id))
        });
        m.add_method("find_by_tag", |_, h, tag: String| {
            h.0.with(|c| c.find_by_tag(&tag).map(entity_id))
        });
        m.add_method("find_all_by_tag", |_, h, tag: String| {
            h.0.with(|c| {
                c.find_all_by_tag(&tag)
                    .into_iter()
                    .map(entity_id)
                    .collect::<Vec<_>>()
            })
        });
        m.add_method("alive", |_, h, id: u64| {
            h.0.with(|c| entity_from(id).is_some_and(|e| c.world().contains(e)))
        });
        m.add_method("world_position_of", |lua, h, id: u64| {
            let p =
                h.0.with(|c| entity_from(id).map(|e| c.world_position_of(e)))?;
            p.map(|p| vec3_table(lua, p)).transpose()
        });
        m.add_method("despawn", |_, h, id: u64| {
            h.0.with(|c| {
                if let Some(e) = entity_from(id) {
                    c.despawn(e);
                }
            })
        });
        m.add_method("destroy_self", |_, h, ()| h.0.with(|c| c.destroy_self()));

        #[cfg(feature = "physics")]
        add_physics(m);
        #[cfg(feature = "input")]
        add_input(m);
    }
}

#[cfg(feature = "physics")]
fn add_physics<M: UserDataMethods<CtxHandle>>(m: &mut M) {
    m.add_method("apply_force", |_, h, v: Vec3Arg| {
        h.0.with(|c| c.apply_force(v.0))
    });
    m.add_method("apply_impulse", |_, h, v: Vec3Arg| {
        h.0.with(|c| c.apply_impulse(v.0))
    });
    m.add_method("set_velocity", |_, h, v: Vec3Arg| {
        h.0.with(|c| c.set_velocity(v.0))
    });
    m.add_method("velocity", |lua, h, ()| {
        h.0.with(|c| c.velocity())?
            .map(|v| vec3_table(lua, v))
            .transpose()
    });
    // Returns nil or `{ entity, point, normal, distance }`.
    m.add_method(
        "raycast",
        |lua, h, (origin, dir, max): (Vec3Arg, Vec3Arg, f32)| {
            let Some(hit) = h.0.with(|c| c.raycast(origin.0, dir.0, max))? else {
                return Ok(None);
            };
            let t = lua.create_table()?;
            t.set("entity", entity_id(hit.entity))?;
            t.set("point", vec3_table(lua, hit.point)?)?;
            t.set("normal", vec3_table(lua, hit.normal)?)?;
            t.set("distance", hit.distance)?;
            Ok(Some(t))
        },
    );
}

#[cfg(feature = "input")]
fn add_input<M: UserDataMethods<CtxHandle>>(m: &mut M) {
    m.add_method("pressed", |_, h, a: String| {
        h.0.with(|c| c.input().pressed(&a))
    });
    m.add_method("just_pressed", |_, h, a: String| {
        h.0.with(|c| c.input().just_pressed(&a))
    });
    m.add_method("just_released", |_, h, a: String| {
        h.0.with(|c| c.input().just_released(&a))
    });
    m.add_method("value", |_, h, a: String| h.0.with(|c| c.input().value(&a)));
    m.add_method("axis2d", |lua, h, a: String| {
        let v = h.0.with(|c| c.input().axis2d(&a))?;
        let t = lua.create_table_with_capacity(0, 2)?;
        t.set("x", v.x)?;
        t.set("y", v.y)?;
        Ok(t)
    });
}

// --- Loaded scripts ------------------------------------------------------

struct Entry {
    path: Option<PathBuf>,
    modified: Option<SystemTime>,
    /// Callbacks of the current version; instances read them through `__index`.
    proto: Table,
    /// Bumped on every successful reload; clears the "failed" state of instances.
    generation: Rc<Cell<u32>>,
}

struct Shared {
    lua: Lua,
    slot: Rc<CtxSlot>,
    ctx: AnyUserData,
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
            "vec3",
            lua.create_function(|lua, (x, y, z): (f64, f64, f64)| {
                vec3_table(lua, Vec3::new(x as f32, y as f32, z as f32))
            })?,
        )?;
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
        let slot = Rc::new(CtxSlot::default());
        let ctx = lua.create_userdata(CtxHandle(slot.clone()))?;
        Ok(Self {
            shared: Rc::new(Shared {
                lua,
                slot,
                ctx,
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
        let source = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let modified = fs::metadata(path).and_then(|m| m.modified()).ok();
        let chunk = path.display().to_string();
        self.install(
            registry,
            &name,
            Some(path.to_owned()),
            modified,
            &source,
            &chunk,
        )
    }

    /// Loads every `*.lua` in `dir` (not recursive) and watches the folder for
    /// new files. Files that fail to load are logged and skipped.
    pub fn load_dir(&mut self, registry: &mut ScriptRegistry, dir: &Path) {
        if !self.dirs.iter().any(|d| d == dir) {
            self.dirs.push(dir.to_owned());
        }
        for path in lua_files(dir) {
            if let Err(e) = self.load_file(registry, &path) {
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
        let fresh: Vec<PathBuf> = self
            .dirs
            .iter()
            .flat_map(|d| lua_files(d))
            .filter(|p| !known.contains(p))
            .collect();
        let mut reloaded = Vec::new();
        for path in changed.into_iter().chain(fresh) {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned();
            match self.load_file(registry, &path) {
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

        let mut entries = self.shared.entries.borrow_mut();
        let proto = match entries.get(name) {
            Some(old) => {
                // Same table, new callbacks: instances keep pointing at it.
                old.proto.clear().map_err(|e| e.to_string())?;
                old.generation.set(old.generation.get() + 1);
                old.proto.clone()
            }
            None => lua.create_table().map_err(|e| e.to_string())?,
        };
        for pair in table.pairs::<Value, Value>() {
            let (k, v) = pair.map_err(|e| e.to_string())?;
            proto.raw_set(k, v).map_err(|e| e.to_string())?;
        }
        let generation = entries
            .get(name)
            .map_or_else(|| Rc::new(Cell::new(0)), |e| e.generation.clone());
        entries.insert(
            name.to_owned(),
            Entry {
                path,
                modified,
                proto: proto.clone(),
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
                generation.clone(),
                params,
            ))
        });
        Ok(())
    }
}

fn lua_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "lua"))
        .collect();
    files.sort();
    files
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
        generation: Rc<Cell<u32>>,
        params: &Params,
    ) -> Self {
        let state = Self::make_state(&shared.lua, proto, specs, params)
            .map_err(|e| log::error!("lua script `{script}`: could not create its state: {e}"))
            .ok();
        Self {
            shared,
            script: script.to_owned(),
            state,
            generation,
            failed_at: None,
        }
    }

    fn make_state(
        lua: &Lua,
        proto: &Table,
        specs: &[ParamSpec],
        params: &Params,
    ) -> mlua::Result<Table> {
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
                    Value::Table(vec3_table(lua, params.vec3(&spec.name, fallback))?)
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

    /// Calls `callback(self, ctx[, other])` when the script defines it.
    fn call(&mut self, ctx: &mut ScriptCtx<'_>, callback: &str, other: Option<Entity>) {
        let Some(state) = &self.state else { return };
        if self.failed_at == Some(self.generation.get()) {
            return;
        }
        let shared = &*self.shared;
        let result = (|| {
            let Some(f) = state.get::<Option<Function>>(callback)? else {
                return Ok(());
            };
            // SAFETY: only the lifetime is erased; `Lent` clears the pointer
            // before `ctx` can be used again (see `CtxSlot::with`).
            let lent = unsafe {
                std::mem::transmute::<
                    std::ptr::NonNull<ScriptCtx<'_>>,
                    std::ptr::NonNull<ScriptCtx<'static>>,
                >(std::ptr::NonNull::from(&mut *ctx))
            };
            shared.slot.ptr.set(Some(lent));
            let _guard = Lent(&shared.slot);
            shared.spent.set(0);
            match other {
                Some(o) => f.call::<()>((state, &shared.ctx, entity_id(o))),
                None => f.call::<()>((state, &shared.ctx)),
            }
        })();
        if let Err(e) = result {
            log::error!(
                target: "script",
                "lua script `{}` failed in {callback}: {e}; skipped until it is reloaded",
                self.script
            );
            self.failed_at = Some(self.generation.get());
        }
    }
}

impl Behaviour for LuaBehaviour {
    fn start(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.call(ctx, "start", None);
    }
    fn update(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.call(ctx, "update", None);
    }
    fn fixed_update(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.call(ctx, "fixed_update", None);
    }
    fn on_collision_enter(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.call(ctx, "on_collision_enter", Some(other));
    }
    fn on_collision_exit(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.call(ctx, "on_collision_exit", Some(other));
    }
    fn on_trigger_enter(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.call(ctx, "on_trigger_enter", Some(other));
    }
    fn on_trigger_exit(&mut self, ctx: &mut ScriptCtx<'_>, other: Entity) {
        self.call(ctx, "on_trigger_exit", Some(other));
    }
    fn on_destroy(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.call(ctx, "on_destroy", None);
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

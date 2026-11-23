//! The `ctx` userdata: Rust primitives that take and return plain numbers
//! (`*_xyz`), strings and entity ids. The vector API (`ctx:position()`,
//! `ctx:translate(vec3(...))`, …) is built on top of them in Lua by
//! `prelude.lua`, which is much cheaper than making tables from Rust.

use std::cell::Cell;
use std::ptr::NonNull;

use glam::{EulerRot, Quat, Vec3};
use mlua::{Function, Lua, LuaString, Table, UserData, UserDataMethods, Value};
use olvin_core::{Events, Time};
use olvin_ecs::Entity;
use olvin_scene::{Tag, Transform};

use crate::{ScriptCtx, ScriptMessage, ScriptValue, ScriptVars};

/// Messages waiting in the queue beyond which `ctx:send` refuses.
pub(crate) const MAX_PENDING_MESSAGES: usize = 4096;
/// Distinct [`ScriptVars`] scripts may create.
pub(crate) const MAX_VARS: usize = 1024;
/// Longest string a message, a variable or a variable name may carry.
pub(crate) const MAX_STRING: usize = 16 * 1024;

pub(crate) fn entity_id(e: Entity) -> u64 {
    e.to_bits().get()
}

pub(crate) fn entity_from(id: u64) -> Option<Entity> {
    Entity::from_bits(id)
}

const DEG: f32 = std::f32::consts::PI / 180.0;

// --- The lent context ------------------------------------------------------

/// Where the current callback's `ScriptCtx` is lent to Lua. Null outside callbacks.
#[derive(Default)]
pub(crate) struct CtxSlot {
    pub(crate) ptr: Cell<Option<NonNull<ScriptCtx<'static>>>>,
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

    /// Lends `ctx` until the returned guard drops.
    pub(crate) fn lend<'s>(&'s self, ctx: &mut ScriptCtx<'_>) -> Lent<'s> {
        // SAFETY: only the lifetime is erased; `Lent` clears the pointer
        // before `ctx` can be used again (see `with`).
        let ptr = unsafe {
            std::mem::transmute::<NonNull<ScriptCtx<'_>>, NonNull<ScriptCtx<'static>>>(
                NonNull::from(ctx),
            )
        };
        self.ptr.set(Some(ptr));
        Lent(self)
    }
}

/// Clears the lent context when the callback ends, even on unwind.
pub(crate) struct Lent<'a>(&'a CtxSlot);

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        self.0.ptr.set(None);
    }
}

// --- Values ------------------------------------------------------------------

/// `{x=, y=, z=}` or `{x, y, z}`; missing axes are 0. Used for rare paths
/// (parameter defaults, message values); hot paths unpack in Lua.
pub(crate) fn vec3_of(t: &Table) -> mlua::Result<Vec3> {
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

fn checked_str(s: &LuaString, what: &str) -> mlua::Result<String> {
    let s = s.to_str()?;
    if s.len() > MAX_STRING {
        return Err(mlua::Error::runtime(format!(
            "{what} is longer than {MAX_STRING} bytes"
        )));
    }
    Ok(s.to_owned())
}

/// A Lua value as a [`ScriptValue`]: nil, boolean, number, string or a vector table.
pub(crate) fn to_script_value(v: Value) -> mlua::Result<ScriptValue> {
    Ok(match v {
        Value::Nil => ScriptValue::Nil,
        Value::Boolean(b) => ScriptValue::Bool(b),
        Value::Integer(i) => ScriptValue::Int(i),
        Value::Number(n) => ScriptValue::Number(n),
        Value::String(s) => ScriptValue::Str(checked_str(&s, "a string value")?),
        Value::Table(t) if t.contains_key("x")? || t.contains_key(1)? => {
            ScriptValue::Vec3(vec3_of(&t)?)
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "messages and variables hold nil, booleans, numbers, strings or vectors, not {}",
                other.type_name()
            )));
        }
    })
}

/// A [`ScriptValue`] for Lua; vectors are made with the prelude's `vec3`.
pub(crate) fn from_script_value(
    lua: &Lua,
    vec3: &Function,
    v: &ScriptValue,
) -> mlua::Result<Value> {
    Ok(match v {
        ScriptValue::Nil => Value::Nil,
        ScriptValue::Bool(b) => Value::Boolean(*b),
        ScriptValue::Int(i) => Value::Integer(*i),
        ScriptValue::Number(n) => Value::Number(*n),
        ScriptValue::Str(s) => Value::String(lua.create_string(s)?),
        ScriptValue::Vec3(v) => vec3.call((v.x, v.y, v.z))?,
    })
}

// --- The handle --------------------------------------------------------------

pub(crate) struct CtxHandle {
    pub(crate) slot: std::rc::Rc<CtxSlot>,
    /// The prelude's `vec3` constructor (set right after the prelude runs).
    pub(crate) vec3: Option<Function>,
}

type V3 = (f32, f32, f32);

fn v3(v: Vec3) -> V3 {
    (v.x, v.y, v.z)
}

/// Three nils when there is no vector.
type OptV3 = (Option<f32>, Option<f32>, Option<f32>);

fn opt3(v: Option<V3>) -> OptV3 {
    match v {
        Some((x, y, z)) => (Some(x), Some(y), Some(z)),
        None => (None, None, None),
    }
}

fn euler_deg(q: Quat) -> V3 {
    let (y, x, z) = q.to_euler(EulerRot::YXZ);
    (x / DEG, y / DEG, z / DEG)
}

fn quat_deg(x: f32, y: f32, z: f32) -> Quat {
    Quat::from_euler(EulerRot::YXZ, y * DEG, x * DEG, z * DEG)
}

/// Edits another entity's `Transform` (no insert). `None`: no such entity or no Transform.
fn edit_of<R>(c: &ScriptCtx<'_>, id: u64, f: impl FnOnce(&mut Transform) -> R) -> Option<R> {
    let e = entity_from(id)?;
    let mut t = c.get_mut::<Transform>(e)?;
    Some(f(&mut t))
}

fn read_of(c: &ScriptCtx<'_>, id: u64) -> Option<Transform> {
    let e = entity_from(id)?;
    c.get::<Transform>(e).map(|t| *t)
}

impl UserData for CtxHandle {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // Identity and time.
        m.add_method("entity", |_, h, ()| h.slot.with(|c| entity_id(c.entity())));
        m.add_method("name", |_, h, ()| h.slot.with(|c| c.name()));
        m.add_method("dt", |_, h, ()| h.slot.with(|c| c.dt()));
        m.add_method("fixed_dt", |_, h, ()| h.slot.with(|c| c.fixed_dt()));
        m.add_method("elapsed", |_, h, ()| h.slot.with(|c| c.elapsed()));
        m.add_method("frame", |_, h, ()| h.slot.with(|c| c.time().frame));
        m.add_method("unscaled_dt", |_, h, ()| {
            h.slot
                .with(|c| c.res::<Time>().map_or(c.dt(), |t| t.unscaled_delta()))
        });
        m.add_method("time_scale", |_, h, ()| {
            h.slot
                .with(|c| c.res::<Time>().map_or(1.0, |t| t.time_scale()))
        });
        m.add_method("paused", |_, h, ()| {
            h.slot.with(|c| c.res::<Time>().is_some_and(|t| t.paused()))
        });
        m.add_method("log", |_, h, msg: LuaString| {
            let msg = msg.to_str()?;
            h.slot.with(|c| c.log(&*msg))
        });
        m.add_method("warn", |_, h, msg: LuaString| {
            let msg = msg.to_str()?;
            h.slot.with(|c| c.warn(&*msg))
        });

        // Transform of this entity (numbers; vectors are wrapped in the prelude).
        m.add_method("position_xyz", |_, h, ()| h.slot.with(|c| v3(c.position())));
        m.add_method("world_position_xyz", |_, h, ()| {
            h.slot.with(|c| v3(c.world_position()))
        });
        m.add_method("set_position_xyz", |_, h, (x, y, z): V3| {
            h.slot.with(|c| c.set_position(Vec3::new(x, y, z)))
        });
        m.add_method("translate_xyz", |_, h, (x, y, z): V3| {
            h.slot.with(|c| c.translate(Vec3::new(x, y, z)))
        });
        m.add_method("translate_local_xyz", |_, h, (x, y, z): V3| {
            h.slot.with(|c| c.translate_local(Vec3::new(x, y, z)))
        });
        m.add_method("rotate_euler_deg_xyz", |_, h, (x, y, z): V3| {
            h.slot.with(|c| c.rotate_euler_deg(Vec3::new(x, y, z)))
        });
        m.add_method("rotation_euler_deg_xyz", |_, h, ()| {
            h.slot.with(|c| euler_deg(c.rotation()))
        });
        m.add_method("set_rotation_euler_deg_xyz", |_, h, (x, y, z): V3| {
            h.slot.with(|c| c.set_rotation(quat_deg(x, y, z)))
        });
        m.add_method("scale_xyz", |_, h, ()| {
            h.slot.with(|c| v3(c.transform().scale))
        });
        m.add_method("set_scale_xyz", |_, h, (x, y, z): V3| {
            h.slot
                .with(|c| c.with_transform(|t| t.scale = Vec3::new(x, y, z)))
        });
        m.add_method(
            "look_at_xyz",
            |_, h, (x, y, z, ux, uy, uz): (f32, f32, f32, Option<f32>, Option<f32>, Option<f32>)| {
                let up = match ux {
                    Some(ux) => Vec3::new(ux, uy.unwrap_or(0.0), uz.unwrap_or(0.0)),
                    None => Vec3::Y,
                };
                h.slot.with(|c| c.look_at(Vec3::new(x, y, z), up))
            },
        );
        m.add_method("forward_xyz", |_, h, ()| h.slot.with(|c| v3(c.forward())));
        m.add_method("right_xyz", |_, h, ()| h.slot.with(|c| v3(c.right())));
        m.add_method("up_xyz", |_, h, ()| h.slot.with(|c| v3(c.up())));

        // Other entities.
        m.add_method("find_by_name", |_, h, name: LuaString| {
            let name = name.to_str()?;
            h.slot.with(|c| c.find_by_name(&name).map(entity_id))
        });
        m.add_method("find_child", |_, h, name: LuaString| {
            let name = name.to_str()?;
            h.slot.with(|c| c.find_child(&name).map(entity_id))
        });
        m.add_method("find_by_tag", |_, h, tag: LuaString| {
            let tag = tag.to_str()?;
            h.slot.with(|c| c.find_by_tag(&tag).map(entity_id))
        });
        m.add_method("find_all_by_tag", |_, h, tag: LuaString| {
            let tag = tag.to_str()?;
            h.slot.with(|c| {
                c.find_all_by_tag(&tag)
                    .into_iter()
                    .map(entity_id)
                    .collect::<Vec<_>>()
            })
        });
        m.add_method("alive", |_, h, id: u64| {
            h.slot
                .with(|c| entity_from(id).is_some_and(|e| c.world().contains(e)))
        });
        m.add_method("name_of", |_, h, id: u64| {
            h.slot.with(|c| {
                entity_from(id)
                    .filter(|e| c.world().contains(*e))
                    .map(|e| c.name_of(e))
            })
        });
        m.add_method("has_tag", |_, h, (id, tag): (u64, LuaString)| {
            let tag = tag.to_str()?;
            h.slot.with(|c| {
                entity_from(id)
                    .and_then(|e| c.get::<Tag>(e))
                    .is_some_and(|t| t.0 == *tag)
            })
        });
        m.add_method("world_position_of_xyz", |_, h, id: u64| {
            h.slot
                .with(|c| {
                    entity_from(id)
                        .filter(|e| c.world().contains(*e))
                        .map(|e| v3(c.world_position_of(e)))
                })
                .map(opt3)
        });
        m.add_method("position_of_xyz", |_, h, id: u64| {
            h.slot
                .with(|c| opt3(read_of(c, id).map(|t| v3(t.translation))))
        });
        m.add_method("rotation_euler_deg_of_xyz", |_, h, id: u64| {
            h.slot
                .with(|c| opt3(read_of(c, id).map(|t| euler_deg(t.rotation))))
        });
        m.add_method("scale_of_xyz", |_, h, id: u64| {
            h.slot.with(|c| opt3(read_of(c, id).map(|t| v3(t.scale))))
        });
        m.add_method(
            "set_position_of_xyz",
            |_, h, (id, x, y, z): (u64, f32, f32, f32)| {
                h.slot
                    .with(|c| edit_of(c, id, |t| t.translation = Vec3::new(x, y, z)).is_some())
            },
        );
        m.add_method(
            "translate_of_xyz",
            |_, h, (id, x, y, z): (u64, f32, f32, f32)| {
                h.slot
                    .with(|c| edit_of(c, id, |t| t.translation += Vec3::new(x, y, z)).is_some())
            },
        );
        m.add_method(
            "set_rotation_euler_deg_of_xyz",
            |_, h, (id, x, y, z): (u64, f32, f32, f32)| {
                h.slot
                    .with(|c| edit_of(c, id, |t| t.rotation = quat_deg(x, y, z)).is_some())
            },
        );
        m.add_method(
            "set_scale_of_xyz",
            |_, h, (id, x, y, z): (u64, f32, f32, f32)| {
                h.slot
                    .with(|c| edit_of(c, id, |t| t.scale = Vec3::new(x, y, z)).is_some())
            },
        );
        m.add_method("despawn", |_, h, id: u64| {
            h.slot.with(|c| {
                if let Some(e) = entity_from(id) {
                    c.despawn(e);
                }
            })
        });
        // Queues a prefab instance (made by the game after the pass; the
        // requester then gets a "spawned" message with the root's id). False
        // when the per-frame or alive cap is reached.
        m.add_method(
            "spawn_prefab",
            |_, h, (path, x, y, z, yaw): (LuaString, f32, f32, f32, Option<f32>)| {
                let path = checked_str(&path, "a prefab path")?;
                crate::check_prefab_path(&path).map_err(mlua::Error::runtime)?;
                if ![x, y, z].iter().all(|v| v.is_finite()) {
                    return Err(mlua::Error::runtime("spawn_prefab: position is not finite"));
                }
                h.slot.with(|c| {
                    let alive = c.world().query::<&crate::ScriptSpawned>().iter().count();
                    let me = c.entity();
                    let Some(mut queue) = c.res_mut::<crate::SpawnRequests>() else {
                        return false;
                    };
                    if queue.len() >= crate::MAX_SPAWNS_PER_FRAME
                        || alive + queue.len() >= crate::MAX_SPAWNED_ALIVE
                    {
                        return false;
                    }
                    queue.push(crate::SpawnRequest {
                        asset: path,
                        transform: Transform::from_xyz(x, y, z).with_rotation(quat_deg(
                            0.0,
                            yaw.unwrap_or(0.0),
                            0.0,
                        )),
                        requester: me,
                    });
                    true
                })
            },
        );
        m.add_method("destroy_self", |_, h, ()| h.slot.with(|c| c.destroy_self()));

        // Messages and shared variables.
        m.add_method(
            "send",
            |_, h, (name, value, target): (LuaString, Value, Option<u64>)| {
                let name = checked_str(&name, "a message name")?;
                let value = to_script_value(value)?;
                let target = match target {
                    Some(id) => Some(entity_from(id).ok_or_else(|| {
                        mlua::Error::runtime(format!("send: {id} is not an entity id"))
                    })?),
                    None => None,
                };
                h.slot.with(|c| {
                    let Some(mut events) = c.res_mut::<Events<ScriptMessage>>() else {
                        return Ok(false);
                    };
                    if events.len() >= MAX_PENDING_MESSAGES {
                        return Err(mlua::Error::runtime(format!(
                            "send: more than {MAX_PENDING_MESSAGES} messages are waiting"
                        )));
                    }
                    events.send(ScriptMessage {
                        name,
                        value,
                        from: Some(c.entity()),
                        target,
                    });
                    Ok(true)
                })?
            },
        );
        m.add_method("var", |lua, h, name: LuaString| {
            let name = name.to_str()?;
            let value = h.slot.with(|c| {
                c.vars()
                    .and_then(|v| v.get(&name).cloned())
                    .unwrap_or_default()
            })?;
            match &h.vec3 {
                Some(f) => from_script_value(lua, f, &value),
                None => Ok(Value::Nil),
            }
        });
        m.add_method("set_var", |_, h, (name, value): (LuaString, Value)| {
            let name = checked_str(&name, "a variable name")?;
            let value = to_script_value(value)?;
            h.slot.with(|c| {
                let Some(mut vars) = c.res_mut::<ScriptVars>() else {
                    return Ok(());
                };
                if vars.get(&name).is_none() && !value.is_nil() && vars.len() >= MAX_VARS {
                    return Err(mlua::Error::runtime(format!(
                        "set_var: more than {MAX_VARS} variables"
                    )));
                }
                vars.set(&name, value);
                Ok(())
            })?
        });

        #[cfg(feature = "physics")]
        add_physics(m);
        #[cfg(feature = "input")]
        add_input(m);
        #[cfg(feature = "audio")]
        super::audio::add_audio(m);
    }
}

#[cfg(feature = "audio")]
impl CtxHandle {
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut ScriptCtx<'_>) -> R) -> mlua::Result<R> {
        self.slot.with(f)
    }
}

#[cfg(feature = "physics")]
fn add_physics<M: UserDataMethods<CtxHandle>>(m: &mut M) {
    m.add_method("apply_force_xyz", |_, h, (x, y, z): V3| {
        h.slot.with(|c| c.apply_force(Vec3::new(x, y, z)))
    });
    m.add_method("apply_impulse_xyz", |_, h, (x, y, z): V3| {
        h.slot.with(|c| c.apply_impulse(Vec3::new(x, y, z)))
    });
    m.add_method("set_velocity_xyz", |_, h, (x, y, z): V3| {
        h.slot.with(|c| c.set_velocity(Vec3::new(x, y, z)))
    });
    m.add_method("velocity_xyz", |_, h, ()| {
        h.slot.with(|c| opt3(c.velocity().map(v3)))
    });
    // Respawn: puts a body (this entity or another) at a position now and
    // stops it (`Physics::teleport`: velocities and the character motor are
    // zeroed). False when there is no such entity.
    m.add_method("teleport_xyz", |_, h, (x, y, z): V3| {
        h.slot.with(|c| teleport(c, c.entity(), Vec3::new(x, y, z)))
    });
    m.add_method(
        "teleport_of_xyz",
        |_, h, (id, x, y, z): (u64, f32, f32, f32)| {
            h.slot
                .with(|c| entity_from(id).is_some_and(|e| teleport(c, e, Vec3::new(x, y, z))))
        },
    );
    // Character controller: its motor's velocity (the character moves by it),
    // whether it stands on the ground, and the world's gravity (m/s², positive).
    m.add_method("motor_velocity_xyz", |_, h, ()| {
        h.slot.with(|c| opt3(c.motor().map(|m| v3(m.velocity))))
    });
    m.add_method("set_motor_velocity_xyz", |_, h, (x, y, z): V3| {
        h.slot.with(|c| match c.motor() {
            Some(mut m) => {
                m.velocity = Vec3::new(x, y, z);
                true
            }
            None => false,
        })
    });
    m.add_method("grounded", |_, h, ()| {
        h.slot.with(|c| c.motor().is_some_and(|m| m.grounded))
    });
    m.add_method("gravity", |_, h, ()| {
        h.slot
            .with(|c| c.physics().map_or(9.81, |p| -p.gravity().y))
    });
    // nil, or entity, point xyz, normal xyz, distance.
    m.add_method(
        "raycast_xyz",
        |_, h, (ox, oy, oz, dx, dy, dz, max): (f32, f32, f32, f32, f32, f32, f32)| {
            let hit = h
                .slot
                .with(|c| c.raycast(Vec3::new(ox, oy, oz), Vec3::new(dx, dy, dz), max))?;
            let Some(hit) = hit else {
                return Ok((None, None, None, None, None, None, None, None));
            };
            let (p, n) = (hit.point, hit.normal);
            Ok((
                Some(entity_id(hit.entity)),
                Some(p.x),
                Some(p.y),
                Some(p.z),
                Some(n.x),
                Some(n.y),
                Some(n.z),
                Some(hit.distance),
            ))
        },
    );
}

/// Moves `e` to `at` (its local position) and tells physics to follow at once.
#[cfg(feature = "physics")]
fn teleport(c: &ScriptCtx<'_>, e: Entity, at: Vec3) -> bool {
    if !c.world().contains(e) || !at.is_finite() {
        return false;
    }
    let Some(mut t) = c.get_mut::<Transform>(e) else {
        return false;
    };
    t.translation = at;
    drop(t);
    if let Some(mut physics) = c.res_mut::<olvin_physics::Physics>() {
        physics.teleport(c.world(), e);
    }
    true
}

#[cfg(feature = "input")]
fn add_input<M: UserDataMethods<CtxHandle>>(m: &mut M) {
    // Action names are borrowed from Lua, not copied.
    m.add_method("pressed", |_, h, a: LuaString| {
        let a = a.to_str()?;
        h.slot.with(|c| c.input().pressed(&a))
    });
    m.add_method("just_pressed", |_, h, a: LuaString| {
        let a = a.to_str()?;
        h.slot.with(|c| c.input().just_pressed(&a))
    });
    m.add_method("just_released", |_, h, a: LuaString| {
        let a = a.to_str()?;
        h.slot.with(|c| c.input().just_released(&a))
    });
    m.add_method("value", |_, h, a: LuaString| {
        let a = a.to_str()?;
        h.slot.with(|c| c.input().value(&a))
    });
    // Grabs (or frees) the mouse cursor, for mouse look.
    m.add_method("set_cursor_locked", |_, h, locked: bool| {
        h.slot.with(|c| {
            if let Some(input) = c.input_mut() {
                input.lock_cursor(locked);
            }
        })
    });
    m.add_method("axis2d_xy", |_, h, a: LuaString| {
        let a = a.to_str()?;
        h.slot.with(|c| {
            let v = c.input().axis2d(&a);
            (v.x, v.y)
        })
    });
}

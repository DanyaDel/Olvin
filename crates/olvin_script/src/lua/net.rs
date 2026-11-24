//! Multiplayer for Lua (feature `net`): who am I, send a message, and replicated variables.
//!
//! ```lua
//! return {
//!     start = function(self, ctx)
//!         if ctx:is_server() then ctx:replicate_var("round") end
//!     end,
//!     update = function(self, ctx)
//!         if ctx:is_client() and ctx:is_mine() and ctx:action_pressed("Fire") then
//!             ctx:net_send("server", "fire", ctx:forward())
//!         end
//!     end,
//!     -- name, value and the sender: a client id, or nil when the server sent it
//!     on_net_message = function(self, ctx, name, value, from)
//!         if ctx:is_server() and name == "fire" then
//!             ctx:set_var("round", (ctx:var("round") or 0) + 1)   -- replicated to everyone
//!         end
//!     end,
//! }
//! ```
//!
//! Messages travel as one registered `olvin_net` message type ("lua"): they
//! are size-limited (name 64 characters, text 256 bytes), rate-limited per
//! client like any other message, and a client can only send to the server.
//! A script that needs trust checks them in `on_net_message` on the server:
//! the sender id is the one the server saw, never a value the client wrote.
//! Add [`LuaNetPlugin`] after `NetPlugin`, in the same order on every peer.

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;
use mlua::{LuaString, UserDataMethods, Value};
use olvin_core::{App, EventCursor, Events, Plugin, Resources, Stage};
use olvin_net::{
    ClientId, MessageOptions, Net, NetAppExt, NetEvent, NetReceived, Owner, Sender, Target, Verdict,
};
use serde::{Deserialize, Serialize};

use super::api::{CtxHandle, to_script_value};
use crate::{ScriptMessage, ScriptValue, ScriptVars};

/// Internal name prefix of a script message that came from the network.
const NET_PREFIX: &str = "\u{1}net\u{1}";
const MAX_NAME: usize = 64;
const MAX_TEXT: usize = 256;
/// `ctx:net_send` calls per frame before the script gets an error.
const MAX_SENDS_PER_FRAME: u32 = 64;
const MAX_NET_VARS: usize = 64;

/// `(sender, name)` of a network message turned script message, if it is one.
/// The sender is a client id, or `None` for the server.
pub(crate) fn split_net_name(name: &str) -> Option<(Option<u32>, &str)> {
    let rest = name.strip_prefix(NET_PREFIX)?;
    let (from, name) = rest.split_once('\u{1}')?;
    Some((from.parse().ok(), name))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum NetValue {
    Nil,
    Bool(bool),
    Int(i64),
    Number(f64),
    Str(String),
    Vec3([f32; 3]),
}

impl From<&ScriptValue> for NetValue {
    fn from(v: &ScriptValue) -> Self {
        match v {
            ScriptValue::Nil => Self::Nil,
            ScriptValue::Bool(b) => Self::Bool(*b),
            ScriptValue::Int(i) => Self::Int(*i),
            ScriptValue::Number(n) => Self::Number(*n),
            ScriptValue::Str(s) => Self::Str(s.clone()),
            ScriptValue::Vec3(v) => Self::Vec3(v.to_array()),
        }
    }
}

impl From<NetValue> for ScriptValue {
    fn from(v: NetValue) -> Self {
        match v {
            NetValue::Nil => Self::Nil,
            NetValue::Bool(b) => Self::Bool(b),
            NetValue::Int(i) => Self::Int(i),
            NetValue::Number(n) => Self::Number(n),
            NetValue::Str(s) => Self::Str(s),
            NetValue::Vec3(v) => Self::Vec3(Vec3::from_array(v)),
        }
    }
}

/// The wire form of a Lua message or a replicated variable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct LuaNet {
    /// A replicated variable update instead of a message.
    var: bool,
    name: String,
    value: NetValue,
}

/// Names and values from the network are checked on both sides.
fn shape_ok(m: &LuaNet) -> Result<(), &'static str> {
    if m.name.is_empty()
        || m.name.chars().count() > MAX_NAME
        || m.name.chars().any(char::is_control)
    {
        return Err("bad message name");
    }
    match &m.value {
        NetValue::Str(s) if s.len() > MAX_TEXT => Err("text too long"),
        NetValue::Number(n) if !n.is_finite() => Err("number is not finite"),
        NetValue::Vec3(v) if v.iter().any(|x| !x.is_finite()) => Err("vector is not finite"),
        _ => Ok(()),
    }
}

/// Replicated variables (server) and the per-frame send budget.
#[derive(Default)]
struct NetVars {
    names: BTreeSet<String>,
    last: BTreeMap<String, ScriptValue>,
    sends: u32,
}

/// Lua multiplayer: registers the "lua" message and turns what arrives into
/// `on_net_message` calls and shared-variable updates. Add after `NetPlugin`.
pub struct LuaNetPlugin;

impl Plugin for LuaNetPlugin {
    fn build(&self, app: &mut App) {
        app.net_command_with::<LuaNet>(
            "lua",
            MessageOptions::default().rate(60).max_size(1024),
            |_, m| match shape_ok(m) {
                Ok(()) => Verdict::Accept,
                Err(why) => Verdict::reject(why),
            },
        );
        app.init_resource::<NetVars>();
        app.add_event::<ScriptMessage>();
        // Deliver what arrived, before scripts run.
        let mut cursor = EventCursor::default();
        app.add_system(Stage::PreUpdate, move |res| deliver(res, &mut cursor));
        // Replicate variables after scripts ran.
        let mut joins = EventCursor::default();
        app.add_system(Stage::PostUpdate, move |res| {
            replicate_vars(res, &mut joins)
        });
    }
}

fn deliver(res: &Resources, cursor: &mut EventCursor) {
    let Some(events) = res.get::<Events<NetReceived<LuaNet>>>() else {
        return;
    };
    let incoming: Vec<_> = events.read(cursor).cloned().collect();
    drop(events);
    res.fetch_mut::<NetVars>().sends = 0;
    for NetReceived { from, msg } in incoming {
        // The server is untrusted by clients too.
        if shape_ok(&msg).is_err() {
            continue;
        }
        if msg.var {
            // Only the server sets replicated variables, and only clients need them.
            let is_server_process = res.get::<Net>().is_some_and(|n| n.is_server());
            if !from.is_server() || is_server_process {
                continue;
            }
            if let Some(mut vars) = res.get_mut::<ScriptVars>() {
                let value = ScriptValue::from(msg.value);
                if vars.get(&msg.name).is_some() || value.is_nil() || vars.len() < MAX_NET_VARS * 16
                {
                    vars.set(&msg.name, value);
                }
            }
        } else if let Some(mut out) = res.get_mut::<Events<ScriptMessage>>() {
            let sender = match from {
                Sender::Server => String::new(),
                Sender::Client(c) => c.0.to_string(),
            };
            out.send(ScriptMessage::new(
                format!("{NET_PREFIX}{sender}\u{1}{}", msg.name),
                ScriptValue::from(msg.value),
            ));
        }
    }
}

fn replicate_vars(res: &Resources, joins: &mut EventCursor) {
    let (Some(mut net), Some(vars)) = (res.get_mut::<Net>(), res.get::<ScriptVars>()) else {
        return;
    };
    if !net.is_server() {
        return;
    }
    let mut state = res.fetch_mut::<NetVars>();
    let state = &mut *state;
    let current = |name: &str| vars.get(name).cloned().unwrap_or_default();
    // A joiner gets everything there is.
    let joined: Vec<ClientId> = res
        .get::<Events<NetEvent>>()
        .map(|e| {
            e.read(joins)
                .filter_map(|ev| match ev {
                    NetEvent::ClientJoined { id, .. } | NetEvent::ClientReconnected { id }
                        if *id != ClientId::HOST =>
                    {
                        Some(*id)
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    for id in joined {
        for name in &state.names {
            let value = NetValue::from(&current(name));
            net.send(
                Target::Client(id),
                &LuaNet {
                    var: true,
                    name: name.clone(),
                    value,
                },
            );
        }
    }
    for name in &state.names {
        let now = current(name);
        if state.last.get(name) != Some(&now) {
            net.send(
                Target::All,
                &LuaNet {
                    var: true,
                    name: name.clone(),
                    value: NetValue::from(&now),
                },
            );
            state.last.insert(name.clone(), now);
        }
    }
}

fn name_arg(s: &LuaString, what: &str) -> mlua::Result<String> {
    let s = s.to_str()?.to_string();
    if s.is_empty() || s.chars().count() > MAX_NAME || s.chars().any(char::is_control) {
        return Err(mlua::Error::runtime(format!(
            "{what}: use 1 to {MAX_NAME} printable characters"
        )));
    }
    Ok(s)
}

pub(crate) fn add_net<M: UserDataMethods<CtxHandle>>(m: &mut M) {
    m.add_method("is_server", |_, h, ()| {
        h.with(|c| c.res::<Net>().is_some_and(|n| n.is_server()))
    });
    m.add_method("is_client", |_, h, ()| {
        h.with(|c| c.res::<Net>().is_none_or(|n| n.is_client()))
    });
    m.add_method("local_client", |_, h, ()| {
        h.with(|c| c.res::<Net>().and_then(|n| n.local_client()).map(|id| id.0))
    });
    m.add_method("owner", |_, h, ()| {
        h.with(|c| c.get::<Owner>(c.entity()).map(|o| o.0.0))
    });
    m.add_method("is_mine", |_, h, ()| {
        h.with(|c| {
            let net = c.res::<Net>();
            net.is_some_and(|n| n.is_mine(c.world(), c.entity()))
        })
    });
    // ctx:net_send(target, name, value): target is "server", "all", "owner",
    // a client id, or { except = client_id }.
    m.add_method(
        "net_send",
        |_, h, (target, name, value): (Value, LuaString, Value)| {
            let name = name_arg(&name, "net_send")?;
            let value = NetValue::from(&to_script_value(value)?);
            let msg = LuaNet {
                var: false,
                name,
                value,
            };
            shape_ok(&msg).map_err(|why| mlua::Error::runtime(format!("net_send: {why}")))?;
            h.with(|c| {
                let target = match &target {
                    Value::String(s) => match &*s.to_str()? {
                        "server" => Target::Server,
                        "all" => Target::All,
                        "owner" => Target::Owner(c.entity()),
                        other => {
                            return Err(mlua::Error::runtime(format!(
                                "net_send: unknown target `{other}` (server, all, owner, a client id, or {{ except = id }})"
                            )));
                        }
                    },
                    Value::Integer(id) => Target::Client(ClientId(*id as u32)),
                    Value::Number(id) => Target::Client(ClientId(*id as u32)),
                    Value::Table(t) => match t.get::<Option<u32>>("except")? {
                        Some(id) => Target::AllExcept(ClientId(id)),
                        None => return Err(mlua::Error::runtime("net_send: a table target needs `except`")),
                    },
                    _ => return Err(mlua::Error::runtime("net_send: bad target")),
                };
                let Some(mut net) = c.res_mut::<Net>() else {
                    return Ok(false);
                };
                if !net.is_server() && target != Target::Server {
                    return Err(mlua::Error::runtime(
                        "net_send: a client can only send to \"server\"",
                    ));
                }
                if let Some(mut vars) = c.res_mut::<NetVars>() {
                    if vars.sends >= MAX_SENDS_PER_FRAME {
                        return Err(mlua::Error::runtime(format!(
                            "net_send: more than {MAX_SENDS_PER_FRAME} sends in one frame"
                        )));
                    }
                    vars.sends += 1;
                }
                net.send(target, &msg);
                Ok(true)
            })?
        },
    );
    // ctx:replicate_var(name): on the server, the shared variable `name` is
    // copied to every client whenever it changes (and to late joiners).
    m.add_method("replicate_var", |_, h, name: LuaString| {
        let name = name_arg(&name, "replicate_var")?;
        h.with(|c| {
            if !c.res::<Net>().is_some_and(|n| n.is_server()) {
                return Ok(false);
            }
            let Some(mut vars) = c.res_mut::<NetVars>() else {
                return Ok(false);
            };
            if vars.names.len() >= MAX_NET_VARS && !vars.names.contains(&name) {
                return Err(mlua::Error::runtime(format!(
                    "replicate_var: more than {MAX_NET_VARS} replicated variables"
                )));
            }
            vars.names.insert(name);
            Ok(true)
        })?
    });
}

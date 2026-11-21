//! Messages between behaviours (Rust or Lua) and the game, and a small
//! shared blackboard of named values.
//!
//! - [`ScriptMessage`] goes through the engine's event queue
//!   (`Events<ScriptMessage>`, registered by [`ScriptPlugin`](crate::ScriptPlugin)).
//!   Behaviours get it in [`Behaviour::on_message`](crate::Behaviour::on_message)
//!   at the start of the next `update` pass, game systems read it with an
//!   `EventCursor` like any other event.
//! - [`ScriptVars`] is a resource: named values both sides can read and
//!   write (game progress, flags, counters). Lua sees it as `ctx:var` /
//!   `ctx:set_var`; the game decides what to mirror into it and back.

use std::collections::HashMap;

use glam::Vec3;
use olvin_ecs::Entity;

/// A value a message or a variable can hold: what Lua and Rust exchange
/// without sharing objects.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ScriptValue {
    #[default]
    Nil,
    Bool(bool),
    Int(i64),
    Number(f64),
    Str(String),
    Vec3(Vec3),
}

impl ScriptValue {
    pub fn is_nil(&self) -> bool {
        matches!(self, Self::Nil)
    }

    /// `Bool` as is; anything else but `Nil` and `Bool(false)` is true (Lua's rule).
    pub fn truthy(&self) -> bool {
        !matches!(self, Self::Nil | Self::Bool(false))
    }

    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Self::Int(i) => Some(i as f64),
            Self::Number(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }
}

impl From<bool> for ScriptValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
impl From<i64> for ScriptValue {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}
impl From<f64> for ScriptValue {
    fn from(v: f64) -> Self {
        Self::Number(v)
    }
}
impl From<f32> for ScriptValue {
    fn from(v: f32) -> Self {
        Self::Number(v.into())
    }
}
impl From<&str> for ScriptValue {
    fn from(v: &str) -> Self {
        Self::Str(v.to_owned())
    }
}
impl From<String> for ScriptValue {
    fn from(v: String) -> Self {
        Self::Str(v)
    }
}
impl From<Vec3> for ScriptValue {
    fn from(v: Vec3) -> Self {
        Self::Vec3(v)
    }
}

/// A named message with one value. `target: None` is a broadcast to every
/// behaviour; `from` is the sending entity (None when the game sent it).
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptMessage {
    pub name: String,
    pub value: ScriptValue,
    pub from: Option<Entity>,
    pub target: Option<Entity>,
}

impl ScriptMessage {
    /// A broadcast from the game.
    pub fn new(name: impl Into<String>, value: impl Into<ScriptValue>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            from: None,
            target: None,
        }
    }

    pub fn to(mut self, target: Entity) -> Self {
        self.target = Some(target);
        self
    }

    /// Whether `entity` should get it.
    pub fn is_for(&self, entity: Entity) -> bool {
        self.target.is_none_or(|t| t == entity)
    }
}

/// Named values shared by behaviours and the game. A resource.
#[derive(Clone, Debug, Default)]
pub struct ScriptVars {
    values: HashMap<String, ScriptValue>,
}

impl ScriptVars {
    pub fn get(&self, name: &str) -> Option<&ScriptValue> {
        self.values.get(name)
    }

    /// Setting `Nil` removes the variable.
    pub fn set(&mut self, name: &str, value: impl Into<ScriptValue>) {
        let value = value.into();
        if value.is_nil() {
            self.values.remove(name);
        } else if let Some(slot) = self.values.get_mut(name) {
            *slot = value;
        } else {
            self.values.insert(name.to_owned(), value);
        }
    }

    /// The variable as a flag (missing = false).
    pub fn flag(&self, name: &str) -> bool {
        self.get(name).is_some_and(ScriptValue::truthy)
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ScriptValue)> {
        self.values.iter().map(|(k, v)| (k.as_str(), v))
    }
}

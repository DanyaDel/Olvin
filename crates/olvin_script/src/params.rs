//! Script parameters: the editor stores them as strings (`ScriptDesc.params`),
//! scripts read them typed through [`Params`].

use std::collections::BTreeMap;
use std::str::FromStr;

use glam::Vec3;
use olvin_ecs::{Entity, World};
use olvin_scene::Name;
use serde::{Deserialize, Serialize};

/// Typed view of a script's string parameters. Missing keys give the default;
/// malformed values log a warning and give the default too.
pub struct Params<'a> {
    map: &'a BTreeMap<String, String>,
    world: Option<&'a World>,
    script: &'a str,
}

impl<'a> Params<'a> {
    pub fn new(
        map: &'a BTreeMap<String, String>,
        world: Option<&'a World>,
        script: &'a str,
    ) -> Self {
        Self { map, world, script }
    }

    /// No parameters at all (code-created scripts, tests).
    pub fn empty() -> Params<'static> {
        static EMPTY: BTreeMap<String, String> = BTreeMap::new();
        Params {
            map: &EMPTY,
            world: None,
            script: "",
        }
    }

    pub fn raw(&self, key: &str) -> Option<&'a str> {
        self.map
            .get(key)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
    }

    pub fn has(&self, key: &str) -> bool {
        self.raw(key).is_some()
    }

    fn bad(&self, key: &str, value: &str, what: &str) {
        log::warn!(
            "script `{}`: parameter `{key}` = {value:?} is not {what}; using the default",
            self.script
        );
    }

    /// Any `FromStr` type.
    pub fn parse<T: FromStr>(&self, key: &str, default: T) -> T {
        match self.raw(key) {
            None => default,
            Some(s) => s.parse().unwrap_or_else(|_| {
                self.bad(key, s, std::any::type_name::<T>());
                default
            }),
        }
    }

    pub fn f32(&self, key: &str, default: f32) -> f32 {
        self.parse(key, default)
    }

    pub fn i32(&self, key: &str, default: i32) -> i32 {
        self.parse(key, default)
    }

    /// `true/false`, `1/0`, `yes/no`, `on/off` (any case).
    pub fn bool(&self, key: &str, default: bool) -> bool {
        match self.raw(key) {
            None => default,
            Some(s) => parse_bool(s).unwrap_or_else(|| {
                self.bad(key, s, "a bool");
                default
            }),
        }
    }

    /// `"x,y,z"` (spaces or `;` also separate), or one number for all three.
    pub fn vec3(&self, key: &str, default: Vec3) -> Vec3 {
        match self.raw(key) {
            None => default,
            Some(s) => parse_vec3(s).unwrap_or_else(|| {
                self.bad(key, s, "a vector \"x,y,z\"");
                default
            }),
        }
    }

    pub fn string(&self, key: &str, default: &str) -> String {
        self.map
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_owned())
    }

    /// The entity whose `Name` is the parameter's value (first match).
    pub fn entity(&self, key: &str) -> Option<Entity> {
        let name = self.raw(key)?;
        let world = self.world?;
        let found = find_by_name(world, name);
        if found.is_none() {
            log::warn!(
                "script `{}`: parameter `{key}`: no entity named {name:?}",
                self.script
            );
        }
        found
    }
}

pub(crate) fn find_by_name(world: &World, name: &str) -> Option<Entity> {
    world
        .query::<(Entity, &Name)>()
        .iter()
        .find(|(_, n)| n.0 == name)
        .map(|(e, _)| e)
}

pub fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

pub fn parse_vec3(s: &str) -> Option<Vec3> {
    let parts: Vec<f32> = s
        .trim()
        .trim_start_matches(['(', '['])
        .trim_end_matches([')', ']'])
        .split([',', ';', ' '])
        .filter(|p| !p.trim().is_empty())
        .map(|p| p.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .ok()?;
    match parts[..] {
        [v] => Some(Vec3::splat(v)),
        [x, y, z] => Some(Vec3::new(x, y, z)),
        _ => None,
    }
}

/// Formats a vector the way [`Params::vec3`] reads it.
pub fn format_vec3(v: Vec3) -> String {
    format!("{},{},{}", v.x, v.y, v.z)
}

/// Built from a script's parameters by the registry.
pub trait FromParams: Sized {
    fn from_params(params: &Params) -> Self;

    /// What the editor shows for this script: parameter names, types, defaults.
    fn schema() -> Vec<ParamSpec> {
        Vec::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParamKind {
    Float,
    Int,
    Bool,
    /// `"x,y,z"`
    Vec3,
    String,
    /// The name of another entity in the scene.
    Entity,
}

/// One parameter of a script, for editors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamSpec {
    pub name: String,
    pub kind: ParamKind,
    /// As stored in the scene file.
    pub default: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
}

impl ParamSpec {
    pub fn new(name: &str, kind: ParamKind, default: impl ToString, doc: &str) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            default: default.to_string(),
            doc: doc.to_owned(),
        }
    }

    pub fn float(name: &str, default: f32, doc: &str) -> Self {
        Self::new(name, ParamKind::Float, default, doc)
    }

    pub fn bool(name: &str, default: bool, doc: &str) -> Self {
        Self::new(name, ParamKind::Bool, default, doc)
    }

    pub fn vec3(name: &str, default: Vec3, doc: &str) -> Self {
        Self::new(name, ParamKind::Vec3, format_vec3(default), doc)
    }

    pub fn string(name: &str, default: &str, doc: &str) -> Self {
        Self::new(name, ParamKind::String, default, doc)
    }

    pub fn entity(name: &str, doc: &str) -> Self {
        Self::new(name, ParamKind::Entity, "", doc)
    }
}

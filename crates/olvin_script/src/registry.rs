//! Script names → factories.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Behaviour, FromParams, ParamSpec, Params};

type Factory = Box<dyn Fn(&Params) -> Box<dyn Behaviour>>;

struct Entry {
    make: Factory,
    schema: Vec<ParamSpec>,
}

/// Every script a game knows, by the name scenes use (`ScriptDesc.name`).
/// A resource: register before the first frame (or any time; entities pick
/// scripts up when they appear).
#[derive(Default)]
pub struct ScriptRegistry {
    entries: BTreeMap<String, Entry>,
}

/// Script name → parameters, what [`ScriptRegistry::export_schema`] writes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptSchema {
    pub scripts: BTreeMap<String, Vec<ParamSpec>>,
}

impl ScriptSchema {
    pub fn from_ron(text: &str) -> Result<Self, String> {
        ron::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_ron(&self) -> String {
        // invariant: plain data (strings, numbers, vectors) always serializes.
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .expect("schemas always serialize")
    }
}

impl ScriptRegistry {
    /// Registry with the built-in behaviours (Rotator, Oscillator, ...).
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        crate::builtin::register_all(&mut r);
        r
    }

    /// Registers `T` under `name`, built with [`FromParams`]. Replaces an earlier entry.
    pub fn register<T: Behaviour + FromParams>(&mut self, name: &str) -> &mut Self {
        self.entries.insert(
            name.to_owned(),
            Entry {
                make: Box::new(|p| Box::new(T::from_params(p))),
                schema: T::schema(),
            },
        );
        self
    }

    /// Registers a closure factory (no [`FromParams`] needed).
    pub fn register_fn(
        &mut self,
        name: &str,
        schema: Vec<ParamSpec>,
        make: impl Fn(&Params) -> Box<dyn Behaviour> + 'static,
    ) -> &mut Self {
        self.entries.insert(
            name.to_owned(),
            Entry {
                make: Box::new(make),
                schema,
            },
        );
        self
    }

    pub(crate) fn register_if_absent<T: Behaviour + FromParams>(&mut self, name: &str) {
        if !self.contains(name) {
            self.register::<T>(name);
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn create(&self, name: &str, params: &Params) -> Option<Box<dyn Behaviour>> {
        self.entries.get(name).map(|e| (e.make)(params))
    }

    pub fn schema(&self, name: &str) -> Option<&[ParamSpec]> {
        self.entries.get(name).map(|e| e.schema.as_slice())
    }

    /// All scripts with their parameters, for editors (e.g. write it to
    /// `assets/scripts.schema.ron` from the game at startup or with a flag).
    pub fn export_schema(&self) -> ScriptSchema {
        ScriptSchema {
            scripts: self
                .entries
                .iter()
                .map(|(k, e)| (k.clone(), e.schema.clone()))
                .collect(),
        }
    }
}

/// Registers types by their own names, or `"Name" => Type`:
///
/// ```ignore
/// olvin_script::register!(registry, Rotator, PlayerController);
/// olvin_script::register!(registry, "Player" => PlayerController);
/// ```
#[macro_export]
macro_rules! register {
    ($registry:expr, $($name:literal => $ty:ty),+ $(,)?) => {{
        $( $registry.register::<$ty>($name); )+
    }};
    ($registry:expr, $($ty:ident),+ $(,)?) => {{
        $( $registry.register::<$ty>(stringify!($ty)); )+
    }};
}

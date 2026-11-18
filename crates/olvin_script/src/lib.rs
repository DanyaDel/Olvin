//! Unity-style behaviours written in Rust.
//!
//! A game implements [`Behaviour`] (+ [`FromParams`] to read editor
//! parameters), registers it by name in the [`ScriptRegistry`], and every
//! entity with a [`Scripts`] component (from a scene's `scripts: [...]` or
//! code) gets an instance: `start` once, `update` every frame,
//! `fixed_update` before each physics step, collision/trigger callbacks from
//! `olvin_physics`, `on_destroy` on despawn.
//!
//! Features: `input` gives `ScriptCtx::input()` (olvin_input actions),
//! `physics` gives fixed updates, collision callbacks, raycasts and the
//! character motor. Without them the crate only needs core + ECS.

mod behaviour;
pub mod builtin;
mod ctx;
mod params;
mod registry;
mod runtime;

pub use behaviour::Behaviour;
pub use ctx::{ScriptCtx, TimeInfo};
pub use olvin_scene::file::ScriptDesc;
pub use params::{FromParams, ParamKind, ParamSpec, Params, format_vec3, parse_bool, parse_vec3};
pub use registry::{ScriptRegistry, ScriptSchema};
pub use runtime::{ScriptRuntime, ScriptsAttached, run_fixed_update, run_update};

use std::collections::BTreeMap;

use olvin_core::{App, Plugin, Stage};
use olvin_ecs::EcsPlugin;

/// Scripts attached in the editor (or code): names + string parameters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scripts(pub Vec<ScriptDesc>);

impl Scripts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a script by name with `(key, value)` parameters.
    pub fn with<'a>(
        mut self,
        name: &str,
        params: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        self.0.push(ScriptDesc {
            name: name.to_owned(),
            params: params
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect::<BTreeMap<_, _>>(),
        });
        self
    }
}

/// Runs scripts. Inserts [`ScriptRegistry`] (with built-ins unless
/// `builtins` is false) and [`ScriptRuntime`]; `update` in `Stage::Update`,
/// `fixed_update` through `olvin_physics::FixedSystems` (feature `physics`).
pub struct ScriptPlugin {
    pub builtins: bool,
}

impl Default for ScriptPlugin {
    fn default() -> Self {
        Self { builtins: true }
    }
}

impl Plugin for ScriptPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(EcsPlugin)
            .init_resource::<ScriptRegistry>()
            .init_resource::<ScriptRuntime>();
        if self.builtins {
            builtin::register_all(&mut app.resources.fetch_mut::<ScriptRegistry>());
        }
        #[cfg(feature = "physics")]
        {
            app.init_resource::<olvin_physics::FixedSystems>();
            app.resources
                .fetch_mut::<olvin_physics::FixedSystems>()
                .add(run_fixed_update);
        }
        app.add_system(Stage::Update, |res| run_update(res, None));
    }
}

/// `app.register_script::<Player>("Player")` sugar.
pub trait ScriptAppExt {
    fn register_script<T: Behaviour + FromParams>(&mut self, name: &str) -> &mut Self;
}

impl ScriptAppExt for App {
    fn register_script<T: Behaviour + FromParams>(&mut self, name: &str) -> &mut Self {
        self.init_resource::<ScriptRegistry>();
        self.resources
            .fetch_mut::<ScriptRegistry>()
            .register::<T>(name);
        self
    }
}

#[cfg(test)]
mod tests;

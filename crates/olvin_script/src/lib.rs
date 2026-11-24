//! Unity-style behaviours written in Rust.
//!
//! A game implements [`Behaviour`] (+ [`FromParams`] to read editor
//! parameters), registers it by name in the [`ScriptRegistry`], and every
//! entity with a [`Scripts`] component (from a scene's `scripts: [...]` or
//! code) gets an instance: `start` once, `update` every frame,
//! `fixed_update` before each physics step, collision/trigger callbacks from
//! `olvin_physics`, `on_destroy` on despawn.
//!
//! Behaviours talk to each other and to the game with [`ScriptMessage`]s
//! (`on_message`) and share named values in [`ScriptVars`].
//!
//! Features: `input` gives `ScriptCtx::input()` (olvin_input actions),
//! `physics` gives fixed updates, collision callbacks, raycasts and the
//! character motor, `audio` lets Lua scripts play sounds (olvin_audio). Without them the crate only needs core + ECS. `lua` (off
//! by default) adds behaviours written in Lua 5.4 with hot reload: see
//! the `lua` module.

mod behaviour;
pub mod builtin;
mod ctx;
mod lend;
#[cfg(feature = "lua")]
pub mod lua;
mod message;
mod params;
mod registry;
mod runtime;
mod spawn;
mod timer;

pub use behaviour::Behaviour;
pub use ctx::{ScriptCtx, TimeInfo};
pub use message::{ScriptMessage, ScriptValue, ScriptVars};
pub use olvin_scene::file::ScriptDesc;
pub use params::{FromParams, ParamKind, ParamSpec, Params, format_vec3, parse_bool, parse_vec3};
pub use registry::{ScriptRegistry, ScriptSchema};
pub use runtime::{ScriptRuntime, ScriptState, ScriptsAttached, run_fixed_update, run_update};
pub use spawn::{
    MAX_SPAWNED_ALIVE, MAX_SPAWNS_PER_FRAME, ScriptSpawned, SpawnRequest, SpawnRequests,
    check_prefab_path,
};
pub use timer::{MAX_SCRIPT_TIMERS, MAX_TIMERS_PER_OBJECT};

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
/// `builtins` is false), [`ScriptRuntime`], [`ScriptVars`] and the
/// `Events<ScriptMessage>` queue; `update` in `Stage::Update`,
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
            .init_resource::<ScriptRuntime>()
            .init_resource::<ScriptVars>()
            .init_resource::<SpawnRequests>()
            .add_event::<ScriptMessage>();
        if !app.has_plugin::<olvin_core::TimerPlugin>() {
            app.add_plugin(olvin_core::TimerPlugin);
        }
        // After `TimerPlugin`'s own system, so a timer that ran out this frame
        // reaches `on_message` in this frame's update pass.
        let mut timers = olvin_core::EventCursor::default();
        app.add_system(Stage::PreUpdate, move |res| timer::bridge(res, &mut timers));
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

/// RON text of a behaviour's state, for [`Behaviour::save_state`].
pub fn state_to_text<T: serde::Serialize>(state: &T) -> Option<String> {
    ron::to_string(state)
        .map_err(|e| log::warn!("script state not saved: {e}"))
        .ok()
}

/// Parses what [`state_to_text`] wrote, for [`Behaviour::load_state`].
pub fn state_from_text<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    ron::from_str(text).map_err(|e| e.to_string())
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

#[cfg(all(test, feature = "lua", feature = "net"))]
mod lua_net_tests;
#[cfg(all(test, feature = "lua"))]
mod lua_tests;
#[cfg(test)]
mod tests;

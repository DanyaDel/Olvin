//! Netcode (`olvin_net`) and its glue to the rest of the engine. Add
//! [`olvin_net::NetPlugin`] yourself; [`DefaultPlugins`](crate::DefaultPlugins) and
//! [`HeadlessPlugins`](crate::HeadlessPlugins) add [`NetGluePlugin`], which connects it
//! to scenes: with the `scene` feature a `Replicated` entry in a scene or prefab
//! becomes a networked object, and clients create replicated prefabs with
//! `scene_file::instantiate_prefab`.

pub use olvin_net::*;

use olvin_core::{App, Plugin, Stage};

/// Connects `olvin_net` to scenes and prefabs. Harmless without `NetPlugin`.
pub struct NetGluePlugin;

impl Plugin for NetGluePlugin {
    fn build(&self, app: &mut App) {
        // Startup runs after every plugin is built, whatever the order they were added in.
        app.add_system(Stage::Startup, |res| {
            #[cfg(feature = "scene")]
            {
                if let Some(mut components) = res.get_mut::<crate::scene_file::SceneComponents>()
                    && !components.contains("Replicated")
                {
                    components.register::<Replicated>("Replicated");
                }
                if let Some(mut net) = res.get_mut::<Net>()
                    && !net.has_prefab_spawner()
                {
                    net.set_prefab_spawner(crate::scene_file::instantiate_prefab);
                }
            }
            #[cfg(not(feature = "scene"))]
            let _ = res;
        });
    }
}

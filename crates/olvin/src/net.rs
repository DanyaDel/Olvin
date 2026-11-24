//! Netcode (`olvin_net`) and its glue to the rest of the engine. Add
//! [`olvin_net::NetPlugin`] yourself; [`DefaultPlugins`](crate::DefaultPlugins) and
//! [`HeadlessPlugins`](crate::HeadlessPlugins) add [`NetGluePlugin`], which connects it
//! to scenes: with the `scene` feature a `Replicated` entry in a scene or prefab
//! becomes a networked object, and clients create replicated prefabs with
//! `scene_file::instantiate_prefab`.

pub use olvin_net::*;
/// Lua multiplayer (`ctx:net_send`, `on_net_message`, replicated variables). Add after `NetPlugin`.
#[cfg(feature = "lua")]
pub use olvin_script::lua::LuaNetPlugin;

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
                // Replicated prefabs are read from the project's assets unless a root was set.
                if let Some(mut assets) = res.get_mut::<crate::render_assets::RenderAssets>()
                    && assets.root().is_none()
                    && let Some(paths) = res.get::<olvin_core::ProjectPaths>()
                {
                    assets.set_root(Some(paths.assets.clone()));
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

/// Server: gives every client that joins a player entity made from a prefab,
/// owned by it (`Owner`), and removes it when the client leaves. The prefab is
/// replicated like any networked object, so it needs a `Replicated` entry.
///
/// ```ignore
/// app.add_plugin(PlayerSpawner::new("prefabs/player.prefab.ron"));
/// ```
#[cfg(feature = "scene")]
pub struct PlayerSpawner {
    prefab: String,
    /// Where player `n` appears: `origin + step * n`.
    pub origin: olvin_scene::Transform,
    pub step: glam::Vec3,
}

#[cfg(feature = "scene")]
impl PlayerSpawner {
    pub fn new(prefab: &str) -> Self {
        Self {
            prefab: prefab.to_owned(),
            origin: olvin_scene::Transform::IDENTITY,
            step: glam::Vec3::new(2.0, 0.0, 0.0),
        }
    }
}

#[cfg(feature = "scene")]
impl Plugin for PlayerSpawner {
    fn build(&self, app: &mut App) {
        use olvin_core::{EventCursor, Events};
        use olvin_ecs::{Entity, World};
        let (prefab, origin, step) = (self.prefab.clone(), self.origin, self.step);
        let mut cursor = EventCursor::default();
        app.add_system(Stage::Update, move |res| {
            let events: Vec<NetEvent> = match res.get::<Events<NetEvent>>() {
                Some(e) => e.read(&mut cursor).cloned().collect(),
                None => return,
            };
            if !res.get::<Net>().is_some_and(|n| n.is_server()) {
                return;
            }
            for ev in events {
                match ev {
                    NetEvent::ClientJoined { id, .. } => {
                        let mut at = origin;
                        at.translation += step * id.0 as f32;
                        match crate::scene_file::instantiate_prefab(res, &prefab, at) {
                            Ok(e) => {
                                let _ = res.fetch_mut::<World>().insert_one(e, Owner(id));
                            }
                            Err(err) => log::error!("net: cannot spawn the player: {err}"),
                        }
                    }
                    NetEvent::ClientLeft { id, .. } => {
                        let mut world = res.fetch_mut::<World>();
                        let gone: Vec<Entity> = world
                            .query::<(Entity, &Owner)>()
                            .iter()
                            .filter(|(_, o)| o.0 == id)
                            .map(|(e, _)| e)
                            .collect();
                        for e in gone {
                            let _ = world.despawn(e);
                        }
                    }
                    _ => {}
                }
            }
        });
    }
}

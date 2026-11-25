//! Olvin: a modular game engine. Each subsystem is its own crate and plugin;
//! this facade re-exports them and wires the default set together.
//! Disable features (`window`, `render`) for servers, tools or custom platforms;
//! `scene` adds loading of scenes saved by olvin-editor, `anim` (animation,
//! no renderer needed) and `assets` (glTF import, asset manager) the crates
//! behind it, `gui` the in-game UI. [`HeadlessPlugins`] runs the same game
//! without a window or GPU (tests, servers).

pub use glam as math;
#[cfg(feature = "anim")]
pub use olvin_anim as anim;
#[cfg(feature = "assets")]
pub use olvin_assets as assets;
#[cfg(feature = "audio")]
pub use olvin_audio as audio;
pub use olvin_core as core;
#[cfg(feature = "cutscene")]
pub use olvin_cutscene as cutscene;
pub use olvin_ecs as ecs;
#[cfg(feature = "game")]
pub use olvin_game as game;
#[cfg(feature = "gui")]
pub use olvin_gui as gui;
#[cfg(feature = "locale")]
pub use olvin_locale as locale;
#[cfg(feature = "nav")]
pub use olvin_nav as nav;
#[cfg(feature = "net")]
pub mod net;
#[cfg(feature = "terrain")]
pub mod terrain;
#[cfg(feature = "input")]
pub use olvin_input as input;
#[cfg(feature = "physics")]
pub use olvin_physics as physics;
#[cfg(feature = "render")]
pub use olvin_render as render;
pub use olvin_scene as scene;
#[cfg(feature = "script")]
pub use olvin_script as script;
#[cfg(feature = "gui")]
pub use olvin_text as text;
pub use olvin_transform as transform;
#[cfg(feature = "window")]
pub use olvin_window as window;

#[cfg(feature = "scene")]
pub use olvin_csg as csg;
#[cfg(feature = "scene")]
pub mod models;
#[cfg(feature = "scene")]
pub mod particles;
#[cfg(feature = "scene")]
pub mod render_assets;
#[cfg(feature = "scene")]
pub mod scene_file;

use olvin_core::{App, Plugin};

/// Every module enabled by features, with default settings. Plugins already
/// added before this one keep their own settings.
#[derive(Default)]
pub struct DefaultPlugins {
    #[cfg(feature = "window")]
    pub window: olvin_window::WindowPlugin,
    #[cfg(feature = "render")]
    pub render: olvin_render::RenderPlugin,
    #[cfg(feature = "physics")]
    pub physics: olvin_physics::PhysicsPlugin,
    #[cfg(feature = "input")]
    pub input: olvin_input::InputPlugin,
    #[cfg(feature = "script")]
    pub script: olvin_script::ScriptPlugin,
    #[cfg(feature = "audio")]
    pub audio: olvin_audio::AudioPlugin,
}

impl Plugin for DefaultPlugins {
    fn build(&self, app: &mut App) {
        app.add_plugin(olvin_ecs::EcsPlugin)
            .add_plugin(olvin_transform::TransformPlugin);
        #[cfg(feature = "window")]
        app.add_plugin(self.window.clone());
        #[cfg(feature = "render")]
        app.add_plugin(olvin_render::RenderPlugin {
            settings: self.render.settings.clone(),
        });
        #[cfg(feature = "anim")]
        app.add_plugin(olvin_anim::AnimPlugin);
        #[cfg(feature = "scene")]
        app.add_plugin(render_assets::RenderAssetsPlugin)
            .add_plugin(models::ModelPlugin)
            .init_resource::<scene_file::SceneComponents>();
        #[cfg(feature = "physics")]
        app.add_plugin(olvin_physics::PhysicsPlugin {
            gravity: self.physics.gravity,
        });
        #[cfg(feature = "input")]
        app.add_plugin(olvin_input::InputPlugin {
            map: self.input.map.clone(),
        });
        #[cfg(feature = "script")]
        app.add_plugin(olvin_script::ScriptPlugin {
            builtins: self.script.builtins,
        });
        #[cfg(all(feature = "script", feature = "scene"))]
        app.add_system(olvin_core::Stage::Update, |res| {
            scene_file::run_prefab_spawns(res)
        });
        #[cfg(feature = "audio")]
        app.add_plugin(self.audio.clone());
        #[cfg(feature = "net")]
        app.add_plugin(net::NetGluePlugin);
        // After input: the UI adds its actions to the map.
        #[cfg(all(feature = "gui", feature = "render"))]
        app.add_plugin(olvin_gui::GuiRenderPlugin::default());
        #[cfg(all(feature = "gui", not(feature = "render")))]
        app.add_plugin(olvin_gui::GuiPlugin);
    }
}

/// [`DefaultPlugins`] without a window and GPU, for tests, servers and
/// tools: the same game logic, scenes, physics, scripts, input (fed through
/// `Input` by hand) and audio (off by default). With the `render` feature the
/// renderer's CPU half comes along (`RenderDataPlugin`: mesh and material
/// registries, CPU skinning, particles), so `scene_file::spawn` and skinned
/// meshes work. Don't combine with `DefaultPlugins` in one app.
pub struct HeadlessPlugins {
    #[cfg(feature = "physics")]
    pub physics: olvin_physics::PhysicsPlugin,
    #[cfg(feature = "input")]
    pub input: olvin_input::InputPlugin,
    #[cfg(feature = "script")]
    pub script: olvin_script::ScriptPlugin,
    #[cfg(feature = "audio")]
    pub audio: olvin_audio::AudioPlugin,
}

// Derivable without the `audio` feature; with it, audio starts disabled.
#[allow(clippy::derivable_impls)]
impl Default for HeadlessPlugins {
    fn default() -> Self {
        Self {
            #[cfg(feature = "physics")]
            physics: Default::default(),
            #[cfg(feature = "input")]
            input: Default::default(),
            #[cfg(feature = "script")]
            script: Default::default(),
            #[cfg(feature = "audio")]
            audio: olvin_audio::AudioPlugin {
                enabled: false,
                ..Default::default()
            },
        }
    }
}

impl Plugin for HeadlessPlugins {
    fn build(&self, app: &mut App) {
        app.add_plugin(olvin_ecs::EcsPlugin)
            .add_plugin(olvin_transform::TransformPlugin);
        #[cfg(feature = "anim")]
        app.add_plugin(olvin_anim::AnimPlugin);
        #[cfg(feature = "render")]
        app.add_plugin(olvin_render::RenderDataPlugin);
        #[cfg(feature = "scene")]
        app.add_plugin(render_assets::RenderAssetsPlugin)
            .add_plugin(models::ModelPlugin)
            .init_resource::<scene_file::SceneComponents>();
        #[cfg(feature = "physics")]
        app.add_plugin(olvin_physics::PhysicsPlugin {
            gravity: self.physics.gravity,
        });
        #[cfg(feature = "input")]
        app.add_plugin(olvin_input::InputPlugin {
            map: self.input.map.clone(),
        });
        #[cfg(feature = "script")]
        app.add_plugin(olvin_script::ScriptPlugin {
            builtins: self.script.builtins,
        });
        #[cfg(all(feature = "script", feature = "scene"))]
        app.add_system(olvin_core::Stage::Update, |res| {
            scene_file::run_prefab_spawns(res)
        });
        #[cfg(feature = "audio")]
        app.add_plugin(self.audio.clone());
        #[cfg(feature = "net")]
        app.add_plugin(net::NetGluePlugin);
        // Headless: the UI is laid out, never drawn.
        #[cfg(feature = "gui")]
        app.add_plugin(olvin_gui::GuiPlugin);
    }
}

pub mod prelude {
    pub use crate::{DefaultPlugins, HeadlessPlugins};
    pub use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
    #[cfg(feature = "anim")]
    pub use olvin_anim::{AnimGraph, AnimPlugin, Animator};
    #[cfg(feature = "audio")]
    pub use olvin_audio::{Audio, AudioListener, AudioPlugin, PlayParams};
    pub use olvin_core::{App, AppExit, Plugin, ProjectPaths, Resources, Stage, Time};
    pub use olvin_ecs::{Entity, World};
    #[cfg(all(feature = "gui", feature = "render"))]
    pub use olvin_gui::GuiRenderPlugin;
    #[cfg(feature = "gui")]
    pub use olvin_gui::{Gui, GuiPlugin, Node, NodeId, UiEvent};
    #[cfg(feature = "input")]
    pub use olvin_input::{ActionState, Binding, InputMap, InputPlugin};
    #[cfg(feature = "physics")]
    pub use olvin_physics::{
        CharacterMotor, CollisionEvents, ControllerHit, Hit, Physics, PhysicsPlugin,
    };
    #[cfg(feature = "render")]
    pub use olvin_render::{
        Color, Fog, HeightFog, Light, LightKind, MeshData, MeshHandle, MeshRenderer, Meshes,
        RenderInfo, RenderPlugin, RenderSettings, RenderStats, Tonemapping,
    };
    pub use olvin_scene::{Camera, Name, SceneId, Tag, Transform};
    #[cfg(feature = "script")]
    pub use olvin_script::{
        Behaviour, FromParams, ParamSpec, Params, ScriptAppExt, ScriptCtx, ScriptPlugin,
        ScriptRegistry, Scripts,
    };
    pub use olvin_transform::{GlobalTransform, Parent};
    #[cfg(feature = "window")]
    pub use olvin_window::{Input, KeyCode, MouseButton, WindowPlugin};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_plugins_run_without_window_or_gpu() {
        let mut app = App::new();
        app.add_plugin(HeadlessPlugins::default());
        for _ in 0..3 {
            app.update();
        }
        #[cfg(feature = "anim")]
        assert!(app.has_plugin::<olvin_anim::AnimPlugin>());
        #[cfg(feature = "window")]
        assert!(!app.has_plugin::<olvin_window::WindowPlugin>());
        #[cfg(feature = "render")]
        {
            assert!(!app.has_plugin::<olvin_render::RenderPlugin>());
            assert!(app.resources.contains::<olvin_render::Meshes>());
        }
        #[cfg(feature = "audio")]
        assert!(!app.resources.fetch::<olvin_audio::Audio>().is_available());
    }
}

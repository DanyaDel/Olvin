//! Olvin: a modular game engine. Each subsystem is its own crate and plugin;
//! this facade re-exports them and wires the default set together.
//! Disable features (`window`, `render`) for servers, tools or custom platforms;
//! `scene` adds loading of scenes saved by olvin-editor, `anim` (animation,
//! no renderer needed) and `assets` (glTF import, asset manager) the crates
//! behind it.

pub use glam as math;
#[cfg(feature = "anim")]
pub use olvin_anim as anim;
#[cfg(feature = "assets")]
pub use olvin_assets as assets;
#[cfg(feature = "audio")]
pub use olvin_audio as audio;
pub use olvin_core as core;
pub use olvin_ecs as ecs;
#[cfg(feature = "game")]
pub use olvin_game as game;
#[cfg(feature = "input")]
pub use olvin_input as input;
#[cfg(feature = "physics")]
pub use olvin_physics as physics;
#[cfg(feature = "render")]
pub use olvin_render as render;
pub use olvin_scene as scene;
#[cfg(feature = "script")]
pub use olvin_script as script;
pub use olvin_transform as transform;
#[cfg(feature = "window")]
pub use olvin_window as window;

#[cfg(feature = "scene")]
pub use olvin_csg as csg;
#[cfg(feature = "scene")]
pub mod models;
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
            .add_plugin(models::ModelPlugin);
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
        #[cfg(feature = "audio")]
        app.add_plugin(olvin_audio::AudioPlugin {
            assets_dir: self.audio.assets_dir.clone(),
            enabled: self.audio.enabled,
        });
    }
}

pub mod prelude {
    pub use crate::DefaultPlugins;
    pub use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
    #[cfg(feature = "anim")]
    pub use olvin_anim::{AnimGraph, AnimPlugin, Animator};
    #[cfg(feature = "audio")]
    pub use olvin_audio::{Audio, AudioListener, AudioPlugin, PlayParams};
    pub use olvin_core::{App, AppExit, Plugin, Resources, Stage, Time};
    pub use olvin_ecs::{Entity, World};
    #[cfg(feature = "input")]
    pub use olvin_input::{ActionState, Binding, InputMap, InputPlugin};
    #[cfg(feature = "physics")]
    pub use olvin_physics::{CharacterMotor, CollisionEvents, Hit, Physics, PhysicsPlugin};
    #[cfg(feature = "render")]
    pub use olvin_render::{
        Color, Fog, Light, LightKind, MeshData, MeshHandle, MeshRenderer, Meshes, RenderInfo,
        RenderPlugin, RenderSettings, RenderStats, Tonemapping,
    };
    pub use olvin_scene::{Camera, Name, Tag, Transform};
    #[cfg(feature = "script")]
    pub use olvin_script::{
        Behaviour, FromParams, ParamSpec, Params, ScriptAppExt, ScriptCtx, ScriptPlugin,
        ScriptRegistry, Scripts,
    };
    pub use olvin_transform::{GlobalTransform, Parent};
    #[cfg(feature = "window")]
    pub use olvin_window::{Input, KeyCode, MouseButton, WindowPlugin};
}

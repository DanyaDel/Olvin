//! Olvin: a modular game engine. Each subsystem is its own crate and plugin;
//! this facade re-exports them and wires the default set together.
//! Disable features (`window`, `render`) for servers, tools or custom platforms;
//! `scene` adds loading of scenes saved by olvin-editor.

pub use glam as math;
pub use olvin_core as core;
pub use olvin_ecs as ecs;
#[cfg(feature = "render")]
pub use olvin_render as render;
pub use olvin_scene as scene;
#[cfg(feature = "window")]
pub use olvin_window as window;

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
}

impl Plugin for DefaultPlugins {
    fn build(&self, app: &mut App) {
        app.add_plugin(olvin_ecs::EcsPlugin);
        #[cfg(feature = "window")]
        app.add_plugin(self.window.clone());
        #[cfg(feature = "render")]
        app.add_plugin(olvin_render::RenderPlugin { settings: self.render.settings.clone() });
    }
}

pub mod prelude {
    pub use crate::DefaultPlugins;
    pub use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
    pub use olvin_core::{App, AppExit, Plugin, Resources, Stage, Time};
    pub use olvin_ecs::{Entity, World};
    #[cfg(feature = "render")]
    pub use olvin_render::{
        Color, MeshData, MeshHandle, MeshRenderer, Meshes, RenderInfo, RenderPlugin,
        RenderSettings, RenderStats,
    };
    pub use olvin_scene::{Camera, Transform};
    #[cfg(feature = "window")]
    pub use olvin_window::{Input, KeyCode, MouseButton, WindowPlugin};
}

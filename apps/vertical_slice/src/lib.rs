//! Vertical slice: a small first-person game on the current Olvin, built the
//! way the engine intends (facade `olvin`, a `.scene.ron` level, `olvin_game`
//! states and save slots, Lua and Rust behaviours). What it had to work
//! around is listed in `GAPS.md` next to this crate.
//!
//! The game is a plugin ([`GamePlugin`]) on top of the engine's plugin set:
//! the binary puts `DefaultPlugins` (window + renderer) in front of it, the
//! tests `HeadlessPlugins`. Files are found through the engine's
//! `ProjectPaths` (this crate's folder under cargo, the exe's folder after
//! `olvin-hub build`).

pub mod door;
pub mod hud;
pub mod level;
pub mod player;
pub mod robot;
pub mod save;

use std::path::PathBuf;

use olvin::game::{SavePlugin, SceneLoaderPlugin, State, StateAppExt};
use olvin::prelude::*;
use olvin::script::lua::{LuaLimits, LuaPlugin};

/// Scene inside `assets/`.
pub const SCENE: &str = "scenes/level.scene.ron";
/// Slot used by F5 / F9.
pub const QUICK_SLOT: &str = "quick";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameState {
    /// The level is parsed on a worker thread (`SceneLoader`).
    Loading,
    Playing,
    /// Simulation time stopped, cursor free.
    Paused,
}

#[derive(Clone, Debug)]
pub struct SliceConfig {
    /// Where save slots go (`None`: `saves/` in the project folder).
    pub saves: Option<PathBuf>,
    /// Re-read changed `*.lua` files while running.
    pub lua_hot_reload: bool,
}

impl Default for SliceConfig {
    fn default() -> Self {
        Self {
            saves: None,
            lua_hot_reload: cfg!(debug_assertions),
        }
    }
}

/// The default action map plus the slice's own actions.
pub fn input_map() -> InputMap {
    let mut map = InputMap::default();
    map.bind("QuickSave", [Binding::Key(KeyCode::F5)]);
    map.bind("QuickLoad", [Binding::Key(KeyCode::F9)]);
    map.bind("Quit", [Binding::Key(KeyCode::KeyQ)]);
    map
}

/// Where the game's files live: the engine's resource (`assets`, `root`).
pub type Paths = ProjectPaths;

/// The whole game: behaviours, save types, states, systems, and the plugins
/// only this game configures (Lua, scene loader, saves). The engine plugins
/// come first, from `DefaultPlugins` or `HeadlessPlugins`.
pub struct GamePlugin {
    pub config: SliceConfig,
}

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        let c = &self.config;
        let paths = app.resources.fetch::<ProjectPaths>().clone();
        // Keeps a saved map, adds the slice's actions if they are missing.
        app.resources
            .fetch_mut::<InputMap>()
            .fill_missing(&input_map());
        app.add_plugin(LuaPlugin {
            dir: paths.asset("scripts"),
            hot_reload: c.lua_hot_reload,
            limits: LuaLimits::default(),
        })
        .add_plugin(SceneLoaderPlugin)
        .add_plugin(SavePlugin::new(
            c.saves.clone().unwrap_or_else(|| paths.resolve("saves")),
        ))
        .init_resource::<hud::Hud>()
        .init_resource::<save::Progress>();
        {
            let mut scripts = app.resources.fetch_mut::<ScriptRegistry>();
            scripts.register::<door::DoorSwitch>("DoorSwitch");
            scripts.register::<door::Thud>("Thud");
        }
        {
            let mut components = app
                .resources
                .fetch_mut::<olvin::scene_file::SceneComponents>();
            components
                .register::<door::DoorState>("DoorState")
                .register::<player::Player>("Player")
                .register::<player::PlayerCamera>("PlayerCamera")
                .register::<player::PlayerLook>("PlayerLook");
        }
        save::register(&mut app.resources.fetch_mut::<olvin::game::SaveRegistry>());

        app.add_state(GameState::Loading)
            .on_enter(GameState::Loading, level::request)
            .add_system_in_state(Stage::Update, GameState::Loading, level::finish_loading)
            .on_enter(GameState::Playing, |res| {
                res.fetch_mut::<Time>().resume();
                res.fetch_mut::<ActionState>().lock_cursor(true);
            })
            .on_enter(GameState::Paused, |res| {
                // Also holds sfx and ambient sounds; music plays on.
                res.fetch_mut::<Time>().pause();
                res.fetch_mut::<ActionState>().lock_cursor(false);
                log::info!("paused (Esc: resume, Q: quit)");
            })
            .add_system(Stage::Update, pause_toggle)
            .add_system_in_state(Stage::Update, GameState::Playing, player::control)
            .add_system_in_state(Stage::Update, GameState::Playing, save::hotkeys)
            .add_system(Stage::Update, door::animate)
            .add_system(Stage::Update, robot::update)
            .add_system(Stage::Startup, hud::build)
            .add_system(Stage::Update, hud::update);
    }
}

/// Esc toggles pause; Q quits while paused.
fn pause_toggle(res: &mut Resources) {
    let (pause, quit) = {
        let actions = res.fetch::<ActionState>();
        (actions.just_pressed("Pause"), actions.just_pressed("Quit"))
    };
    let mut state = res.fetch_mut::<State<GameState>>();
    match state.get() {
        GameState::Playing if pause => state.set(GameState::Paused),
        GameState::Paused if pause => state.set(GameState::Playing),
        GameState::Paused if quit => res.fetch_mut::<AppExit>().requested = true,
        _ => {}
    }
}

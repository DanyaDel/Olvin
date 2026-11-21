//! Vertical slice: a small first-person game on the current Olvin, built the
//! way the engine intends (facade `olvin`, a `.scene.ron` level, `olvin_game`
//! states and save slots, Lua and Rust behaviours). What it had to work
//! around is listed in `GAPS.md` next to this crate.
//!
//! The game is a plugin ([`GamePlugin`]) so tests can run it headless: the
//! binary adds `DefaultPlugins` (window + renderer) in front of it.

pub mod door;
pub mod hud;
pub mod level;
pub mod player;
pub mod robot;
pub mod save;

use std::path::{Path, PathBuf};

use olvin::audio::BusId;
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
    /// The game's `assets/` folder (scene, scripts, model, sounds).
    pub assets: PathBuf,
    /// Where save slots go.
    pub saves: PathBuf,
    /// Open an audio device (off in tests).
    pub audio: bool,
    /// Re-read changed `*.lua` files while running.
    pub lua_hot_reload: bool,
}

impl SliceConfig {
    /// Assets found by [`find_assets`], saves in `saves/` next to them.
    pub fn detect() -> Self {
        let assets = find_assets();
        let root = assets.parent().unwrap_or(Path::new(".")).to_path_buf();
        Self {
            saves: root.join("saves"),
            assets,
            audio: true,
            lua_hot_reload: cfg!(debug_assertions),
        }
    }

    /// Folder that holds `assets/`, `saves/` and `logs/`.
    pub fn root(&self) -> PathBuf {
        self.assets.parent().unwrap_or(Path::new(".")).to_path_buf()
    }
}

/// The engine resolves every asset path against the current directory, so the
/// game looks for its `assets/` itself (see GAPS.md): next to the executable
/// (the `olvin-hub build` layout), then in this crate's folder (`cargo run -p
/// vertical_slice` from anywhere in the workspace), then `./assets`.
pub fn find_assets() -> PathBuf {
    let marker = |dir: &Path| dir.join(SCENE).is_file();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("assets")))
        .filter(|d| marker(d))
    {
        return dir;
    }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    if marker(&dev) {
        return dev;
    }
    PathBuf::from("assets")
}

/// The default action map plus the slice's own actions.
pub fn input_map() -> InputMap {
    let mut map = InputMap::default();
    map.bind("QuickSave", [Binding::Key(KeyCode::F5)]);
    map.bind("QuickLoad", [Binding::Key(KeyCode::F9)]);
    map.bind("Quit", [Binding::Key(KeyCode::KeyQ)]);
    map
}

/// Where the game's files live, as a resource for systems.
#[derive(Clone, Debug)]
pub struct Paths {
    pub assets: PathBuf,
}

/// The whole game. Adds the engine plugins it needs (each is skipped if
/// `DefaultPlugins` added it before), behaviours, save types, states, systems.
pub struct GamePlugin {
    pub config: SliceConfig,
}

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        let c = &self.config;
        app.add_plugin(olvin::ecs::EcsPlugin)
            .add_plugin(olvin::transform::TransformPlugin)
            // Scene spawning needs the mesh registry even without a renderer.
            .init_resource::<Meshes>()
            // RenderPlugin adds it; headless runs need it too.
            .add_plugin(olvin::anim::AnimPlugin)
            // Models in the scene (the robot): built once the asset manager
            // has imported them. Headless runs have no RenderAssets root.
            .insert_resource(olvin::assets::AssetManager::new(c.assets.clone()))
            .add_plugin(olvin::models::ModelPlugin)
            .add_plugin(PhysicsPlugin::default())
            .add_plugin(InputPlugin {
                map: Some(input_map()),
            })
            .add_plugin(AudioPlugin {
                assets_dir: c.assets.clone(),
                enabled: c.audio,
            })
            .add_plugin(LuaPlugin {
                dir: c.assets.join("scripts"),
                hot_reload: c.lua_hot_reload,
                limits: LuaLimits::default(),
            })
            .add_plugin(SceneLoaderPlugin)
            .add_plugin(SavePlugin::new(c.saves.clone()))
            .insert_resource(Paths {
                assets: c.assets.clone(),
            })
            .init_resource::<hud::Hud>()
            .init_resource::<save::Progress>();
        {
            let mut scripts = app.resources.fetch_mut::<ScriptRegistry>();
            scripts.register::<door::DoorSwitch>("DoorSwitch");
            scripts.register::<door::Thud>("Thud");
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
                res.fetch_mut::<Time>().pause();
                res.fetch_mut::<ActionState>().lock_cursor(false);
                // No way to pause sounds: mute the effect buses instead (GAPS.md).
                let mut audio = res.fetch_mut::<Audio>();
                for bus in [BusId::SFX, BusId::AMBIENT] {
                    audio.set_bus_muted(bus, true);
                }
                log::info!("paused (Esc: resume, Q: quit)");
            })
            .on_exit(GameState::Paused, |res| {
                let mut audio = res.fetch_mut::<Audio>();
                for bus in [BusId::SFX, BusId::AMBIENT] {
                    audio.set_bus_muted(bus, false);
                }
            })
            .add_system(Stage::Update, pause_toggle)
            .add_system_in_state(Stage::Update, GameState::Playing, player::control)
            .add_system_in_state(Stage::Update, GameState::Playing, save::hotkeys)
            .add_system(Stage::Update, door::animate)
            .add_system(Stage::Update, robot::update)
            .add_system(Stage::PostUpdate, hud::update);
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

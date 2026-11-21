//! Loading the level: `SceneLoader` parses the scene on a worker thread
//! (state `Loading`), then the facade spawns it, its models load in the
//! background too, and the game attaches what a scene file cannot express
//! (game components, save ids, HUD).

use olvin::game::{LoadHandle, LoadStatus, SaveId, SceneLoader, State};
use olvin::prelude::*;
use olvin::render_assets::RenderAssets;

use crate::door::DoorState;
use crate::hud::{self, Hud};
use crate::player::{Player, PlayerCamera, PlayerLook};
use crate::{GameState, Paths, SCENE, robot};

/// The load in flight.
pub struct LevelLoad(pub LoadHandle);

/// The level is spawned; its models (the robot) are still loading.
pub struct ModelsLoading;

/// Save ids of level objects. Spawned entities do not keep their scene id,
/// so they are matched by name (GAPS.md).
pub const SAVED: &[(&str, u64)] = &[
    ("Player", 100),
    ("Door", 20),
    ("Crate 1", 40),
    ("Crate 2", 41),
    ("Crate 3", 42),
    ("Crate 4", 43),
    ("Ball", 44),
];

pub const DOOR_LIFT: f32 = 3.1;

/// `on_enter(Loading)`: starts parsing the scene in the background.
pub fn request(res: &mut Resources) {
    let path = res.fetch::<Paths>().assets.join(SCENE);
    let handle = res.fetch_mut::<SceneLoader>().request(&path);
    res.insert(LevelLoad(handle));
}

/// While `Loading`: once the scene is parsed, spawn it and start playing.
pub fn finish_loading(res: &mut Resources) {
    if res.get::<ModelsLoading>().is_some() {
        if olvin::models::pending_count(res) == 0 {
            res.remove::<ModelsLoading>();
            setup(res);
            res.fetch_mut::<State<GameState>>().set(GameState::Playing);
        }
        return;
    }
    let Some(handle) = res.get::<LevelLoad>().map(|l| l.0) else {
        return;
    };
    let status = res.fetch::<SceneLoader>().status(handle);
    match status {
        LoadStatus::Pending => return,
        LoadStatus::Ready | LoadStatus::Failed => {}
        LoadStatus::Gone => {
            log::error!("level load handle lost");
            res.fetch_mut::<AppExit>().requested = true;
            return;
        }
    }
    let Some(result) = res.fetch_mut::<SceneLoader>().take(handle) else {
        return;
    };
    res.remove::<LevelLoad>();
    let scene = match result {
        Ok(scene) => scene,
        Err(e) => {
            log::error!("level failed to load: {e}");
            res.fetch_mut::<AppExit>().requested = true;
            return;
        }
    };
    // `scene_file::load` points material lookups at assets/, a background
    // load has to do it by hand.
    let assets = res.fetch::<Paths>().assets.clone();
    if let Some(mut ra) = res.get_mut::<RenderAssets>() {
        ra.set_root(Some(assets.clone()));
    }
    let count = olvin::scene_file::spawn(res, &scene);
    log::info!("level: {count} objects");
    // Playing starts once the models are built (next frames).
    res.insert(ModelsLoading);
}

fn find(world: &World, name: &str) -> Option<Entity> {
    world
        .query::<(Entity, &Name)>()
        .iter()
        .find(|(_, n)| n.0 == name)
        .map(|(e, _)| e)
}

/// Attaches game components to the spawned level and adds the rest.
pub fn setup(res: &mut Resources) {
    {
        let mut world = res.fetch_mut::<World>();
        for (name, id) in SAVED {
            match find(&world, name) {
                Some(e) => {
                    let _ = world.insert_one(e, SaveId(*id));
                }
                None => log::warn!("level has no `{name}`"),
            }
        }
        if let Some(e) = find(&world, "Player") {
            let _ = world.insert(e, (Player, PlayerLook::default()));
        }
        if let Some(e) = find(&world, "Player Camera") {
            let _ = world.insert_one(e, PlayerCamera);
        }
        if let Some(e) = find(&world, "Door") {
            let y = world.get::<&Transform>(e).map_or(1.5, |t| t.translation.y);
            let _ = world.insert_one(
                e,
                DoorState {
                    open: false,
                    closed_y: y,
                    lift: DOOR_LIFT,
                },
            );
        }
        match find(&world, robot::NAME) {
            Some(e) => {
                let _ = world.insert_one(e, robot::Robot);
            }
            None => log::warn!("level has no `{}`", robot::NAME),
        }
        hud::spawn(&mut world);
    }
    res.fetch_mut::<Audio>()
        .play_music("music/theme.wav", 0.35, true);
    res.fetch_mut::<Hud>().show(
        "WASD — ходить, мышь — смотреть, Space — прыжок, Shift — бег, \
         Esc — пауза, F5/F9 — сохранить/загрузить. Встань на жёлтую плиту.",
    );
}

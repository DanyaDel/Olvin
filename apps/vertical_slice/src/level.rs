//! Loading the level: `SceneLoader` parses the scene on a worker thread
//! (state `Loading`), then the facade spawns it (scene ids and game components
//! included), its models load in the background too, and the game adds what a
//! scene file cannot express (the robot marker, music, the first hint).

use olvin::game::{LoadHandle, LoadStatus, SceneLoader, State};
use olvin::prelude::*;

use crate::hud::Hud;
use crate::{GameState, Paths, SCENE, robot};

/// The load in flight.
pub struct LevelLoad(pub LoadHandle);

/// The level is spawned; its models (the robot) are still loading.
pub struct ModelsLoading;

/// `on_enter(Loading)`: starts parsing the scene in the background.
pub fn request(res: &mut Resources) {
    let path = res.fetch::<Paths>().assets.join(SCENE);
    let handle = res.fetch_mut::<SceneLoader>().request(&path);
    res.insert(LevelLoad(handle));
}

/// While `Loading`: once the scene is parsed, spawn it; once its models are
/// built, start playing.
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
    // Materials resolve like in `scene_file::load`; game components and save
    // ids come from the scene file itself (`SceneId`, `components`).
    let Some(result) = olvin::scene_file::spawn_loaded(res, handle) else {
        return;
    };
    res.remove::<LevelLoad>();
    match result {
        Ok(ids) => log::info!("level: {} objects", ids.len()),
        Err(e) => {
            log::error!("level failed to load: {e}");
            res.fetch_mut::<AppExit>().requested = true;
            return;
        }
    }
    // Playing starts once the models are built (next frames).
    res.insert(ModelsLoading);
}

/// Adds what a scene file cannot express: the robot marker, music and the hint.
pub fn setup(res: &mut Resources) {
    {
        let mut world = res.fetch_mut::<World>();
        let robot_entity = world
            .query::<(Entity, &Name)>()
            .iter()
            .find(|(_, n)| n.0 == robot::NAME)
            .map(|(e, _)| e);
        match robot_entity {
            Some(e) => {
                let _ = world.insert_one(e, robot::Robot);
            }
            None => log::warn!("level has no `{}`", robot::NAME),
        }
    }
    res.fetch_mut::<Audio>()
        .play_music("music/theme.wav", 0.35, true);
    res.fetch_mut::<Hud>().show(
        "WASD — ходить, мышь — смотреть, Space — прыжок, Shift — бег\n\
         Esc — пауза, F5/F9 — сохранить/загрузить\n\
         Встань на жёлтую плиту",
    );
}

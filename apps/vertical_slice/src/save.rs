//! Save slots through `olvin_game::save`: F5 saves to the quick slot, F9
//! loads it. Level objects are saved under their scene ids; `SavePlugin`
//! registers `Transform`, velocities and the character motor, the game adds
//! its own components and [`Progress`].

use olvin::game::{SaveMeta, SaveRegistry, SaveSlots};
use olvin::prelude::*;
use serde::{Deserialize, Serialize};

use crate::door;
use crate::hud::Hud;
use crate::player::PlayerLook;
use crate::{QUICK_SLOT, SCENE};

/// Session-wide state, saved as a resource. `door_opened` mirrors the Lua
/// scripts' shared variable `door_open` (see `door::bridge`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub door_opened: bool,
    pub saves: u32,
}

pub fn register(registry: &mut SaveRegistry) {
    // Transforms, velocities and the character motor come with `SavePlugin`.
    registry
        .register_component::<PlayerLook>("look")
        .register_resource::<Progress>("progress");
}

/// F5 / F9 while playing.
pub fn hotkeys(res: &mut Resources) {
    let (save, load) = {
        let a = res.fetch::<ActionState>();
        (a.just_pressed("QuickSave"), a.just_pressed("QuickLoad"))
    };
    if save {
        let text = match save_slot(res, QUICK_SLOT) {
            Ok(()) => "Сохранено (F9 — загрузить)".to_string(),
            Err(e) => format!("Не сохранилось: {e}"),
        };
        res.fetch_mut::<Hud>().show(&text);
    }
    if load {
        let text = match load_slot(res, QUICK_SLOT) {
            Ok(()) => "Загружено".to_string(),
            Err(e) => format!("Не загрузилось: {e}"),
        };
        res.fetch_mut::<Hud>().show(&text);
    }
}

/// Snapshots every scene object and [`Progress`] into `slot`.
pub fn save_slot(res: &Resources, slot: &str) -> Result<(), String> {
    door::sync_progress(res);
    res.fetch_mut::<Progress>().saves += 1;
    let meta = SaveMeta {
        title: "Quick save".into(),
        scene: SCENE.into(),
        playtime_secs: res.fetch::<Time>().elapsed(),
        saved_at: 0,
    };
    let data = res
        .fetch::<SaveRegistry>()
        .capture(res, meta)
        .map_err(|e| e.to_string())?;
    res.fetch::<SaveSlots>()
        .save(slot, &data)
        .map_err(|e| e.to_string())?;
    log::info!("saved slot `{slot}`: {} entities", data.entities.len());
    Ok(())
}

/// Restores `slot` into the running level (same scene); the engine
/// registrations teleport bodies and restore their velocities.
pub fn load_slot(res: &Resources, slot: &str) -> Result<(), String> {
    let data = res
        .fetch::<SaveSlots>()
        .load(slot)
        .map_err(|e| e.to_string())?;
    if data.meta.scene != SCENE {
        return Err(format!("save is for another scene: {}", data.meta.scene));
    }
    res.fetch::<SaveRegistry>()
        .restore(res, &data)
        .map_err(|e| e.to_string())?;
    door::restore_vars(res);
    Ok(())
}

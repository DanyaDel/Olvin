//! Save slots through `olvin_game::save`: F5 saves to the quick slot, F9
//! loads it.
//!
//! Engine types are not serializable (`Transform`, physics state), so poses
//! go through a [`Pose`] copy, and after a restore the game moves the
//! physics bodies itself: a dynamic body ignores its `Transform`.

use olvin::game::{SaveId, SaveMeta, SaveRegistry, SaveSlots};
use olvin::physics::Body;
use olvin::physics::rapier3d::prelude as rp;
use olvin::prelude::*;
use serde::{Deserialize, Serialize};

use crate::door::DoorState;
use crate::hud::Hud;
use crate::player::PlayerLook;
use crate::{QUICK_SLOT, SCENE};

/// A saved world pose (copied from `Transform` before capture).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub translation: Vec3,
    pub rotation: Quat,
}

/// Session-wide state, saved as a resource.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub door_opened: bool,
    pub saves: u32,
}

pub fn register(registry: &mut SaveRegistry) {
    registry
        .register_component::<Pose>("pose")
        .register_component::<DoorState>("door")
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

/// Snapshots every `SaveId` entity and [`Progress`] into `slot`.
pub fn save_slot(res: &Resources, slot: &str) -> Result<(), String> {
    {
        let mut world = res.fetch_mut::<World>();
        let poses: Vec<(Entity, Pose)> = world
            .query::<(Entity, &Transform)>()
            .with::<&SaveId>()
            .iter()
            .map(|(e, t)| {
                (
                    e,
                    Pose {
                        translation: t.translation,
                        rotation: t.rotation,
                    },
                )
            })
            .collect();
        for (e, pose) in poses {
            let _ = world.insert_one(e, pose);
        }
    }
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

/// Restores `slot` into the running level (same scene), then moves physics
/// bodies to the restored poses and stops them.
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
    apply_poses(res);
    Ok(())
}

fn apply_poses(res: &Resources) {
    let mut world = res.fetch_mut::<World>();
    let mut physics = res.fetch_mut::<Physics>();
    for (pose, t, body, motor) in world.query_mut::<(
        &Pose,
        &mut Transform,
        Option<&Body>,
        Option<&mut CharacterMotor>,
    )>() {
        t.translation = pose.translation;
        t.rotation = pose.rotation;
        // A character teleports when its Transform jumps; just drop its speed.
        if let Some(motor) = motor {
            motor.velocity = Vec3::ZERO;
        }
        // A dynamic body owns its pose: move it in Rapier directly.
        if let Some(body) = body
            && let Some(rb) = physics.world.bodies.get_mut(body.handle)
            && rb.is_dynamic()
        {
            let iso = rp::Pose::from_parts(
                rp::Vector::from_array(pose.translation.to_array()),
                rp::Rotation::from_array(pose.rotation.to_array()),
            );
            rb.set_position(iso, true);
            rb.set_linvel(rp::Vector::ZERO, true);
            rb.set_angvel(rp::Vector::ZERO, true);
        }
    }
}

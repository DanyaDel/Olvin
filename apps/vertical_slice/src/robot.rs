//! The animated character: a skinned glTF robot that idles, turns to the
//! player and waves when the player comes close.
//!
//! Scenes cannot reference models, and the facade exposes neither the glTF
//! importer nor the Animator, so this is spawned by hand from `olvin_assets`
//! and `olvin_anim` (see GAPS.md).

use std::path::Path;

use olvin::prelude::*;
use olvin::render::{SkinnedMesh, Vertex};
use olvin::scene::file::{ColliderDesc, ColliderShape};
use olvin_anim::{AnimGraph, Animator, Motion, State as AnimState, Test, Transition};
use olvin_assets::{ModelImport, import_gltf};

use crate::player::Player;

pub const MODEL: &str = "models/robot.gltf";
/// The robot waves while the player is closer than this.
pub const WAVE_DISTANCE: f32 = 3.5;
/// Animator parameter driving Idle ↔ Wave.
pub const NEAR_PARAM: &str = "near";

/// Marks the robot's root (it carries the `Animator`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Robot;

/// Idle ↔ Wave on the bool parameter [`NEAR_PARAM`].
pub fn graph(idle: usize, wave: usize) -> AnimGraph {
    AnimGraph {
        states: vec![
            AnimState::new("Idle", Motion::Clip(idle)),
            AnimState::new("Wave", Motion::Clip(wave)),
        ],
        transitions: vec![
            Transition::new(Some(0), 1, 0.25).when(NEAR_PARAM, Test::True),
            Transition::new(Some(1), 0, 0.4).when(NEAR_PARAM, Test::False),
        ],
        default: 0,
    }
}

/// Imports the model and spawns the robot at `at`: a root with the
/// `Animator` and a static capsule, plus one skinned mesh child per glTF
/// primitive. Returns the root.
pub fn spawn(res: &Resources, assets: &Path, at: Transform) -> Result<Entity, String> {
    let model = import_gltf(assets.join(MODEL), assets, &ModelImport::default())?;
    for s in &model.skipped {
        log::warn!("robot.gltf: {s}");
    }
    let clip = |name: &str| {
        model
            .clips
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| format!("{MODEL}: no clip `{name}`"))
    };
    let animator: Animator = model
        .animator(0)
        .ok_or_else(|| format!("{MODEL}: no skin"))?
        .with_graph(graph(clip("Idle")?, clip("Wave")?));

    let mut world = res.fetch_mut::<World>();
    let mut meshes = res.fetch_mut::<Meshes>();
    let mut collider = ColliderDesc::new(ColliderShape::Capsule {
        radius: 0.35,
        height: 1.9,
    });
    collider.center = Vec3::new(0.0, 0.95, 0.0);
    let root = world.spawn((Name("Robot".into()), at, Robot, animator, collider));
    for (i, prim) in model.meshes.iter().enumerate() {
        if prim.skin != Some(0) {
            continue;
        }
        let vertices = prim
            .positions
            .iter()
            .zip(&prim.normals)
            .map(|(p, n)| Vertex::new(p.to_array(), n.to_array()))
            .collect();
        let bind = MeshData {
            vertices,
            indices: prim.indices.clone(),
        };
        let skinned = SkinnedMesh::new(bind, prim.joints.clone(), prim.weights.clone(), root)?;
        let mesh = meshes.add(skinned.bind_mesh());
        let color = prim
            .material
            .and_then(|m| model.materials.get(m))
            .map_or([0.8; 4], |m| {
                // No textures here: fold emissive into the flat colour.
                let [r, g, b, a] = m.base_color;
                let [er, eg, eb] = m.emissive;
                [r.max(er), g.max(eg), b.max(eb), a]
            });
        world.spawn((
            Name(format!(
                "Robot/{}",
                if prim.name.is_empty() {
                    i.to_string()
                } else {
                    prim.name.clone()
                }
            )),
            Transform::IDENTITY,
            Parent(root),
            MeshRenderer {
                mesh,
                color: Color(color),
                material: None,
            },
            skinned,
        ));
    }
    Ok(root)
}

/// Faces the player and waves when they are near. Simulation time, so it
/// freezes on pause like the animation itself.
pub fn update(res: &mut Resources) {
    let dt = res.fetch::<Time>().delta();
    let mut world = res.fetch_mut::<World>();
    let Some(player) = world
        .query_mut::<(&Transform, &Player)>()
        .into_iter()
        .map(|(t, _)| t.translation)
        .next()
    else {
        return;
    };
    for (t, animator, _) in world.query_mut::<(&mut Transform, &mut Animator, &Robot)>() {
        let to = player - t.translation;
        let flat = Vec2::new(to.x, to.z);
        let near = flat.length() < WAVE_DISTANCE;
        animator.params.set_bool(NEAR_PARAM, near);
        if near && flat.length() > 0.1 {
            // The model faces +Z.
            let want = Quat::from_rotation_y(flat.x.atan2(flat.y));
            t.rotation = t.rotation.slerp(want, (dt * 6.0).min(1.0));
        }
    }
}

/// Name of the state the robot's base layer is in (tests, debugging).
pub fn current_state(animator: &Animator) -> Option<&str> {
    let layer = animator.layers.first()?;
    layer
        .graph
        .states
        .get(layer.state.current)
        .map(|s| s.name.as_str())
}

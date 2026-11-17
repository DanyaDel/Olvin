//! Scenes saved by olvin-editor (`assets/scenes/*.scene.ron`).

use std::collections::HashMap;
use std::path::Path;

use olvin_core::Resources;
use olvin_ecs::{EntityBuilder, World};
use olvin_render::{Color, MeshData, MeshHandle, MeshRenderer, Meshes};
pub use olvin_scene::file::{CameraDesc, EntityDesc, MeshDesc, SceneFile, Shape};
use olvin_scene::{Camera, Transform};

/// Spawns every entity with its world transform (the hierarchy is flattened:
/// moving a parent at runtime won't move its children yet). Returns the count.
pub fn spawn(res: &Resources, scene: &SceneFile) -> usize {
    let worlds = scene.world_matrices();
    let mut meshes = res.fetch_mut::<Meshes>();
    let mut world = res.fetch_mut::<World>();
    let mut shapes: HashMap<Shape, MeshHandle> = HashMap::new();

    for e in &scene.entities {
        let matrix = worlds.get(&e.id).copied().unwrap_or_else(|| e.local_transform().matrix());
        let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
        let mut builder = EntityBuilder::new();
        builder.add(Transform { translation, rotation, scale });
        if let Some(mesh) = &e.mesh {
            let handle = *shapes.entry(mesh.shape).or_insert_with(|| {
                meshes.add(match mesh.shape {
                    Shape::Cube => MeshData::cube(1.0),
                    Shape::Plane => MeshData::plane(1.0),
                })
            });
            builder.add(MeshRenderer { mesh: handle, color: Color(mesh.color) });
        }
        if let Some(camera) = &e.camera {
            builder.add(Camera::from(camera));
        }
        world.spawn(builder.build());
    }
    scene.entities.len()
}

/// Loads a scene file and spawns it.
pub fn load(res: &Resources, path: impl AsRef<Path>) -> Result<usize, String> {
    let scene = SceneFile::load(path)?;
    Ok(spawn(res, &scene))
}

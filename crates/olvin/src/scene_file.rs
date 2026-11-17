//! Scenes saved by olvin-editor (`assets/scenes/*.scene.ron`).

use std::collections::HashMap;
use std::path::Path;

use olvin_core::Resources;
use olvin_ecs::{EntityBuilder, World};
use olvin_render::{Color, MeshData, MeshHandle, MeshRenderer, Meshes, Vertex};
pub use olvin_scene::file::{BrushDesc, BrushShape, CameraDesc, CsgOp, EntityDesc, MeshDesc, SceneFile, Shape};
use olvin_scene::{Camera, Transform};

/// Spawns every entity with its world transform (the hierarchy is flattened:
/// moving a parent at runtime won't move its children yet) plus the CSG level
/// geometry built from the brushes. Returns the entity count.
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

    // CSG: one static mesh per face colour.
    let polygons = olvin_csg::build(&scene.csg_brushes());
    let mut by_color: HashMap<[u32; 4], Vec<&olvin_csg::Polygon>> = HashMap::new();
    for p in &polygons {
        by_color.entry(p.color.map(f32::to_bits)).or_default().push(p);
    }
    for (color, polys) in by_color {
        let tri = olvin_csg::TriMesh::from_polygons(polys);
        let vertices = tri
            .positions
            .iter()
            .zip(&tri.normals)
            .map(|(p, n)| Vertex { position: p.to_array(), normal: n.to_array() })
            .collect();
        let mesh = meshes.add(MeshData { vertices, indices: tri.indices });
        world.spawn((Transform::IDENTITY, MeshRenderer { mesh, color: Color(color.map(f32::from_bits)) }));
    }
    scene.entities.len()
}

/// Loads a scene file and spawns it.
pub fn load(res: &Resources, path: impl AsRef<Path>) -> Result<usize, String> {
    let scene = SceneFile::load(path)?;
    Ok(spawn(res, &scene))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn spawns_entities_and_csg_meshes_per_color() {
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());

        let brush = |id: u64, op: CsgOp, scale: Vec3, color: [f32; 4]| {
            let mut e = EntityDesc::new(id, "brush");
            e.scale = scale;
            e.brush = Some(BrushDesc { shape: BrushShape::Box, op, color });
            e
        };
        let scene = SceneFile {
            entities: vec![
                brush(1, CsgOp::Add, Vec3::splat(4.0), [1.0, 0.0, 0.0, 1.0]),
                brush(2, CsgOp::Subtract, Vec3::splat(2.0), [0.0, 1.0, 0.0, 1.0]),
            ],
            ..Default::default()
        };
        assert_eq!(spawn(&res, &scene), 2);
        // Two brush entities + one mesh per face colour (outer red, inner green).
        let world = res.fetch::<World>();
        assert_eq!(world.len(), 4);
        assert_eq!(world.query::<&MeshRenderer>().iter().count(), 2);
    }
}

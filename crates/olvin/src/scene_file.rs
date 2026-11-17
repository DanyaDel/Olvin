//! Scenes saved by olvin-editor (`assets/scenes/*.scene.ron`).

use std::collections::HashMap;
use std::path::Path;

use olvin_core::Resources;
use olvin_ecs::{EntityBuilder, World};
use olvin_render::{Color, MeshData, MeshHandle, MeshRenderer, Meshes, RenderSettings, Vertex};
pub use olvin_scene::file::*;
use olvin_scene::primitives::Primitive;
use olvin_scene::{Camera, MeshGeometry, Name, Tag, Transform};
use olvin_transform::{GlobalTransform, Parent};

/// Behaviours attached in the editor; game code looks them up by name.
#[derive(Clone, Debug, Default)]
pub struct Scripts(pub Vec<ScriptDesc>);

/// Marks the entity that hears audio (usually the camera).
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioListener;

/// Spawns every active entity with its world transform and components
/// (Light, Collider, Rigidbody, AudioSource, ... as plain data components),
/// the CSG level as static meshes with a mesh collider, and points the
/// renderer's sun at the first directional light. Returns the entity count.
///
/// The hierarchy is kept: `Transform` is local, `Parent` links children, and
/// `GlobalTransform` starts at the world matrix.
pub fn spawn(res: &Resources, scene: &SceneFile) -> usize {
    let worlds = scene.world_matrices();
    let mut meshes = res.fetch_mut::<Meshes>();
    let mut world = res.fetch_mut::<World>();
    let mut shapes: HashMap<Shape, MeshHandle> = HashMap::new();
    let mut entities: HashMap<u64, olvin_ecs::Entity> = HashMap::new();
    let mut spawned = 0;

    // Parents come before children in scene files, so parent entities exist already.
    for e in scene.entities.iter().filter(|e| scene.active_in_hierarchy(e.id)) {
        let matrix = worlds.get(&e.id).copied().unwrap_or_else(|| e.local_transform().matrix());
        let mut b = EntityBuilder::new();
        b.add(e.local_transform()).add(GlobalTransform(matrix)).add(Name(e.name.clone()));
        if let Some(parent) = e.parent.and_then(|p| entities.get(&p)) {
            b.add(Parent(*parent));
        }
        if !e.tag.is_empty() {
            b.add(Tag(e.tag.clone()));
        }
        if let Some(mesh) = &e.mesh {
            let handle = *shapes.entry(mesh.shape).or_insert_with(|| meshes.add(MeshData::primitive(mesh.shape)));
            b.add(MeshRenderer { mesh: handle, color: Color(mesh.color) });
        }
        if let Some(camera) = &e.camera {
            b.add(Camera::from(camera));
        }
        // A brush with a Rigidbody leaves the static level and becomes its own
        // body: its convex geometry (local space; the Transform scales it).
        if let (Some(brush), Some(_)) = (&e.brush, &e.rigidbody) {
            let input = olvin_csg::BrushInput {
                id: e.id,
                shape: brush.shape,
                op: CsgOp::Add,
                transform: glam::Mat4::IDENTITY,
                color: brush.color,
                vertices: brush.vertices.clone(),
            };
            let tri = olvin_csg::TriMesh::from_polygons(&olvin_csg::build(&[input]));
            let vertices = tri.positions.iter().zip(&tri.normals).map(|(p, n)| Vertex { position: p.to_array(), normal: n.to_array() }).collect();
            let mesh = meshes.add(MeshData { vertices, indices: tri.indices.clone() });
            b.add(MeshRenderer { mesh, color: Color(brush.color) });
            b.add(MeshGeometry { positions: tri.positions, indices: tri.indices.as_chunks::<3>().0.to_vec() });
            if e.collider.is_none() {
                b.add(ColliderDesc::new(ColliderShape::Mesh));
            }
        }
        if let Some(collider) = &e.collider {
            if collider.shape == ColliderShape::Mesh
                && let Some(mesh) = &e.mesh
            {
                let p = Primitive::new(mesh.shape);
                let indices = p.indices.as_chunks::<3>().0.to_vec();
                b.add(MeshGeometry { positions: p.positions, indices });
            }
            b.add(collider.clone());
        }
        if let Some(c) = &e.rigidbody {
            b.add(c.clone());
        }
        if let Some(c) = &e.light {
            b.add(c.clone()).add(runtime_light(c));
        }
        if let Some(c) = &e.character_controller {
            b.add(c.clone());
        }
        if let Some(c) = &e.audio_source {
            b.add(c.clone());
        }
        if e.audio_listener {
            b.add(AudioListener);
        }
        if !e.scripts.is_empty() {
            b.add(Scripts(e.scripts.clone()));
        }
        entities.insert(e.id, world.spawn(b.build()));
        spawned += 1;
    }

    if let Some((dir, color, intensity)) = scene.sun()
        && let Some(mut settings) = res.get_mut::<RenderSettings>()
    {
        settings.light_dir = dir;
        settings.light_color = Color([color[0] * intensity, color[1] * intensity, color[2] * intensity, 1.0]);
    }

    // CSG: one static mesh per face colour, and one mesh collider for the level.
    let brushes: Vec<_> = scene
        .csg_brushes()
        .into_iter()
        .filter(|b| scene.active_in_hierarchy(b.id) && scene.get(b.id).is_some_and(|e| e.rigidbody.is_none()))
        .collect();
    let polygons = olvin_csg::build(&brushes);
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
    if !polygons.is_empty() {
        let tri = olvin_csg::TriMesh::from_polygons(&polygons);
        let indices = tri.indices.as_chunks::<3>().0.to_vec();
        world.spawn((
            Transform::IDENTITY,
            Name("CSG Level".into()),
            ColliderDesc::new(ColliderShape::Mesh),
            MeshGeometry { positions: tri.positions, indices },
        ));
    }
    spawned
}

/// The renderer's light for a scene light.
pub fn runtime_light(d: &LightDesc) -> olvin_render::Light {
    olvin_render::Light {
        kind: match d.kind {
            LightKind::Directional => olvin_render::LightKind::Directional,
            LightKind::Point => olvin_render::LightKind::Point,
            LightKind::Spot => olvin_render::LightKind::Spot,
        },
        color: d.color,
        intensity: d.intensity,
        range: d.range,
        spot_angle_deg: d.spot_angle,
        shadows: d.shadows,
    }
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
    use olvin_scene::file::LightKind;

    #[test]
    fn spawns_entities_and_csg_meshes_per_color() {
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());

        let brush = |id: u64, op: CsgOp, scale: Vec3, color: [f32; 4]| {
            let mut e = EntityDesc::new(id, "brush");
            e.scale = scale;
            e.brush = Some(BrushDesc { shape: BrushShape::Box, op, color, ..Default::default() });
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
        // Two brushes + one mesh per face colour (outer red, inner green) + the level collider.
        let world = res.fetch::<World>();
        assert_eq!(world.len(), 5);
        assert_eq!(world.query::<&MeshRenderer>().iter().count(), 2);
        assert_eq!(world.query::<(&ColliderDesc, &MeshGeometry)>().iter().count(), 1);
    }

    #[test]
    fn components_inactive_objects_and_sun() {
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());

        let mut sun = EntityDesc::new(1, "Sun");
        sun.rotation = Vec3::new(-90.0, 0.0, 0.0);
        sun.light = Some(LightDesc::new(LightKind::Directional));
        let mut crate_ = EntityDesc::new(2, "Crate");
        crate_.mesh = Some(MeshDesc { shape: Shape::Sphere, color: [1.0; 4] });
        crate_.collider = Some(ColliderDesc::for_shape(Shape::Sphere));
        crate_.rigidbody = Some(RigidbodyDesc::default());
        crate_.tag = "Player".into();
        let mut hidden = EntityDesc::new(3, "Hidden");
        hidden.active = false;
        let mut child = EntityDesc::new(4, "Child of hidden");
        child.parent = Some(3);
        let scene = SceneFile { entities: vec![sun, crate_, hidden, child], ..Default::default() };

        assert_eq!(spawn(&res, &scene), 2, "inactive parents hide their children");
        assert_eq!(res.fetch::<World>().query::<&Parent>().iter().count(), 0);
        let world = res.fetch::<World>();
        assert_eq!(world.query::<(&RigidbodyDesc, &ColliderDesc, &Tag)>().iter().count(), 1);
        let dir = res.fetch::<RenderSettings>().light_dir;
        assert!(dir.abs_diff_eq(Vec3::NEG_Y, 1e-4), "sun pitched -90° shines down: {dir}");
    }

    #[test]
    fn hierarchy_stays_live_at_runtime() {
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());
        let mut parent = EntityDesc::new(1, "Parent");
        parent.position = Vec3::new(10.0, 0.0, 0.0);
        let mut child = EntityDesc::new(2, "Child");
        child.parent = Some(1);
        child.position = Vec3::new(1.0, 0.0, 0.0);
        spawn(&res, &SceneFile { entities: vec![parent, child], ..Default::default() });

        let mut world = res.fetch_mut::<World>();
        let find = |w: &World, name: &str| w.query::<(olvin_ecs::Entity, &Name)>().iter().find(|(_, n)| n.0 == name).map(|(e, _)| e).unwrap();
        let (p, c) = (find(&world, "Parent"), find(&world, "Child"));
        assert_eq!(world.get::<&Transform>(c).unwrap().translation.x, 1.0, "child keeps its local transform");
        world.get::<&mut Transform>(p).unwrap().translation.x = 20.0;
        olvin_transform::propagate(&mut world);
        let x = world.get::<&GlobalTransform>(c).unwrap().0.w_axis.x;
        assert_eq!(x, 21.0, "moving the parent moves the child");
    }

    #[test]
    fn brushes_with_rigidbody_become_bodies() {
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());
        let mut floor = EntityDesc::new(1, "Floor");
        floor.scale = Vec3::new(10.0, 1.0, 10.0);
        floor.brush = Some(BrushDesc::default());
        let mut crate_ = EntityDesc::new(2, "Crate");
        crate_.position = Vec3::new(0.0, 3.0, 0.0);
        crate_.brush = Some(BrushDesc::default());
        crate_.rigidbody = Some(RigidbodyDesc::default());
        spawn(&res, &SceneFile { entities: vec![floor, crate_], ..Default::default() });

        let world = res.fetch::<World>();
        let crate_ = world.query::<(&Name, &MeshRenderer, &ColliderDesc, &RigidbodyDesc)>().iter().filter(|(n, ..)| n.0 == "Crate").count();
        assert_eq!(crate_, 1, "the crate brush is its own rendered, colliding body");
        let level = world.query::<(&Name, &MeshGeometry)>().iter().find(|(n, _)| n.0 == "CSG Level").map(|(_, m)| m.indices.len()).unwrap();
        assert_eq!(level, 12, "the level is only the floor box (6 quads)");
    }
}

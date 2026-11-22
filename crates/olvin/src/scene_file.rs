//! Scenes saved by olvin-editor (`assets/scenes/*.scene.ron`).

use std::collections::HashMap;
use std::path::Path;

use olvin_core::Resources;
use olvin_ecs::{Component, Entity, EntityBuilder, World};
use olvin_render::{Color, MeshData, MeshHandle, MeshRenderer, Meshes, RenderSettings, Vertex};
pub use olvin_scene::file::*;
use olvin_scene::primitives::Primitive;
use olvin_scene::{Camera, MeshGeometry, Name, SceneId, Tag, Transform};
use olvin_transform::{GlobalTransform, Parent};
use serde::de::DeserializeOwned;

// Gameplay components. With the `script` / `audio` features they are the
// types those plugins consume, so spawned scenes run scripts and play sound
// with no extra wiring (CharacterControllerDesc and AudioSourceDesc are picked
// up by olvin_physics / olvin_audio directly). Without them: plain data.

/// Behaviours attached in the editor; game code looks them up by name.
#[cfg(feature = "script")]
pub use olvin_script::Scripts;
/// Behaviours attached in the editor; game code looks them up by name.
#[cfg(not(feature = "script"))]
#[derive(Clone, Debug, Default)]
pub struct Scripts(pub Vec<ScriptDesc>);

/// Marks the entity that hears audio (usually the camera).
#[cfg(feature = "audio")]
pub use olvin_audio::AudioListener;
/// Marks the entity that hears audio (usually the camera).
#[cfg(not(feature = "audio"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioListener;

type Insert = Box<dyn Fn(&str, &mut EntityBuilder) -> Result<(), String>>;

/// Resource: game-defined component types a scene may carry
/// (`EntityDesc::components`, name → RON text). Game crates register their
/// own: `res.fetch_mut::<SceneComponents>().register::<DoorState>("DoorState")`.
/// Like `SaveRegistry`, the name is the file format. `DefaultPlugins` (feature
/// `scene`) inserts it; without it scene components are skipped.
#[derive(Default)]
pub struct SceneComponents {
    types: Vec<(String, Insert)>,
}

impl SceneComponents {
    pub fn register<T>(&mut self, name: &str) -> &mut Self
    where
        T: Component + DeserializeOwned,
    {
        assert!(
            !self.contains(name),
            "scene component `{name}` is registered twice"
        );
        let insert: Insert = Box::new(|text, b| {
            let value: T = ron::from_str(text).map_err(|e| e.to_string())?;
            b.add(value);
            Ok(())
        });
        self.types.push((name.to_string(), insert));
        self
    }

    pub fn contains(&self, name: &str) -> bool {
        self.types.iter().any(|(n, _)| n == name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.types.iter().map(|(n, _)| n.as_str())
    }

    /// Adds the object's components; unknown names and bad values are logged and skipped.
    fn add(&self, e: &EntityDesc, b: &mut EntityBuilder) {
        for (name, text) in &e.components {
            match self.types.iter().find(|(n, _)| n == name) {
                Some((_, insert)) => {
                    if let Err(err) = insert(text, b) {
                        log::warn!("`{}`: component `{name}` skipped: {err}", e.name);
                    }
                }
                None => log::warn!("`{}`: unknown scene component `{name}` skipped", e.name),
            }
        }
    }
}

/// The spawned entity of scene object `id` (its [`SceneId`]).
pub fn find(world: &World, id: u64) -> Option<Entity> {
    world
        .query::<(Entity, &SceneId)>()
        .iter()
        .find(|(_, s)| s.0 == id)
        .map(|(e, _)| e)
}

/// Spawns every active entity with its world transform and components
/// (Light, Collider, Rigidbody, AudioSource, ... as plain data components,
/// game components registered in [`SceneComponents`], and the object's
/// [`SceneId`]), the CSG level as static meshes with a mesh collider, and
/// points the renderer's sun at the first directional light. Returns the
/// entity count; [`spawn_ids`] returns the scene id → entity map instead.
///
/// Objects with a `model` get a [`PendingModel`](crate::models::PendingModel):
/// the model loads in the background and [`ModelPlugin`](crate::models::ModelPlugin)
/// builds it a few frames later (wait for [`models::pending_count`](crate::models::pending_count)).
///
/// The hierarchy is kept: `Transform` is local, `Parent` links children, and
/// `GlobalTransform` starts at the world matrix. Material paths resolve
/// against the `RenderAssets` root; [`spawn_from`] sets it for a scene file.
pub fn spawn(res: &Resources, scene: &SceneFile) -> usize {
    spawn_ids(res, scene).len()
}

/// [`spawn`], returning the entity of every spawned scene object by its id.
pub fn spawn_ids(res: &Resources, scene: &SceneFile) -> HashMap<u64, Entity> {
    let worlds = scene.world_matrices();
    let registry = res.get::<SceneComponents>();
    let mut meshes = res.fetch_mut::<Meshes>();
    let mut world = res.fetch_mut::<World>();
    let mut shapes: HashMap<Shape, MeshHandle> = HashMap::new();
    let mut entities: HashMap<u64, Entity> = HashMap::new();

    // Parents are spawned before their children (file order otherwise), so `Parent` links
    // even when a hand-edited file lists a child first.
    for e in scene
        .parents_first()
        .into_iter()
        .map(|i| &scene.entities[i])
        .filter(|e| scene.active_in_hierarchy(e.id))
    {
        let matrix = worlds
            .get(&e.id)
            .copied()
            .unwrap_or_else(|| e.local_transform().matrix());
        let mut b = EntityBuilder::new();
        b.add(e.local_transform())
            .add(GlobalTransform(matrix))
            .add(Name(e.name.clone()))
            .add(SceneId(e.id));
        if let Some(parent) = e.parent.and_then(|p| entities.get(&p)) {
            b.add(Parent(*parent));
        }
        if !e.tag.is_empty() {
            b.add(Tag(e.tag.clone()));
        }
        if let Some(poly) = e.poly.as_ref().filter(|_| e.brush.is_none()) {
            // Edited mesh: its own geometry, and the triangles for a Mesh Collider.
            let (parts, data) = crate::render_assets::poly_mesh_data(poly);
            let (positions, indices) = (parts.positions, parts.indices);
            let color = e.mesh.as_ref().map_or([0.8; 4], |m| m.color);
            let handle = meshes.add(data);
            b.add(MeshRenderer {
                mesh: handle,
                color: Color(color),
                material: crate::render_assets::material_in(res, e.material.as_deref()),
            });
            b.add(MeshGeometry {
                positions,
                indices: indices.as_chunks::<3>().0.to_vec(),
            });
        } else if let Some(mesh) = &e.mesh {
            let handle = *shapes
                .entry(mesh.shape)
                .or_insert_with(|| meshes.add(MeshData::primitive(mesh.shape)));
            b.add(MeshRenderer {
                mesh: handle,
                color: Color(mesh.color),
                material: crate::render_assets::material_in(res, e.material.as_deref()),
            });
        }
        if let Some(camera) = &e.camera {
            b.add(Camera::from(camera));
        }
        // A brush with a Rigidbody leaves the static level and becomes its own
        // body: its convex geometry (local space; the Transform scales it).
        if let (Some(brush), Some(_)) = (&e.brush, &e.rigidbody)
            && let Some(mut input) = e.brush_input(glam::Mat4::IDENTITY)
        {
            input.op = CsgOp::Add;
            let tri = olvin_csg::TriMesh::from_polygons(&olvin_csg::build(&[input]));
            let vertices = tri
                .positions
                .iter()
                .zip(&tri.normals)
                .map(|(p, n)| Vertex::new(p.to_array(), n.to_array()))
                .collect();
            let mesh = meshes.add(MeshData {
                vertices,
                indices: tri.indices.clone(),
            });
            b.add(MeshRenderer {
                mesh,
                color: Color(brush.color),
                material: None,
            });
            b.add(MeshGeometry {
                positions: tri.positions,
                indices: tri.indices.as_chunks::<3>().0.to_vec(),
            });
            if e.collider.is_none() {
                b.add(ColliderDesc::new(ColliderShape::Mesh));
            }
        }
        // A model is built once its file is loaded (`models::ModelPlugin`);
        // its Mesh Collider waits for the model's triangles.
        let model_collider = e
            .collider
            .as_ref()
            .filter(|c| c.shape == ColliderShape::Mesh && e.mesh.is_none() && e.poly.is_none())
            .filter(|_| e.model.is_some());
        if let Some(model) = &e.model {
            b.add(crate::models::pending(res, model, model_collider.cloned()));
        }
        if let Some(collider) = e.collider.as_ref().filter(|_| model_collider.is_none()) {
            if collider.shape == ColliderShape::Mesh
                && e.poly.is_none()
                && let Some(mesh) = &e.mesh
            {
                let p = Primitive::new(mesh.shape);
                let indices = p.indices.as_chunks::<3>().0.to_vec();
                b.add(MeshGeometry {
                    positions: p.positions,
                    indices,
                });
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
        if let Some(registry) = &registry {
            registry.add(e, &mut b);
        } else if !e.components.is_empty() {
            log::warn!(
                "`{}`: no SceneComponents resource, components skipped",
                e.name
            );
        }
        entities.insert(e.id, world.spawn(b.build()));
    }

    if let Some(mut settings) = res.get_mut::<RenderSettings>() {
        apply_lighting(scene, &mut settings);
        if let (Some(mut assets), Some(mut textures)) = (
            res.get_mut::<crate::render_assets::RenderAssets>(),
            res.get_mut::<olvin_render::Textures>(),
        ) {
            settings.sky = assets.sky(&scene.environment.skybox, &mut textures);
        }
    }

    // CSG: one static mesh per face colour and material, one mesh collider for the level.
    let brushes: Vec<_> = scene
        .csg_brushes()
        .into_iter()
        .filter(|b| {
            scene.active_in_hierarchy(b.id)
                && scene.get(b.id).is_some_and(|e| e.rigidbody.is_none())
        })
        .collect();
    let polygons = olvin_csg::build(&brushes);
    let mut by_color: HashMap<([u32; 4], Option<String>), Vec<&olvin_csg::Polygon>> =
        HashMap::new();
    for p in &polygons {
        by_color
            .entry((
                p.color.map(f32::to_bits),
                scene.get(p.brush).and_then(|e| e.material.clone()),
            ))
            .or_default()
            .push(p);
    }
    for ((color, material), polys) in by_color {
        let tri = olvin_csg::TriMesh::from_polygons(polys);
        // Box-projected UVs, one repeat per metre (triplanar materials look best).
        let mesh = meshes.add(MeshData::from_triangles(
            &tri.positions,
            &tri.normals,
            tri.indices,
        ));
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer {
                mesh,
                color: Color(color.map(f32::from_bits)),
                material: crate::render_assets::material_in(res, material.as_deref()),
            },
        ));
    }
    if !polygons.is_empty() {
        let tri = olvin_csg::TriMesh::from_polygons(&polygons);
        let indices = tri.indices.as_chunks::<3>().0.to_vec();
        world.spawn((
            Transform::IDENTITY,
            Name("CSG Level".into()),
            ColliderDesc::new(ColliderShape::Mesh),
            MeshGeometry {
                positions: tri.positions,
                indices,
            },
        ));
    }
    // The scene's object animation (F-curves), played by `AnimPlugin`.
    if !scene.animation.curves.is_empty()
        && let Some(mut player) = res.get_mut::<olvin_anim::ActionPlayer>()
    {
        let bases = scene
            .entities
            .iter()
            .map(|e| (e.id, [e.position, e.rotation, e.scale]))
            .collect();
        *player = olvin_anim::ActionPlayer::new(scene.animation.clone(), bases);
    }
    entities
}

/// Applies the scene's sun and [`Environment`](olvin_scene::file::Environment)
/// to render settings (window options like vsync stay untouched).
pub fn apply_lighting(scene: &SceneFile, settings: &mut RenderSettings) {
    let env = &scene.environment;
    let scaled = |c: [f32; 3], k: f32| Color([c[0] * k, c[1] * k, c[2] * k, 1.0]);
    if let Some((dir, color, intensity)) = scene.sun() {
        settings.light_dir = dir;
        settings.light_color = scaled(color, intensity);
    }
    settings.ambient = scaled(env.sky, env.ambient_intensity);
    settings.ground_ambient = scaled(env.ground, env.ambient_intensity);
    settings.clear_color = scaled(env.background, 1.0);
    settings.exposure = env.exposure;
    settings.tonemapping = if env.filmic {
        olvin_render::Tonemapping::Aces
    } else {
        olvin_render::Tonemapping::None
    };
    settings.fog = env.fog.map(|f| olvin_render::Fog {
        color: scaled(f.color, 1.0),
        start: f.start,
        end: f.end,
    });
    settings.shadow_distance = env.shadow_distance;
    settings.shadow_map_size = env.shadow_resolution.clamp(256, 8192);
    settings.shadow_filter = match env.shadow_filter {
        olvin_scene::file::ShadowFilter::Hard => olvin_render::ShadowFilter::Hard,
        olvin_scene::file::ShadowFilter::Pcf => olvin_render::ShadowFilter::Pcf,
        olvin_scene::file::ShadowFilter::Soft => olvin_render::ShadowFilter::Soft,
        olvin_scene::file::ShadowFilter::Pcss => olvin_render::ShadowFilter::Pcss,
    };
    settings.shadow_cascades = env.shadow_cascades.clamp(1, 4);
    settings.max_spot_shadows = env.max_spot_shadows.min(4);
    settings.max_point_shadows = env.max_point_shadows.min(2);
    settings.local_shadow_size = env.local_shadow_resolution.clamp(512, 8192);
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
        shadow_bias: d.shadow_bias.clamp(0.0, 16.0),
        size: d.size.clamp(0.0, 60.0),
    }
}

/// Loads a scene file and spawns it.
pub fn load(res: &Resources, path: impl AsRef<Path>) -> Result<usize, String> {
    let path = path.as_ref();
    let scene = SceneFile::load(path)?;
    Ok(spawn_from(res, &scene, path).len())
}

/// [`spawn_ids`] for a scene read from `path`: materials and textures resolve
/// against the `assets/` folder above it (unless a root is set already).
pub fn spawn_from(res: &Resources, scene: &SceneFile, path: &Path) -> HashMap<u64, Entity> {
    if let Some(mut assets) = res.get_mut::<crate::render_assets::RenderAssets>()
        && assets.root().is_none()
    {
        assets.set_root(crate::render_assets::RenderAssets::root_for_scene(path));
    }
    spawn_ids(res, scene)
}

/// Spawns a scene parsed in the background (`olvin_game::SceneLoader`) the way
/// [`load`] does, materials included. `None` while it is pending or when the
/// handle is unknown; `Some(Err)` when the file failed to load.
#[cfg(feature = "game")]
pub fn spawn_loaded(
    res: &Resources,
    handle: olvin_game::LoadHandle,
) -> Option<Result<HashMap<u64, Entity>, String>> {
    let (path, result) = res
        .get_mut::<olvin_game::SceneLoader>()?
        .take_with_path(handle)?;
    Some(result.map(|scene| spawn_from(res, &scene, &path)))
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
            e.brush = Some(BrushDesc {
                shape: BrushShape::Box,
                op,
                color,
                ..Default::default()
            });
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
        assert_eq!(
            world
                .query::<(&ColliderDesc, &MeshGeometry)>()
                .iter()
                .count(),
            1
        );
    }

    #[test]
    fn scene_animation_reaches_the_player() {
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());
        res.insert(olvin_anim::ActionPlayer::default());
        let mut box_ = EntityDesc::new(3, "Box");
        box_.position = Vec3::new(0.0, 4.0, 0.0);
        let mut scene = SceneFile {
            entities: vec![box_],
            ..Default::default()
        };
        scene
            .animation
            .insert_key(3, olvin_scene::file::Prop::Location, 0, 1.0, 0.0);
        scene
            .animation
            .insert_key(3, olvin_scene::file::Prop::Location, 0, 25.0, 6.0);
        spawn(&res, &scene);
        let mut player = res.fetch_mut::<olvin_anim::ActionPlayer>();
        assert!(player.playing && player.looping);
        player.frame = 25.0;
        let t = player.transform(3).unwrap();
        assert_eq!(t.translation, Vec3::new(6.0, 4.0, 0.0));
    }

    #[test]
    fn a_child_listed_before_its_parent_is_still_parented() {
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());
        let mut child = EntityDesc::new(2, "child");
        child.parent = Some(1);
        child.position = Vec3::X;
        let mut parent = EntityDesc::new(1, "parent");
        parent.position = Vec3::Y;
        let mut orphan = EntityDesc::new(3, "orphan");
        orphan.parent = Some(99); // no such object: a root
        let scene = SceneFile {
            entities: vec![child, parent, orphan],
            ..Default::default()
        };
        assert_eq!(scene.parents_first(), [1, 2, 0]);
        let ids = spawn_ids(&res, &scene);
        let world = res.fetch::<World>();
        assert_eq!(world.get::<&Parent>(ids[&2]).unwrap().0, ids[&1]);
        assert!(world.get::<&Parent>(ids[&3]).is_err());
    }

    #[derive(Clone, Debug, PartialEq, serde::Deserialize)]
    struct Door {
        open: bool,
    }

    #[test]
    fn scene_ids_and_game_components_reach_entities() {
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());
        let mut components = SceneComponents::default();
        components.register::<Door>("Door");
        res.insert(components);

        let mut door = EntityDesc::new(7, "Door");
        door.components.insert("Door".into(), "(open: true)".into());
        door.components.insert("Nope".into(), "()".into());
        door.components.insert("Broken".into(), "(".into());
        let lamp = EntityDesc::new(8, "Lamp");
        let scene = SceneFile {
            entities: vec![door, lamp],
            ..Default::default()
        };
        let ids = spawn_ids(&res, &scene);
        let world = res.fetch::<World>();
        assert_eq!(find(&world, 7), Some(ids[&7]));
        assert_eq!(find(&world, 8), Some(ids[&8]));
        assert!(world.get::<&Door>(ids[&7]).unwrap().open);
        assert!(world.get::<&Door>(ids[&8]).is_err());
        // The field is optional on disk, and absent when empty: no version bump.
        let path =
            std::env::temp_dir().join(format!("olvin-comp-{}.scene.ron", std::process::id()));
        SceneFile {
            entities: vec![EntityDesc::new(1, "A")],
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("components")
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[cfg(feature = "game")]
    #[test]
    fn background_loaded_scene_resolves_assets_like_load() {
        use crate::render_assets::RenderAssets;
        let dir = std::env::temp_dir().join(format!("olvin-bg-{}", std::process::id()));
        let scenes = dir.join("assets").join("scenes");
        std::fs::create_dir_all(&scenes).unwrap();
        let path = scenes.join("a.scene.ron");
        let mut e = EntityDesc::new(3, "Thing");
        e.mesh = Some(MeshDesc {
            shape: Shape::Cube,
            color: [1.0; 4],
        });
        SceneFile {
            entities: vec![e],
            ..Default::default()
        }
        .save(&path)
        .unwrap();

        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());
        res.insert(RenderAssets::new(None));
        res.insert(olvin_game::SceneLoader::default());
        let handle = res.fetch_mut::<olvin_game::SceneLoader>().request(&path);
        let ids = loop {
            res.fetch_mut::<olvin_game::SceneLoader>().poll();
            if let Some(r) = spawn_loaded(&res, handle) {
                break r.unwrap();
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(ids.len(), 1);
        let root = res.fetch::<RenderAssets>().root().map(Path::to_path_buf);
        assert_eq!(root, Some(dir.join("assets")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn environment_reaches_render_settings_and_files() {
        let mut scene = SceneFile {
            environment: (olvin_scene::file::Environment::PRESETS[1].1)(),
            ..Default::default()
        };
        scene.environment.exposure = 2.0;
        let mut settings = RenderSettings::default();
        apply_lighting(&scene, &mut settings);
        assert_eq!(settings.exposure, 2.0);
        assert!(settings.fog.is_some());
        assert!(settings.vsync, "window options stay");
        let path = std::env::temp_dir().join(format!("olvin-env-{}.scene.ron", std::process::id()));
        scene.save(&path).unwrap();
        assert_eq!(SceneFile::load(&path).unwrap(), scene);
        // Default environments are not written at all.
        SceneFile::default().save(&path).unwrap();
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("environment")
        );
        std::fs::remove_file(&path).unwrap();
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
        crate_.mesh = Some(MeshDesc {
            shape: Shape::Sphere,
            color: [1.0; 4],
        });
        crate_.collider = Some(ColliderDesc::for_shape(Shape::Sphere));
        crate_.rigidbody = Some(RigidbodyDesc::default());
        crate_.tag = "Player".into();
        let mut hidden = EntityDesc::new(3, "Hidden");
        hidden.active = false;
        let mut child = EntityDesc::new(4, "Child of hidden");
        child.parent = Some(3);
        let scene = SceneFile {
            entities: vec![sun, crate_, hidden, child],
            ..Default::default()
        };

        assert_eq!(
            spawn(&res, &scene),
            2,
            "inactive parents hide their children"
        );
        assert_eq!(res.fetch::<World>().query::<&Parent>().iter().count(), 0);
        let world = res.fetch::<World>();
        assert_eq!(
            world
                .query::<(&RigidbodyDesc, &ColliderDesc, &Tag)>()
                .iter()
                .count(),
            1
        );
        let dir = res.fetch::<RenderSettings>().light_dir;
        assert!(
            dir.abs_diff_eq(Vec3::NEG_Y, 1e-4),
            "sun pitched -90° shines down: {dir}"
        );
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
        spawn(
            &res,
            &SceneFile {
                entities: vec![parent, child],
                ..Default::default()
            },
        );

        let mut world = res.fetch_mut::<World>();
        let find = |w: &World, name: &str| {
            w.query::<(olvin_ecs::Entity, &Name)>()
                .iter()
                .find(|(_, n)| n.0 == name)
                .map(|(e, _)| e)
                .unwrap()
        };
        let (p, c) = (find(&world, "Parent"), find(&world, "Child"));
        assert_eq!(
            world.get::<&Transform>(c).unwrap().translation.x,
            1.0,
            "child keeps its local transform"
        );
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
        spawn(
            &res,
            &SceneFile {
                entities: vec![floor, crate_],
                ..Default::default()
            },
        );

        let world = res.fetch::<World>();
        let crate_ = world
            .query::<(&Name, &MeshRenderer, &ColliderDesc, &RigidbodyDesc)>()
            .iter()
            .filter(|(n, ..)| n.0 == "Crate")
            .count();
        assert_eq!(
            crate_, 1,
            "the crate brush is its own rendered, colliding body"
        );
        let level = world
            .query::<(&Name, &MeshGeometry)>()
            .iter()
            .find(|(n, _)| n.0 == "CSG Level")
            .map(|(_, m)| m.indices.len())
            .unwrap();
        assert_eq!(level, 12, "the level is only the floor box (6 quads)");
    }
}

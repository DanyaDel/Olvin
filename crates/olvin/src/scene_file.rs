//! Scenes saved by olvin-editor (`assets/scenes/*.scene.ron`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use olvin_core::Resources;
use olvin_ecs::{Component, Entity, EntityBuilder, World};
use olvin_render::{Color, MeshData, MeshHandle, MeshRenderer, Meshes, RenderSettings, Vertex};
pub use olvin_scene::PrefabInstanceOf;
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
///
/// Prefab instances are expanded first (see [`olvin_scene::prefab`]); their
/// assets are read from the `RenderAssets` root. A missing or broken prefab
/// is logged and its object spawns empty.
pub fn spawn_ids(res: &Resources, scene: &SceneFile) -> HashMap<u64, Entity> {
    let scene = &*expand_prefabs(res, scene);
    let entities = spawn_entities(res, scene);
    spawn_environment(res, scene);
    spawn_csg(res, scene);
    spawn_animation(res, scene);
    entities
}

/// The scene with its prefab instances expanded (borrowed when there are none).
fn expand_prefabs<'a>(res: &Resources, scene: &'a SceneFile) -> std::borrow::Cow<'a, SceneFile> {
    if !scene
        .entities
        .iter()
        .any(|e| e.prefab.is_some() && e.prefab_node.is_none())
    {
        return std::borrow::Cow::Borrowed(scene);
    }
    let root = res
        .get::<crate::render_assets::RenderAssets>()
        .and_then(|a| a.root().map(Path::to_path_buf));
    let mut expanded = scene.clone();
    for warning in expanded.expand_prefabs(&mut prefab_loader(root)) {
        log::warn!("{warning}");
    }
    std::borrow::Cow::Owned(expanded)
}

/// Reads prefab assets (`*.prefab.ron`, paths relative to the assets root),
/// each file once.
fn prefab_loader(root: Option<PathBuf>) -> impl FnMut(&str) -> Result<SceneFile, String> {
    let mut cache: HashMap<String, Result<SceneFile, String>> = HashMap::new();
    move |rel| {
        cache
            .entry(rel.to_owned())
            .or_insert_with(|| match &root {
                Some(root) => SceneFile::load(root.join(rel)),
                None => Err("no assets folder to read it from".into()),
            })
            .clone()
    }
}

/// Spawns the prefab at `asset` (a `*.prefab.ron` path relative to the assets
/// root) at `transform` and returns its root entity. Gameplay code uses it to
/// create objects at run time; the root carries [`PrefabInstanceOf`] and its
/// [`SceneId`] is fresh (above every id in the world). Errors (missing or
/// broken file, no assets root) are returned, never panics.
pub fn instantiate_prefab(
    res: &Resources,
    asset: &str,
    transform: Transform,
) -> Result<Entity, String> {
    let id = {
        let world = res.fetch::<World>();
        world
            .query::<&SceneId>()
            .iter()
            .map(|s| s.0)
            .filter(|id| !olvin_scene::prefab::is_member_id(*id))
            .max()
            .map_or(1, |m| m + 1)
    };
    let (y, x, z) = transform.rotation.to_euler(glam::EulerRot::YXZ);
    let mut root = EntityDesc::new(id, asset.rsplit('/').next().unwrap_or(asset));
    root.name = root.name.trim_end_matches(".prefab.ron").to_owned();
    root.position = transform.translation;
    root.rotation = glam::Vec3::new(x, y, z).map(f32::to_degrees);
    root.scale = transform.scale;
    root.prefab = Some(PrefabInstance::new(asset));
    let mut scene = SceneFile {
        entities: vec![root],
        ..Default::default()
    };
    let assets_root = res
        .get::<crate::render_assets::RenderAssets>()
        .and_then(|a| a.root().map(Path::to_path_buf));
    let warnings = scene.expand_prefabs(&mut prefab_loader(assets_root));
    if scene.entities.len() == 1 && !warnings.is_empty() {
        return Err(warnings.join("; "));
    }
    for warning in warnings {
        log::warn!("{warning}");
    }
    let entities = spawn_entities(res, &scene);
    spawn_csg(res, &scene);
    entities
        .get(&id)
        .copied()
        .ok_or_else(|| format!("prefab `{asset}`: the root is inactive"))
}

/// Carries out the prefab spawns scripts queued with `ctx:spawn_prefab`
/// ([`olvin_script::SpawnRequests`]): instantiates each at its place, marks the
/// root [`olvin_script::ScriptSpawned`] and sends the requester a `"spawned"`
/// message whose value is the root's entity id. `DefaultPlugins` and
/// `HeadlessPlugins` run it after the script pass; an app that builds its own
/// plugin set adds it as an `Update` system. Failures are logged.
#[cfg(feature = "script")]
pub fn run_prefab_spawns(res: &Resources) {
    use olvin_script::{ScriptMessage, ScriptSpawned, SpawnRequests};
    let requests = match res.get_mut::<SpawnRequests>() {
        Some(mut queue) if !queue.is_empty() => queue.take(),
        _ => return,
    };
    for request in requests {
        match instantiate_prefab(res, &request.asset, request.transform) {
            Ok(root) => {
                let _ = res.fetch_mut::<World>().insert_one(root, ScriptSpawned);
                if let Some(mut events) = res.get_mut::<olvin_core::Events<ScriptMessage>>() {
                    events.send(ScriptMessage {
                        name: "spawned".to_owned(),
                        value: olvin_script::ScriptValue::Int(root.to_bits().get() as i64),
                        from: Some(root),
                        target: Some(request.requester),
                    });
                }
            }
            Err(e) => log::warn!("spawn_prefab `{}`: {e}", request.asset),
        }
    }
}

fn spawn_entities(res: &Resources, scene: &SceneFile) -> HashMap<u64, Entity> {
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
        if let Some(c) = &e.joint {
            b.add(c.clone());
        }
        if let Some(c) = &e.particles {
            b.add(crate::particles::emitter_in(res, c));
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
        if let Some(p) = &e.prefab {
            b.add(PrefabInstanceOf(p.asset.clone()));
        }
        entities.insert(e.id, world.spawn(b.build()));
    }
    entities
}

fn spawn_environment(res: &Resources, scene: &SceneFile) {
    if let Some(mut settings) = res.get_mut::<RenderSettings>() {
        apply_lighting(scene, &mut settings);
        if let (Some(mut assets), Some(mut textures)) = (
            res.get_mut::<crate::render_assets::RenderAssets>(),
            res.get_mut::<olvin_render::Textures>(),
        ) {
            settings.sky = assets.sky(&scene.environment.skybox, &mut textures);
            settings.post.grade.lut = scene
                .environment
                .post
                .lut
                .as_deref()
                .filter(|p| !p.is_empty())
                .and_then(|p| assets.texture(p, false, &mut textures));
        }
    }
}

/// CSG: one static mesh per face colour and material, one mesh collider for the level.
fn spawn_csg(res: &Resources, scene: &SceneFile) {
    let mut meshes = res.fetch_mut::<Meshes>();
    let mut world = res.fetch_mut::<World>();
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
}

/// The scene's object animation (F-curves), played by `AnimPlugin`.
fn spawn_animation(res: &Resources, scene: &SceneFile) {
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
    let post = &env.post;
    settings.post.fxaa = post.fxaa;
    settings.bloom = post.bloom.map(|b| olvin_render::Bloom {
        threshold: b.threshold,
        intensity: b.intensity,
    });
    settings.post.ao = post.ao.map(|a| olvin_render::AmbientOcclusion {
        radius: a.radius,
        strength: a.strength,
        bias: a.bias,
    });
    settings.post.grade = olvin_render::ColorGrade {
        // The LUT image is loaded where assets are at hand (`spawn_ids`).
        lut: None,
        lut_strength: post.lut_strength,
        saturation: post.saturation,
        contrast: post.contrast,
        lift: post.lift,
        gamma: post.gamma,
        gain: post.gain,
    };
    settings.height_fog = env.height_fog.map(|f| olvin_render::HeightFog {
        color: scaled(f.color, 1.0),
        density: f.density,
        falloff: f.falloff,
        height: f.height,
    });
    settings.retro = env.retro.map_or_else(Default::default, retro_settings);
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

/// The renderer's retro settings for a scene's retro block (on).
pub fn retro_settings(d: olvin_scene::file::RetroDesc) -> olvin_render::RetroSettings {
    olvin_render::RetroSettings {
        enabled: true,
        snap_resolution: d.snap_resolution,
        snap_grid_m: d.snap_grid_m.max(0.0),
        affine_strength: d.affine_strength.clamp(0.0, 1.0),
        affine_fade_near: d.affine_fade_near.max(0.0),
        color_bits: d.color_bits,
        dither_strength: d.dither_strength.clamp(0.0, 2.0),
        vertex_lighting: d.vertex_lighting,
        shadows: d.shadows,
        draw_distance: d.draw_distance.max(0.0),
        dissolve: d.dissolve,
        cull_small_px: d.cull_small_px.max(0.0),
        force_nearest: d.force_nearest,
        internal_resolution: d.internal_resolution,
        pixel_scale: d.pixel_scale,
        letterbox: d.letterbox,
    }
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
    fn spawn_attaches_a_particle_emitter_with_its_texture_or_the_soft_circle() {
        use olvin_render::{ParticleEmitter, Textures};
        let dir = std::env::temp_dir().join(format!("olvin_particles_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("fx")).unwrap();
        // A 4 x 4 uncompressed 32-bit TGA.
        let mut tga = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 4, 0, 32, 8];
        tga.extend([0u8, 200, 255, 255].repeat(16));
        std::fs::write(dir.join("fx/spark.tga"), tga).unwrap();
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());
        res.insert(Textures::default());
        res.insert(crate::render_assets::RenderAssets::new(Some(dir.clone())));

        let mut spark = EntityDesc::new(1, "Spark");
        spark.particles = Some(ParticleDesc {
            texture: Some("fx/spark.tga".into()),
            gpu: true,
            rate: 123.0,
            rotation: [0.0, 180.0],
            ..Default::default()
        });
        let mut plain = EntityDesc::new(2, "Plain");
        plain.particles = Some(ParticleDesc::default());
        let mut missing = EntityDesc::new(3, "Missing");
        missing.particles = Some(ParticleDesc {
            texture: Some("nope.png".into()),
            play_on_awake: false,
            ..Default::default()
        });
        let scene = SceneFile {
            entities: vec![spark, plain, missing, EntityDesc::new(4, "None")],
            ..Default::default()
        };
        let entities = spawn_ids(&res, &scene);
        let world = res.fetch::<World>();
        assert_eq!(world.query::<&ParticleEmitter>().iter().count(), 3);
        assert!(world.get::<&ParticleEmitter>(entities[&4]).is_err());
        let get = |id: u64| world.get::<&ParticleEmitter>(entities[&id]).unwrap();
        let (a, b, c) = (get(1), get(2), get(3));
        assert!(a.params.gpu && a.params.rate == 123.0 && a.playing);
        assert!((a.params.rotation[1] - std::f32::consts::PI).abs() < 1e-5);
        assert_eq!(a.params.texture_path.as_deref(), Some("fx/spark.tga"));
        // The file's texture, the built-in soft circle for no path and for a missing
        // file (one shared sprite), and "Play On Awake" off.

        assert_ne!(a.texture, b.texture);
        assert_eq!(b.texture, c.texture);
        assert!(!c.playing);
        drop((a, b, c));
        drop(world);
        let _ = std::fs::remove_dir_all(&dir);
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
        // A prefab instance expands against the assets folder found from the scene path.
        SceneFile {
            entities: vec![EntityDesc::new(1, "PrefabRoot"), EntityDesc::new(2, "Part")],
            ..Default::default()
        }
        .save(dir.join("assets/prefabs/thing.prefab.ron"))
        .unwrap();
        let mut thing = EntityDesc::new(4, "Placed");
        thing.prefab = Some(PrefabInstance::new("prefabs/thing.prefab.ron"));
        SceneFile {
            entities: vec![e, thing],
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
        assert_eq!(ids.len(), 3, "the thing and the two nodes of the prefab");
        let root = res.fetch::<RenderAssets>().root().map(Path::to_path_buf);
        assert_eq!(root, Some(dir.join("assets")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A project's `assets/` with `prefabs/crate.prefab.ron` (Crate > Lid), a game world
    /// and the resources spawning needs.
    fn prefab_project(name: &str) -> (PathBuf, Resources) {
        use crate::render_assets::RenderAssets;
        let assets = std::env::temp_dir()
            .join(format!("olvin-prefab-{}-{name}", std::process::id()))
            .join("assets");
        let _ = std::fs::remove_dir_all(&assets);
        let mut crate_ = EntityDesc::new(1, "Crate");
        crate_.mesh = Some(MeshDesc {
            shape: Shape::Cube,
            color: [1.0; 4],
        });
        let mut lid = EntityDesc::new(2, "Lid");
        lid.parent = Some(1);
        lid.position = Vec3::Y;
        lid.mesh = crate_.mesh.clone();
        SceneFile {
            entities: vec![crate_, lid],
            ..Default::default()
        }
        .save(assets.join("prefabs/crate.prefab.ron"))
        .unwrap();
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());
        res.insert(RenderAssets::new(Some(assets.clone())));
        (assets, res)
    }

    fn instance(id: u64, name: &str, asset: &str, at: Vec3) -> EntityDesc {
        let mut e = EntityDesc::new(id, name);
        e.position = at;
        e.prefab = Some(PrefabInstance::new(asset));
        e
    }

    #[test]
    fn prefab_instances_spawn_with_overrides_and_keep_the_root_id() {
        let (assets, res) = prefab_project("spawn");
        let mut a = instance(
            10,
            "A",
            "prefabs/crate.prefab.ron",
            Vec3::new(5.0, 0.0, 0.0),
        );
        let mut b = instance(11, "B", "prefabs/crate.prefab.ron", Vec3::ZERO);
        let p = b.prefab.as_mut().unwrap();
        p.overrides.insert(
            "2".into(),
            std::collections::BTreeMap::from([("position".into(), "(0.0,4.0,0.0)".into())]),
        );
        a.parent = None;
        let scene = SceneFile {
            entities: vec![a, b],
            ..Default::default()
        };
        let ids = spawn_ids(&res, &scene);
        assert_eq!(ids.len(), 4, "two roots and two lids");
        let world = res.fetch::<World>();
        assert_eq!(find(&world, 10), Some(ids[&10]));
        assert_eq!(
            world.get::<&PrefabInstanceOf>(ids[&10]).unwrap().0,
            "prefabs/crate.prefab.ron"
        );
        assert!(
            world.get::<&MeshRenderer>(ids[&10]).is_ok(),
            "from the asset"
        );
        let lid = |root: u64| {
            let id = olvin_scene::prefab::member_id(root, 2);
            let e = find(&world, id).unwrap();
            assert_eq!(world.get::<&Parent>(e).unwrap().0, ids[&root]);
            let y = world.get::<&Transform>(e).unwrap().translation.y;
            (world.get::<&Name>(e).unwrap().0.clone(), y)
        };
        assert_eq!(lid(10), ("Lid".into(), 1.0));
        assert_eq!(lid(11), ("Lid".into(), 4.0), "the override");
        let at = world.get::<&GlobalTransform>(ids[&10]).unwrap().0.w_axis.x;
        assert_eq!(at, 5.0);
        let _ = std::fs::remove_dir_all(assets.parent().unwrap());
    }

    #[test]
    fn broken_and_recursive_prefabs_are_skipped_not_fatal() {
        let (assets, res) = prefab_project("broken");
        // A prefab that contains itself, and one that is not RON at all.
        let mut me = EntityDesc::new(1, "Me");
        let mut inner = instance(2, "Inner", "prefabs/loop.prefab.ron", Vec3::ZERO);
        inner.parent = Some(1);
        me.position = Vec3::X;
        SceneFile {
            entities: vec![me, inner],
            ..Default::default()
        }
        .save(assets.join("prefabs/loop.prefab.ron"))
        .unwrap();
        std::fs::write(assets.join("prefabs/bad.prefab.ron"), "(entities: [ (id: ").unwrap();
        let scene = SceneFile {
            entities: vec![
                instance(1, "Loop", "prefabs/loop.prefab.ron", Vec3::ZERO),
                instance(2, "Bad", "prefabs/bad.prefab.ron", Vec3::ZERO),
                instance(3, "Gone", "prefabs/nope.prefab.ron", Vec3::ZERO),
                instance(4, "Fine", "prefabs/crate.prefab.ron", Vec3::ZERO),
            ],
            ..Default::default()
        };
        let ids = spawn_ids(&res, &scene);
        // Loop: its root and the empty inner root; Bad and Gone: an empty object each;
        // Fine: root and lid.
        assert_eq!(ids.len(), 2 + 1 + 1 + 2);
        for id in [1, 2, 3, 4] {
            assert!(ids.contains_key(&id));
        }
        // No assets folder at all: still no panic.
        let mut bare = Resources::default();
        bare.insert(World::new());
        bare.insert(Meshes::default());
        bare.insert(RenderSettings::default());
        assert_eq!(spawn_ids(&bare, &scene).len(), 4);
        let _ = std::fs::remove_dir_all(assets.parent().unwrap());
    }

    #[test]
    fn instantiate_prefab_places_a_fresh_instance_at_runtime() {
        let (assets, res) = prefab_project("instantiate");
        spawn(
            &res,
            &SceneFile {
                entities: vec![EntityDesc::new(7, "Existing")],
                ..Default::default()
            },
        );
        let at = Transform::from_xyz(1.0, 2.0, 3.0)
            .with_rotation(glam::Quat::from_rotation_y(1.0))
            .with_scale(Vec3::splat(2.0));
        let first = instantiate_prefab(&res, "prefabs/crate.prefab.ron", at).unwrap();
        let second = instantiate_prefab(&res, "prefabs/crate.prefab.ron", at).unwrap();
        assert_ne!(first, second);
        let world = res.fetch::<World>();
        let (id1, id2) = (
            world.get::<&SceneId>(first).unwrap().0,
            world.get::<&SceneId>(second).unwrap().0,
        );
        assert_eq!((id1, id2), (8, 9), "ids continue after the scene's");
        let t = *world.get::<&Transform>(first).unwrap();
        assert!(t.translation.abs_diff_eq(Vec3::new(1.0, 2.0, 3.0), 1e-6));
        assert!(t.rotation.abs_diff_eq(at.rotation, 1e-5));
        assert_eq!(t.scale, Vec3::splat(2.0));
        assert_eq!(world.query::<&Parent>().iter().count(), 2, "both lids");
        assert!(world.get::<&PrefabInstanceOf>(first).is_ok());
        drop(world);
        let err = instantiate_prefab(&res, "prefabs/nope.prefab.ron", at).unwrap_err();
        assert!(err.contains("nope.prefab.ron"), "{err}");
        let _ = std::fs::remove_dir_all(assets.parent().unwrap());
    }

    #[cfg(feature = "script")]
    #[test]
    fn script_spawn_requests_become_instances_and_a_message() {
        use olvin_script::{ScriptMessage, ScriptSpawned, SpawnRequest, SpawnRequests};
        let (assets, mut res) = prefab_project("spawn-requests");
        res.insert(SpawnRequests::default());
        res.insert(olvin_core::Events::<ScriptMessage>::default());
        let requester = res.fetch_mut::<World>().spawn((Name("Spawner".into()),));
        let mut cursor = olvin_core::EventCursor::default();
        for asset in ["prefabs/crate.prefab.ron", "prefabs/nope.prefab.ron"] {
            res.fetch_mut::<SpawnRequests>().push(SpawnRequest {
                asset: asset.into(),
                transform: Transform::from_xyz(4.0, 0.0, 0.0),
                requester,
            });
        }
        run_prefab_spawns(&res);
        assert!(res.fetch::<SpawnRequests>().is_empty());
        let world = res.fetch::<World>();
        let marked: Vec<_> = world
            .query::<(Entity, &ScriptSpawned)>()
            .iter()
            .map(|(e, _)| e)
            .collect();
        assert_eq!(marked.len(), 1, "the broken prefab is logged, not spawned");
        let events = res.fetch::<olvin_core::Events<ScriptMessage>>();
        let sent: Vec<_> = events.read(&mut cursor).collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].name, "spawned");
        assert_eq!(sent[0].target, Some(requester));
        assert_eq!(
            sent[0].value.as_f64(),
            Some(marked[0].to_bits().get() as f64)
        );
        let _ = std::fs::remove_dir_all(assets.parent().unwrap());
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
        // Post and height fog follow the environment.
        scene.environment.height_fog = Some(Default::default());
        scene.environment.post.fxaa = false;
        scene.environment.post.ao = Some(Default::default());
        scene.environment.post.saturation = 0.5;
        scene.environment.post.bloom = Some(Default::default());
        apply_lighting(&scene, &mut settings);
        assert_eq!(settings.bloom.map(|b| b.intensity), Some(0.3));
        assert!(settings.height_fog.is_some());
        assert!(!settings.post.fxaa && settings.post.ao.is_some());
        assert_eq!(settings.post.grade.saturation, 0.5);
        // The retro block turns the renderer's retro style on, and off again.
        assert!(!settings.retro.enabled);
        scene.environment.retro = Some(olvin_scene::file::RetroDesc::ps1());
        apply_lighting(&scene, &mut settings);
        assert!(settings.retro.enabled && settings.retro.color_bits == 5);
        scene.environment.retro = None;
        apply_lighting(&scene, &mut settings);
        assert!(!settings.retro.enabled);
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
    fn a_joint_becomes_a_component_that_finds_its_partner_by_scene_id() {
        use olvin_scene::file::{JointDesc, JointKind};
        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());
        let mut post = EntityDesc::new(10, "Post");
        post.collider = Some(ColliderDesc::for_shape(Shape::Cube));
        let mut door = EntityDesc::new(11, "Door");
        door.collider = Some(ColliderDesc::for_shape(Shape::Cube));
        door.rigidbody = Some(RigidbodyDesc::default());
        let mut joint = JointDesc::new(JointKind::Ball);
        joint.connected = Some(10);
        door.joint = Some(joint);
        let scene = SceneFile {
            entities: vec![post, door],
            ..Default::default()
        };
        let ids = spawn_ids(&res, &scene);
        let world = res.fetch::<World>();
        let j = world.get::<&JointDesc>(ids[&11]).unwrap();
        assert_eq!(j.connected, Some(10));
        // The partner is found by the SceneId the post was spawned with.
        let partner = world
            .query::<(Entity, &SceneId)>()
            .iter()
            .find(|(_, s)| s.0 == 10)
            .map(|(e, _)| e);
        assert_eq!(partner, Some(ids[&10]));
        assert!(world.get::<&JointDesc>(ids[&10]).is_err());
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

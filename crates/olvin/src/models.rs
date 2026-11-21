//! glTF models placed by scenes (`EntityDesc::model`). `scene_file::spawn`
//! leaves a [`PendingModel`] on the object; the [`AssetManager`] imports the
//! file on a worker thread, and [`ModelPlugin`] then builds the model under
//! the object: one child per mesh primitive with its material, and for a
//! skinned model the [`Animator`] (on the object itself) running the scene's
//! animation setup, with a [`SkinnedMesh`] on each skinned primitive.

use std::collections::HashMap;
use std::path::Path;

use glam::Mat4;
use olvin_anim::Animator;
use olvin_assets::{AssetManager, Handle, LoadState, MeshPrimitive, ModelData};
use olvin_core::{App, Plugin, Resources, Stage};
use olvin_ecs::{EcsPlugin, Entity, World};
use olvin_render::{Color, MeshData, MeshHandle, MeshRenderer, Meshes, SkinnedMesh};
use olvin_scene::file::{AnimGraphDesc, AnimationDesc, ColliderDesc, ModelDesc};
use olvin_scene::{MeshGeometry, Name, Transform};
use olvin_transform::{GlobalTransform, Parent};

use crate::render_assets::RenderAssets;

/// A scene object whose model is still loading.
pub struct PendingModel {
    pub desc: ModelDesc,
    model: Option<Handle<ModelData>>,
    graph: Option<Handle<AnimGraphDesc>>,
    /// A Mesh Collider waits for the model's triangles.
    collider: Option<ColliderDesc>,
}

/// On a model's root once it is built: keeps the asset loaded.
#[derive(Clone, Debug)]
pub struct ModelInstance {
    pub handle: Handle<ModelData>,
    /// The spawned children (meshes, extra skeletons).
    pub parts: Vec<Entity>,
}

/// Mesh handles of static primitives, shared by every instance of a model.
#[derive(Default)]
pub struct ModelMeshes(HashMap<(String, usize), MeshHandle>);

/// Builds pending models in `PreUpdate` once their files are loaded. Needs an
/// [`AssetManager`]; without one it makes its own for the `assets/` folder
/// that [`RenderAssets`] points at (set by `scene_file::load`).
pub struct ModelPlugin;

impl Plugin for ModelPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(EcsPlugin)
            .init_resource::<ModelMeshes>()
            .add_system(Stage::PreUpdate, build_pending);
    }
}

/// The pending component for a scene's model; starts loading right away when
/// there is an [`AssetManager`]. `collider` is the object's Mesh Collider.
pub(crate) fn pending(
    res: &Resources,
    desc: &ModelDesc,
    collider: Option<ColliderDesc>,
) -> PendingModel {
    let mut p = PendingModel {
        desc: desc.clone(),
        model: None,
        graph: None,
        collider,
    };
    if let Some(mut manager) = res.get_mut::<AssetManager>() {
        start(&mut manager, &mut p);
    }
    p
}

fn start(manager: &mut AssetManager, p: &mut PendingModel) {
    if p.model.is_none() {
        p.model = Some(manager.load(&p.desc.path));
    }
    if let (None, Some(AnimationDesc::Graph(path))) = (&p.graph, &p.desc.animation) {
        p.graph = Some(manager.load(path));
    }
}

/// Models still loading (a loading screen waits for 0).
pub fn pending_count(res: &Resources) -> usize {
    res.get::<World>()
        .map_or(0, |w| w.query::<&PendingModel>().iter().count())
}

fn ensure_manager(res: &mut Resources) -> bool {
    if res.get::<AssetManager>().is_some() {
        return true;
    }
    let root = res
        .get::<RenderAssets>()
        .and_then(|r| r.root().map(Path::to_path_buf));
    match root {
        Some(root) => {
            res.insert(AssetManager::new(root));
            true
        }
        None => false,
    }
}

enum Outcome {
    Wait,
    Ready(
        std::sync::Arc<ModelData>,
        Option<std::sync::Arc<AnimGraphDesc>>,
    ),
    Failed(String),
}

fn build_pending(res: &mut Resources) {
    if pending_count(res) == 0 {
        return;
    }
    if !ensure_manager(res) {
        // Nothing can ever load them: drop the requests once, loudly.
        let mut world = res.fetch_mut::<World>();
        let stuck: Vec<Entity> = world
            .query::<(Entity, &PendingModel)>()
            .iter()
            .map(|(e, _)| e)
            .collect();
        for e in stuck {
            if let Ok(p) = world.remove_one::<PendingModel>(e) {
                log::error!(
                    "{}: no AssetManager and no assets root (use scene_file::load or add AssetsPlugin)",
                    p.desc.path
                );
            }
        }
        return;
    }
    let mut done = Vec::new();
    {
        let mut manager = res.fetch_mut::<AssetManager>();
        manager.update();
        let mut world = res.fetch_mut::<World>();
        for (e, p) in world.query_mut::<(Entity, &mut PendingModel)>() {
            start(&mut manager, p);
            let model = p.model.as_ref().expect("started");
            let outcome = match (manager.state(model), &p.graph) {
                (LoadState::Failed(err), _) => Outcome::Failed(err),
                (LoadState::Loading, _) => Outcome::Wait,
                (LoadState::Ready, None) => Outcome::Ready(manager.get(model).unwrap(), None),
                (LoadState::Ready, Some(g)) => match manager.state(g) {
                    LoadState::Loading => Outcome::Wait,
                    LoadState::Ready => Outcome::Ready(manager.get(model).unwrap(), manager.get(g)),
                    // A broken graph leaves the model in its rest pose.
                    LoadState::Failed(err) => {
                        log::warn!("{}: {err}", g.path());
                        Outcome::Ready(manager.get(model).unwrap(), None)
                    }
                },
            };
            if !matches!(outcome, Outcome::Wait) {
                done.push((e, outcome));
            }
        }
    }
    for (e, outcome) in done {
        let Ok(p) = res.fetch_mut::<World>().remove_one::<PendingModel>(e) else {
            continue;
        };
        match outcome {
            Outcome::Ready(model, graph) => {
                let handle = p.model.clone().expect("started");
                if let Err(err) = instantiate(res, e, &p, handle, &model, graph.as_deref()) {
                    log::error!("{}: {err}", p.desc.path);
                }
            }
            Outcome::Failed(err) => log::error!("{}: {err}", p.desc.path),
            Outcome::Wait => {}
        }
    }
}

/// Mesh data of a glTF primitive (its UVs; the file's tangents when it has them).
pub fn primitive_mesh(prim: &MeshPrimitive) -> MeshData {
    let uvs: Vec<[f32; 2]> = prim.uvs.iter().map(|uv| uv.to_array()).collect();
    let mut data =
        MeshData::from_triangles_uv(&prim.positions, &prim.normals, &uvs, prim.indices.clone());
    if let Some(tangents) = &prim.tangents {
        for (v, t) in data.vertices.iter_mut().zip(tangents) {
            v.tangent = t.to_array();
        }
    }
    data
}

/// Where a model's primitives go, in the model's space: (primitive, matrix).
/// Static primitives sit at their node; skinned ones at the root (glTF skins
/// ignore their node's transform, the skeleton places them).
pub fn placements(model: &ModelData) -> Vec<(usize, Mat4)> {
    let mut world: Vec<Mat4> = Vec::with_capacity(model.nodes.len());
    let mut out = Vec::new();
    for n in &model.nodes {
        let local = Mat4::from_scale_rotation_translation(
            n.scale,
            olvin_scene::file::euler_to_quat(n.rotation_euler_deg),
            n.translation,
        );
        // Parents come first.
        let m = n
            .parent
            .and_then(|p| world.get(p))
            .map_or(local, |p| *p * local);
        world.push(m);
        if let Some(p) = n.mesh.filter(|&p| p < model.meshes.len()) {
            let at = if model.meshes[p].skin.is_some() {
                Mat4::IDENTITY
            } else {
                m
            };
            out.push((p, at));
        }
    }
    out
}

/// Builds `model` under `root`: animators, meshes, materials, the Mesh Collider.
fn instantiate(
    res: &Resources,
    root: Entity,
    p: &PendingModel,
    handle: Handle<ModelData>,
    model: &ModelData,
    graph: Option<&AnimGraphDesc>,
) -> Result<(), String> {
    let materials = crate::render_assets::model_materials(res, &p.desc.path, &model.materials);
    let mut world = res.fetch_mut::<World>();
    let mut meshes = res.fetch_mut::<Meshes>();
    let mut shared = res.get_mut::<ModelMeshes>();
    if !world.contains(root) {
        return Err("object despawned while loading".into());
    }
    let name = world
        .get::<&Name>(root)
        .map_or_else(|_| model.name.clone(), |n| n.0.clone());
    let root_global = world
        .get::<&GlobalTransform>(root)
        .map_or(Mat4::IDENTITY, |g| g.0);
    let mut parts = Vec::new();

    // One animator per skin: the first on the root, others on children.
    let mut animators = Vec::with_capacity(model.skeletons.len());
    for skin in 0..model.skeletons.len() {
        let animator = match model.animator_with(skin, p.desc.animation.as_ref(), graph) {
            Ok(a) => a,
            Err(err) => {
                log::warn!("{}: {err}; rest pose", p.desc.path);
                model.animator_with(skin, None, None)?
            }
        };
        animators.push(animator_entity(
            &mut world,
            root,
            root_global,
            &name,
            skin,
            animator,
            &mut parts,
        ));
    }

    let mut geometry = MeshGeometry::default();
    for (i, at) in placements(model) {
        let prim = &model.meshes[i];
        if p.collider.is_some() {
            let base = geometry.positions.len() as u32;
            geometry
                .positions
                .extend(prim.positions.iter().map(|&v| at.transform_point3(v)));
            geometry.indices.extend(
                prim.indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|t| [t[0] + base, t[1] + base, t[2] + base]),
            );
        }
        let material = prim
            .material
            .and_then(|m| materials.get(m).copied().flatten());
        // A material carries the colour; without one, the base colour does.
        let color = match (material, prim.material.and_then(|m| model.materials.get(m))) {
            (None, Some(m)) => m.base_color,
            (None, None) => [0.8; 4],
            (Some(_), _) => [1.0; 4],
        };
        let skinned = prim
            .skin
            .and_then(|s| animators.get(s).copied())
            .filter(|_| !prim.joints.is_empty());
        let (mesh, skin) = match skinned {
            Some(animator) => {
                let skin = SkinnedMesh::new(
                    primitive_mesh(prim),
                    prim.joints.clone(),
                    prim.weights.clone(),
                    animator,
                )?;
                // Posed on its own: a mesh slot per instance.
                (meshes.add(skin.bind_mesh()), Some(skin))
            }
            None => {
                let key = (p.desc.path.clone(), i);
                let mesh = match shared.as_mut().and_then(|s| s.0.get(&key).copied()) {
                    Some(h) => h,
                    None => {
                        let h = meshes.add(primitive_mesh(prim));
                        if let Some(s) = shared.as_mut() {
                            s.0.insert(key, h);
                        }
                        h
                    }
                };
                (mesh, None)
            }
        };
        let (scale, rotation, translation) = at.to_scale_rotation_translation();
        let label = if prim.name.is_empty() {
            i.to_string()
        } else {
            prim.name.clone()
        };
        let part = world.spawn((
            Name(format!("{name}/{label}")),
            Transform {
                translation,
                rotation,
                scale,
            },
            GlobalTransform(root_global * at),
            Parent(root),
            MeshRenderer {
                mesh,
                color: Color(color),
                material,
            },
        ));
        if let Some(skin) = skin {
            let _ = world.insert_one(part, skin);
        }
        parts.push(part);
    }

    if let Some(collider) = p.collider.clone() {
        let _ = world.insert(root, (collider, geometry));
    }
    let _ = world.insert_one(root, ModelInstance { handle, parts });
    Ok(())
}

fn animator_entity(
    world: &mut World,
    root: Entity,
    root_global: Mat4,
    name: &str,
    skin: usize,
    animator: Animator,
    parts: &mut Vec<Entity>,
) -> Entity {
    if skin == 0 {
        let _ = world.insert_one(root, animator);
        return root;
    }
    let e = world.spawn((
        Name(format!("{name}/Skeleton {skin}")),
        Transform::IDENTITY,
        GlobalTransform(root_global),
        Parent(root),
        animator,
    ));
    parts.push(e);
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_anim::AnimPlugin;
    use olvin_core::Time;
    use olvin_scene::file::{ColliderShape, EntityDesc, SceneFile};
    use olvin_transform::TransformPlugin;

    /// A two-joint bar (4 vertices, the top pair on the tip joint) with a
    /// one-second "bend" clip, as `<dir>/models/rig.gltf` + `rig.bin`.
    fn write_rig(dir: &Path) {
        let f = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let parts: [Vec<u8>; 7] = [
            f(&[0., 0., 0., 1., 0., 0., 0., 1., 0., 1., 1., 0.]),
            vec![0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0],
            f(&[
                1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.,
            ]),
            [0u16, 1, 2, 1, 3, 2]
                .iter()
                .flat_map(|i| i.to_le_bytes())
                .collect(),
            f(&[
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., //
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., -1., 0., 1.,
            ]),
            f(&[0.0, 1.0]),
            f(&[0., 0., 0., 1., 0., 0., h, h]),
        ];
        let mut bin = Vec::new();
        let mut views = Vec::new();
        for p in &parts {
            views.push(format!(
                r#"{{"buffer": 0, "byteOffset": {}, "byteLength": {}}}"#,
                bin.len(),
                p.len()
            ));
            bin.extend(p);
            bin.resize(bin.len().next_multiple_of(4), 0);
        }
        let json = format!(
            r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0, 1]}}],
  "nodes": [
    {{"name": "Body", "mesh": 0, "skin": 0}},
    {{"name": "Root", "children": [2]}},
    {{"name": "Tip", "translation": [0, 1, 0]}}
  ],
  "meshes": [{{"name": "Bar", "primitives": [{{"attributes": {{"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}}, "indices": 3, "material": 0}}]}}],
  "materials": [{{"name": "Red", "pbrMetallicRoughness": {{"baseColorFactor": [1, 0, 0, 1]}}}}],
  "skins": [{{"joints": [1, 2], "inverseBindMatrices": 4}}],
  "animations": [{{"name": "bend",
    "channels": [{{"sampler": 0, "target": {{"node": 1, "path": "rotation"}}}}],
    "samplers": [{{"input": 5, "output": 6}}]}}],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]}},
    {{"bufferView": 1, "componentType": 5121, "count": 4, "type": "VEC4"}},
    {{"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC4"}},
    {{"bufferView": 3, "componentType": 5123, "count": 6, "type": "SCALAR"}},
    {{"bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4"}},
    {{"bufferView": 5, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]}},
    {{"bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC4"}}
  ],
  "bufferViews": [{}],
  "buffers": [{{"uri": "rig.bin", "byteLength": {}}}]
}}"#,
            views.join(", "),
            bin.len()
        );
        std::fs::create_dir_all(dir.join("models")).unwrap();
        std::fs::write(dir.join("models/rig.bin"), bin).unwrap();
        std::fs::write(dir.join("models/rig.gltf"), json).unwrap();
    }

    fn app(assets: &Path) -> App {
        let mut app = App::new();
        app.insert_resource(Time::with_fixed_delta(0.1))
            .add_plugin(TransformPlugin)
            .add_plugin(AnimPlugin)
            .add_plugin(ModelPlugin)
            .init_resource::<Meshes>()
            .insert_resource(AssetManager::new(assets));
        app
    }

    fn wait_for_models(app: &mut App) {
        for _ in 0..2000 {
            if pending_count(&app.resources) == 0 {
                return;
            }
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("models did not load");
    }

    fn temp_assets(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("olvin_models_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_rig(&dir);
        dir
    }

    #[test]
    fn scene_spawns_a_skinned_animated_model_headless() {
        let dir = temp_assets("skin");
        let mut app = app(&dir);

        let mut rig = EntityDesc::new(1, "Rig");
        rig.position = glam::Vec3::new(5.0, 0.0, 0.0);
        rig.model = Some(ModelDesc {
            path: "models/rig.gltf".into(),
            animation: Some(AnimationDesc::Clip("bend".into())),
        });
        rig.collider = Some(ColliderDesc::new(ColliderShape::Mesh));
        let mut missing = EntityDesc::new(2, "Missing");
        missing.model = Some(ModelDesc::new("models/none.gltf"));
        let scene = SceneFile {
            entities: vec![rig, missing],
            ..Default::default()
        };
        assert_eq!(crate::scene_file::spawn(&app.resources, &scene), 2);
        assert_eq!(pending_count(&app.resources), 2);
        wait_for_models(&mut app);

        let root = {
            let world = app.resources.fetch::<World>();
            let (root, _) = world
                .query::<(Entity, &Name)>()
                .iter()
                .find(|(_, n)| n.0 == "Rig")
                .unwrap();
            assert!(world.get::<&ModelInstance>(root).is_ok());
            assert_eq!(
                world.query::<&ModelInstance>().iter().count(),
                1,
                "the missing model is dropped with an error"
            );
            let geometry = world.get::<&MeshGeometry>(root).expect("mesh collider");
            assert_eq!(geometry.indices.len(), 2);
            assert!(world.get::<&ColliderDesc>(root).is_ok());
            assert_eq!(world.get::<&Animator>(root).unwrap().layers.len(), 1);
            let skinned: Vec<_> = world
                .query::<(&SkinnedMesh, &MeshRenderer, &Parent, &Name)>()
                .iter()
                .map(|(s, r, p, n)| (s.animator, r.color, p.0, n.0.clone()))
                .collect();
            assert_eq!(skinned.len(), 1);
            let (animator, color, parent, name) = &skinned[0];
            assert_eq!((*animator, *parent), (root, root));
            assert_eq!(name, "Rig/Bar");
            assert_eq!(
                color.0,
                [1.0, 0.0, 0.0, 1.0],
                "material colour without a renderer"
            );
            root
        };

        // The clip plays (it already ran in the frame that built it): at
        // 0.9 s of the looping second the tip has swung most of 90°.
        let v0 = app
            .resources
            .fetch::<World>()
            .get::<&Animator>(root)
            .unwrap()
            .version;
        for _ in 0..8 {
            app.update();
        }
        {
            let world = app.resources.fetch::<World>();
            let animator = world.get::<&Animator>(root).unwrap();
            assert!(animator.version > v0);
            let mut skins = world.query::<&SkinnedMesh>();
            let posed = skins.iter().next().unwrap().skin(&animator.matrices);
            let top = posed.vertices[2].position;
            assert!(top[0] < -0.9, "tip vertex bent over: {top:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn graph_files_and_instances_share_the_asset() {
        let dir = temp_assets("graph");
        let graph = AnimGraphDesc {
            states: vec![olvin_scene::file::AnimStateDesc::clip("Bend", "bend")],
            ..Default::default()
        };
        std::fs::create_dir_all(dir.join("anim")).unwrap();
        graph.save(dir.join("anim/rig.animgraph.ron")).unwrap();
        let mut app = app(&dir);
        let entities = (1..=2)
            .map(|id| {
                let mut e = EntityDesc::new(id, format!("Rig {id}"));
                e.model = Some(ModelDesc {
                    path: "models/rig.gltf".into(),
                    animation: Some(AnimationDesc::Graph("anim/rig.animgraph.ron".into())),
                });
                e
            })
            .collect();
        crate::scene_file::spawn(
            &app.resources,
            &SceneFile {
                entities,
                ..Default::default()
            },
        );
        wait_for_models(&mut app);
        let world = app.resources.fetch::<World>();
        let states: Vec<_> = world
            .query::<&Animator>()
            .iter()
            .map(|a| a.layers[0].graph.states[0].name.clone())
            .collect();
        assert_eq!(states, ["Bend", "Bend"]);
        let handles: Vec<_> = world
            .query::<&ModelInstance>()
            .iter()
            .map(|m| m.handle.clone())
            .collect();
        assert_eq!(handles[0], handles[1], "one asset for both instances");
        drop(world);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

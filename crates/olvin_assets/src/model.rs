//! glTF 2.0 / GLB import into plain data, plus conversion to scene entities
//! and `.mat.ron` materials.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use glam::{EulerRot, Mat4, Quat, Vec2, Vec3, Vec4};
use gltf::mesh::Mode;
use gltf::texture::MagFilter;
use olvin_anim::{Animator, Channel, Clip, Interpolation, Joint, JointPose, Property, Skeleton};
use olvin_scene::file::{CameraDesc, EntityDesc, MeshDesc, PolyMesh, Shape};
use olvin_scene::material::MaterialDesc;

use crate::AssetKind;
use crate::meta::{Importer, Meta, ModelImport, TextureImport, meta_path};
use crate::root::relative_to;

/// One drawable piece: a glTF primitive (one material), always triangles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshPrimitive {
    pub name: String,
    pub positions: Vec<Vec3>,
    /// Same length as `positions`; flat normals are generated when the file has none.
    pub normals: Vec<Vec3>,
    /// TEXCOORD_0, empty if absent. Origin top-left (glTF / wgpu convention).
    pub uvs: Vec<Vec2>,
    /// xyz + handedness in w, if the file has them.
    pub tangents: Option<Vec<Vec4>>,
    /// Triangle list, counter-clockwise front faces.
    pub indices: Vec<u32>,
    /// Index into [`ModelData::materials`].
    pub material: Option<usize>,
    /// JOINTS_0 (indices into the skin's joints) and WEIGHTS_0 (normalized),
    /// both empty unless the primitive is skinned.
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    /// Index into [`ModelData::skeletons`]: the skin of the node using this
    /// primitive (the first one, if several nodes share the mesh).
    pub skin: Option<usize>,
}

/// A node of the model's hierarchy, parents always before children.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeData {
    pub name: String,
    /// Index into [`ModelData::nodes`].
    pub parent: Option<usize>,
    pub translation: Vec3,
    /// Degrees, applied Y, then X, then Z (same as `EntityDesc::rotation`).
    pub rotation_euler_deg: Vec3,
    pub scale: Vec3,
    /// Index into [`ModelData::meshes`].
    pub mesh: Option<usize>,
    /// Index into [`ModelData::cameras`].
    pub camera: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraProjection {
    /// Vertical field of view in radians; `far` is `None` for an infinite plane.
    Perspective {
        yfov: f32,
        aspect: Option<f32>,
        near: f32,
        far: Option<f32>,
    },
    Orthographic {
        xmag: f32,
        ymag: f32,
        near: f32,
        far: f32,
    },
}

/// A glTF camera on a node (looks down its -Z, like Olvin's cameras).
#[derive(Clone, Debug, PartialEq)]
pub struct CameraData {
    pub name: String,
    /// Index into [`ModelData::nodes`].
    pub node: usize,
    pub projection: CameraProjection,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelData {
    /// File stem of the model.
    pub name: String,
    pub meshes: Vec<MeshPrimitive>,
    pub materials: Vec<MaterialDesc>,
    /// Names for `materials` (from the file, or `Material N`), same length.
    pub material_names: Vec<String>,
    pub nodes: Vec<NodeData>,
    /// Texture files written while importing (embedded images), assets-relative.
    pub extracted_textures: Vec<String>,
    /// One per glTF skin. Joint order is the skin's, so `MeshPrimitive::joints`
    /// index it directly; `Joint::target` is the node index in `nodes`. Poses
    /// are in model space (the model's scale applied, nodes above the skeleton
    /// baked into the roots); a skinned mesh is drawn at the model's root,
    /// like glTF ignores the transform of a skinned node.
    pub skeletons: Vec<Skeleton>,
    /// glTF animations; `Channel::target` is a node index in `nodes`.
    pub clips: Vec<Clip>,
    pub cameras: Vec<CameraData>,
    /// Things the importer ignored: morph targets, sparse/other features, unsupported extensions.
    pub skipped: Vec<String>,
}

impl ModelData {
    /// An [`Animator`] over skeleton `skin` with all of the model's clips and
    /// no layers yet (add one with `with_graph`). `None` for a bad index.
    pub fn animator(&self, skin: usize) -> Option<Animator> {
        let skeleton = self.skeletons.get(skin)?;
        Some(Animator::new(
            std::sync::Arc::new(skeleton.clone()),
            self.clips.clone().into(),
        ))
    }
}

/// Entities made from a model by [`model_to_entities`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelEntities {
    /// Root first (named after the model), then the nodes; parents before children.
    pub entities: Vec<EntityDesc>,
    /// Per entity id with a `poly`: UVs per face corner, parallel to `poly.faces`
    /// (`uvs_per_face[f][k]` is the UV of `poly.faces[f][k]`). Empty when the
    /// primitive has no UVs.
    pub uvs_per_face: BTreeMap<u64, Vec<Vec<Vec2>>>,
}

/// Imports a `.gltf`/`.glb`. Embedded images (GLB buffer views, data URIs)
/// are written next to the model as `<stem>_textures/<name>.<png|jpg>` (only
/// when their bytes changed) and referenced, like external images, by path
/// relative to `assets_root` (or to the model's folder when outside it).
pub fn import_gltf(
    path: impl AsRef<Path>,
    assets_root: &Path,
    settings: &ModelImport,
) -> Result<ModelData, String> {
    let path = path.as_ref();
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let bytes = std::fs::read(path).map_err(|e| err(&e))?;
    let gltf::Gltf {
        document: doc,
        blob,
    } = gltf::Gltf::from_slice(&bytes).map_err(|e| err(&e))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let buffers = gltf::import_buffers(&doc, Some(base), blob).map_err(|e| err(&e))?;
    let stem = file_stem(path);

    let mut model = ModelData {
        name: stem.clone(),
        ..Default::default()
    };
    let scale = if settings.scale > 0.0 {
        settings.scale
    } else {
        1.0
    };

    // Images used as non-colour data get a linear `.meta` when extracted.
    let mut linear_images = HashSet::new();
    for m in doc.materials() {
        let pbr = m.pbr_metallic_roughness();
        let used = [
            m.normal_texture().map(|t| t.texture()),
            m.occlusion_texture().map(|t| t.texture()),
            pbr.metallic_roughness_texture().map(|t| t.texture()),
        ];
        linear_images.extend(used.into_iter().flatten().map(|t| t.source().index()));
    }

    // Images → assets-relative paths.
    let tex_dir = base.join(format!("{stem}_textures"));
    let mut used_names = HashSet::new();
    let mut image_paths: Vec<Option<String>> = Vec::new();
    for image in doc.images() {
        let i = image.index();
        let embedded: Option<(Vec<u8>, String)> = match image.source() {
            gltf::image::Source::View { view, mime_type } => {
                let data = &buffers[view.buffer().index()];
                let range = view.offset()..view.offset() + view.length();
                data.get(range).map(|b| (b.to_vec(), mime_type.to_string()))
            }
            gltf::image::Source::Uri { uri, mime_type } => match uri.strip_prefix("data:") {
                Some(rest) => {
                    let (head, payload) = rest.split_once(',').unwrap_or(("", rest));
                    let mime = head.split(';').next().filter(|m| !m.is_empty());
                    let mime = mime.or(mime_type).unwrap_or("image/png").to_string();
                    decode_base64(payload).map(|b| (b, mime))
                }
                None => {
                    let abs = base.join(percent_decode(uri));
                    image_paths.push(Some(
                        relative_to(assets_root, &abs).unwrap_or_else(|| percent_decode(uri)),
                    ));
                    continue;
                }
            },
        };
        let Some((data, mime)) = embedded else {
            model
                .skipped
                .push(format!("image {i}: unreadable embedded data"));
            image_paths.push(None);
            continue;
        };
        let ext = if mime.contains("jpeg") || mime.contains("jpg") {
            "jpg"
        } else {
            "png"
        };
        let mut name = sanitize(image.name().unwrap_or(&format!("image_{i}")));
        if !used_names.insert(name.clone()) {
            name = format!("{name}_{i}");
            used_names.insert(name.clone());
        }
        let file = tex_dir.join(format!("{name}.{ext}"));
        let data = if ext == "png" && !data.starts_with(b"\x89PNG") {
            // Some other format (webp, ktx…): re-encode to PNG via the image crate.
            match reencode_png(&data) {
                Some(d) => d,
                None => {
                    model
                        .skipped
                        .push(format!("image {i}: unsupported format `{mime}`"));
                    image_paths.push(None);
                    continue;
                }
            }
        } else {
            data
        };
        write_if_changed(&file, &data)?;
        if linear_images.contains(&i) && !meta_path(&file).exists() {
            let mut meta = Meta::new(AssetKind::Texture);
            meta.importer = Importer::Texture(TextureImport {
                srgb: false,
                ..Default::default()
            });
            meta.save(meta_path(&file))?;
        }
        let rel = relative_to(assets_root, &file)
            .unwrap_or_else(|| format!("{stem}_textures/{name}.{ext}"));
        model.extracted_textures.push(rel.clone());
        image_paths.push(Some(rel));
    }

    let tex_path = |info: Option<gltf::texture::Texture>| -> Option<String> {
        info.and_then(|t| image_paths.get(t.source().index()).cloned().flatten())
    };

    // Per material: KHR_texture_transform with rotation, baked into the UVs of
    // the primitives that use it (the material only has scale and offset).
    let mut uv_bake: Vec<Option<UvTransform>> = Vec::new();
    for (mi, m) in doc.materials().enumerate() {
        let label = m
            .name()
            .map_or_else(|| format!("Material {mi}"), str::to_string);
        let pbr = m.pbr_metallic_roughness();
        let mut desc = MaterialDesc {
            base_color: pbr.base_color_factor(),
            base_texture: tex_path(pbr.base_color_texture().map(|t| t.texture())),
            metallic: pbr.metallic_factor(),
            roughness: pbr.roughness_factor(),
            orm_texture: tex_path(pbr.metallic_roughness_texture().map(|t| t.texture())),
            emissive: m
                .emissive_factor()
                .map(|c| c * m.emissive_strength().unwrap_or(1.0)),
            emissive_texture: tex_path(m.emissive_texture().map(|t| t.texture())),
            unlit: m.unlit(),
            ..Default::default()
        };
        if let Some(n) = m.normal_texture() {
            desc.normal_texture = tex_path(Some(n.texture()));
            desc.normal_strength = n.scale();
        }
        if let Some(occ) = m.occlusion_texture() {
            let mr = pbr
                .metallic_roughness_texture()
                .map(|t| t.texture().source().index());
            if mr != Some(occ.texture().source().index()) {
                // The ORM texture expects occlusion in R: merge into one.
                let file = |rel: &Option<String>| {
                    let rel = rel.as_deref()?;
                    [assets_root.join(rel), base.join(rel)]
                        .into_iter()
                        .find(|p| p.is_file())
                };
                let occ_file = image_paths
                    .get(occ.texture().source().index())
                    .and_then(file);
                let mr_file = desc.orm_texture.iter().find_map(|r| file(&Some(r.clone())));
                let out = tex_dir.join(format!("{}_orm.png", sanitize(&label)));
                match occ_file.map(|o| merge_orm(&o, mr_file.as_deref(), &out)) {
                    Some(Ok(())) => {
                        let rel = relative_to(assets_root, &out).unwrap_or_else(|| {
                            format!("{stem}_textures/{}_orm.png", sanitize(&label))
                        });
                        if !meta_path(&out).exists() {
                            let mut meta = Meta::new(AssetKind::Texture);
                            meta.importer = Importer::Texture(TextureImport {
                                srgb: false,
                                ..Default::default()
                            });
                            meta.save(meta_path(&out))?;
                        }
                        model.extracted_textures.push(rel.clone());
                        desc.orm_texture = Some(rel);
                    }
                    Some(Err(e)) => model
                        .skipped
                        .push(format!("{label}: occlusion texture not merged: {e}")),
                    None => model
                        .skipped
                        .push(format!("{label}: occlusion texture file not found")),
                }
            }
        }
        match m.alpha_mode() {
            gltf::material::AlphaMode::Opaque => {}
            gltf::material::AlphaMode::Mask => desc.alpha_cutoff = m.alpha_cutoff().unwrap_or(0.5),
            gltf::material::AlphaMode::Blend => {
                model.skipped.push(format!(
                    "{label}: alpha blending not supported, imported opaque"
                ));
            }
        }
        let mut bake = None;
        if let Some(info) = pbr.base_color_texture() {
            if info.texture().sampler().mag_filter() == Some(MagFilter::Nearest) {
                desc.point_filter = true;
            }
            if info.tex_coord() != 0 {
                model.skipped.push(format!(
                    "{label}: TEXCOORD_{} not supported, using 0",
                    info.tex_coord()
                ));
            }
            if let Some(tt) = info.texture_transform() {
                if tt.rotation().abs() < 1e-6 {
                    desc.uv_scale = tt.scale();
                    desc.uv_offset = tt.offset();
                } else {
                    bake = Some(UvTransform {
                        scale: Vec2::from_array(tt.scale()),
                        rotation: tt.rotation(),
                        offset: Vec2::from_array(tt.offset()),
                    });
                }
            }
        }
        uv_bake.push(bake);
        model.materials.push(desc);
        model.material_names.push(label);
    }

    // Meshes: every glTF primitive becomes a MeshPrimitive.
    let mut mesh_prims: Vec<Vec<usize>> = Vec::new();
    for mesh in doc.meshes() {
        let mut list = Vec::new();
        let mesh_name = mesh
            .name()
            .map_or_else(|| format!("Mesh {}", mesh.index()), str::to_string);
        if mesh.weights().is_some()
            || mesh
                .primitives()
                .any(|p| p.morph_targets().next().is_some())
        {
            model
                .skipped
                .push(format!("{mesh_name}: morph targets ignored"));
        }
        for (pi, prim) in mesh.primitives().enumerate() {
            let name = if pi == 0 {
                mesh_name.clone()
            } else {
                format!("{mesh_name}.{pi}")
            };
            match read_primitive(&prim, &buffers, name.clone(), scale) {
                Ok(p) => {
                    list.push(model.meshes.len());
                    model.meshes.push(p);
                }
                Err(e) => model.skipped.push(format!("{name}: {e}")),
            }
        }
        mesh_prims.push(list);
    }

    for prim in &mut model.meshes {
        if let Some(t) = prim.material.and_then(|m| uv_bake[m]) {
            for uv in &mut prim.uvs {
                *uv = t.apply(*uv);
            }
            // Tangents follow the UV directions: let the renderer recompute them.
            prim.tangents = None;
        }
    }

    // Nodes, depth-first from the scene roots (parents first).
    let mut node_map: HashMap<usize, usize> = HashMap::new();
    let mut skinned: Vec<(usize, Vec<usize>)> = Vec::new();
    let roots: Vec<gltf::Node> = match doc.default_scene().or_else(|| doc.scenes().next()) {
        Some(scene) => scene.nodes().collect(),
        None => {
            let children: HashSet<usize> = doc
                .nodes()
                .flat_map(|n| n.children().map(|c| c.index()))
                .collect();
            doc.nodes()
                .filter(|n| !children.contains(&n.index()))
                .collect()
        }
    };
    let mut visited = HashSet::new();
    let mut stack: Vec<(gltf::Node, Option<usize>)> =
        roots.into_iter().rev().map(|n| (n, None)).collect();
    while let Some((node, parent)) = stack.pop() {
        if !visited.insert(node.index()) {
            continue;
        }
        let name = node
            .name()
            .map_or_else(|| format!("Node {}", node.index()), str::to_string);
        let (t, r, s) = node.transform().decomposed();
        let (y, x, z) = Quat::from_array(r).normalize().to_euler(EulerRot::YXZ);
        let prims = node
            .mesh()
            .map(|m| mesh_prims[m.index()].clone())
            .unwrap_or_default();
        let me = model.nodes.len();
        node_map.insert(node.index(), me);
        if let Some(skin) = node.skin() {
            skinned.push((skin.index(), prims.clone()));
        }
        let camera = node.camera().map(|c| {
            let projection = match c.projection() {
                gltf::camera::Projection::Perspective(p) => CameraProjection::Perspective {
                    yfov: p.yfov(),
                    aspect: p.aspect_ratio(),
                    near: p.znear() * scale,
                    far: p.zfar().map(|f| f * scale),
                },
                gltf::camera::Projection::Orthographic(o) => CameraProjection::Orthographic {
                    xmag: o.xmag() * scale,
                    ymag: o.ymag() * scale,
                    near: o.znear() * scale,
                    far: o.zfar() * scale,
                },
            };
            model.cameras.push(CameraData {
                name: c.name().map_or_else(|| name.clone(), str::to_string),
                node: me,
                projection,
            });
            model.cameras.len() - 1
        });
        model.nodes.push(NodeData {
            name: name.clone(),
            parent,
            translation: Vec3::from_array(t) * scale,
            rotation_euler_deg: Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees()),
            scale: Vec3::from_array(s),
            mesh: prims.first().copied(),
            camera,
        });
        // Extra primitives (other materials) become identity children.
        for &p in prims.iter().skip(1) {
            model.nodes.push(NodeData {
                name: model.meshes[p].name.clone(),
                parent: Some(me),
                translation: Vec3::ZERO,
                rotation_euler_deg: Vec3::ZERO,
                scale: Vec3::ONE,
                mesh: Some(p),
                camera: None,
            });
        }
        for child in node.children().collect::<Vec<_>>().into_iter().rev() {
            stack.push((child, Some(me)));
        }
    }

    // Skins → skeletons.
    let parent_of: HashMap<usize, usize> = doc
        .nodes()
        .flat_map(|n| n.children().map(move |c| (c.index(), n.index())))
        .collect();
    let local: Vec<Mat4> = doc
        .nodes()
        .map(|n| Mat4::from_cols_array_2d(&n.transform().matrix()))
        .collect();
    let world_of = |mut i: usize| {
        let mut m = local[i];
        while let Some(&p) = parent_of.get(&i) {
            m = local[p] * m;
            i = p;
        }
        m.w_axis *= Vec4::new(scale, scale, scale, 1.0);
        m
    };
    let mut skin_map: Vec<Option<usize>> = Vec::new();
    for skin in doc.skins() {
        let label = skin
            .name()
            .map_or_else(|| format!("Skin {}", skin.index()), str::to_string);
        match build_skeleton(
            &skin, &buffers, &node_map, &parent_of, &world_of, &model, scale,
        ) {
            Ok(sk) => {
                skin_map.push(Some(model.skeletons.len()));
                model.skeletons.push(sk);
            }
            Err(e) => {
                skin_map.push(None);
                model.skipped.push(format!("{label}: skin ignored ({e})"));
            }
        }
    }
    for (skin, prims) in skinned {
        for p in prims {
            let prim = &mut model.meshes[p];
            if prim.skin.is_none() && !prim.joints.is_empty() {
                prim.skin = skin_map[skin];
            }
        }
    }
    // Skinned data without a usable skin is just a static mesh.
    for prim in model.meshes.iter_mut().filter(|p| p.skin.is_none()) {
        prim.joints.clear();
        prim.weights.clear();
    }

    for anim in doc.animations() {
        let name = anim
            .name()
            .map_or_else(|| format!("#{}", anim.index()), str::to_string);
        let mut channels = Vec::new();
        let mut morph = false;
        for ch in anim.channels() {
            let Some(&target) = node_map.get(&ch.target().node().index()) else {
                model.skipped.push(format!(
                    "animation {name}: channel on a node outside the scene"
                ));
                continue;
            };
            let reader = ch.reader(|b| buffers.get(b.index()).map(|d| &d[..]));
            let (Some(times), Some(outputs)) = (reader.read_inputs(), reader.read_outputs()) else {
                model
                    .skipped
                    .push(format!("animation {name}: channel without data"));
                continue;
            };
            use gltf::animation::util::ReadOutputs;
            let (property, values): (_, Vec<Vec4>) = match outputs {
                ReadOutputs::Translations(t) => (
                    Property::Translation,
                    t.map(|v| (Vec3::from_array(v) * scale).extend(0.0))
                        .collect(),
                ),
                ReadOutputs::Scales(s) => (
                    Property::Scale,
                    s.map(|v| Vec3::from_array(v).extend(0.0)).collect(),
                ),
                ReadOutputs::Rotations(r) => (
                    Property::Rotation,
                    r.into_f32().map(Vec4::from_array).collect(),
                ),
                ReadOutputs::MorphTargetWeights(_) => {
                    morph = true;
                    continue;
                }
            };
            let interpolation = match ch.sampler().interpolation() {
                gltf::animation::Interpolation::Step => Interpolation::Step,
                gltf::animation::Interpolation::Linear => Interpolation::Linear,
                gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
            };
            let times: Vec<f32> = times.collect();
            let ok = if interpolation == Interpolation::CubicSpline {
                values.len() == times.len() * 3
            } else {
                values.len() == times.len()
            };
            if !ok || times.is_empty() {
                model
                    .skipped
                    .push(format!("animation {name}: channel with mismatched keys"));
                continue;
            }
            channels.push(Channel {
                target,
                property,
                interpolation,
                times,
                values,
            });
        }
        if morph {
            model
                .skipped
                .push(format!("animation {name}: morph target weights ignored"));
        }
        if !channels.is_empty() {
            model.clips.push(Clip::new(name, channels));
        }
    }
    const SUPPORTED: [&str; 3] = [
        "KHR_materials_emissive_strength",
        "KHR_materials_unlit",
        "KHR_texture_transform",
    ];
    for ext in doc.extensions_used().filter(|e| !SUPPORTED.contains(e)) {
        model.skipped.push(format!("extension {ext} ignored"));
    }
    Ok(model)
}

fn read_primitive(
    prim: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
    name: String,
    scale: f32,
) -> Result<MeshPrimitive, String> {
    let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d[..]));
    let positions: Vec<Vec3> = reader
        .read_positions()
        .ok_or("no POSITION")?
        .map(|p| Vec3::from_array(p) * scale)
        .collect();
    let mut normals: Vec<Vec3> = reader
        .read_normals()
        .map(|n| n.map(Vec3::from_array).collect())
        .unwrap_or_default();
    let uvs: Vec<Vec2> = reader
        .read_tex_coords(0)
        .map(|t| t.into_f32().map(Vec2::from_array).collect())
        .unwrap_or_default();
    let tangents: Option<Vec<Vec4>> = reader
        .read_tangents()
        .map(|t| t.map(Vec4::from_array).collect());
    let joints: Vec<[u16; 4]> = reader
        .read_joints(0)
        .map(|j| j.into_u16().collect())
        .unwrap_or_default();
    let weights: Vec<[f32; 4]> = reader
        .read_weights(0)
        .map(|w| {
            w.into_f32()
                .map(|w| {
                    let sum: f32 = w.iter().sum();
                    if sum > 1e-6 {
                        w.map(|x| x / sum)
                    } else {
                        [1.0, 0.0, 0.0, 0.0]
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let raw: Vec<u32> = match reader.read_indices() {
        Some(i) => i.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };
    let indices = match prim.mode() {
        Mode::Triangles => raw.as_chunks::<3>().0.concat(),
        Mode::TriangleStrip => (2..raw.len())
            .flat_map(|i| {
                if i % 2 == 0 {
                    [raw[i - 2], raw[i - 1], raw[i]]
                } else {
                    [raw[i - 1], raw[i - 2], raw[i]]
                }
            })
            .collect(),
        Mode::TriangleFan => (2..raw.len())
            .flat_map(|i| [raw[0], raw[i - 1], raw[i]])
            .collect(),
        other => return Err(format!("{other:?} primitives not supported")),
    };
    let n = positions.len();
    if indices.iter().any(|&i| i as usize >= n) {
        return Err("index out of range".into());
    }
    if uvs.len() != n && !uvs.is_empty() {
        return Err("TEXCOORD_0 length mismatch".into());
    }
    let skinned = joints.len() == n && weights.len() == n;
    let mut out = MeshPrimitive {
        name,
        positions,
        normals: Vec::new(),
        uvs,
        tangents: tangents.filter(|t| t.len() == n),
        indices,
        material: prim.material().index(),
        joints: if skinned { joints } else { Vec::new() },
        weights: if skinned { weights } else { Vec::new() },
        skin: None,
    };
    if normals.len() != n {
        flatten_with_normals(&mut out);
    } else {
        for v in &mut normals {
            *v = v.normalize_or(Vec3::Y);
        }
        out.normals = normals;
    }
    Ok(out)
}

/// Un-indexes the primitive and gives every triangle its face normal.
fn flatten_with_normals(p: &mut MeshPrimitive) {
    let idx = std::mem::take(&mut p.indices);
    let pick = |src: &[Vec3]| idx.iter().map(|&i| src[i as usize]).collect::<Vec<_>>();
    let positions = pick(&p.positions);
    p.normals = positions
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|[a, b, c]| [(*b - *a).cross(*c - *a).normalize_or(Vec3::Y); 3])
        .collect();
    if !p.uvs.is_empty() {
        p.uvs = idx.iter().map(|&i| p.uvs[i as usize]).collect();
    }
    if let Some(t) = &mut p.tangents {
        *t = idx.iter().map(|&i| t[i as usize]).collect();
    }
    if !p.joints.is_empty() {
        p.joints = idx.iter().map(|&i| p.joints[i as usize]).collect();
        p.weights = idx.iter().map(|&i| p.weights[i as usize]).collect();
    }
    p.indices = (0..positions.len() as u32).collect();
    p.positions = positions;
}

impl MeshPrimitive {
    /// Editable mesh: vertices welded by exact position (UV seams don't cut
    /// it apart), one triangle face per index triple; degenerate triangles are
    /// dropped. Returns UVs per face corner alongside (empty without UVs).
    pub fn to_poly(&self) -> (PolyMesh, Vec<Vec<Vec2>>) {
        let mut poly = PolyMesh::default();
        let mut weld: HashMap<[u32; 3], u32> = HashMap::new();
        let remap: Vec<u32> = self
            .positions
            .iter()
            .map(|p| {
                let key = p.to_array().map(f32::to_bits);
                *weld.entry(key).or_insert_with(|| {
                    poly.positions.push(*p);
                    poly.positions.len() as u32 - 1
                })
            })
            .collect();
        let mut uvs = Vec::new();
        for tri in self.indices.as_chunks::<3>().0 {
            let face = tri.map(|i| remap[i as usize]);
            if face[0] == face[1] || face[1] == face[2] || face[0] == face[2] {
                continue;
            }
            poly.faces.push(face.to_vec());
            if !self.uvs.is_empty() {
                uvs.push(tri.iter().map(|&i| self.uvs[i as usize]).collect());
            }
        }
        (poly, uvs)
    }
}

/// Scene entities for a model: a root named after it, then one entity per
/// node (ids `first_id + 1 + node index`). Mesh nodes get `poly` (see
/// [`MeshPrimitive::to_poly`]), `mesh` with the material's base colour, and
/// `material: material_paths[i]` when given (from [`write_materials`]).
pub fn model_to_entities(
    model: &ModelData,
    first_id: u64,
    material_paths: &[String],
) -> ModelEntities {
    let mut out = ModelEntities::default();
    out.entities
        .push(EntityDesc::new(first_id, model.name.clone()));
    for (i, node) in model.nodes.iter().enumerate() {
        let id = first_id + 1 + i as u64;
        let mut e = EntityDesc::new(id, node.name.clone());
        e.parent = Some(node.parent.map_or(first_id, |p| first_id + 1 + p as u64));
        e.position = node.translation;
        e.rotation = node.rotation_euler_deg;
        e.scale = node.scale;
        if let Some(cam) = node.camera.and_then(|c| model.cameras.get(c))
            && let CameraProjection::Perspective {
                yfov, near, far, ..
            } = cam.projection
        {
            e.camera = Some(CameraDesc {
                fov_deg: yfov.to_degrees(),
                near,
                far: far.unwrap_or(1000.0),
            });
        }
        if let Some(prim) = node.mesh.and_then(|m| model.meshes.get(m)) {
            let (poly, uvs) = prim.to_poly();
            let color = prim
                .material
                .and_then(|m| model.materials.get(m))
                .map_or([0.8, 0.8, 0.8, 1.0], |m| m.base_color);
            e.mesh = Some(MeshDesc {
                shape: Shape::Cube,
                color,
            });
            e.poly = Some(poly);
            e.material = prim.material.and_then(|m| material_paths.get(m)).cloned();
            out.uvs_per_face.insert(id, uvs);
        }
        out.entities.push(e);
    }
    out
}

/// Saves `model.materials` as `<dir>/<name>.mat.ron` (names sanitized and
/// made unique). Returns the written paths, parallel to `model.materials`.
pub fn write_materials(model: &ModelData, dir: impl AsRef<Path>) -> Result<Vec<PathBuf>, String> {
    let dir = dir.as_ref();
    let mut used = HashSet::new();
    let mut out = Vec::new();
    for (i, mat) in model.materials.iter().enumerate() {
        let label = model
            .material_names
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("Material {i}"));
        let mut name = sanitize(&label);
        if !used.insert(name.clone()) {
            name = format!("{name}_{i}");
            used.insert(name.clone());
        }
        let path = dir.join(format!("{name}.mat.ron"));
        mat.save(&path)?;
        out.push(path);
    }
    Ok(out)
}

/// `KHR_texture_transform`: `offset + R(rotation) * (scale * uv)`.
#[derive(Clone, Copy)]
struct UvTransform {
    scale: Vec2,
    rotation: f32,
    offset: Vec2,
}

impl UvTransform {
    fn apply(&self, uv: Vec2) -> Vec2 {
        let (s, c) = self.rotation.sin_cos();
        let p = uv * self.scale;
        self.offset + Vec2::new(c * p.x + s * p.y, -s * p.x + c * p.y)
    }
}

/// Writes an ORM texture (R occlusion, G roughness, B metallic) from a
/// separate occlusion image and the metallic-roughness image, if any.
fn merge_orm(occlusion: &Path, mr: Option<&Path>, out: &Path) -> Result<(), String> {
    let load = |p: &Path| {
        image::open(p)
            .map(|i| i.to_rgba8())
            .map_err(|e| format!("{}: {e}", p.display()))
    };
    let occ = load(occlusion)?;
    let mr = mr.map(load).transpose()?;
    let (w, h) = mr.as_ref().map_or(occ.dimensions(), |m| m.dimensions());
    let occ = if occ.dimensions() == (w, h) {
        occ
    } else {
        image::imageops::resize(&occ, w, h, image::imageops::FilterType::Triangle)
    };
    let merged = image::RgbaImage::from_fn(w, h, |x, y| {
        let [_, g, b, _] = mr
            .as_ref()
            .map_or([0, 255, 255, 255], |m| m.get_pixel(x, y).0);
        image::Rgba([occ.get_pixel(x, y)[0], g, b, 255])
    });
    let mut png = std::io::Cursor::new(Vec::new());
    merged
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    write_if_changed(out, &png.into_inner())
}

/// Joints in the skin's order; see [`ModelData::skeletons`] for the space.
fn build_skeleton(
    skin: &gltf::Skin,
    buffers: &[gltf::buffer::Data],
    node_map: &HashMap<usize, usize>,
    parent_of: &HashMap<usize, usize>,
    world_of: &dyn Fn(usize) -> Mat4,
    model: &ModelData,
    scale: f32,
) -> Result<Skeleton, String> {
    let nodes: Vec<gltf::Node> = skin.joints().collect();
    let joint_of: HashMap<usize, usize> = nodes
        .iter()
        .enumerate()
        .map(|(j, n)| (n.index(), j))
        .collect();
    let reader = skin.reader(|b| buffers.get(b.index()).map(|d| &d[..]));
    let ibm: Vec<Mat4> = match reader.read_inverse_bind_matrices() {
        Some(m) => m
            .map(|m| {
                let mut m = Mat4::from_cols_array_2d(&m);
                m.w_axis *= Vec4::new(scale, scale, scale, 1.0);
                m
            })
            .collect(),
        None => vec![Mat4::IDENTITY; nodes.len()],
    };
    if ibm.len() != nodes.len() {
        return Err("inverse bind matrices do not match the joints".into());
    }
    let mut joints = Vec::new();
    for (j, node) in nodes.iter().enumerate() {
        let &target = node_map
            .get(&node.index())
            .ok_or("joint outside the scene")?;
        let parent = parent_of.get(&node.index()).and_then(|p| joint_of.get(p));
        let rest = match parent {
            Some(_) => {
                let (t, r, s) = node.transform().decomposed();
                JointPose {
                    translation: Vec3::from_array(t) * scale,
                    rotation: Quat::from_array(r).normalize(),
                    scale: Vec3::from_array(s),
                }
            }
            None => JointPose::from(world_of(node.index())),
        };
        joints.push(Joint {
            name: model.nodes[target].name.clone(),
            parent: parent.copied(),
            target,
            rest,
            inverse_bind: ibm[j],
        });
    }
    Skeleton::new(joints)
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map_or_else(|| "model".into(), |s| s.to_string_lossy().into_owned())
}

/// Safe file name: letters, digits, `-`, `_`, `.`; the rest becomes `_`.
fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_matches('.').to_string();
    if s.is_empty() { "unnamed".into() } else { s }
}

fn write_if_changed(path: &Path, data: &[u8]) -> Result<(), String> {
    if std::fs::read(path).is_ok_and(|old| old == data) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))
}

fn reencode_png(data: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(data).ok()?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// Standard base64 (padding optional, whitespace ignored).
pub(crate) fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

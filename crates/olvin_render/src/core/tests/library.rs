//! The built-in library (`library/<pack>/`): every surface shader validates
//! for both tiers (WGSL, and GLSL ES 3.00 like the Compat / WebGL2 path), and
//! every material draws on a sphere without NaN or black frames. With
//! `OLVIN_LIBRARY_PREVIEWS=<dir>` the preview PNGs of the packs are rendered.

use std::path::{Path, PathBuf};

use olvin_scene::file::Environment;
use olvin_scene::look::LookDesc;
use olvin_scene::material::MaterialDesc;
use olvin_scene::primitives::Shape;

use super::post::post_world_sized;
use super::*;
use crate::BlendMode;

fn library_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

/// Files under `library/*/assets/` with the given suffix: (pack, path
/// relative to the project's `assets/`, absolute path).
fn assets_with(suffix: &str) -> Vec<(String, String, PathBuf)> {
    let mut found = Vec::new();
    let Ok(packs) = std::fs::read_dir(library_root()) else {
        return found;
    };
    for pack in packs.flatten() {
        let assets = pack.path().join("assets");
        let pack_id = pack.file_name().to_string_lossy().into_owned();
        walk(&assets, &assets, &mut |rel, abs| {
            if rel.ends_with(suffix) {
                found.push((pack_id.clone(), rel, abs));
            }
        });
    }
    found.sort();
    found
}

fn walk(root: &Path, dir: &Path, f: &mut impl FnMut(String, PathBuf)) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for e in read.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, f);
        } else if let Ok(rel) = p.strip_prefix(root) {
            f(rel.to_string_lossy().replace('\\', "/"), p.clone());
        }
    }
}

/// WGSL -> naga module -> GLSL ES 3.00 (WebGL2) for both entry points, with the
/// Compat tier's template adaptation.
fn glsl_es(surface_source: &str) -> Result<(), String> {
    let full = crate::shader::mesh_source(surface_source);
    let full = crate::shader::for_device(&full, false);
    let module = naga::front::wgsl::parse_str(&full).map_err(|e| e.to_string())?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| e.to_string())?;
    for (stage, entry) in [
        (naga::ShaderStage::Vertex, "vs_main"),
        (naga::ShaderStage::Fragment, "fs_main"),
    ] {
        let mut out = String::new();
        let options = naga::back::glsl::Options {
            version: naga::back::glsl::Version::Embedded {
                version: 300,
                is_webgl: true,
            },
            ..Default::default()
        };
        naga::back::glsl::Writer::new(
            &mut out,
            &module,
            &info,
            &options,
            &naga::back::glsl::PipelineOptions {
                shader_stage: stage,
                entry_point: entry.into(),
                multiview: None,
            },
            naga::proc::BoundsCheckPolicies::default(),
        )
        .map_err(|e| e.to_string())?
        .write()
        .map_err(|e| format!("{entry}: {e}"))?;
    }
    Ok(())
}

#[test]
fn every_library_shader_validates_for_both_tiers() {
    let shaders = assets_with(".wgsl");
    for (pack, rel, path) in &shaders {
        let src = std::fs::read_to_string(path).unwrap();
        crate::validate_surface_shader(&src).unwrap_or_else(|e| panic!("{pack}/{rel}: {e}"));
        glsl_es(&src).unwrap_or_else(|e| panic!("{pack}/{rel} (GLSL ES): {e}"));
    }
}

#[test]
fn library_shaders_reference_only_existing_files() {
    let have: Vec<String> = assets_with("").into_iter().map(|(_, r, _)| r).collect();
    for (pack, rel, path) in assets_with(".mat.ron") {
        let desc = MaterialDesc::load(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            rel.starts_with(&format!("{pack}/")),
            "{rel} is not namespaced under {pack}"
        );
        for r in desc
            .shader
            .iter()
            .map(String::as_str)
            .chain(desc.textures())
        {
            assert!(
                have.iter().any(|h| h == r),
                "{rel}: {r} is not in the library"
            );
        }
    }
}

/// The engine's material for a loaded description (mirrors
/// `olvin::render_assets`, minus the texture loading the library does not need).
fn material(h: &mut Harness, d: &MaterialDesc, shader: Option<ShaderHandle>) -> Material {
    Material {
        base_color: Color(d.base_color),
        metallic: d.metallic,
        roughness: d.roughness,
        emissive: d.emissive,
        alpha_cutoff: d.alpha_cutoff,
        unlit: d.unlit,
        blend: if d.alpha_blend {
            BlendMode::Alpha
        } else {
            BlendMode::Opaque
        },
        shader,
        ..{
            let _ = h;
            Material::default()
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Scene {
    Sphere,
    Water,
}

/// A material on a sphere (or a water surface) in front of a checkerboard.
fn spawn_scene(
    meshes: &mut Meshes,
    materials: &mut Materials,
    textures: &mut Textures,
    world: &mut World,
    subject: Material,
    scene: Scene,
) {
    let checker = textures.add(TextureData::rgba8(
        2,
        2,
        [
            [34, 36, 46, 255],
            [58, 60, 74, 255],
            [58, 60, 74, 255],
            [34, 36, 46, 255],
        ]
        .concat(),
        true,
    ));
    let backdrop = materials.add(Material {
        base_texture: Some(checker),
        uv_scale: [6.0, 6.0],
        point_filter: true,
        roughness: 0.9,
        ..Default::default()
    });
    let subject = materials.add(subject);
    match scene {
        Scene::Sphere => {
            let sphere = meshes.add(MeshData::primitive(Shape::Sphere));
            let plane = meshes.add(MeshData::plane(8.0));
            world.spawn((
                Transform::from_xyz(0.0, 0.0, -1.2)
                    .with_rotation(glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                MeshRenderer::new(plane, Color::WHITE).with_material(backdrop),
            ));
            world.spawn((
                Transform::IDENTITY,
                MeshRenderer::new(sphere, Color::WHITE).with_material(subject),
            ));
            world.spawn((Transform::from_xyz(0.0, 0.0, 1.7), Camera::default()));
        }
        Scene::Water => {
            let plane = meshes.add(MeshData::plane(30.0));
            let sphere = meshes.add(MeshData::primitive(Shape::Sphere));
            world.spawn((
                Transform::from_xyz(0.0, -0.6, 0.0),
                MeshRenderer::new(plane, Color::WHITE).with_material(backdrop),
            ));
            world.spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                MeshRenderer::new(plane, Color::WHITE).with_material(subject),
            ));
            world.spawn((
                Transform::from_xyz(0.2, -0.05, -0.3),
                MeshRenderer::new(sphere, Color::rgb(0.9, 0.5, 0.2)),
            ));
            world.spawn((
                Transform::from_xyz(0.0, 1.5, 2.3).looking_at(Vec3::new(0.0, -0.2, -0.2), Vec3::Y),
                Camera::default(),
            ));
        }
    }
}

fn scene_settings() -> RenderSettings {
    RenderSettings {
        light_dir: Vec3::new(-0.5, -0.6, -0.7).normalize(),
        light_color: Color::rgb(1.25, 1.2, 1.1),
        ambient: Color::rgb(0.22, 0.26, 0.34),
        ground_ambient: Color::rgb(0.12, 0.1, 0.09),
        clear_color: Color::rgb(0.05, 0.06, 0.08),
        ..Default::default()
    }
}

/// `time` for animated shaders: partway through a dissolve.
const SHOT_TIME: f32 = 1.5;

/// One material's tile: an RGB image `size` square. `None` without a GPU.
fn shot(
    h: &mut Harness,
    desc: Option<&MaterialDesc>,
    scene: Scene,
    size: u32,
    shader_src: Option<&str>,
) -> Vec<[u8; 3]> {
    h.size = size;
    h.core.set_time(SHOT_TIME);
    h.meshes = Meshes::default();
    h.materials = Materials::default();
    h.gpu = GpuMeshes::default();
    h.view = h.core.create_view();
    let shader = shader_src.map(|s| {
        let handle = h.shaders.add(s.to_owned());
        assert_eq!(h.shaders.error(handle), None);
        handle
    });
    let subject = match desc {
        Some(d) => material(h, d, shader),
        None => Material::default(),
    };
    let mut world = World::new();
    let (mut meshes, mut materials, mut textures) =
        (Meshes::default(), Materials::default(), Textures::default());
    spawn_scene(
        &mut meshes,
        &mut materials,
        &mut textures,
        &mut world,
        subject,
        scene,
    );
    h.meshes = meshes;
    h.materials = materials;
    h.textures = textures;
    let (_, at) = h.render(&world, &scene_settings());
    (0..size * size).map(|i| at(i % size, i / size)).collect()
}

fn mean_diff(a: &[[u8; 3]], b: &[[u8; 3]]) -> f32 {
    let sum: u32 = a
        .iter()
        .zip(b)
        .map(|(p, q)| {
            p.iter()
                .zip(q)
                .map(|(&x, &y)| x.abs_diff(y) as u32)
                .sum::<u32>()
        })
        .sum();
    sum as f32 / (a.len() * 3) as f32
}

fn mean_luma(a: &[[u8; 3]]) -> f32 {
    a.iter().map(|p| luma(*p)).sum::<u32>() as f32 / a.len() as f32
}

fn scene_for(rel: &str) -> Scene {
    if rel.contains("water") {
        Scene::Water
    } else {
        Scene::Sphere
    }
}

#[test]
fn library_materials_draw_on_both_tiers_and_differ_from_the_default() {
    let materials = assets_with(".mat.ron");
    assert!(!materials.is_empty(), "no library materials found");
    for tier in [RenderTier::Compat, RenderTier::Full] {
        let Some(mut h) = Harness::new(false) else {
            return;
        };
        h.core.set_tier(tier);
        let size = 64;
        let mut baselines = std::collections::HashMap::new();
        for (_, rel, path) in &materials {
            let desc = MaterialDesc::load(path).unwrap();
            let src = desc.shader.as_ref().map(|s| {
                let (_, _, p) = assets_with(".wgsl")
                    .into_iter()
                    .find(|(_, r, _)| r == s)
                    .unwrap_or_else(|| panic!("{rel}: shader {s} missing"));
                std::fs::read_to_string(p).unwrap()
            });
            let scene = scene_for(rel);
            let base = baselines
                .entry(scene as u8)
                .or_insert_with(|| shot(&mut h, None, scene, size, None))
                .clone();
            let got = shot(&mut h, Some(&desc), scene, size, src.as_deref());
            let diff = mean_diff(&got, &base);
            let light = mean_luma(&got);
            assert!(
                diff > 0.5,
                "{tier:?} {rel}: looks like the default material ({diff})"
            );
            assert!(light > 15.0, "{tier:?} {rel}: a black frame ({light})");
            // No NaN pixels: they would read back as a saturated or zero blob;
            // a fully-saturated white frame is just as wrong.
            assert!(light < 740.0, "{tier:?} {rel}: blown out ({light})");
        }
    }
}

// ---- previews --------------------------------------------------------------

/// `rows` of tiles into one image, written as an optimised PNG.
fn write_sheet(dir: &Path, pack: &str, tile: u32, cols: u32, tiles: &[Vec<[u8; 3]>]) {
    let rows = (tiles.len() as u32).div_ceil(cols);
    let mut img = image::RgbImage::new(tile * cols, tile * rows);
    for (i, t) in tiles.iter().enumerate() {
        let (ox, oy) = ((i as u32 % cols) * tile, (i as u32 / cols) * tile);
        for (j, p) in t.iter().enumerate() {
            img.put_pixel(ox + j as u32 % tile, oy + j as u32 / tile, image::Rgb(*p));
        }
    }
    std::fs::create_dir_all(dir).unwrap();
    let file = std::fs::File::create(dir.join(format!("{pack}.png"))).unwrap();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new_with_quality(
            std::io::BufWriter::new(file),
            image::codecs::png::CompressionType::Best,
            image::codecs::png::FilterType::Adaptive,
        ),
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgb8,
    )
    .unwrap();
}

/// The demo scene of the looks previews: shapes on a floor under a sky.
fn look_scene(meshes: &mut Meshes, materials: &mut Materials, world: &mut World) {
    let floor = meshes.add(MeshData::plane(30.0));
    let sphere = meshes.add(MeshData::primitive(Shape::Sphere));
    let cube = meshes.add(MeshData::cube(1.0));
    let cylinder = meshes.add(MeshData::primitive(Shape::Cylinder));
    let mut paint = |c: [f32; 4], rough: f32, metal: f32, emissive: [f32; 3]| {
        materials.add(Material {
            base_color: Color(c),
            roughness: rough,
            metallic: metal,
            emissive,
            ..Default::default()
        })
    };
    let ground = paint([0.22, 0.2, 0.18, 1.0], 0.9, 0.0, [0.0; 3]);
    let red = paint([0.8, 0.12, 0.1, 1.0], 0.35, 0.0, [0.0; 3]);
    let blue = paint([0.12, 0.3, 0.8, 1.0], 0.5, 0.0, [0.0; 3]);
    let gold = paint([1.0, 0.7, 0.25, 1.0], 0.25, 1.0, [0.0; 3]);
    let lamp = paint([0.0, 0.0, 0.0, 1.0], 1.0, 0.0, [6.0, 4.6, 2.4]);
    let mut put = |mesh, mat, pos: Vec3, scale: Vec3| {
        world.spawn((
            Transform::from_translation(pos).with_scale(scale),
            MeshRenderer::new(mesh, Color::WHITE).with_material(mat),
        ));
    };
    put(floor, ground, Vec3::ZERO, Vec3::ONE);
    put(sphere, red, Vec3::new(-0.85, 0.5, 0.0), Vec3::splat(1.0));
    put(
        cube,
        blue,
        Vec3::new(0.2, 0.4, -0.6),
        Vec3::new(0.8, 0.8, 0.8),
    );
    put(
        cylinder,
        gold,
        Vec3::new(1.0, 0.5, 0.3),
        Vec3::new(0.7, 0.5, 0.7),
    );
    put(sphere, lamp, Vec3::new(0.25, 1.25, -0.5), Vec3::splat(0.22));
    world.spawn((
        Transform::from_xyz(0.2, 1.0, 2.7).looking_at(Vec3::new(0.1, 0.4, 0.0), Vec3::Y),
        Camera::default(),
    ));
}

/// What the renderer gets from an environment (the scene loader does the
/// same in `olvin::scene_file::apply_lighting`).
fn settings_with_look(look: &LookDesc) -> RenderSettings {
    let mut env = Environment::default();
    look.apply_to(&mut env);
    let mut s = RenderSettings {
        light_dir: Vec3::new(-0.45, -0.55, -0.7).normalize(),
        light_color: Color::rgb(1.7, 1.55, 1.3),
        ambient: Color::rgb(0.2, 0.25, 0.34),
        ground_ambient: Color::rgb(0.14, 0.12, 0.1),
        sky: SkyMode::Procedural {
            zenith: [0.06, 0.14, 0.35],
            horizon: [0.32, 0.4, 0.5],
            ground: [0.15, 0.14, 0.13],
            sun_size: 0.0,
        },
        ..Default::default()
    };
    s.exposure = env.exposure;
    s.tonemapping = if env.filmic {
        Tonemapping::Aces
    } else {
        Tonemapping::None
    };
    s.bloom = env.post.bloom.map(|b| crate::Bloom {
        threshold: b.threshold,
        intensity: b.intensity,
    });
    s.fog = env.fog.map(|f| crate::Fog {
        color: Color::rgb(f.color[0], f.color[1], f.color[2]),
        start: f.start,
        end: f.end,
    });
    s.height_fog = env.height_fog.map(|f| crate::HeightFog {
        color: Color::rgb(f.color[0], f.color[1], f.color[2]),
        density: f.density,
        falloff: f.falloff,
        height: f.height,
    });
    s.post.fxaa = env.post.fxaa;
    s.post.ao = env.post.ao.map(|a| crate::AmbientOcclusion {
        radius: a.radius,
        strength: a.strength,
        bias: a.bias,
    });
    s.post.grade = crate::ColorGrade {
        lut: None,
        lut_strength: env.post.lut_strength,
        saturation: env.post.saturation,
        contrast: env.post.contrast,
        lift: env.post.lift,
        gamma: env.post.gamma,
        gain: env.post.gain,
    };
    s
}

/// The demo scene through the Full-tier post chain with `look`; its LUT (a
/// path under `assets/`) is read from the looks pack when there is one.
fn look_shot(look: &LookDesc, size: u32) -> Option<Vec<[u8; 3]>> {
    let lut = look.lut.as_ref().map(|rel| {
        let (_, _, path) = assets_with(rel)
            .into_iter()
            .find(|(_, r, _)| r == rel)
            .unwrap_or_else(|| panic!("LUT {rel} missing"));
        let img = image::open(path).unwrap().to_rgba8();
        TextureData {
            width: img.width(),
            height: img.height(),
            format: TextureFormat::Rgba8Linear,
            pixels: img.into_raw(),
        }
    });
    let settings = settings_with_look(look);
    let frame = post_world_sized(size, &settings, lut, look_scene)?;
    Some(
        (0..size * size)
            .map(|i| frame.at(i % size, i / size))
            .collect(),
    )
}

/// Every pack's looks, in the demo scene.
fn looks_of_pack(pack: &str) -> Vec<LookDesc> {
    assets_with(".look.ron")
        .into_iter()
        .filter(|(p, _, _)| p == pack)
        .map(|(_, _, path)| LookDesc::load(path).unwrap())
        .collect()
}

#[test]
fn library_looks_render_through_the_post_chain() {
    // Every look file loads, and each changes the frame against Neutral.
    let neutral = look_shot(&LookDesc::default(), 64);
    let Some(neutral) = neutral else {
        return;
    };
    let looks = assets_with(".look.ron");
    assert!(!looks.is_empty(), "no look files in the library");
    for (_, rel, path) in looks {
        let look = LookDesc::load(&path).unwrap_or_else(|e| panic!("{e}"));
        let got = look_shot(&look, 64).unwrap();
        assert!(mean_luma(&got) > 10.0, "{rel}: black frame");
        if look != LookDesc::default() {
            assert!(mean_diff(&got, &neutral) > 1.0, "{rel}: no visible change");
        }
    }
}

/// Renders `preview.png` of every library pack into `$OLVIN_LIBRARY_PREVIEWS/<pack>.png`.
#[test]
fn render_library_previews() {
    let Some(dir) = std::env::var_os("OLVIN_LIBRARY_PREVIEWS") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let mut by_pack: std::collections::BTreeMap<String, Vec<(String, PathBuf)>> =
        Default::default();
    for (pack, rel, path) in assets_with(".mat.ron") {
        by_pack.entry(pack).or_default().push((rel, path));
    }
    for (pack, mats) in by_pack {
        let tile = if mats.len() > 6 { 128 } else { 192 };
        let mut tiles = Vec::new();
        for (rel, path) in &mats {
            let desc = MaterialDesc::load(path).unwrap();
            let src = desc.shader.as_ref().map(|s| {
                let (_, _, p) = assets_with(".wgsl")
                    .into_iter()
                    .find(|(_, r, _)| r == s)
                    .unwrap();
                std::fs::read_to_string(p).unwrap()
            });
            tiles.push(shot(
                &mut h,
                Some(&desc),
                scene_for(rel),
                tile,
                src.as_deref(),
            ));
        }
        let cols = if mats.len() > 6 {
            5
        } else {
            mats.len().min(3) as u32
        };
        write_sheet(&dir, &pack, tile, cols, &tiles);
    }
    // Looks: the demo scene without a look (Neutral), then each look of the pack.
    let tile = 192;
    for pack in ["looks-cinematic"] {
        let mut looks = vec![LookDesc::default()];
        looks.extend(
            looks_of_pack(pack)
                .into_iter()
                .filter(|l| l.name != "Neutral"),
        );
        if looks.len() < 2 {
            continue;
        }
        let tiles: Vec<_> = looks.iter().filter_map(|l| look_shot(l, tile)).collect();
        write_sheet(&dir, pack, tile, 4, &tiles);
    }
}

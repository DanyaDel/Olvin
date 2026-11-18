use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use glam::Vec3;

use crate::*;

/// Tiny temp directory under `std::env::temp_dir()`, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "olvin_assets_{tag}_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, rel: &str, data: &[u8]) -> PathBuf {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, data).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x * 40) as u8, (y * 40) as u8, 128, 255])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn meta_created_and_guid_survives_move() {
    let dir = TempDir::new("meta");
    dir.write("tex/a.png", &png_bytes(2, 2));
    dir.write("level.scene.ron", b"()");
    let mut root = AssetRoot::new(dir.path());
    let entries = root.scan();
    assert_eq!(entries.len(), 2);
    assert!(dir.path().join("tex/a.png.meta").is_file());
    assert!(dir.path().join("level.scene.ron.meta").is_file());

    let meta = Meta::load(dir.path().join("tex/a.png.meta")).unwrap();
    assert_eq!(meta.texture(), Some(&TextureImport::default()));
    let guid = root.guid_of("tex/a.png").unwrap();
    assert_eq!(meta.guid, guid);
    assert_eq!(root.path_of(guid), Some("tex/a.png"));

    // Rescan keeps GUIDs; metas are not listed as assets.
    let again = root.scan();
    assert_eq!(again, entries);

    root.move_asset("tex/a.png", "textures/b.png").unwrap();
    assert!(!dir.path().join("tex/a.png.meta").exists());
    assert!(dir.path().join("textures/b.png.meta").is_file());
    assert_eq!(root.path_of(guid), Some("textures/b.png"));
    assert_eq!(root.guid_of("textures/b.png"), Some(guid));

    // Folder move re-keys everything inside.
    root.move_asset("textures", "art/tex").unwrap();
    assert_eq!(root.path_of(guid), Some("art/tex/b.png"));
    root.scan();
    assert_eq!(root.guid_of("art/tex/b.png"), Some(guid));

    // A file copied together with its meta gets a fresh GUID.
    std::fs::copy(
        dir.path().join("art/tex/b.png"),
        dir.path().join("art/tex/c.png"),
    )
    .unwrap();
    std::fs::copy(
        dir.path().join("art/tex/b.png.meta"),
        dir.path().join("art/tex/c.png.meta"),
    )
    .unwrap();
    root.scan();
    assert_eq!(root.guid_of("art/tex/b.png"), Some(guid));
    assert_ne!(root.guid_of("art/tex/c.png"), Some(guid));

    root.delete_asset("art/tex/c.png").unwrap();
    assert!(!dir.path().join("art/tex/c.png.meta").exists());
    assert!(root.move_asset("missing.png", "x.png").is_err());

    // Importer settings round-trip without changing the GUID.
    root.set_importer(
        "art/tex/b.png",
        Importer::Texture(TextureImport {
            srgb: false,
            max_size: 1,
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(root.guid_of("art/tex/b.png"), Some(guid));
    let tex = root.load_texture("art/tex/b.png").unwrap();
    assert_eq!((tex.width, tex.format), (1, ImageFormat::Rgba8Linear));
}

#[test]
fn paths_and_kinds() {
    let dir = TempDir::new("kinds");
    for f in [
        "a.scene.ron",
        "m/b.mat.ron",
        "c.jpg",
        "d.glb",
        "e.wgsl",
        "f.rs",
        "g.ogg",
        "h.prefab.ron",
        "i.txt",
        ".hidden",
        "j.png~",
        "k.tmp",
    ] {
        dir.write(f, b"x");
    }
    let mut root = AssetRoot::new(dir.path());
    let kinds: Vec<(String, AssetKind)> =
        root.scan().into_iter().map(|e| (e.path, e.kind)).collect();
    use AssetKind::*;
    let expect = [
        ("a.scene.ron", Scene),
        ("c.jpg", Texture),
        ("d.glb", Model),
        ("e.wgsl", Shader),
        ("f.rs", Script),
        ("g.ogg", Audio),
        ("h.prefab.ron", Prefab),
        ("i.txt", Other),
        ("m/b.mat.ron", Material),
    ];
    assert_eq!(kinds, expect.map(|(p, k)| (p.to_string(), k)).to_vec());

    assert_eq!(
        root.resolve("m/b.mat.ron"),
        dir.path().join("m").join("b.mat.ron")
    );
    assert_eq!(
        root.relative(dir.path().join("m/b.mat.ron")).as_deref(),
        Some("m/b.mat.ron")
    );
    assert_eq!(root.relative("/somewhere/else"), None);
    // A Guid prints as 32 hex digits and parses back.
    let g = Guid::new_random();
    assert_eq!(Guid::parse(&g.to_string()), Some(g));
    assert_ne!(Guid::new_random(), g);
}

#[test]
fn image_formats_and_downscale() {
    let dir = TempDir::new("img");
    let png = dir.write("a.png", &png_bytes(8, 4));
    let img = load_image(&png, true).unwrap();
    assert_eq!(
        (img.width, img.height, img.format),
        (8, 4, ImageFormat::Rgba8Srgb)
    );
    assert_eq!(img.pixels.len(), 8 * 4 * 4);
    assert_eq!(&img.pixels[4..8], &[40, 0, 128, 255]);
    assert_eq!(
        load_image(&png, false).unwrap().format,
        ImageFormat::Rgba8Linear
    );

    let small = load_image_sized(&png, true, 4).unwrap();
    assert_eq!((small.width, small.height), (4, 2));

    // HDR → floats above 1.
    let hdr = dir.path().join("sky.hdr");
    let rgb = image::Rgb32FImage::from_fn(4, 2, |x, _| image::Rgb([x as f32 * 2.0, 0.5, 0.25]));
    image::DynamicImage::ImageRgb32F(rgb).save(&hdr).unwrap();
    let h = load_image(&hdr, true).unwrap();
    assert_eq!(
        (h.width, h.height, h.format),
        (4, 2, ImageFormat::Rgba32Float)
    );
    assert_eq!(h.pixels.len(), 4 * 2 * 16);
    let f = h.floats();
    assert!((f[3 * 4] - 6.0).abs() < 0.1, "{}", f[12]);
    assert_eq!(f[3], 1.0);

    // EXR too (image's exr feature).
    let exr = dir.path().join("sky.exr");
    let rgba = image::Rgba32FImage::from_pixel(2, 2, image::Rgba([3.0, 2.0, 1.0, 1.0]));
    image::DynamicImage::ImageRgba32F(rgba).save(&exr).unwrap();
    let e = load_image(&exr, true).unwrap();
    assert_eq!(e.format, ImageFormat::Rgba32Float);
    assert_eq!(e.floats()[0], 3.0);

    assert!(load_image(dir.path().join("missing.png"), true).is_err());
}

#[test]
fn thumbnails_fit_and_cache() {
    let dir = TempDir::new("thumb");
    let wide = dir.write("wide.png", &png_bytes(16, 8));
    let t = thumbnail(&wide, 4).unwrap();
    assert_eq!(
        (t.width, t.height, t.format),
        (4, 2, ImageFormat::Rgba8Srgb)
    );
    assert_eq!(t.pixels.len(), 4 * 2 * 4);
    let tall = dir.write("tall.png", &png_bytes(3, 9));
    let t = thumbnail(&tall, 6).unwrap();
    assert_eq!((t.width, t.height), (2, 6));
    assert!(thumbnail(dir.write("a.txt", b"hi"), 4).is_none());

    let hdr = dir.path().join("sky.hdr");
    image::DynamicImage::ImageRgb32F(image::Rgb32FImage::from_pixel(
        8,
        8,
        image::Rgb([4.0, 1.0, 0.0]),
    ))
    .save(&hdr)
    .unwrap();
    let t = thumbnail(&hdr, 4).unwrap();
    assert_eq!(t.format, ImageFormat::Rgba8Srgb);
    assert!(
        t.pixels[0] > 200 && t.pixels[0] < 255 && t.pixels[2] == 0,
        "{:?}",
        &t.pixels[..4]
    );

    let mut cache = ThumbnailCache::new(2);
    let a = cache.get(&wide, 4).unwrap();
    let b = cache.get(&wide, 4).unwrap();
    assert!(std::sync::Arc::ptr_eq(&a, &b));
    cache.get(&tall, 4);
    cache.get(&hdr, 4);
    assert_eq!(cache.len(), 2);
    // `wide` was least recently used: decoded again.
    let c = cache.get(&wide, 4).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&a, &c));
}

fn base64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[test]
fn base64_roundtrip() {
    for len in 0..8 {
        let data: Vec<u8> = (0..len).map(|i| i * 37 + 1).collect();
        assert_eq!(model::decode_base64(&base64(&data)).unwrap(), data);
    }
}

/// A quad (no normals, with UVs) on a child node under a rotated parent,
/// material with an embedded PNG in the buffer.
fn write_test_gltf(dir: &TempDir) -> PathBuf {
    let mut buf = Vec::new();
    for p in [
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ] {
        p.iter().for_each(|v| buf.extend(v.to_le_bytes()));
    }
    for uv in [[0.0f32, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]] {
        uv.iter().for_each(|v| buf.extend(v.to_le_bytes()));
    }
    for i in [0u16, 1, 2, 0, 2, 3] {
        buf.extend(i.to_le_bytes());
    }
    buf.resize(92, 0);
    let png = png_bytes(2, 2);
    let png_len = png.len();
    buf.extend(png);
    let json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [
    {{"name": "Root", "translation": [1, 0, 0], "rotation": [0, 0.70710678, 0, 0.70710678], "children": [1]}},
    {{"name": "Child", "translation": [0, 2, 0], "scale": [2, 2, 2], "mesh": 0}}
  ],
  "meshes": [{{"name": "Quad", "primitives": [{{"attributes": {{"POSITION": 0, "TEXCOORD_0": 1}}, "indices": 2, "material": 0}}]}}],
  "materials": [{{"name": "Brick Wall", "pbrMetallicRoughness": {{"baseColorFactor": [1, 0.5, 0.5, 1], "baseColorTexture": {{"index": 0}}, "metallicFactor": 0.25}}, "emissiveFactor": [1, 0, 0], "alphaMode": "MASK"}}],
  "textures": [{{"source": 0, "sampler": 0}}],
  "samplers": [{{"magFilter": 9728}}],
  "images": [{{"name": "bricks", "bufferView": 3, "mimeType": "image/png"}}],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]}},
    {{"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC2"}},
    {{"bufferView": 2, "componentType": 5123, "count": 6, "type": "SCALAR"}}
  ],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 48}},
    {{"buffer": 0, "byteOffset": 48, "byteLength": 32}},
    {{"buffer": 0, "byteOffset": 80, "byteLength": 12}},
    {{"buffer": 0, "byteOffset": 92, "byteLength": {png_len}}}
  ],
  "buffers": [{{"byteLength": {}, "uri": "data:application/octet-stream;base64,{}"}}]
}}"#,
        buf.len(),
        base64(&buf)
    );
    dir.write("models/crate.gltf", json.as_bytes())
}

#[test]
fn gltf_import_hierarchy_uvs_materials() {
    let dir = TempDir::new("gltf");
    write_test_gltf(&dir);
    let mut root = AssetRoot::new(dir.path());
    let model = root.import_model("models/crate.gltf").unwrap();
    assert_eq!(model.name, "crate");
    assert!(model.skipped.is_empty(), "{:?}", model.skipped);

    // Hierarchy: Root then Child.
    assert_eq!(model.nodes.len(), 2);
    let (r, c) = (&model.nodes[0], &model.nodes[1]);
    assert_eq!((r.name.as_str(), r.parent, r.mesh), ("Root", None, None));
    assert_eq!(
        (c.name.as_str(), c.parent, c.mesh),
        ("Child", Some(0), Some(0))
    );
    assert!((r.rotation_euler_deg - Vec3::new(0.0, 90.0, 0.0)).length() < 1e-3);
    assert_eq!(r.translation, Vec3::X);
    assert_eq!(c.scale, Vec3::splat(2.0));
    // Euler convention matches EntityDesc (YXZ).
    let q = olvin_scene::file::euler_to_quat(r.rotation_euler_deg);
    assert!(q.angle_between(glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)) < 1e-3);

    // Mesh: flat normals generated (un-indexed), UVs kept.
    let m = &model.meshes[0];
    assert_eq!(m.positions.len(), 6);
    assert_eq!(m.indices, (0..6).collect::<Vec<u32>>());
    assert!(m.normals.iter().all(|n| (*n - Vec3::Z).length() < 1e-5));
    assert_eq!(m.uvs[2], glam::Vec2::new(1.0, 0.0));
    assert_eq!(m.material, Some(0));
    assert!(m.tangents.is_none());

    // Material: embedded texture extracted next to the model.
    let mat = &model.materials[0];
    assert_eq!(model.material_names, ["Brick Wall"]);
    assert_eq!(
        mat.base_texture.as_deref(),
        Some("models/crate_textures/bricks.png")
    );
    assert_eq!(
        model.extracted_textures,
        ["models/crate_textures/bricks.png"]
    );
    assert_eq!(mat.base_color, [1.0, 0.5, 0.5, 1.0]);
    assert_eq!((mat.metallic, mat.roughness), (0.25, 1.0));
    assert_eq!(mat.emissive, [1.0, 0.0, 0.0]);
    assert_eq!(mat.alpha_cutoff, 0.5);
    assert!(mat.point_filter);
    let tex = root
        .load_texture("models/crate_textures/bricks.png")
        .unwrap();
    assert_eq!((tex.width, tex.height), (2, 2));
    // Re-import does not rewrite an unchanged texture.
    let mtime = |p: &str| {
        std::fs::metadata(root.resolve(p))
            .unwrap()
            .modified()
            .unwrap()
    };
    let before = mtime("models/crate_textures/bricks.png");
    root.import_model("models/crate.gltf").unwrap();
    assert_eq!(before, mtime("models/crate_textures/bricks.png"));

    // Materials → .mat.ron, entities → hierarchy with poly + per-face UVs.
    let mats = root
        .write_materials(&model, "models/crate_materials")
        .unwrap();
    assert_eq!(mats, ["models/crate_materials/Brick_Wall.mat.ron"]);
    let loaded = olvin_scene::material::MaterialDesc::load(root.resolve(&mats[0])).unwrap();
    assert_eq!(&loaded, mat);

    let ents = model_to_entities(&model, 100, &mats);
    let e = &ents.entities;
    assert_eq!(e.len(), 3);
    assert_eq!(
        (e[0].id, e[0].name.as_str(), e[0].parent),
        (100, "crate", None)
    );
    assert_eq!((e[1].id, e[1].parent), (101, Some(100)));
    assert_eq!((e[2].id, e[2].parent), (102, Some(101)));
    assert_eq!(e[1].rotation, r.rotation_euler_deg);
    let poly = e[2].poly.as_ref().unwrap();
    assert_eq!(poly.positions.len(), 4, "welded by position");
    assert_eq!(poly.faces.len(), 2);
    poly.validate().unwrap();
    assert_eq!(e[2].material.as_deref(), Some(mats[0].as_str()));
    assert_eq!(e[2].mesh.as_ref().unwrap().color, [1.0, 0.5, 0.5, 1.0]);
    let uvs = &ents.uvs_per_face[&102];
    assert_eq!(uvs.len(), 2);
    // UV of each corner follows its position (quad maps x→u, y→1-v).
    for (face, fuv) in poly.faces.iter().zip(uvs) {
        for (&v, uv) in face.iter().zip(fuv) {
            let p = poly.positions[v as usize];
            assert_eq!(*uv, glam::Vec2::new(p.x, 1.0 - p.y));
        }
    }
    // Scene file with these entities round-trips.
    let scene = olvin_scene::file::SceneFile {
        entities: ents.entities.clone(),
        ..Default::default()
    };
    let path = root.resolve("crate.scene.ron");
    scene.save(&path).unwrap();
    assert_eq!(olvin_scene::file::SceneFile::load(&path).unwrap(), scene);

    // The extracted texture and the model got metas on scan.
    let kinds: Vec<_> = root.scan().into_iter().map(|e| (e.path, e.kind)).collect();
    assert!(kinds.contains(&(
        "models/crate_textures/bricks.png".into(),
        AssetKind::Texture
    )));
    assert!(kinds.contains(&("models/crate.gltf".into(), AssetKind::Model)));
}

#[test]
fn gltf_scale_setting() {
    let dir = TempDir::new("gltf_scale");
    let path = write_test_gltf(&dir);
    let settings = ModelImport {
        scale: 0.5,
        ..Default::default()
    };
    let model = import_gltf(&path, dir.path(), &settings).unwrap();
    assert_eq!(model.nodes[0].translation, Vec3::new(0.5, 0.0, 0.0));
    assert_eq!(model.meshes[0].positions[2], Vec3::new(0.5, 0.5, 0.0));
}

fn wait_for(w: &mut AssetWatcher, want: &AssetEvent, seen: &mut Vec<AssetEvent>) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        seen.extend(w.poll());
        if seen.contains(want) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn watcher_events() {
    let dir = TempDir::new("watch");
    let mut w = match AssetWatcher::new(dir.path()) {
        Ok(w) => w.with_debounce(Duration::from_millis(50)),
        Err(e) => {
            eprintln!("skipping watcher test: {e}");
            return;
        }
    };
    let mut seen = Vec::new();
    dir.write("a.txt", b"1");
    dir.write("a.txt.meta", b"(guid: \"x\")");
    dir.write("b.tmp", b"temp");
    assert!(
        wait_for(&mut w, &AssetEvent::Created("a.txt".into()), &mut seen),
        "{seen:?}"
    );
    std::thread::sleep(Duration::from_millis(100));
    seen.extend(w.poll());
    assert!(
        seen.iter()
            .all(|e| matches!(e, AssetEvent::Created(p) if p == "a.txt")),
        "meta/temp must be ignored, create+write coalesced: {seen:?}"
    );

    seen.clear();
    dir.write("a.txt", b"22");
    assert!(
        wait_for(&mut w, &AssetEvent::Changed("a.txt".into()), &mut seen),
        "{seen:?}"
    );

    // Create the folder first: inotify watches a new folder only after its event.
    seen.clear();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    assert!(
        wait_for(&mut w, &AssetEvent::Created("sub".into()), &mut seen),
        "{seen:?}"
    );
    seen.clear();
    std::fs::rename(dir.path().join("a.txt"), dir.path().join("sub/c.txt")).unwrap();
    let moved = AssetEvent::Moved {
        from: "a.txt".into(),
        to: "sub/c.txt".into(),
    };
    assert!(wait_for(&mut w, &moved, &mut seen), "{seen:?}");
    std::thread::sleep(Duration::from_millis(100));
    seen.extend(w.poll());
    assert_eq!(seen, [moved], "one event per rename");

    seen.clear();
    std::fs::remove_file(dir.path().join("sub/c.txt")).unwrap();
    assert!(
        wait_for(&mut w, &AssetEvent::Removed("sub/c.txt".into()), &mut seen),
        "{seen:?}"
    );
}

#[test]
fn plugin_inserts_root() {
    let dir = TempDir::new("plugin");
    dir.write("x.wgsl", b"");
    let mut app = olvin_core::App::new();
    app.add_plugin(AssetsPlugin::new(dir.path()));
    let root = app.resources.fetch::<AssetRoot>();
    assert!(root.guid_of("x.wgsl").is_some());
    assert!(dir.path().join("x.wgsl.meta").is_file());
}

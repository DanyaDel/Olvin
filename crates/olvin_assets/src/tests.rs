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
fn resolve_never_leaves_the_assets_folder() {
    let dir = TempDir::new("resolve");
    let root = AssetRoot::new(dir.path());
    let base = dir.path().to_path_buf();
    assert_eq!(root.resolve("tex/a.png"), base.join("tex").join("a.png"));
    assert_eq!(
        root.resolve("tex\\.\\a.png"),
        base.join("tex").join("a.png")
    );
    assert_eq!(root.resolve("tex/../b.png"), base.join("b.png"));
    assert_eq!(
        root.resolve("../../etc/passwd"),
        base.join("etc").join("passwd")
    );
    assert_eq!(root.resolve("/etc/passwd"), base.join("etc").join("passwd"));
    assert_eq!(
        root.resolve("C:/Windows/x.dll"),
        base.join("Windows").join("x.dll")
    );
    assert_eq!(root.resolve(".."), base);
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

const WAIT: Duration = Duration::from_secs(10);

fn write_material(dir: &TempDir, rel: &str, base: Option<&str>, normal: Option<&str>) {
    let mat = olvin_scene::material::MaterialDesc {
        base_texture: base.map(Into::into),
        normal_texture: normal.map(Into::into),
        ..Default::default()
    };
    mat.save(dir.path().join(rel)).unwrap();
}

#[test]
fn manager_loads_in_background_and_shares_handles() {
    let dir = TempDir::new("mgr_load");
    dir.write("tex/a.png", &png_bytes(4, 4));
    let mut mgr = AssetManager::new(dir.path());
    let a = mgr.load::<ImageData>("tex/a.png");
    let b = mgr.load::<ImageData>("./tex\\a.png");
    assert_eq!(a, b, "same asset, same handle");
    assert_eq!(mgr.len(), 1);
    assert_eq!(mgr.ref_count(&a), 2);
    assert!(mgr.wait_idle(WAIT));
    assert_eq!(mgr.state(&a), LoadState::Ready);
    let img = mgr.get(&a).expect("loaded");
    assert_eq!((img.width, img.height), (4, 4));
    assert_eq!(mgr.version(&a), 1);
    assert_eq!(
        mgr.take_changes(),
        [AssetChange {
            path: "tex/a.png".into(),
            kind: AssetKind::Texture,
            version: 1
        }]
    );
    let missing = mgr.load::<ImageData>("tex/none.png");
    assert!(mgr.wait_idle(WAIT));
    assert!(matches!(mgr.state(&missing), LoadState::Failed(_)));
    assert!(mgr.get(&missing).is_none());
}

#[test]
fn manager_unloads_only_unused() {
    let dir = TempDir::new("mgr_unload");
    dir.write("a.png", &png_bytes(2, 2));
    dir.write("b.png", &png_bytes(2, 2));
    let mut mgr = AssetManager::new(dir.path());
    mgr.set_auto_unload(false);
    let a = mgr.load::<ImageData>("a.png");
    let b = mgr.load::<ImageData>("b.png");
    assert!(mgr.wait_idle(WAIT));
    drop(b);
    assert_eq!(mgr.unload_unused(), 1);
    assert!(mgr.contains::<ImageData>("a.png"));
    assert!(!mgr.contains::<ImageData>("b.png"));
    // Dropped but not swept: asking again revives the cached data without a reload.
    drop(a);
    let again = mgr.load::<ImageData>("a.png");
    assert_eq!(mgr.pending(), 0);
    assert!(mgr.get(&again).is_some());
    assert_eq!(mgr.version(&again), 1);
    // Auto unload does it on update.
    mgr.set_auto_unload(true);
    drop(again);
    mgr.update();
    assert!(mgr.is_empty());
}

#[test]
fn material_keeps_its_textures_alive() {
    let dir = TempDir::new("mgr_deps");
    dir.write("t/base.png", &png_bytes(2, 2));
    dir.write("t/norm.png", &png_bytes(2, 2));
    write_material(&dir, "m/a.mat.ron", Some("t/base.png"), Some("t/norm.png"));
    write_material(&dir, "m/b.mat.ron", Some("t/base.png"), None);
    let mut mgr = AssetManager::new(dir.path());
    let a = mgr.load::<olvin_scene::material::MaterialDesc>("m/a.mat.ron");
    let b = mgr.load::<olvin_scene::material::MaterialDesc>("m/b.mat.ron");
    assert!(!mgr.is_fully_loaded(&a));
    // Dependencies are requested once the material is parsed: wait until it settles.
    let start = Instant::now();
    while !(mgr.is_fully_loaded(&a) && mgr.is_fully_loaded(&b)) {
        assert!(start.elapsed() < WAIT, "never settled");
        mgr.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(mgr.len(), 4);
    let deps = mgr.dependencies(&a);
    let paths: Vec<&str> = deps.iter().map(AnyHandle::path).collect();
    assert_eq!(paths, ["t/base.png", "t/norm.png"]);
    let base = deps[0].typed::<ImageData>().unwrap();
    assert!(
        deps[0]
            .typed::<olvin_scene::material::MaterialDesc>()
            .is_none()
    );
    // Both materials, our clone and the untyped one in `deps`.
    assert_eq!(mgr.ref_count(&base), 4);
    drop(base);
    drop(deps);

    // Nobody holds a handle to the textures, yet they live as long as materials do.
    mgr.update();
    assert_eq!(mgr.len(), 4);
    drop(a);
    mgr.update();
    assert!(!mgr.contains::<olvin_scene::material::MaterialDesc>("m/a.mat.ron"));
    assert!(!mgr.contains::<ImageData>("t/norm.png"), "only a used it");
    assert!(mgr.contains::<ImageData>("t/base.png"), "b still uses it");
    drop(b);
    mgr.update();
    assert!(mgr.is_empty());
}

#[test]
fn model_pulls_in_its_textures() {
    let dir = TempDir::new("mgr_model");
    write_test_gltf(&dir);
    let mut mgr = AssetManager::new(dir.path());
    let model = mgr.load::<ModelData>("models/crate.gltf");
    let start = Instant::now();
    while !mgr.is_fully_loaded(&model) {
        assert!(start.elapsed() < WAIT, "never settled");
        mgr.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    let data = mgr.get(&model).unwrap();
    assert_eq!(data.meshes.len(), 1);
    let deps = mgr.dependencies(&model);
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].kind(), AssetKind::Texture);
    assert!(deps[0].path().ends_with("bricks.png"), "{}", deps[0].path());
    assert!(mgr.get(&deps[0].typed::<ImageData>().unwrap()).is_some());
    drop(deps);
    drop(model);
    mgr.update();
    assert!(mgr.is_empty());
}

#[test]
fn manual_reload_swaps_data_and_keeps_old_on_failure() {
    let dir = TempDir::new("mgr_reload");
    let path = dir.write("a.png", &png_bytes(2, 2));
    let mut mgr = AssetManager::new(dir.path());
    let a = mgr.load::<ImageData>("a.png");
    assert!(mgr.wait_idle(WAIT));
    assert_eq!(mgr.get(&a).unwrap().width, 2);

    std::fs::write(&path, png_bytes(8, 8)).unwrap();
    mgr.reload(&a);
    assert_eq!(mgr.state(&a), LoadState::Loading);
    assert_eq!(
        mgr.get(&a).unwrap().width,
        2,
        "old data until the new one is ready"
    );
    assert!(mgr.wait_idle(WAIT));
    assert_eq!(mgr.get(&a).unwrap().width, 8);
    assert_eq!(mgr.version(&a), 2);

    std::fs::write(&path, b"not an image").unwrap();
    mgr.reload(&a);
    assert!(mgr.wait_idle(WAIT));
    assert!(matches!(mgr.state(&a), LoadState::Failed(_)));
    assert_eq!(mgr.get(&a).unwrap().width, 8, "last good data stays");
}

#[test]
fn hot_reload_follows_file_changes() {
    let dir = TempDir::new("mgr_hot");
    let path = dir.write("a.png", &png_bytes(2, 2));
    let mut mgr = AssetManager::new(dir.path());
    mgr.enable_hot_reload().unwrap();
    assert!(mgr.hot_reload_enabled());
    let a = mgr.load::<ImageData>("a.png");
    assert!(mgr.wait_idle(WAIT));
    mgr.take_changes();

    std::fs::write(&path, png_bytes(6, 6)).unwrap();
    let start = Instant::now();
    while mgr.version(&a) < 2 {
        assert!(start.elapsed() < WAIT, "no reload");
        mgr.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(mgr.get(&a).unwrap().width, 6);
    assert_eq!(mgr.take_changes().last().map(|c| c.version), Some(2));
}

#[test]
fn plugin_inserts_manager() {
    let dir = TempDir::new("mgr_plugin");
    dir.write("x.png", &png_bytes(2, 2));
    let mut app = olvin_core::App::new();
    app.add_plugin(AssetsPlugin::new(dir.path()));
    let handle = app
        .resources
        .fetch_mut::<AssetManager>()
        .load::<ImageData>("x.png");
    let start = Instant::now();
    while app.resources.fetch::<AssetManager>().get(&handle).is_none() {
        assert!(start.elapsed() < WAIT);
        app.update();
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Buffer builder for glTF fixtures: 4-byte aligned views.
#[derive(Default)]
struct GltfBuf {
    data: Vec<u8>,
    views: Vec<(usize, usize)>,
}

impl GltfBuf {
    fn add(&mut self, bytes: &[u8]) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        self.views.push((self.data.len(), bytes.len()));
        self.data.extend(bytes);
        self.views.len() - 1
    }

    fn floats(&mut self, v: &[f32]) -> usize {
        self.add(&v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<_>>())
    }

    fn views_json(&self) -> String {
        let v: Vec<String> = self
            .views
            .iter()
            .map(|(o, l)| format!(r#"{{"buffer": 0, "byteOffset": {o}, "byteLength": {l}}}"#))
            .collect();
        format!("[{}]", v.join(","))
    }

    fn buffer_json(&self) -> String {
        format!(
            r#"[{{"byteLength": {}, "uri": "data:application/octet-stream;base64,{}"}}]"#,
            self.data.len(),
            base64(&self.data)
        )
    }
}

/// Two-joint bar (Root → Tip at y = 1) skinned onto a unit quad: the bottom
/// vertices follow Root, the top ones Tip. Animation "bend" turns Root by 90°
/// about Z over one second. Plus a perspective camera node.
fn write_skinned_gltf(dir: &TempDir) -> PathBuf {
    let mut b = GltfBuf::default();
    let pos = b.floats(&[0., 0., 0., 1., 0., 0., 0., 1., 0., 1., 1., 0.]);
    let joints = b.add(&[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
    let weights = b.floats(&[
        1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.,
    ]);
    let idx = b.add(
        &[0u16, 1, 2, 1, 3, 2]
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let ibm = b.floats(&[
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., //
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., -1., 0., 1.,
    ]);
    let times = b.floats(&[0.0, 1.0]);
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let rots = b.floats(&[0., 0., 0., 1., 0., 0., h, h]);
    let json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0, 1, 3]}}],
  "nodes": [
    {{"name": "Body", "mesh": 0, "skin": 0}},
    {{"name": "Root", "children": [2]}},
    {{"name": "Tip", "translation": [0, 1, 0]}},
    {{"name": "Cam", "camera": 0, "translation": [0, 0, 5]}}
  ],
  "meshes": [{{"name": "Bar", "primitives": [{{"attributes": {{"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}}, "indices": 3}}]}}],
  "skins": [{{"name": "Rig", "joints": [1, 2], "inverseBindMatrices": 4}}],
  "cameras": [{{"name": "Main", "type": "perspective", "perspective": {{"yfov": 0.8, "znear": 0.1, "zfar": 50}}}}],
  "animations": [{{"name": "bend",
    "channels": [{{"sampler": 0, "target": {{"node": 1, "path": "rotation"}}}}],
    "samplers": [{{"input": 5, "output": 6, "interpolation": "LINEAR"}}]}}],
  "accessors": [
    {{"bufferView": {pos}, "componentType": 5126, "count": 4, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]}},
    {{"bufferView": {joints}, "componentType": 5121, "count": 4, "type": "VEC4"}},
    {{"bufferView": {weights}, "componentType": 5126, "count": 4, "type": "VEC4"}},
    {{"bufferView": {idx}, "componentType": 5123, "count": 6, "type": "SCALAR"}},
    {{"bufferView": {ibm}, "componentType": 5126, "count": 2, "type": "MAT4"}},
    {{"bufferView": {times}, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]}},
    {{"bufferView": {rots}, "componentType": 5126, "count": 2, "type": "VEC4"}}
  ],
  "bufferViews": {},
  "buffers": {}
}}"#,
        b.views_json(),
        b.buffer_json()
    );
    dir.write("models/rig.gltf", json.as_bytes())
}

#[test]
fn gltf_imports_skin_animation_and_camera() {
    let dir = TempDir::new("gltf_skin");
    let path = write_skinned_gltf(&dir);
    let model = import_gltf(&path, dir.path(), &ModelImport::default()).unwrap();
    assert!(model.skipped.is_empty(), "{:?}", model.skipped);

    assert_eq!(model.skeletons.len(), 1);
    let sk = &model.skeletons[0];
    assert_eq!(sk.len(), 2);
    assert_eq!(sk.joints[0].name, "Root");
    assert_eq!(sk.joints[0].parent, None);
    assert_eq!(sk.joints[1].parent, Some(0));
    assert_eq!(sk.joints[1].rest.translation, Vec3::Y);
    // Joint targets are model node indices.
    assert_eq!(model.nodes[sk.joints[1].target].name, "Tip");

    let bar = &model.meshes[0];
    assert_eq!(bar.skin, Some(0));
    // No normals in the file: un-indexed, joints follow.
    assert_eq!(bar.joints.len(), bar.positions.len());
    assert_eq!(bar.weights.len(), bar.positions.len());
    assert_eq!(bar.joints[2], [1, 0, 0, 0]);
    assert_eq!(bar.weights[0], [1.0, 0.0, 0.0, 0.0]);

    assert_eq!(model.clips.len(), 1);
    assert_eq!(model.clips[0].name, "bend");
    assert_eq!(model.clips[0].duration, 1.0);

    // End of the clip: the tip (bind position (0, 1, 0)) swings to (-1, 0, 0).
    let mut animator = model.animator(0).unwrap();
    let mut pose = sk.rest_pose();
    model.clips[0].apply(1.0, sk, &mut pose);
    let tip = sk.skin_matrices(&pose)[1].transform_point3(Vec3::Y);
    assert!(tip.abs_diff_eq(Vec3::new(-1.0, 0.0, 0.0), 1e-5), "{tip:?}");
    // The same through the runtime component.
    animator = animator.with_graph(olvin_anim::AnimGraph::single(0));
    // 0.9 s of a looping second: 81 degrees.
    for _ in 0..9 {
        animator.update(0.1);
    }
    assert!(animator.matrices[1].transform_point3(Vec3::Y).x < -0.98);
    assert!(model.animator(5).is_none());

    assert_eq!(model.cameras.len(), 1);
    let cam = &model.cameras[0];
    assert_eq!(model.nodes[cam.node].name, "Cam");
    assert!(matches!(
        cam.projection,
        CameraProjection::Perspective { far: Some(f), .. } if f == 50.0
    ));
    let ents = model_to_entities(&model, 100, &[]);
    let cam_entity = ents.entities.iter().find(|e| e.name == "Cam").unwrap();
    let desc = cam_entity.camera.as_ref().unwrap();
    assert!((desc.fov_deg - 0.8f32.to_degrees()).abs() < 1e-4 && desc.far == 50.0);
}

#[test]
fn scene_animation_setups_resolve_clip_names() {
    use olvin_scene::anim::*;
    let dir = TempDir::new("anim_setup");
    let path = write_skinned_gltf(&dir);
    let model = import_gltf(&path, dir.path(), &ModelImport::default()).unwrap();

    // No setup: rest pose, no layers.
    assert!(
        model
            .animator_with(0, None, None)
            .unwrap()
            .layers
            .is_empty()
    );
    let looping = model
        .animator_with(0, Some(&AnimationDesc::Clip("bend".into())), None)
        .unwrap();
    assert_eq!(
        looping.layers[0].graph.states[0].motion,
        olvin_anim::Motion::Clip(0)
    );
    assert!(
        model
            .animator_with(0, Some(&AnimationDesc::Clip("nope".into())), None)
            .unwrap_err()
            .contains("nope")
    );

    let graph = AnimGraphDesc {
        states: vec![
            AnimStateDesc::clip("Rest", "bend"),
            AnimStateDesc::clip("Bend", "bend"),
        ],
        transitions: vec![TransitionDesc {
            from: None,
            to: "Bend".into(),
            when: vec![("go".into(), TestDesc::True)],
            exit_time: None,
            duration: 0.0,
        }],
        initial: "Bend".into(),
        params: [("go".to_string(), ParamDesc::Bool(true))].into(),
    };
    let mut a = model
        .animator_with(0, Some(&AnimationDesc::Inline(graph.clone())), None)
        .unwrap();
    assert_eq!(a.layers[0].state.current, 1, "initial state by name");
    assert!(a.params.bool("go"));
    a.update(0.5);
    assert!(a.matrices[1] != glam::Mat4::IDENTITY, "the clip plays");

    // A graph file goes through the manager like any asset.
    std::fs::create_dir_all(dir.path().join("anim")).unwrap();
    graph
        .save(dir.path().join("anim/rig.animgraph.ron"))
        .unwrap();
    assert_eq!(
        AssetKind::of("anim/rig.animgraph.ron"),
        AssetKind::AnimGraph
    );
    let mut manager = AssetManager::new(dir.path());
    let h = manager.load::<AnimGraphDesc>("anim/rig.animgraph.ron");
    assert!(manager.wait_idle(Duration::from_secs(5)));
    let file = manager.get(&h).unwrap();
    let setup = AnimationDesc::Graph("anim/rig.animgraph.ron".into());
    assert!(model.animator_with(0, Some(&setup), None).is_err());
    let a = model.animator_with(0, Some(&setup), Some(&file)).unwrap();
    assert_eq!(a.layers[0].graph.states.len(), 2);

    let mut bad = graph;
    bad.transitions[0].to = "Missing".into();
    assert!(
        build_graph(&bad, &model.clips)
            .unwrap_err()
            .contains("Missing")
    );
}

#[test]
fn gltf_scale_applies_to_skeleton_and_clips() {
    let dir = TempDir::new("gltf_skin_scale");
    let path = write_skinned_gltf(&dir);
    let settings = ModelImport {
        scale: 2.0,
        ..Default::default()
    };
    let model = import_gltf(&path, dir.path(), &settings).unwrap();
    let sk = &model.skeletons[0];
    assert_eq!(sk.joints[1].rest.translation, Vec3::new(0.0, 2.0, 0.0));
    let mut pose = sk.rest_pose();
    model.clips[0].apply(1.0, sk, &mut pose);
    // Vertices are scaled too: the tip bind position (0, 2, 0) lands at (-2, 0, 0).
    let tip = sk.skin_matrices(&pose)[1].transform_point3(Vec3::new(0.0, 2.0, 0.0));
    assert!(tip.abs_diff_eq(Vec3::new(-2.0, 0.0, 0.0), 1e-4), "{tip:?}");
    // At rest, skinning changes nothing.
    for m in sk.skin_matrices(&sk.rest_pose()) {
        assert!(m.abs_diff_eq(glam::Mat4::IDENTITY, 1e-5));
    }
}

#[test]
fn gltf_skin_with_joint_outside_the_scene_is_skipped() {
    let dir = TempDir::new("gltf_skin_bad");
    let path = write_skinned_gltf(&dir);
    let text = std::fs::read_to_string(&path).unwrap();
    // Drop the joints from the scene roots: Root / Tip are no longer imported.
    let text = text.replace(r#""nodes": [0, 1, 3]"#, r#""nodes": [0, 3]"#);
    std::fs::write(&path, text).unwrap();
    let model = import_gltf(&path, dir.path(), &ModelImport::default()).unwrap();
    assert!(model.skeletons.is_empty());
    assert!(model.meshes[0].joints.is_empty() && model.meshes[0].skin.is_none());
    assert!(
        model.skipped.iter().any(|s| s.contains("skin ignored")),
        "{:?}",
        model.skipped
    );
}

fn solid_png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// One quad whose material has a rotated KHR_texture_transform and a
/// separate occlusion image (2×2) next to a metallic-roughness image (4×4).
fn write_material_gltf(dir: &TempDir) -> PathBuf {
    let mut b = GltfBuf::default();
    let pos = b.floats(&[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.]);
    let uv = b.floats(&[0., 0., 1., 0., 1., 1., 0., 1.]);
    let idx = b.add(
        &[0u16, 1, 2, 0, 2, 3]
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let img = |png: Vec<u8>| format!(r#"{{"uri": "data:image/png;base64,{}"}}"#, base64(&png));
    let json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "extensionsUsed": ["KHR_texture_transform"],
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"name": "Quad", "mesh": 0}}],
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0, "TEXCOORD_0": 1}}, "indices": 2, "material": 0}}]}}],
  "materials": [{{"name": "Pan", "pbrMetallicRoughness": {{
      "baseColorTexture": {{"index": 0, "extensions": {{"KHR_texture_transform": {{"rotation": 1.5707964, "scale": [2, 1], "offset": [0.5, 0]}}}}}},
      "metallicRoughnessTexture": {{"index": 2}}}},
    "occlusionTexture": {{"index": 1}}}}],
  "textures": [{{"source": 0}}, {{"source": 1}}, {{"source": 2}}],
  "images": [{}, {}, {}],
  "accessors": [
    {{"bufferView": {pos}, "componentType": 5126, "count": 4, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]}},
    {{"bufferView": {uv}, "componentType": 5126, "count": 4, "type": "VEC2"}},
    {{"bufferView": {idx}, "componentType": 5123, "count": 6, "type": "SCALAR"}}
  ],
  "bufferViews": {},
  "buffers": {}
}}"#,
        img(solid_png(2, 2, [255, 255, 255])),
        img(solid_png(2, 2, [200, 7, 7])),
        img(solid_png(4, 4, [9, 100, 50])),
        b.views_json(),
        b.buffer_json()
    );
    dir.write("models/pan.gltf", json.as_bytes())
}

#[test]
fn gltf_rotated_texture_transform_is_baked_and_occlusion_merged() {
    let dir = TempDir::new("gltf_mat");
    let path = write_material_gltf(&dir);
    let model = import_gltf(&path, dir.path(), &ModelImport::default()).unwrap();
    assert!(model.skipped.is_empty(), "{:?}", model.skipped);

    // uv' = offset + R(90°) * (scale * uv): (1, 0) -> (2, 0) -> (0, -2) -> + (0.5, 0).
    let uvs = &model.meshes[0].uvs;
    assert!(
        uvs[1].abs_diff_eq(glam::Vec2::new(0.5, -2.0), 1e-5),
        "{:?}",
        uvs[1]
    );
    assert!(uvs[0].abs_diff_eq(glam::Vec2::new(0.5, 0.0), 1e-5));
    // The material no longer scales or offsets on top of it.
    assert_eq!(model.materials[0].uv_scale, [1.0, 1.0]);
    assert_eq!(model.materials[0].uv_offset, [0.0, 0.0]);

    // R from the occlusion image, G and B from metallic-roughness, mr's size.
    let orm = model.materials[0].orm_texture.clone().unwrap();
    assert!(orm.ends_with("Pan_orm.png"), "{orm}");
    let img = image::open(dir.path().join(&orm)).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (4, 4));
    assert_eq!(img.get_pixel(1, 2).0, [200, 100, 50, 255]);
    assert!(dir.path().join(format!("{orm}.meta")).is_file());
    assert!(model.extracted_textures.contains(&orm));
}

/// An asset whose decoder panics, like a crate choking on a corrupt file.
struct Exploding;

impl Asset for Exploding {
    const KIND: AssetKind = AssetKind::Other;

    fn prepare(_: &AssetRoot, _: &str) -> Result<Job<Self>, String> {
        Ok(Box::new(|| panic!("corrupt")))
    }

    fn dependencies(&self) -> Vec<Dependency> {
        Vec::new()
    }
}

#[test]
fn a_panicking_loader_fails_the_asset_and_keeps_the_worker() {
    let dir = TempDir::new("mgr_panic");
    dir.write("a.png", &png_bytes(2, 2));
    let mut mgr = AssetManager::with_workers(dir.path(), 1);
    let bad = mgr.load::<Exploding>("boom");
    assert!(mgr.wait_idle(WAIT), "a panic must not leave it loading");
    match mgr.state(&bad) {
        LoadState::Failed(e) => assert!(e.contains("corrupt"), "{e}"),
        other => panic!("{other:?}"),
    }
    // The single worker survived.
    let ok = mgr.load::<ImageData>("a.png");
    assert!(mgr.wait_idle(WAIT));
    assert_eq!(mgr.state(&ok), LoadState::Ready);
}

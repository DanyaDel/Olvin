use glam::{Vec2, Vec3};
use olvin_render::Meshes;

use super::*;

const SIZE: Vec3 = Vec3::new(100.0, 20.0, 100.0);

/// A hill in the middle: height falls off with the distance from the centre.
fn hill(res: u32) -> TerrainData {
    let mut d = TerrainData::flat(res);
    let c = (res - 1) as f32 / 2.0;
    for z in 0..res {
        for x in 0..res {
            let r = Vec2::new(x as f32 - c, z as f32 - c).length() / c;
            d.set_height(x, z, (1.0 - r).max(0.0));
        }
    }
    d
}

#[test]
fn file_round_trips_and_rejects_garbage() {
    let mut d = hill(33);
    d.set_weights(3, 4, [0, 255, 0, 0]);
    let back = TerrainData::from_bytes(&d.to_bytes()).unwrap();
    assert_eq!(back.resolution(), 33);
    assert_eq!(back.weights(3, 4), [0, 255, 0, 0]);
    for (a, b) in d.heights().iter().zip(back.heights()) {
        assert!((a - b).abs() < 1.0 / 65535.0, "heights keep 16 bits");
    }
    assert!(TerrainData::from_bytes(b"nope").is_err());
    let mut cut = d.to_bytes();
    cut.truncate(cut.len() - 1);
    assert!(
        TerrainData::from_bytes(&cut).is_err(),
        "a short file is refused"
    );
    let mut huge = d.to_bytes();
    huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(
        TerrainData::from_bytes(&huge).is_err(),
        "no giant allocation from a header"
    );
}

#[test]
fn heights_clamp_and_sample_bilinearly() {
    let mut d = TerrainData::flat(3);
    d.set_height(0, 0, 2.0);
    d.set_height(1, 0, f32::NAN);
    assert_eq!((d.height(0, 0), d.height(1, 0)), (1.0, 0.0));
    // Halfway between 1 and 0.
    assert!((d.sample(0.25, 0.0) - 0.5).abs() < 1e-5);
    assert_eq!(
        d.height(99, 99),
        d.height(2, 2),
        "outside: the nearest edge"
    );
}

#[test]
fn chunks_have_levels_of_detail_and_skirts() {
    let d = hill(129);
    assert_eq!(mesh::chunks(&d).len(), 16, "128 quads: 4 × 4 chunks of 32");
    let full = mesh::chunk_mesh(&d, SIZE, (32, 32), 1);
    let coarse = mesh::chunk_mesh(&d, SIZE, (32, 32), 8);
    // 33² grid + 4 skirt edges of 33.
    assert_eq!(full.vertices.len(), 33 * 33 + 4 * 33);
    assert_eq!(coarse.vertices.len(), 5 * 5 + 4 * 5);
    assert!(
        full.indices
            .iter()
            .all(|&i| (i as usize) < full.vertices.len())
    );
    // The first skirt vertex hangs under the chunk's first corner.
    let (top, skirt) = (full.vertices[0].position, full.vertices[33 * 33].position);
    assert_eq!((top[0], top[2]), (skirt[0], skirt[2]));
    assert!(
        (top[1] - skirt[1] - 2.0).abs() < 1e-4,
        "10 % of the height (2 m) down"
    );
    // Corners are shared by every level, so chunks always meet.
    let corner = |m: &olvin_render::MeshData| m.vertices[0].position;
    assert_eq!(corner(&full), corner(&coarse));

    let mut meshes = Meshes::default();
    let built = mesh::add_chunks(&mut meshes, &d, SIZE);
    assert_eq!(built.len(), 16);
    assert!(built.iter().all(|c| c.lods.len() == 3));
}

#[test]
fn collision_and_ray_cast_follow_the_surface() {
    let d = hill(65);
    let geo = mesh::collision(&d, SIZE);
    assert_eq!(geo.positions.len(), 65 * 65);
    assert_eq!(geo.indices.len(), 64 * 64 * 2);
    let top = mesh::height_at(&d, SIZE, Vec2::ZERO).unwrap();
    assert!(
        (top - 20.0).abs() < 0.01,
        "the hill's top is the full height"
    );
    assert_eq!(mesh::height_at(&d, SIZE, Vec2::new(60.0, 0.0)), None);

    // Straight down onto the top, and slanted onto the slope.
    let hit = mesh::raycast(&d, SIZE, Vec3::new(0.0, 50.0, 0.0), Vec3::NEG_Y).unwrap();
    assert!((hit.y - top).abs() < 0.05, "{hit}");
    let hit = mesh::raycast(
        &d,
        SIZE,
        Vec3::new(-60.0, 30.0, 0.0),
        Vec3::new(1.0, -0.5, 0.0),
    )
    .unwrap();
    let ground = mesh::height_at(&d, SIZE, Vec2::new(hit.x, hit.z)).unwrap();
    assert!((hit.y - ground).abs() < 0.05, "{hit} vs ground {ground}");
    assert_eq!(
        mesh::raycast(&d, SIZE, Vec3::new(0.0, 50.0, 0.0), Vec3::Y),
        None
    );
}

#[test]
fn layer_shader_validates_and_atlas_wraps() {
    for n in 0..=4 {
        let layers: Vec<TerrainLayer> = (0..n).map(|_| TerrainLayer::default()).collect();
        let src = look::surface_shader(&layers, 129);
        if let Err(e) = olvin_render::validate_surface_shader(&src) {
            panic!("{n} layers: {e}\n{src}");
        }
    }
    // A 2×2 checker as layer 0: the tile repeats it, the gutter wraps it.
    let checker = olvin_render::TextureData {
        width: 2,
        height: 2,
        format: olvin_render::TextureFormat::Rgba8Srgb,
        pixels: [
            [255, 0, 0, 255],
            [0, 0, 255, 255],
            [0, 0, 255, 255],
            [255, 0, 0, 255],
        ]
        .concat(),
    };
    let layers = vec![
        TerrainLayer {
            texture: "checker.png".into(),
            ..Default::default()
        },
        TerrainLayer::default(),
    ];
    let atlas = look::atlas(&layers, |p| (p == "checker.png").then(|| checker.clone()));
    assert_eq!(
        (atlas.width, atlas.height),
        (look::TILE * 2, look::TILE * 2)
    );
    let px = |x: u32, y: u32| {
        let i = ((y * atlas.width + x) * 4) as usize;
        [atlas.pixels[i], atlas.pixels[i + 1], atlas.pixels[i + 2]]
    };
    // Layer 1 has no image: its fallback colour fills tile (1, 0).
    assert_eq!(px(look::TILE + 100, 100), [118, 112, 104]);
    // The gutter's first column equals the inner area's last column (wrap).
    let inner_last = look::TILE - look::GUTTER - 1;
    let gutter_first = look::GUTTER - 1;
    assert_eq!(px(gutter_first, 200), px(inner_last, 200));
}

#[test]
fn brushes_raise_smooth_flatten_and_paint() {
    use brush::{Brush, BrushOp, apply};
    let size = Vec3::new(64.0, 10.0, 64.0);
    let mut d = TerrainData::flat(65);
    let brush = Brush {
        center: Vec2::ZERO,
        radius: 8.0,
        strength: 1.0,
    };
    let rect = apply(&mut d, size, &brush, BrushOp::Raise(0.5), 1.0).unwrap();
    // 8 m on a 1 m grid around the centre vertex (32, 32).
    assert_eq!(rect, ((24, 24), (40, 40)));
    assert!(
        (d.height(32, 32) - 0.5).abs() < 1e-5,
        "full strength at the centre"
    );
    assert!(
        d.height(38, 32) > 0.0 && d.height(38, 32) < 0.5,
        "falls off"
    );
    assert_eq!(d.height(41, 32), 0.0, "nothing past the radius");
    // Lower takes it back down; heights never go below 0.
    apply(&mut d, size, &brush, BrushOp::Raise(-2.0), 1.0);
    assert_eq!(d.height(32, 32), 0.0);

    // A spike smooths out; the sum over the area is about kept.
    let mut spike = TerrainData::flat(65);
    spike.set_height(32, 32, 1.0);
    apply(&mut spike, size, &brush, BrushOp::Smooth, 0.1);
    assert!(spike.height(32, 32) < 1.0 && spike.height(33, 32) > 0.0);

    // Flatten pulls towards the target.
    let mut slope = TerrainData::flat(65);
    for x in 0..65 {
        for z in 0..65 {
            slope.set_height(x, z, x as f32 / 64.0);
        }
    }
    apply(&mut slope, size, &brush, BrushOp::Flatten(0.5), 1.0);
    assert!((slope.height(30, 32) - 0.5).abs() < 0.01);

    // Paint: layer 2 takes over at the centre, weights still sum to 255.
    let mut p = TerrainData::flat(65);
    apply(&mut p, size, &brush, BrushOp::Paint(2), 1.0);
    let w = p.weights(32, 32);
    assert!(w[2] > 200, "{w:?}");
    assert_eq!(w.iter().map(|&v| v as u32).sum::<u32>(), 255);
    assert_eq!(p.weights(0, 0), [255, 0, 0, 0], "far away untouched");

    // Outside the terrain: nothing.
    let away = Brush {
        center: Vec2::new(500.0, 0.0),
        ..brush
    };
    assert_eq!(apply(&mut p, size, &away, BrushOp::Paint(1), 1.0), None);
}

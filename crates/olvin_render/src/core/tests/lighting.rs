//! Lights, the sun and its cascaded shadows.

use super::*;

#[test]
fn renders_a_lit_cube_offscreen() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let cube = h.meshes.add(MeshData::cube(1.0));
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(cube, Color::rgb(1.0, 0.0, 0.0)),
    ));
    world.spawn((Transform::from_xyz(0.0, 0.0, 3.0), Camera::default()));
    let mut settings = RenderSettings {
        light_dir: Vec3::new(0.0, 0.0, -1.0),
        clear_color: Color::rgb(0.0, 0.0, 1.0),
        ..Default::default()
    };
    let (stats, at) = h.render(&world, &settings);
    assert_eq!((stats.draw_calls, stats.instances), (1, 1));
    assert_eq!(stats.triangles, 12, "one cube");
    let lit = at(SIZE / 2, SIZE / 2);
    // Red dominates; the dielectric highlight may tint green/blue a little.
    assert!(
        lit[0] > 200 && lit[1] < 120 && lit[2] < 120 && lit[0] > lit[1] + 100,
        "lit red cube in the middle: {lit:?}"
    );
    let corner = at(1, 1);
    assert!(
        corner[2] > 200 && corner[0] < 30,
        "clear colour in the corner: {corner:?}"
    );

    // Light from behind: only ambient on the visible face.
    settings.light_dir = Vec3::new(0.0, 0.0, 1.0);
    let (_, at) = h.render(&world, &settings);
    let unlit = at(SIZE / 2, SIZE / 2);
    assert!(
        unlit[0] > 20 && unlit[0] + 80 < lit[0],
        "unlit side is dimmer but not black: {unlit:?} vs {lit:?}"
    );
}

#[test]
fn red_point_light_brightens_the_floor_near_it() {
    for editor_like in [false, true] {
        let Some(mut h) = Harness::new(editor_like) else {
            return;
        };
        let plane = h.meshes.add(MeshData::plane(20.0));
        let mut world = World::new();
        world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
        world.spawn(top_down_camera(8.0));
        world.spawn((
            Transform::from_xyz(-2.0, 0.7, 0.0),
            Light::point([1.0, 0.0, 0.0], 4.0, 4.0),
        ));
        // A dark sun so only the point light paints the floor red.
        let settings = RenderSettings {
            light_color: Color::BLACK,
            ..Default::default()
        };
        let (_, at) = h.render(&world, &settings);
        let (y, near, far) = (
            SIZE / 2,
            at(column(-2.0, 8.0), SIZE / 2),
            at(column(3.0, 8.0), SIZE / 2),
        );
        assert!(
            near[0] > far[0] + 60 && near[0] > near[1] + 60,
            "editor_like={editor_like}: near {near:?} far {far:?} (row {y})"
        );
        assert!(luma(far) > 0, "ambient still lights the far floor");
    }
}

#[test]
fn directional_light_casts_a_shadow() {
    for editor_like in [false, true] {
        let Some(mut h) = Harness::new(editor_like) else {
            return;
        };
        let plane = h.meshes.add(MeshData::plane(20.0));
        let cube = h.meshes.add(MeshData::cube(1.0));
        let mut world = World::new();
        world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
        world.spawn((
            Transform::from_xyz(0.0, 1.5, 0.0),
            MeshRenderer::new(cube, Color::WHITE),
        ));
        world.spawn(top_down_camera(8.0));
        // Travels +X and down at 45°: the cube's shadow lands on x ∈ [0.5, 2.5].
        let sun_dir = Vec3::new(1.0, -1.0, 0.0).normalize();
        let sun = world.spawn((
            Transform::IDENTITY.looking_at(sun_dir, Vec3::Y),
            Light::directional([1.0; 3], 1.0),
        ));
        let settings = RenderSettings::default();
        let (shadowed, open) = ((column(1.5, 8.0), SIZE / 2), (column(-2.5, 8.0), SIZE / 2));

        let (_, at) = h.render(&world, &settings);
        assert!(h.view.shadows_active());
        let (s, o) = (at(shadowed.0, shadowed.1), at(open.0, open.1));
        assert!(
            luma(s) + 150 < luma(o),
            "editor_like={editor_like}: shadowed {s:?} vs open {o:?}"
        );

        // Shadows off: the same pixel is lit like the rest of the floor.
        world.get::<&mut Light>(sun).unwrap().shadows = false;
        let (_, at) = h.render(&world, &settings);
        assert!(!h.view.shadows_active());
        let (s, o) = (at(shadowed.0, shadowed.1), at(open.0, open.1));
        assert!(
            luma(s).abs_diff(luma(o)) < 30,
            "editor_like={editor_like}: no shadow {s:?} vs {o:?}"
        );
    }
}

/// The sun's shadow test scene: a cube casting a shadow onto x ∈ [0.5, 2.5];
/// returns (shadowed, open) pixel columns.
fn shadow_scene(h: &mut Harness) -> (World, (u32, u32), (u32, u32)) {
    let plane = h.meshes.add(MeshData::plane(20.0));
    let cube = h.meshes.add(MeshData::cube(1.0));
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
    world.spawn((
        Transform::from_xyz(0.0, 1.5, 0.0),
        MeshRenderer::new(cube, Color::WHITE),
    ));
    world.spawn(top_down_camera(8.0));
    let sun_dir = Vec3::new(1.0, -1.0, 0.0).normalize();
    world.spawn((
        Transform::IDENTITY.looking_at(sun_dir, Vec3::Y),
        Light::directional([1.0; 3], 1.0),
    ));
    let (shadowed, open) = ((column(1.5, 8.0), SIZE / 2), (column(-2.5, 8.0), SIZE / 2));
    (world, shadowed, open)
}

#[test]
fn cascades_and_filters_all_shadow_the_cube() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let (world, shadowed, open) = shadow_scene(&mut h);
    for tier in [RenderTier::Compat, RenderTier::Full] {
        h.core.set_tier(tier);
        for cascades in 1..=4 {
            for filter in [ShadowFilter::Hard, ShadowFilter::Pcf, ShadowFilter::Soft] {
                let settings = RenderSettings {
                    shadow_cascades: cascades,
                    shadow_filter: filter,
                    shadow_distance: 20.0,
                    ..Default::default()
                };
                let (_, at) = h.render(&world, &settings);
                let cap = if tier == RenderTier::Full { 4 } else { 2 };
                assert_eq!(h.view.shadow_cascades(), (cascades as usize).min(cap));
                let (s, o) = (at(shadowed.0, shadowed.1), at(open.0, open.1));
                assert!(
                    luma(s) + 150 < luma(o),
                    "{tier:?} {cascades} cascades {filter:?}: shadowed {s:?} vs open {o:?}"
                );
            }
        }
    }
}

#[test]
fn first_directional_light_replaces_the_settings_sun() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let plane = h.meshes.add(MeshData::plane(20.0));
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
    world.spawn(top_down_camera(8.0));
    let settings = RenderSettings {
        light_dir: Vec3::NEG_Y,
        light_color: Color::WHITE,
        ..Default::default()
    };
    let (_, at) = h.render(&world, &settings);
    let bright = at(SIZE / 2, SIZE / 2);
    // A green sun overrides the white settings sun.
    world.spawn((
        Transform::IDENTITY.looking_at(Vec3::NEG_Y, Vec3::Z),
        Light::directional([0.0, 1.0, 0.0], 1.0),
    ));
    let (_, at) = h.render(&world, &settings);
    let green = at(SIZE / 2, SIZE / 2);
    assert!(
        green[1] > green[0] + 60 && green[0] + 60 < bright[0],
        "{green:?} vs {bright:?}"
    );
    // Solid view: scene lights off, the settings sun again.
    let solid = RenderSettings {
        scene_lights: false,
        ..settings
    };
    let (_, at) = h.render(&world, &solid);
    assert_eq!(at(SIZE / 2, SIZE / 2), bright);
}

/// Ground, a unit cube standing on it and a top-down camera; the sun is off,
/// so only the lights the test adds shine (plus a little ambient).
fn local_scene(h: &mut Harness) -> World {
    let plane = h.meshes.add(MeshData::plane(20.0));
    let cube = h.meshes.add(MeshData::cube(1.0));
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
    world.spawn((
        Transform::from_xyz(0.0, 0.5, 0.0),
        MeshRenderer::new(cube, Color::WHITE),
    ));
    world.spawn(top_down_camera(8.0));
    world
}

fn local_settings() -> RenderSettings {
    RenderSettings {
        light_color: Color::BLACK,
        ..Default::default()
    }
}

/// Renders `world` at screenshot size into `$OLVIN_SHOTS_DIR` (when set).
fn shot(h: &mut Harness, name: &str, world: &World, settings: &RenderSettings) {
    if std::env::var_os("OLVIN_SHOTS_DIR").is_none() {
        return;
    }
    h.size = 512;
    let (_, at) = h.render(world, settings);
    save_shot(name, 512, at);
    h.size = SIZE;
}

fn spot_light(x: f32, shadows: bool) -> (Transform, Light) {
    // Aims at the cube's middle from the side and above.
    let pos = Vec3::new(x, 4.0, 0.0);
    let mut light = Light::spot([1.0, 1.0, 1.0], 40.0, 12.0, 60.0);
    light.shadows = shadows;
    (
        Transform::from_translation(pos).looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
        light,
    )
}

#[test]
fn spot_light_casts_a_cube_shadow() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    for tier in [RenderTier::Compat, RenderTier::Full] {
        h.core.set_tier(tier);
        let mut world = local_scene(&mut h);
        let light = world.spawn(spot_light(-3.0, true));
        let settings = local_settings();
        // Shadow of the cube lands on x in [0.5, 1.7]; x = -1.5 is lit, x = 7
        // is beyond the cone (and the 12 m range of the shadow map).
        let row = SIZE / 2;
        let (shadowed, lit, outside) = (column(1.1, 8.0), column(-1.5, 8.0), column(4.2, 8.0));
        let (_, at) = h.render(&world, &settings);
        assert_eq!(h.view.local_shadow_tiles(), 1);
        let (s, l, o) = (at(shadowed, row), at(lit, row), at(outside, row));
        assert!(
            luma(s) + 60 < luma(l),
            "{tier:?}: shadowed {s:?} vs lit {l:?}"
        );
        // The shadow keeps the ambient light: darker than lit, not black.
        assert!(luma(s) > 0, "{tier:?}: ambient remains {s:?}");
        shot(&mut h, &format!("spot_{tier:?}"), &world, &settings);

        // Shadows off: the same pixel is lit like the open ground.
        world.get::<&mut Light>(light).unwrap().shadows = false;
        let (_, at) = h.render(&world, &settings);
        assert_eq!(h.view.local_shadow_tiles(), 0);
        let (s2, l2, o2) = (at(shadowed, row), at(lit, row), at(outside, row));
        assert!(
            luma(s2) > luma(s) + 60,
            "{tier:?}: no shadow {s2:?} vs {s:?}"
        );
        // Pixels the shadow doesn't touch don't change.
        assert!(luma(l).abs_diff(luma(l2)) < 12, "{l:?} vs {l2:?}");
        assert!(luma(o).abs_diff(luma(o2)) < 12, "{o:?} vs {o2:?}");
    }
}

fn point_light(x: f32, y: f32, z: f32, shadows: bool) -> (Transform, Light) {
    let mut light = Light::point([1.0; 3], 30.0, 12.0);
    light.shadows = shadows;
    (Transform::from_xyz(x, y, z), light)
}

#[test]
fn point_light_shadow_lands_on_the_far_side() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let settings = local_settings();
    let row = SIZE / 2;
    for tier in [RenderTier::Compat, RenderTier::Full] {
        h.core.set_tier(tier);
        let mut world = local_scene(&mut h);
        let light = world.spawn(point_light(-3.0, 2.5, 0.0, true));
        // Light at -X: the cube's shadow covers x in [0.5, 2.8], x = -1.5 is lit.
        let (behind, front) = (column(1.6, 8.0), column(-1.5, 8.0));
        let (_, at) = h.render(&world, &settings);
        assert_eq!(
            h.view.local_shadow_tiles(),
            6,
            "one point light = six faces"
        );
        let (b, f) = (at(behind, row), at(front, row));
        assert!(
            luma(b) + 60 < luma(f),
            "{tier:?}: behind {b:?} vs front {f:?}"
        );
        shot(&mut h, &format!("point_x_{tier:?}"), &world, &settings);

        // Moved to +X the shadow flips sides (the other cube faces' tiles).
        world.get::<&mut Transform>(light).unwrap().translation.x = 3.0;
        let (_, at) = h.render(&world, &settings);
        let (b, f) = (at(column(-1.6, 8.0), row), at(column(1.5, 8.0), row));
        assert!(luma(b) + 60 < luma(f), "{tier:?}: flipped {b:?} vs {f:?}");

        // From -Z, the shadow lands on +Z (rows below the cube's centre).
        {
            let mut t = world.get::<&mut Transform>(light).unwrap();
            t.translation = Vec3::new(0.0, 2.5, -3.0);
        }
        let (_, at) = h.render(&world, &settings);
        let (b, f) = (
            at(SIZE / 2, column(1.6, 8.0)),
            at(SIZE / 2, column(-1.5, 8.0)),
        );
        assert!(luma(b) + 60 < luma(f), "{tier:?}: along z {b:?} vs {f:?}");
        let shadowed = luma(b);

        // Off: no shadow, no tiles.
        world.get::<&mut Light>(light).unwrap().shadows = false;
        let (_, at) = h.render(&world, &settings);
        assert_eq!(h.view.local_shadow_tiles(), 0);
        let b = at(SIZE / 2, column(1.6, 8.0));
        assert!(luma(b) > shadowed + 100, "{tier:?}: unshadowed {b:?}");
    }
}

#[test]
fn shadow_casters_beyond_the_caps_are_dropped_quietly() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let mut world = local_scene(&mut h);
    for i in 0..10 {
        let a = i as f32 * 0.6;
        world.spawn(spot_light(-3.0 + a.cos(), true));
        world.spawn(point_light(a.sin() * 3.0, 2.5, a.cos() * 3.0, true));
    }
    for (tier, spots, points, tiles) in [
        (RenderTier::Full, 4, 2, 16),
        (RenderTier::Full, 1, 1, 7),
        (RenderTier::Full, 0, 0, 0),
        // Compat serves at most 2 spot and 1 point lights.
        (RenderTier::Compat, 4, 2, 8),
        // Huge caps still fit the 16-tile atlas.
        (RenderTier::Full, 100, 100, 16),
    ] {
        h.core.set_tier(tier);
        let settings = RenderSettings {
            max_spot_shadows: spots,
            max_point_shadows: points,
            ..local_settings()
        };
        let (stats, _) = h.render(&world, &settings);
        assert!(stats.instances > 0);
        assert_eq!(
            h.view.local_shadow_tiles(),
            tiles,
            "{tier:?} {spots}/{points}"
        );
    }
}

#[test]
fn point_shadow_has_no_light_leaks_across_cube_face_seams() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let plane = h.meshes.add(MeshData::plane(30.0));
    let cube = h.meshes.add(MeshData::cube(1.0));
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
    // A wide plate hovering over the ground, the light above its middle: the
    // ground below is wholly shadowed, across every seam between faces.
    world.spawn((
        Transform::from_xyz(0.0, 1.5, 0.0).with_scale(Vec3::new(12.0, 0.1, 12.0)),
        MeshRenderer::new(cube, Color::WHITE),
    ));
    world.spawn((
        Transform::from_xyz(0.0, 1.2, 5.0).looking_at(Vec3::new(0.0, 0.3, -5.0), Vec3::Y),
        Camera::default(),
    ));
    world.spawn(point_light(0.0, 3.0, 0.0, true));
    let settings = local_settings();
    // The bottom half of the view is ground under the plate.
    let spread = |at: &dyn Fn(u32, u32) -> [u8; 3]| {
        let lumas = (SIZE * 5 / 8..SIZE).flat_map(|y| (0..SIZE).map(move |x| (x, y)));
        let lumas: Vec<u32> = lumas.map(|(x, y)| luma(at(x, y))).collect();
        (*lumas.iter().min().unwrap(), *lumas.iter().max().unwrap())
    };
    let (_, at) = h.render(&world, &settings);
    assert_eq!(h.view.local_shadow_tiles(), 6);
    let (dark, darkest_max) = spread(&at);
    assert!(
        darkest_max < dark + 30,
        "an even shadow, no streaks at the seams: {dark}..{darkest_max}"
    );
    shot(&mut h, "point_seams", &world, &settings);

    let mut query = world.query::<&mut Light>();
    for light in query.iter() {
        light.shadows = false;
    }
    drop(query);
    let (_, at) = h.render(&world, &settings);
    let (_, lit_max) = spread(&at);
    assert!(
        lit_max > darkest_max + 150,
        "unshadowed ground is lit: {lit_max}"
    );
}

/// Screenshots only (`OLVIN_SHOTS_DIR`): an oblique view of tilted cubes lit
/// by a spot and a point light, to eyeball acne and peter-panning.
#[test]
fn local_shadow_overview_shot() {
    if std::env::var_os("OLVIN_SHOTS_DIR").is_none() {
        return;
    }
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    let plane = h.meshes.add(MeshData::plane(30.0));
    let cube = h.meshes.add(MeshData::cube(1.0));
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
    for (x, z, rot, s) in [
        (-1.5, 0.0, 0.5, 1.0),
        (1.0, -0.5, 1.0, 1.6),
        (0.0, 2.0, 0.2, 0.7),
    ] {
        world.spawn((
            Transform::from_xyz(x, s * 0.5, z)
                .with_rotation(glam::Quat::from_rotation_y(rot))
                .with_scale(Vec3::splat(s)),
            MeshRenderer::new(cube, Color::WHITE),
        ));
    }
    world.spawn((
        Transform::from_xyz(0.0, 4.5, 7.0).looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
        Camera::default(),
    ));
    world.spawn(spot_light(-4.0, true));
    world.spawn(point_light(3.5, 2.5, 2.5, true));
    let settings = local_settings();
    for filter in [ShadowFilter::Hard, ShadowFilter::Pcf, ShadowFilter::Soft] {
        let settings = RenderSettings {
            shadow_filter: filter,
            ..settings.clone()
        };
        shot(
            &mut h,
            &format!("local_overview_{filter:?}"),
            &world,
            &settings,
        );
    }
}

/// Ground and a cube hovering `height` above it under a 45° sun; the cube's
/// shadow ends at x = height + 1. The camera looks straight down.
fn hovering_cube_scene(h: &mut Harness, height: f32, size: f32) -> World {
    let plane = h.meshes.add(MeshData::plane(40.0));
    let cube = h.meshes.add(MeshData::cube(1.0));
    let mut world = World::new();
    world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
    world.spawn((
        Transform::from_xyz(0.0, height, 0.0),
        MeshRenderer::new(cube, Color::WHITE),
    ));
    world.spawn(top_down_camera(8.0));
    let mut sun = Light::directional([1.0; 3], 1.0);
    sun.size = size;
    world.spawn((
        Transform::IDENTITY.looking_at(Vec3::new(1.0, -1.0, 0.0).normalize(), Vec3::Y),
        sun,
    ));
    world
}

/// Width in pixels of the dark-to-lit transition around world x = `edge`
/// (between 10% and 90% of the step), along the middle row.
fn edge_width(at: impl Fn(u32, u32) -> [u8; 3], size: u32, edge: f32) -> u32 {
    let half = 8.0 * 30f32.to_radians().tan();
    let col = |x: f32| (((0.5 + x / half * 0.5) * size as f32) as u32).min(size - 1);
    let row = size / 2;
    let span: Vec<u32> = (col(edge - 1.8)..col(edge + 1.8))
        .map(|x| luma(at(x, row)))
        .collect();
    let (lo, hi) = (*span.iter().min().unwrap(), *span.iter().max().unwrap());
    let range = (hi - lo) as f32;
    assert!(range > 150.0, "there is an edge: {lo}..{hi}");
    span.iter()
        .filter(|&&v| v as f32 > lo as f32 + 0.1 * range && (v as f32) < hi as f32 - 0.1 * range)
        .count() as u32
}

#[test]
fn pcss_penumbra_widens_with_the_occluder_distance() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    if !h.core.pcss_shader {
        eprintln!("no raw depth reads on this backend (Compat): skipping PCSS");
        return;
    }
    h.core.set_tier(RenderTier::Full);
    h.size = 256;
    let width_at = |h: &mut Harness, filter, height: f32| {
        let world = hovering_cube_scene(h, height, 10.0);
        let settings = RenderSettings {
            shadow_filter: filter,
            shadow_distance: 30.0,
            shadow_cascades: 1,
            shadow_map_size: 4096,
            ..Default::default()
        };
        let (_, at) = h.render(&world, &settings);
        if filter == ShadowFilter::Pcss {
            save_shot(&format!("pcss_h{height}"), 256, &at);
        }
        edge_width(at, 256, height + 1.0)
    };
    let (near, far) = (
        width_at(&mut h, ShadowFilter::Pcss, 0.7),
        width_at(&mut h, ShadowFilter::Pcss, 3.0),
    );
    assert!(
        far > near * 2,
        "penumbra grows with distance: {near} px near, {far} px far"
    );
    // PCF is the same everywhere.
    let (pcf_near, pcf_far) = (
        width_at(&mut h, ShadowFilter::Pcf, 0.7),
        width_at(&mut h, ShadowFilter::Pcf, 3.0),
    );
    assert!(
        pcf_far < pcf_near * 2 + 2,
        "PCF: {pcf_near} px vs {pcf_far} px"
    );
    assert!(far > pcf_far * 2, "PCSS far {far} px vs PCF {pcf_far} px");

    // A smaller light is sharper.
    let world = hovering_cube_scene(&mut h, 3.0, 1.0);
    let settings = RenderSettings {
        shadow_filter: ShadowFilter::Pcss,
        shadow_distance: 30.0,
        shadow_cascades: 1,
        shadow_map_size: 4096,
        ..Default::default()
    };
    let (_, at) = h.render(&world, &settings);
    let small = edge_width(at, 256, 4.0);
    assert!(
        small * 2 < far,
        "1° light: {small} px vs 10° light: {far} px"
    );
}

#[test]
fn pcss_falls_back_to_pcf_on_compat() {
    let Some(mut h) = Harness::new(false) else {
        return;
    };
    h.core.set_tier(RenderTier::Compat);
    let (world, shadowed, open) = shadow_scene(&mut h);
    let settings = RenderSettings {
        shadow_filter: ShadowFilter::Pcss,
        ..Default::default()
    };
    let (_, at) = h.render(&world, &settings);
    let (s, o) = (at(shadowed.0, shadowed.1), at(open.0, open.1));
    assert!(
        luma(s) + 150 < luma(o),
        "Compat PCSS: shadowed {s:?} vs open {o:?}"
    );
}

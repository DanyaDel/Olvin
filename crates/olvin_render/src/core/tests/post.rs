//! Post-processing.

use super::*;

/// A frame through the HDR target and the post chain.
struct Frame {
    pixels: Vec<u8>,
}

impl Frame {
    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * SIZE + x) * 4) as usize;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]]
    }

    /// Pixels that are neither black nor white (soft edges).
    fn grey_pixels(&self) -> usize {
        (0..SIZE * SIZE)
            .filter(|i| {
                let l = self.at(i % SIZE, i / SIZE)[0];
                (40..215).contains(&l)
            })
            .count()
    }
}

/// Draws the world `build` makes (the camera included) the way the window
/// renderer does on the Full tier, and reads the output back. `None` without
/// a GPU.
fn post_world(
    settings: &RenderSettings,
    build: impl FnOnce(&mut Meshes, &mut Materials, &mut World),
) -> Option<Frame> {
    post_world_lut(settings, None, build)
}

/// [`post_world`] with a colour grading LUT.
fn post_world_lut(
    settings: &RenderSettings,
    lut: Option<TextureData>,
    build: impl FnOnce(&mut Meshes, &mut Materials, &mut World),
) -> Option<Frame> {
    let (device, queue) = device()?;
    let mut core = RenderCore::new(
        device.clone(),
        queue,
        crate::HDR_FORMAT,
        1,
        wgpu::TextureFormat::Depth24Plus,
    );
    core.set_hdr_output(true);
    let mut post = crate::PostProcess::new(&device, FORMAT);
    post.resize(&device, (SIZE, SIZE));
    let (mut gpu, mut meshes, mut materials) = (
        GpuMeshes::default(),
        Meshes::default(),
        Materials::default(),
    );
    let (mut textures, mut shaders) = (Textures::default(), Shaders::default());
    let mut settings = settings.clone();
    if let Some(lut) = lut {
        settings.post.grade.lut = Some(textures.add(lut));
    }
    let settings = &settings;
    let mut world = World::new();
    build(&mut meshes, &mut materials, &mut world);
    core.upload(&mut gpu, &mut meshes);
    core.upload_assets(&mut gpu, &mut textures, &mut materials, &mut shaders);
    post.sync_lut(&device, &gpu, settings);
    let mut view = core.create_view();
    let view_proj = camera_view_proj(&world, 1.0).unwrap();
    core.prepare(&mut view, &gpu, &world, view_proj, settings);
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let out = texture(
        FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (SIZE * SIZE * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let hdr = post.hdr_view().unwrap();
        let depth = post.depth_view().unwrap();
        let mut pass = begin_scene_pass(
            &mut encoder,
            hdr,
            depth,
            settings,
            settings.post.ao.is_some(),
        );
        core.draw(&view, &gpu, &mut pass);
    }
    post.run(
        &core.queue,
        &mut encoder,
        &out.create_view(&Default::default()),
        settings,
        view_proj,
    );
    encoder.copy_texture_to_buffer(
        out.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    core.queue.submit([encoder.finish()]);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let pixels = readback.slice(..).get_mapped_range().unwrap().to_vec();
    Some(Frame { pixels })
}

/// An emissive 8.0 square in the middle of a black frame, through the
/// HDR target and the post chain; returns the output's RGB at (x, y).
fn post_frame(settings: &RenderSettings) -> Option<impl Fn(u32, u32) -> [u8; 3] + use<>> {
    let frame = post_world(settings, |meshes, materials, world| {
        let quad = meshes.add(MeshData::plane(1.0));
        let glow = materials.add(Material {
            base_color: Color::BLACK,
            emissive: [8.0; 3],
            unlit: true,
            ..Default::default()
        });
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(quad, Color::WHITE).with_material(glow),
        ));
        world.spawn(top_down_camera(8.0));
    })?;
    Some(move |x: u32, y: u32| frame.at(x, y))
}

/// A white unlit square turned 25 degrees on black: a hard, aliased edge.
fn tilted_square(meshes: &mut Meshes, materials: &mut Materials, world: &mut World) {
    let quad = meshes.add(MeshData::plane(1.0));
    let white = materials.add(Material {
        base_color: Color::BLACK,
        emissive: [1.0; 3],
        unlit: true,
        ..Default::default()
    });
    world.spawn((
        Transform::IDENTITY.with_rotation(glam::Quat::from_rotation_y(25f32.to_radians())),
        MeshRenderer::new(quad, Color::WHITE).with_material(white),
    ));
    world.spawn(top_down_camera(3.0));
}

#[test]
fn post_chain_tonemaps_the_hdr_frame_and_blooms() {
    let settings = RenderSettings {
        clear_color: Color::BLACK,
        tonemapping: Tonemapping::None,
        ..Default::default()
    };
    let Some(plain) = post_frame(&settings) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    // 8.0 is brighter than white (clamped); away from the square it's black.
    assert_eq!(plain(SIZE / 2, SIZE / 2), [255, 255, 255]);
    let near = (SIZE / 2 + 10, SIZE / 2);
    assert!(
        luma(plain(near.0, near.1)) < 6,
        "{:?}",
        plain(near.0, near.1)
    );

    // ACES compresses the highlight below pure white only for dimmer
    // values: exposure 0.05 puts the square at 0.4.
    let dim = RenderSettings {
        exposure: 0.05,
        tonemapping: Tonemapping::Aces,
        ..settings.clone()
    };
    let Some(dim_frame) = post_frame(&dim) else {
        return;
    };
    let c = dim_frame(SIZE / 2, SIZE / 2);
    assert!(c[0] > 150 && c[0] < 250 && c[0] == c[1], "{c:?}");

    // Bloom lights up the pixels around the square.
    let bloom = RenderSettings {
        bloom: Some(crate::Bloom {
            threshold: 1.0,
            intensity: 2.0,
        }),
        ..settings
    };
    let Some(bloomed) = post_frame(&bloom) else {
        return;
    };
    let glow = bloomed(near.0, near.1);
    assert!(luma(glow) > 60, "{glow:?}");
    // Far from it, still dark.
    assert!(luma(bloomed(2, 2)) < luma(glow), "{:?}", bloomed(2, 2));
}

#[test]
fn fxaa_smooths_a_hard_edge_and_leaves_flat_areas() {
    let off = RenderSettings {
        clear_color: Color::BLACK,
        tonemapping: Tonemapping::None,
        post: crate::PostSettings {
            fxaa: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let on = RenderSettings {
        post: crate::PostSettings {
            fxaa: true,
            ..Default::default()
        },
        ..off.clone()
    };
    let (Some(hard), Some(soft)) = (
        post_world(&off, tilted_square),
        post_world(&on, tilted_square),
    ) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    // Without FXAA the edge is a staircase of black and white.
    assert!(hard.grey_pixels() < 4, "{}", hard.grey_pixels());
    // With it, the staircase is blended into greys, which a flat image has none of.
    assert!(
        soft.grey_pixels() > hard.grey_pixels() + 10,
        "{} vs {}",
        soft.grey_pixels(),
        hard.grey_pixels()
    );
    // Flat areas stay exactly as they were.
    for (x, y) in [
        (2, 2),
        (SIZE - 3, 5),
        (SIZE / 2, SIZE / 2),
        (SIZE / 2 + 3, SIZE / 2 - 2),
    ] {
        assert_eq!(soft.at(x, y), hard.at(x, y), "({x}, {y})");
    }
}

/// A grey floor with a cube on it, seen from an angle, lit by ambient only.
fn cube_on_floor(meshes: &mut Meshes, materials: &mut Materials, world: &mut World) {
    let floor = meshes.add(MeshData::plane(40.0));
    let cube = meshes.add(MeshData::cube(1.0));
    let grey = materials.add(Material {
        base_color: Color::rgb(0.6, 0.6, 0.6),
        roughness: 1.0,
        ..Default::default()
    });
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(floor, Color::WHITE).with_material(grey),
    ));
    let red = materials.add(Material {
        base_color: Color::rgb(0.7, 0.25, 0.2),
        roughness: 1.0,
        ..Default::default()
    });
    world.spawn((
        Transform::from_xyz(0.0, 0.5, 0.0),
        MeshRenderer::new(cube, Color::WHITE).with_material(red),
    ));
    world.spawn((
        Transform::from_xyz(0.0, 4.0, 5.0).looking_at(Vec3::new(0.0, 0.4, 0.0), Vec3::Y),
        olvin_scene::Camera::default(),
    ));
}

fn ambient_only() -> RenderSettings {
    RenderSettings {
        clear_color: Color::BLACK,
        tonemapping: Tonemapping::None,
        light_color: Color::BLACK,
        ambient: Color::rgb(1.0, 1.0, 1.0),
        ground_ambient: Color::rgb(1.0, 1.0, 1.0),
        post: crate::PostSettings {
            fxaa: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn ao_darkens_the_crease_and_leaves_open_ground() {
    let off = ambient_only();
    let on = RenderSettings {
        post: crate::PostSettings {
            ao: Some(crate::AmbientOcclusion::default()),
            ..off.post.clone()
        },
        ..off.clone()
    };
    let (Some(flat), Some(occluded)) = (
        post_world(&off, cube_on_floor),
        post_world(&on, cube_on_floor),
    ) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    save_shot("ao_off", &flat);
    save_shot("ao_on", &occluded);
    // Somewhere down the middle column (the cube's foot) AO darkens a lot.
    let drop = (0..SIZE)
        .map(|y| flat.at(SIZE / 2, y)[0].saturating_sub(occluded.at(SIZE / 2, y)[0]))
        .max()
        .unwrap();
    assert!(drop > 20, "{drop}");
    // Open ground, far from the cube, is the same.
    for (x, y) in [
        (3, SIZE - 3),
        (SIZE - 4, SIZE - 4),
        (SIZE - 4, SIZE / 2 + 8),
    ] {
        let (a, b) = (flat.at(x, y)[0], occluded.at(x, y)[0]);
        assert!(a.abs_diff(b) <= 6, "({x}, {y}): {a} vs {b}");
    }
}

/// With `OLVIN_POST_SHOTS=<dir>`: a PPM of the frame there.
fn save_shot(name: &str, frame: &Frame) {
    let Ok(dir) = std::env::var("OLVIN_POST_SHOTS") else {
        return;
    };
    let mut out = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
    for i in 0..SIZE * SIZE {
        out.extend(frame.at(i % SIZE, i / SIZE));
    }
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(format!("{dir}/{name}.ppm"), out);
}

/// The whole frame is one unlit colour (linear 0.2, 0.5, 0.8 before grading).
fn flat_colour(meshes: &mut Meshes, materials: &mut Materials, world: &mut World) {
    let quad = meshes.add(MeshData::plane(40.0));
    let m = materials.add(Material {
        base_color: Color::BLACK,
        emissive: [0.2, 0.5, 0.8],
        unlit: true,
        ..Default::default()
    });
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(quad, Color::WHITE).with_material(m),
    ));
    world.spawn(top_down_camera(8.0));
}

fn graded(grade: crate::ColorGrade) -> RenderSettings {
    RenderSettings {
        clear_color: Color::BLACK,
        tonemapping: Tonemapping::None,
        post: crate::PostSettings {
            fxaa: false,
            grade,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn identity_lut_changes_nothing_and_a_tint_lut_shifts_colours() {
    let plain = graded(Default::default());
    let with_lut = |data| {
        let settings = graded(crate::ColorGrade {
            // Replaced by the handle of `data`.
            lut: Some(TextureHandle(0)),
            ..Default::default()
        });
        post_world_lut(&settings, Some(data), flat_colour)
    };
    let (Some(base), Some(same), Some(tinted)) = (
        post_world(&plain, flat_colour),
        with_lut(crate::identity_lut(32)),
        with_lut(crate::lut_strip(16, |[r, g, b]| {
            [(r * 0.5 + 0.5).min(1.0), g * 0.5, b * 0.5]
        })),
    ) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let c = base.at(SIZE / 2, SIZE / 2);
    // 0.2, 0.5, 0.8 linear, sRGB-encoded.
    assert!(close(c, [124, 188, 231]), "{c:?}");
    for (x, y) in [(SIZE / 2, SIZE / 2), (3, 3), (SIZE - 4, 9)] {
        let (a, b) = (base.at(x, y), same.at(x, y));
        assert!(
            a.iter().zip(b).all(|(&a, b)| a.abs_diff(b) <= 1),
            "identity LUT: {a:?} -> {b:?}"
        );
    }
    let t = tinted.at(SIZE / 2, SIZE / 2);
    assert!(
        t[0] > c[0] + 40 && t[1] + 40 < c[1] && t[2] + 40 < c[2],
        "{c:?} -> {t:?}"
    );
}

#[test]
fn simple_grading_works_without_a_lut() {
    let grey = graded(crate::ColorGrade {
        saturation: 0.0,
        ..Default::default()
    });
    let bright = graded(crate::ColorGrade {
        gain: [1.0, 1.0, 0.5],
        lift: [0.1, 0.0, 0.0],
        ..Default::default()
    });
    let (Some(base), Some(grey), Some(bright)) = (
        post_world(&graded(Default::default()), flat_colour),
        post_world(&grey, flat_colour),
        post_world(&bright, flat_colour),
    ) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let g = grey.at(SIZE / 2, SIZE / 2);
    assert!(
        g[0].abs_diff(g[1]) <= 1 && g[1].abs_diff(g[2]) <= 1,
        "{g:?}"
    );
    let (c, b) = (base.at(SIZE / 2, SIZE / 2), bright.at(SIZE / 2, SIZE / 2));
    assert!(b[0] > c[0] + 10, "lift: {c:?} -> {b:?}");
    assert!(b[2] + 40 < c[2], "gain: {c:?} -> {b:?}");
}

//! Skinned meshes: the Full tier poses them with a compute pass, Compat on
//! the CPU; both must agree, follow the pose in the shadow passes and not be
//! culled by their bind-pose bounds.

use glam::{Mat4, Quat};

use super::*;
use crate::{SkinnedMesh, Vertex};

/// A capsule along Y skinned to two joints: the lower half to joint 0, the
/// upper to joint 1, blended over |y| < 0.3.
fn capsule() -> (MeshData, SkinnedMesh) {
    let bind = MeshData::primitive(olvin_scene::primitives::Shape::Capsule);
    let (joints, weights): (Vec<_>, Vec<_>) = bind
        .vertices
        .iter()
        .map(|v| {
            let t = ((v.position[1] + 0.3) / 0.6).clamp(0.0, 1.0);
            ([0u16, 1, 0, 0], [1.0 - t, t, 0.0, 0.0])
        })
        .unzip();
    let skinned =
        SkinnedMesh::new(bind.clone(), joints, weights, olvin_ecs::Entity::DANGLING).unwrap();
    (bind, skinned)
}

/// Joint 1 bent 50° about Z (around the origin) and lifted.
fn bent() -> [Mat4; 2] {
    [
        Mat4::IDENTITY,
        Mat4::from_rotation_translation(
            Quat::from_rotation_z(50f32.to_radians()),
            Vec3::new(0.2, 0.3, 0.1),
        ),
    ]
}

fn harness(tier: RenderTier) -> Option<Harness> {
    let mut h = Harness::new(false)?;
    h.core.set_tier(tier);
    Some(h)
}

fn read_vertices(h: &Harness, slot: u32, count: usize) -> Vec<Vertex> {
    let buffer = &h.gpu.meshes[slot as usize].as_ref().unwrap().vertices;
    let size = (count * size_of::<Vertex>()) as u64;
    let readback = h.core.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = h.core.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &readback, 0, size);
    h.core.queue.submit([encoder.finish()]);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    h.core
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let data = readback.slice(..).get_mapped_range().unwrap();
    bytemuck::cast_slice::<u8, Vertex>(&data).to_vec()
}

fn lit_world(mesh: crate::MeshHandle) -> World {
    let mut world = World::new();
    world.spawn((
        Transform::IDENTITY,
        MeshRenderer::new(mesh, Color::rgb(0.9, 0.5, 0.3)),
    ));
    world.spawn((
        Transform::from_xyz(2.5, 1.5, 4.5).looking_at(Vec3::ZERO, Vec3::Y),
        Camera::default(),
    ));
    world.spawn((
        Transform::IDENTITY.looking_at(Vec3::new(-0.4, -1.0, -0.6), Vec3::Y),
        Light::directional([1.0; 3], 2.0),
    ));
    world
}

fn frame(h: &mut Harness, world: &World) -> Vec<[u8; 3]> {
    let settings = RenderSettings {
        shadow_distance: 0.0,
        ..Default::default()
    };
    let (_, at) = h.render(world, &settings);
    (0..SIZE * SIZE).map(|i| at(i % SIZE, i / SIZE)).collect()
}

#[test]
fn gpu_skinning_matches_the_cpu_reference() {
    let Some(mut gpu) = harness(RenderTier::Full) else {
        return;
    };
    if !gpu.core.gpu_skinning() {
        eprintln!("no compute / storage buffers on this device: skipping");
        return;
    }
    let Some(mut cpu) = harness(RenderTier::Compat) else {
        return;
    };
    assert!(!cpu.core.gpu_skinning());
    let (bind, skinned) = capsule();
    let m = bent();
    let mut shots = Vec::new();
    for h in [&mut gpu, &mut cpu] {
        let mesh = h.meshes.add(bind.clone());
        h.meshes.skin(mesh, &skinned, &m);
        let world = lit_world(mesh);
        shots.push((frame(h, &world), mesh));
    }
    // Same pose on the same pipeline: pixels agree.
    let (a, b) = (&shots[0].0, &shots[1].0);
    let worst = a
        .iter()
        .zip(b)
        .map(|(p, q)| p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap())
        .max()
        .unwrap();
    assert!(worst <= 3, "GPU vs CPU skinned render differ by {worst}");
    // And it is not the bind pose (the bend moved the top by half a metre).
    let rest = {
        let mesh = gpu.meshes.add(bind.clone());
        let id = [Mat4::IDENTITY; 2];
        gpu.meshes.skin(mesh, &skinned, &id);
        frame(&mut gpu, &lit_world(mesh))
    };
    let moved = rest.iter().zip(a).filter(|(p, q)| p != q).count();
    assert!(moved > 40, "the pose changes the picture ({moved} pixels)");

    // Vertices straight from the GPU buffer: positions, normals, tangents.
    assert_eq!(gpu.gpu.skins.len(), 2);
    let want = skinned.skin(&m);
    let got = read_vertices(&gpu, shots[0].1.0, want.vertices.len());
    for (i, (g, w)) in got.iter().zip(&want.vertices).enumerate() {
        for (a, b) in g
            .position
            .iter()
            .chain(&g.normal)
            .chain(&g.tangent)
            .zip(w.position.iter().chain(&w.normal).chain(&w.tangent))
        {
            assert!((a - b).abs() < 1e-4, "vertex {i}: {g:?} vs {w:?}");
        }
        assert_eq!(g.uv, w.uv);
    }
}

#[test]
fn pose_updates_reuse_the_gpu_skin_and_bad_joints_fall_back() {
    let Some(mut h) = harness(RenderTier::Full) else {
        return;
    };
    if !h.core.gpu_skinning() {
        return;
    }
    // Influences of joints beyond the palette are dropped, like on the CPU.
    let (bind, _) = capsule();
    let n = bind.vertices.len();
    let mut joints = vec![[0u16, 1, 0, 0]; n];
    let mut weights = vec![[0.5, 0.5, 0.0, 0.0]; n];
    joints[0] = [7, 0, 0, 0];
    weights[0] = [0.5, 0.5, 0.0, 0.0];
    let skinned =
        SkinnedMesh::new(bind.clone(), joints, weights, olvin_ecs::Entity::DANGLING).unwrap();
    let mesh = h.meshes.add(bind);
    let m = bent();
    h.meshes.skin(mesh, &skinned, &m);
    h.core.upload(&mut h.gpu, &mut h.meshes);
    let first = h
        .gpu
        .skins
        .get(&mesh.0)
        .map(|s| std::sync::Arc::as_ptr(&s.source));
    let again = [Mat4::from_translation(Vec3::X), Mat4::IDENTITY];
    h.meshes.skin(mesh, &skinned, &again);
    h.core.upload(&mut h.gpu, &mut h.meshes);
    assert_eq!(
        first,
        h.gpu
            .skins
            .get(&mesh.0)
            .map(|s| std::sync::Arc::as_ptr(&s.source))
    );
    let want = skinned.skin(&again);
    let got = read_vertices(&h, mesh.0, want.vertices.len());
    for (g, w) in got.iter().zip(&want.vertices) {
        for (a, b) in g.position.iter().zip(&w.position) {
            assert!((a - b).abs() < 1e-4, "{g:?} vs {w:?}");
        }
    }
}

/// A unit cube skinned to one joint, floating above a floor lit from the
/// side: its shadow lands at x in [0.5, 2.5], z in [-0.5, 0.5] + the pose.
#[test]
fn shadows_follow_the_pose() {
    for tier in [RenderTier::Full, RenderTier::Compat] {
        let Some(mut h) = harness(tier) else {
            return;
        };
        let cube = MeshData::cube(1.0);
        let n = cube.vertices.len();
        let skinned = SkinnedMesh::new(
            cube.clone(),
            vec![[0; 4]; n],
            vec![[1.0, 0.0, 0.0, 0.0]; n],
            olvin_ecs::Entity::DANGLING,
        )
        .unwrap();
        let plane = h.meshes.add(MeshData::plane(20.0));
        let mesh = h.meshes.add(cube);
        let mut world = World::new();
        world.spawn((Transform::IDENTITY, MeshRenderer::new(plane, Color::WHITE)));
        world.spawn((
            Transform::from_xyz(0.0, 1.5, 0.0),
            MeshRenderer::new(mesh, Color::WHITE),
        ));
        world.spawn(top_down_camera(8.0));
        let sun_dir = Vec3::new(1.0, -1.0, 0.0).normalize();
        world.spawn((
            Transform::IDENTITY.looking_at(sun_dir, Vec3::Y),
            Light::directional([1.0; 3], 1.0),
        ));
        let settings = RenderSettings::default();
        let row =
            |z: f32| ((0.5 + z / (8.0 * 30f32.to_radians().tan()) * 0.5) * SIZE as f32) as u32;
        let (x, near, far) = (column(1.5, 8.0), row(0.0), row(3.0));

        h.meshes.skin(mesh, &skinned, &[Mat4::IDENTITY]);
        let (_, at) = h.render(&world, &settings);
        assert!(h.view.shadows_active());
        let (a0, b0) = (at(x, near), at(x, far));
        assert!(
            luma(a0) + 150 < luma(b0),
            "{tier:?} rest: {a0:?} in shadow vs {b0:?}"
        );

        // Slide the cube 3 m along +Z: the shadow goes with it.
        h.meshes
            .skin(mesh, &skinned, &[Mat4::from_translation(Vec3::Z * 3.0)]);
        let (_, at) = h.render(&world, &settings);
        let (a1, b1) = (at(x, near), at(x, far));
        assert!(
            luma(b1) + 150 < luma(a1),
            "{tier:?} moved: {a1:?} vs {b1:?} in shadow"
        );
        if tier == RenderTier::Full && h.core.gpu_skinning() {
            assert_eq!(h.gpu.skins.len(), 1);
        } else {
            assert!(h.gpu.skins.is_empty(), "{tier:?} poses on the CPU");
        }
    }
}

/// The bind pose sits at the origin; the pose throws the mesh 20 m away,
/// where only the camera looks. Culling by the bind-pose sphere would hide it.
#[test]
fn animated_mesh_is_not_culled_by_its_bind_bounds() {
    for tier in [RenderTier::Full, RenderTier::Compat] {
        let Some(mut h) = harness(tier) else {
            return;
        };
        let slab = MeshData::cube(1.0);
        let n = slab.vertices.len();
        let skinned = SkinnedMesh::new(
            slab.clone(),
            vec![[0; 4]; n],
            vec![[1.0, 0.0, 0.0, 0.0]; n],
            olvin_ecs::Entity::DANGLING,
        )
        .unwrap();
        let mesh = h.meshes.add(slab);
        let material = h.materials.add(Material {
            base_color: Color::WHITE,
            unlit: true,
            ..Default::default()
        });
        let mut world = World::new();
        world.spawn((
            Transform::IDENTITY,
            MeshRenderer::new(mesh, Color::rgb(1.0, 0.0, 0.0)).with_material(material),
        ));
        world.spawn((
            Transform::from_xyz(20.0, 8.0, 0.0).looking_at(Vec3::new(20.0, 0.0, 0.0), Vec3::NEG_Z),
            Camera::default(),
        ));
        h.meshes
            .skin(mesh, &skinned, &[Mat4::from_translation(Vec3::X * 20.0)]);
        let (stats, at) = h.render(&world, &flat_settings());
        assert_eq!(stats.culled, 0, "{tier:?}");
        assert!(close(at(SIZE / 2, SIZE / 2), [255, 0, 0]), "{tier:?}");
        // Back at the bind pose it is out of the camera's view.
        h.meshes.skin(mesh, &skinned, &[Mat4::IDENTITY]);
        let (stats, _) = h.render(&world, &flat_settings());
        assert_eq!(stats.culled, 1, "{tier:?}");
    }
}

#[test]
fn compat_falls_back_to_cpu_skinning() {
    let Some(mut h) = harness(RenderTier::Compat) else {
        return;
    };
    assert!(!h.core.gpu_skinning());
    let (bind, skinned) = capsule();
    let mesh = h.meshes.add(bind);
    h.meshes.skin(mesh, &skinned, &bent());
    let pixels = frame(&mut h, &lit_world(mesh));
    assert!(h.gpu.skins.is_empty());
    assert!(pixels.iter().any(|p| *p != pixels[0]), "something drew");
    // A device on the Full tier without compute limits falls back too.
    h.core.set_tier(RenderTier::Full);
    let ok = h.core.gpu_skinning();
    let l = h.core.device.limits();
    assert_eq!(
        ok,
        l.max_storage_buffers_per_shader_stage >= 4 && l.max_compute_workgroups_per_dimension > 0
    );
}

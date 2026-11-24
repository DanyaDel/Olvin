//! The cases. Every one builds its scene outside the timed part and measures
//! one thing, so a change in a number points at a subsystem.

use std::hint::black_box;

use glam::{Quat, Vec3};
use olvin_core::{App, Resources};
use olvin_ecs::{Entity, World};
use olvin_scene::Transform;

use crate::{Case, measure, measure_with};

pub fn all() -> Vec<Case> {
    vec![
        Case {
            name: "ecs_iterate_100k",
            what: "move 100 000 entities through a query",
            run: ecs_iterate,
        },
        Case {
            name: "transform_propagation_20k",
            what: "world matrices of 20 000 objects in a hierarchy",
            run: transform_propagation,
        },
        Case {
            name: "physics_pile_500_60_steps",
            what: "60 fixed steps of 500 boxes dropped on a floor",
            run: physics_pile,
        },
        Case {
            name: "physics_raycast_10k",
            what: "10 000 rays against 500 boxes",
            run: physics_raycast,
        },
        Case {
            name: "nav_bake_160k_cells",
            what: "bake a 100 x 100 m level with walls into 160 000 cells",
            run: nav_bake,
        },
        Case {
            name: "nav_find_path_12_maze",
            what: "12 A* paths through that level (a maze: the search covers most of it)",
            run: nav_paths,
        },
        Case {
            name: "lod_simplify_100k_triangles",
            what: "simplify a 100 000 triangle mesh to a quarter",
            run: lod_simplify,
        },
        Case {
            name: "scene_parse_5000_objects",
            what: "parse a scene file of 5 000 objects",
            run: scene_parse,
        },
        Case {
            name: "save_sign_check_1mb",
            what: "sign and verify 1 MB of save text",
            run: save_sign,
        },
        Case {
            name: "timers_advance_20k",
            what: "advance 20 000 repeating timers 100 frames",
            run: timers,
        },
        Case {
            name: "lua_update_5000",
            what: "one update of 5 000 Lua behaviours",
            run: lua_update,
        },
        Case {
            name: "render_prepare_5k_200_lights",
            what: "CPU side of a frame: 5 000 objects, 200 lights (needs a GPU)",
            run: render_prepare,
        },
    ]
}

struct Velocity(Vec3);

fn ecs_iterate(scale: f64) -> Option<Vec<f64>> {
    let mut world = World::new();
    for i in 0..100_000 {
        world.spawn((
            Transform::from_xyz(i as f32, 0.0, 0.0),
            Velocity(Vec3::new(1.0, 0.5, 0.0)),
        ));
    }
    Some(measure(3, 30, scale, || {
        for (t, v) in world.query_mut::<(&mut Transform, &Velocity)>() {
            t.translation += v.0 * 0.016;
        }
        black_box(&world);
    }))
}

fn transform_propagation(scale: f64) -> Option<Vec<f64>> {
    let mut app = App::new();
    app.add_plugin(olvin_transform::TransformPlugin);
    let mut roots = Vec::new();
    {
        let mut world = app.resources.fetch_mut::<World>();
        for r in 0..2_000 {
            let root = world.spawn((Transform::from_xyz(r as f32, 0.0, 0.0),));
            roots.push(root);
            let mut parent = root;
            for c in 0..9 {
                parent = world.spawn((
                    Transform::from_xyz(0.0, c as f32, 0.0),
                    olvin_transform::Parent(parent),
                ));
            }
        }
    }
    app.update();
    Some(measure(3, 20, scale, || {
        {
            let world = app.resources.fetch::<World>();
            for &root in &roots {
                if let Ok(mut t) = world.get::<&mut Transform>(root) {
                    t.translation.y += 0.01;
                }
            }
        }
        app.update();
    }))
}

/// A floor and `n` boxes in a grid above it, physics in a bare `Resources`.
fn physics_scene(n: usize) -> Resources {
    use olvin_scene::file::{ColliderDesc, ColliderShape, RigidbodyDesc};
    let mut res = Resources::default();
    let mut world = World::new();
    world.spawn((
        Transform::from_xyz(0.0, -0.5, 0.0),
        ColliderDesc::new(ColliderShape::Box {
            size: Vec3::new(100.0, 1.0, 100.0),
        }),
    ));
    for i in 0..n {
        let (x, y, z) = (
            (i % 10) as f32 * 1.2 - 6.0,
            1.0 + (i / 100) as f32 * 1.2,
            ((i / 10) % 10) as f32 * 1.2 - 6.0,
        );
        world.spawn((
            Transform::from_xyz(x, y, z),
            ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
            RigidbodyDesc::default(),
        ));
    }
    res.insert(world);
    res.insert(olvin_physics::Physics::new(Vec3::new(0.0, -9.81, 0.0)));
    res
}

fn physics_pile(scale: f64) -> Option<Vec<f64>> {
    Some(measure_with(
        5,
        scale,
        || physics_scene(500),
        |res| {
            for _ in 0..60 {
                olvin_physics::step(&res, olvin_physics::FIXED_DT);
            }
            black_box(&res);
        },
    ))
}

fn physics_raycast(scale: f64) -> Option<Vec<f64>> {
    let res = physics_scene(500);
    for _ in 0..5 {
        olvin_physics::step(&res, olvin_physics::FIXED_DT);
    }
    let physics = res.fetch::<olvin_physics::Physics>();
    Some(measure(2, 10, scale, || {
        let mut hits = 0;
        for i in 0..10_000 {
            let x = (i % 100) as f32 * 0.12 - 6.0;
            let z = (i / 100) as f32 * 0.12 - 6.0;
            if physics
                .raycast(Vec3::new(x, 30.0, z), Vec3::NEG_Y, 100.0)
                .is_some()
            {
                hits += 1;
            }
        }
        black_box(hits);
    }))
}

/// A 100 x 100 m floor with a few walls, as triangles.
fn nav_level() -> Vec<olvin_nav::Triangle> {
    let quad = |y: f32, x0: f32, z0: f32, x1: f32, z1: f32| {
        let (a, b, c, d) = (
            Vec3::new(x0, y, z0),
            Vec3::new(x1, y, z0),
            Vec3::new(x1, y, z1),
            Vec3::new(x0, y, z1),
        );
        [[a, b, c], [a, c, d]]
    };
    let wall = |z: f32, x0: f32, x1: f32| {
        let (a, b, c, d) = (
            Vec3::new(x0, 0.0, z),
            Vec3::new(x1, 0.0, z),
            Vec3::new(x1, 3.0, z),
            Vec3::new(x0, 3.0, z),
        );
        [[a, b, c], [a, c, d]]
    };
    let mut tris: Vec<olvin_nav::Triangle> = quad(0.0, 0.0, 0.0, 100.0, 100.0).to_vec();
    for k in 1..10 {
        let z = k as f32 * 10.0;
        // Every wall has a gap, alternating sides, so paths must snake.
        let (x0, x1) = if k % 2 == 0 {
            (10.0, 100.0)
        } else {
            (0.0, 90.0)
        };
        tris.extend(wall(z, x0, x1));
    }
    tris
}

fn nav_bake(scale: f64) -> Option<Vec<f64>> {
    let tris = nav_level();
    Some(measure(1, 5, scale, || {
        let grid = olvin_nav::NavGrid::bake(&tris, olvin_nav::NavConfig::default());
        black_box(grid.map(|g| g.node_count()).ok());
    }))
}

fn nav_paths(scale: f64) -> Option<Vec<f64>> {
    let grid = olvin_nav::NavGrid::bake(&nav_level(), olvin_nav::NavConfig::default()).ok()?;
    Some(measure(1, 5, scale, || {
        let mut found = 0;
        for i in 0..12 {
            let x = 5.0 + (i % 10) as f32 * 9.0;
            if grid
                .find_path(Vec3::new(x, 0.0, 2.0), Vec3::new(100.0 - x, 0.0, 98.0), 1.0)
                .is_some()
            {
                found += 1;
            }
        }
        black_box(found);
    }))
}

fn sphere(rings: u32, segments: u32) -> olvin_render::MeshData {
    use olvin_render::{MeshData, Vertex};
    let mut mesh = MeshData::default();
    for r in 0..=rings {
        let theta = std::f32::consts::PI * r as f32 / rings as f32;
        for s in 0..=segments {
            let phi = std::f32::consts::TAU * s as f32 / segments as f32;
            let n = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            mesh.vertices.push(Vertex {
                position: n.into(),
                normal: n.into(),
                uv: [0.0; 2],
                tangent: [1.0, 0.0, 0.0, 1.0],
            });
        }
    }
    for r in 0..rings {
        for s in 0..segments {
            let a = r * (segments + 1) + s;
            let b = a + segments + 1;
            mesh.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    mesh
}

fn lod_simplify(scale: f64) -> Option<Vec<f64>> {
    let mesh = sphere(160, 320); // 102 400 triangles
    Some(measure(1, 5, scale, || {
        black_box(mesh.simplified(0.25, 0.05).indices.len());
    }))
}

fn scene_parse(scale: f64) -> Option<Vec<f64>> {
    use olvin_scene::file::{ColliderDesc, EntityDesc, MeshDesc, SceneFile, Shape};
    let mut scene = SceneFile::default();
    for i in 0..5_000u64 {
        let mut e = EntityDesc::new(i + 1, format!("Object {i}"));
        e.position = Vec3::new(i as f32, 0.0, 0.0);
        e.mesh = Some(MeshDesc {
            shape: Shape::Cube,
            color: [0.5; 4],
        });
        e.collider = Some(ColliderDesc::for_shape(Shape::Cube));
        scene.entities.push(e);
    }
    let text = ron::ser::to_string_pretty(&scene, ron::ser::PrettyConfig::new()).ok()?;
    Some(measure(1, 10, scale, || {
        black_box(SceneFile::parse(&text).map(|s| s.entities.len()).ok());
    }))
}

fn save_sign(scale: f64) -> Option<Vec<f64>> {
    use olvin_core::secret::SecretBytes;
    use olvin_game::SaveSigner;
    let signer = SaveSigner::new(SecretBytes::new(b"bench key".to_vec()));
    let text = "(gold: 100, name: \"hero\")\n".repeat(40_000);
    Some(measure(1, 15, scale, || {
        let signed = signer.sign(&text).unwrap_or_default();
        black_box(signer.check(&signed));
    }))
}

fn timers(scale: f64) -> Option<Vec<f64>> {
    Some(measure_with(
        10,
        scale,
        || {
            let mut t = olvin_core::Timers::default();
            for i in 0..20_000 {
                t.every(0.05 + (i % 50) as f32 * 0.01, format!("t{}", i % 64));
            }
            t
        },
        |mut t| {
            for _ in 0..100 {
                black_box(t.advance(0.016, 0.016).len());
            }
        },
    ))
}

fn lua_update(scale: f64) -> Option<Vec<f64>> {
    use olvin_script::lua::LuaScripts;
    use olvin_script::{ScriptPlugin, ScriptRegistry, Scripts, run_update};
    let mut app = App::new();
    app.add_plugin(ScriptPlugin::default());
    let mut lua = LuaScripts::new().ok()?;
    lua.load_source(
        &mut app.resources.fetch_mut::<ScriptRegistry>(),
        "Walker",
        "return { update = function(self, ctx) ctx:translate_xyz(0.01, 0, 0) end }",
    )
    .ok()?;
    {
        let mut world = app.resources.fetch_mut::<World>();
        for _ in 0..5_000 {
            world.spawn((Transform::IDENTITY, Scripts::new().with("Walker", [])));
        }
    }
    for _ in 0..3 {
        run_update(&app.resources, Some(1.0 / 60.0));
    }
    let _keep = lua;
    Some(measure(3, 30, scale, || {
        run_update(&app.resources, Some(1.0 / 60.0))
    }))
}

fn render_prepare(scale: f64) -> Option<Vec<f64>> {
    use olvin_render::{
        Color, GpuMeshes, Light, Materials, MeshData, MeshRenderer, Meshes, RenderCore,
        RenderSettings, Shaders, Textures, camera_view_proj,
    };
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: olvin_render::full_tier_limits(&adapter),
        ..Default::default()
    }))
    .ok()?;
    let core = RenderCore::new(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        1,
        wgpu::TextureFormat::Depth24Plus,
    );
    let mut view = core.create_view();
    let (mut gpu, mut meshes, mut textures, mut materials, mut shaders) = (
        GpuMeshes::default(),
        Meshes::default(),
        Textures::default(),
        Materials::default(),
        Shaders::default(),
    );
    let cube = meshes.add(MeshData::cube(1.0));
    let mut world = World::new();
    for i in 0..5_000 {
        let (x, z) = (
            (i % 100) as f32 * 2.0 - 100.0,
            (i / 100) as f32 * 2.0 - 60.0,
        );
        world.spawn((
            Transform::from_xyz(x, 0.5, z),
            MeshRenderer::new(cube, Color::rgb(0.8, 0.6, 0.4)),
        ));
    }
    for i in 0..200 {
        let (x, z) = ((i % 20) as f32 * 8.0 - 80.0, (i / 20) as f32 * 8.0 - 40.0);
        world.spawn((
            Transform::from_xyz(x, 3.0, z),
            Light::point([1.0, 0.9, 0.7], 3.0, 8.0),
        ));
    }
    world.spawn((
        Transform::from_xyz(0.0, 12.0, 40.0).looking_at(Vec3::new(0.0, 0.0, -20.0), Vec3::Y),
        olvin_scene::Camera::default(),
    ));
    core.upload(&mut gpu, &mut meshes);
    core.upload_assets(&mut gpu, &mut textures, &mut materials, &mut shaders);
    let view_proj = camera_view_proj(&world, 16.0 / 9.0)?;
    let settings = RenderSettings::default();
    let _ = (Quat::IDENTITY, Entity::DANGLING);
    Some(measure(5, 30, scale, || {
        black_box(core.prepare(&mut view, &gpu, &world, view_proj, &settings));
    }))
}

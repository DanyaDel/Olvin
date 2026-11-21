//! Headless README screenshots of the editor with every element, rendered
//! with wgpu through egui_kittest. The app is created by
//! `olvin_editor::build_app`, like `olvin_editor::run`, so the scene view
//! draws the real 3D scene. Ignored by default (needs a GPU or llvmpipe):
//!
//! ```sh
//! __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl \
//! cargo test -p editor_dev --test screenshots -- --ignored
//! ```
//!
//! PNGs go to `docs/screenshots/` (or `$OLVIN_EDITOR_SHOTS`).

use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui_wgpu::{self, RenderState, ScreenDescriptor, wgpu};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, TestRenderer};
use olvin_editor::glam::Vec3;
use olvin_editor::olvin_scene::file::{
    BrushDesc, BrushShape, CameraDesc, ColliderDesc, CsgOp, EntityDesc, Environment, LightDesc,
    LightKind, MeshDesc, SceneFile, Shape,
};
use olvin_editor::theme::ThemeKind;
use olvin_editor::{EditorApp, EditorPlugin, egui};

const SIZE: egui::Vec2 = egui::vec2(1600.0, 940.0);
const SELECTED: u64 = 10;

fn plugins() -> Vec<Box<dyn EditorPlugin>> {
    // Same list as src/main.rs.
    vec![
        Box::new(olvin_editor_console::ConsolePlugin),
        Box::new(olvin_editor_animator::AnimatorPlugin),
        Box::new(olvin_editor_csg::CsgPlugin),
        Box::new(olvin_editor_modeling::ModelingPlugin),
        Box::new(olvin_editor_uv::UvPlugin),
        Box::new(olvin_editor_ide::IdePlugin),
    ]
}

/// `color` as egui's colour button writes it back (through HSV). The
/// inspector stores that value every frame, so any other value would mark
/// the freshly opened scene as changed.
fn shown(color: [f32; 4]) -> [f32; 4] {
    let mut c = color;
    for _ in 0..8 {
        c = egui::ecolor::Hsva::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
            .to_rgba_unmultiplied();
    }
    c
}

fn mesh(id: u64, name: &str, shape: Shape, color: [f32; 4], pos: Vec3) -> EntityDesc {
    let mut e = EntityDesc::new(id, name);
    e.position = pos;
    e.mesh = Some(MeshDesc {
        shape,
        color: shown(color),
    });
    e.collider = Some(ColliderDesc::for_shape(shape));
    e
}

fn brush(id: u64, name: &str, shape: BrushShape, op: CsgOp, pos: Vec3, scale: Vec3) -> EntityDesc {
    let mut e = EntityDesc::new(id, name);
    e.position = pos;
    e.scale = scale;
    e.brush = Some(BrushDesc {
        shape,
        op,
        color: [0.62, 0.6, 0.56, 1.0],
        ..Default::default()
    });
    e
}

/// A small courtyard: CSG walls with a doorway and stairs, a few props, sun
/// and a warm lamp.
fn demo_scene() -> SceneFile {
    let mut camera = EntityDesc::new(1, "Main Camera");
    camera.position = Vec3::new(0.0, 3.0, 9.0);
    camera.rotation = Vec3::new(-16.0, 0.0, 0.0);
    camera.camera = Some(CameraDesc::default());
    camera.audio_listener = true;

    let mut sun = EntityDesc::new(2, "Sun");
    sun.position = Vec3::new(-5.5, 4.5, -2.5);
    sun.rotation = Vec3::new(-48.0, -35.0, 0.0);
    sun.light = Some(LightDesc::new(LightKind::Directional));

    let mut lamp = EntityDesc::new(3, "Lamp");
    lamp.position = Vec3::new(2.6, 2.4, 0.4);
    lamp.light = Some(LightDesc {
        color: {
            let [r, g, b, _] = shown([1.0, 0.72, 0.42, 1.0]);
            [r, g, b]
        },
        intensity: 2.5,
        range: 7.0,
        ..LightDesc::new(LightKind::Point)
    });

    let mut ground = mesh(
        4,
        "Ground",
        Shape::Plane,
        [0.13, 0.14, 0.13, 1.0],
        Vec3::ZERO,
    );
    ground.scale = Vec3::splat(3.0);

    let level = EntityDesc::new(5, "Level");
    let mut level_children = vec![
        brush(
            6,
            "Back Wall",
            BrushShape::Box,
            CsgOp::Add,
            Vec3::new(-1.0, 1.5, -3.0),
            Vec3::new(8.0, 3.0, 0.6),
        ),
        brush(
            7,
            "Doorway",
            BrushShape::Box,
            CsgOp::Subtract,
            Vec3::new(-2.5, 1.1, -3.0),
            Vec3::new(1.4, 2.2, 1.2),
        ),
        brush(
            8,
            "Stairs",
            BrushShape::Stairs { steps: 6 },
            CsgOp::Add,
            Vec3::new(3.6, 0.75, -1.6),
            Vec3::new(1.6, 1.5, 2.2),
        ),
        brush(
            9,
            "Pillar",
            BrushShape::Cylinder { sides: 12 },
            CsgOp::Add,
            Vec3::new(-4.6, 1.5, -1.8),
            Vec3::new(0.7, 3.0, 0.7),
        ),
    ];
    for b in &mut level_children {
        b.parent = Some(5);
    }

    let mut crate_ = mesh(
        SELECTED,
        "Crate",
        Shape::Cube,
        [0.9, 0.45, 0.15, 1.0],
        Vec3::new(0.0, 0.5, 0.0),
    );
    crate_.rotation = Vec3::new(0.0, 20.0, 0.0);
    let mut small = mesh(
        11,
        "Crate (1)",
        Shape::Cube,
        [0.25, 0.5, 0.85, 1.0],
        Vec3::new(1.3, 0.35, 0.9),
    );
    small.scale = Vec3::splat(0.7);
    small.rotation = Vec3::new(0.0, -25.0, 0.0);
    let ball = mesh(
        12,
        "Ball",
        Shape::Sphere,
        [0.85, 0.85, 0.82, 1.0],
        Vec3::new(-1.4, 0.5, 1.2),
    );
    let mut barrel = mesh(
        13,
        "Barrel",
        Shape::Cylinder,
        [0.35, 0.6, 0.35, 1.0],
        Vec3::new(-0.6, 0.5, -1.4),
    );
    barrel.scale = Vec3::new(0.8, 0.5, 0.8);
    let player = mesh(
        14,
        "Player",
        Shape::Capsule,
        [0.2, 0.7, 0.65, 1.0],
        Vec3::new(-3.4, 1.0, 2.6),
    );
    let mut ramp = brush(
        15,
        "Ramp",
        BrushShape::Wedge,
        CsgOp::Add,
        Vec3::new(3.4, 0.5, 2.4),
        Vec3::new(1.6, 1.0, 2.0),
    );
    ramp.parent = Some(5);
    ramp.rotation = Vec3::new(0.0, -90.0, 0.0);

    let mut entities = vec![camera, sun, lamp, ground, level];
    entities.extend(level_children);
    entities.push(ramp);
    entities.extend([crate_, small, ball, barrel, player]);
    SceneFile {
        entities,
        environment: (Environment::PRESETS[0].1)(),
        ..Default::default()
    }
}

/// A throwaway project: the scene plus a few folders for the Assets panel.
fn demo_project(root: &Path) {
    let _ = fs::remove_dir_all(root);
    let assets = root.join("assets");
    for dir in [
        "scenes",
        "materials",
        "models",
        "textures",
        "scripts",
        "audio",
    ] {
        fs::create_dir_all(assets.join(dir)).unwrap();
    }
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"courtyard\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    demo_scene()
        .save(assets.join("scenes/main.scene.ron"))
        .unwrap();
    demo_scene()
        .save(assets.join("scenes/night.scene.ron"))
        .unwrap();
}

/// egui's wgpu painter as eframe sets it up for the editor: MSAA colour,
/// depth, resolved into a texture we read back. kittest's own renderer has
/// neither, and the scene view's pipelines need both.
pub struct EditorRenderer {
    render_state: RenderState,
}

impl EditorRenderer {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let options = egui_wgpu::RendererOptions {
            msaa_samples: olvin_editor::MSAA,
            depth_stencil_format: Some(olvin_editor::DEPTH_FORMAT),
            ..egui_wgpu::RendererOptions::PREDICTABLE
        };
        let render_state = egui_kittest::wgpu::create_render_state(
            egui_kittest::wgpu::default_wgpu_setup(),
            options,
        );
        // A broken frame should fail the test, not save a black PNG.
        render_state
            .device
            .on_uncaptured_error(std::sync::Arc::new(|err| panic!("wgpu: {err}")));
        Self { render_state }
    }

    fn target(&self, size: [u32; 2], samples: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
        self.render_state
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("screenshot target"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                // Only the resolved texture is read back. On GL a multisampled
                // texture that may be copied is not a renderbuffer and never resolves.
                usage: if samples == 1 {
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::RENDER_ATTACHMENT
                },
                view_formats: &[],
            })
    }
}

impl TestRenderer for EditorRenderer {
    fn setup_eframe(&self, cc: &mut eframe::CreationContext<'_>, frame: &mut eframe::Frame) {
        cc.wgpu_render_state = Some(self.render_state.clone());
        frame.wgpu_render_state = Some(self.render_state.clone());
    }

    fn handle_delta(&mut self, delta: &mut egui::TexturesDelta) {
        let rs = &self.render_state;
        let mut renderer = rs.renderer.write();
        for (id, images) in delta.set.drain() {
            for image in images {
                renderer.update_texture(&rs.device, &rs.queue, id, &image);
            }
        }
        for id in delta.free.drain() {
            renderer.free_texture(&id);
        }
    }

    fn render(
        &mut self,
        ctx: &egui::Context,
        output: &egui::FullOutput,
    ) -> Result<image::RgbaImage, String> {
        let rs = self.render_state.clone();
        let mut renderer = rs.renderer.write();
        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let ppp = ctx.pixels_per_point();
        let size = ctx.content_rect().size() * ppp;
        let screen = ScreenDescriptor {
            pixels_per_point: ppp,
            size_in_pixels: [size.x.round() as u32, size.y.round() as u32],
        };
        let shapes = ctx.tessellate(output.shapes.clone(), ppp);
        let user_buffers =
            renderer.update_buffers(&rs.device, &rs.queue, &mut encoder, &shapes, &screen);

        let resolved = self.target(screen.size_in_pixels, 1, rs.target_format);
        let msaa = self.target(screen.size_in_pixels, olvin_editor::MSAA, rs.target_format);
        let depth = self.target(
            screen.size_in_pixels,
            olvin_editor::MSAA,
            olvin_editor::DEPTH_FORMAT,
        );
        let resolved_view = resolved.create_view(&Default::default());
        let msaa_view = msaa.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        {
            let clear = ctx
                .global_style()
                .visuals
                .panel_fill
                .to_normalized_gamma_f32();
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("screenshot pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &msaa_view,
                        resolve_target: Some(&resolved_view),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: clear[0] as f64,
                                g: clear[1] as f64,
                                b: clear[2] as f64,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                })
                .forget_lifetime();
            renderer.render(&mut pass, &shapes, &screen);
        }
        rs.queue
            .submit(user_buffers.into_iter().chain([encoder.finish()]));
        drop(renderer);
        read_back(&rs, &resolved)
    }
}

fn read_back(rs: &RenderState, texture: &wgpu::Texture) -> Result<image::RgbaImage, String> {
    let (w, h) = (texture.width(), texture.height());
    let row = w as usize * 4;
    let padded = row.div_ceil(256) * 256;
    let buffer = rs.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot readback"),
        size: (padded * h as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = rs
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    let index = rs.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| drop(tx.send(r)));
    rs.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: Some(std::time::Duration::from_secs(60)),
        })
        .map_err(|e| format!("poll: {e}"))?;
    rx.recv()
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let data = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|e| e.to_string())?;
    let bgra = matches!(
        texture.format(),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    let mut pixels = Vec::with_capacity(row * h as usize);
    for line in data.chunks_exact(padded) {
        for px in line[..row].as_chunks::<4>().0 {
            let (r, b) = if bgra { (px[2], px[0]) } else { (px[0], px[2]) };
            pixels.extend_from_slice(&[r, px[1], b, 255]);
        }
    }
    image::RgbaImage::from_raw(w, h, pixels).ok_or_else(|| "bad image size".to_owned())
}

/// One screenshot. With `edit_mode`, the selected object is in the Modeling
/// element's Edit Mode with every vertex selected (faces, all selected).
fn shot(project: &Path, out: &Path, theme: ThemeKind, workspace: Option<&str>, edit_mode: bool) {
    shot_with(project, out, theme, workspace, |harness| {
        if edit_mode {
            // The Modeling panel's buttons, like Tab / 3 / A in the scene view.
            for label in ["◆ Edit Mode", "Face 3", "All  A"] {
                harness.get_by_label(label).click();
                harness.run_steps(2);
            }
        }
    });
}

/// A screenshot after `drive` has operated the editor.
fn shot_with(
    project: &Path,
    out: &Path,
    theme: ThemeKind,
    workspace: Option<&str>,
    drive: impl FnOnce(&mut Harness<'_, EditorApp>),
) {
    let project = project.to_path_buf();
    let workspace = workspace.map(str::to_owned);
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .renderer(EditorRenderer::new())
        .build_eframe(move |cc| -> EditorApp {
            let registry = olvin_editor::registry(&plugins());
            let mut app = olvin_editor::build_app(cc, registry, Some(project)).expect("editor app");
            // Defaults, not whatever this machine's editor has saved.
            app.reset_layout();
            if let Some(name) = &workspace {
                assert!(app.show_workspace(name), "no workspace {name}");
            }
            let state = app.state_mut();
            state.prefs = olvin_editor::prefs::Preferences {
                theme,
                ..Default::default()
            };
            state.select(Some(SELECTED));
            app
        });
    harness.run_steps(30);
    drive(&mut harness);
    // kittest paints its pointer into the image; a screenshot has none.
    harness.remove_cursor();
    // 1.5 s in all: wordmark intro, tab easing and fades settle.
    harness.run_steps(60);
    let image = harness.render().expect("wgpu render");
    // Opaque anyway; RGB keeps the PNG smaller.
    image::DynamicImage::ImageRgba8(image)
        .to_rgb8()
        .save(out)
        .unwrap();
}

#[test]
#[ignore = "renders PNGs with wgpu; run with --ignored"]
fn screenshots() {
    let out = std::env::var_os("OLVIN_EDITOR_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots")
        });
    fs::create_dir_all(&out).unwrap();
    let project = std::env::temp_dir().join(format!(
        "olvin-editor-shots-{}/courtyard",
        std::process::id()
    ));
    demo_project(&project);

    shot(
        &project,
        &out.join("editor-dark.png"),
        ThemeKind::Dark,
        None,
        false,
    );
    shot(
        &project,
        &out.join("editor-light.png"),
        ThemeKind::Light,
        None,
        false,
    );
    shot(
        &project,
        &out.join("editor-modeling.png"),
        ThemeKind::Dark,
        Some("Modeling"),
        true,
    );
    let _ = fs::remove_dir_all(project.parent().unwrap());
}

/// Every workspace in every theme, for a visual review (`$OLVIN_EDITOR_REVIEW`).
#[test]
#[ignore = "renders PNGs with wgpu; run with --ignored"]
fn review() {
    let Some(out) = std::env::var_os("OLVIN_EDITOR_REVIEW").map(PathBuf::from) else {
        return;
    };
    fs::create_dir_all(&out).unwrap();
    let project =
        std::env::temp_dir().join(format!("olvin-editor-review-{}/yard", std::process::id()));
    demo_project(&project);
    let only = std::env::var("OLVIN_EDITOR_REVIEW_ONLY").ok();
    for theme in [ThemeKind::Dark, ThemeKind::Light] {
        for ws in [
            "Layout",
            "Lighting",
            "Animation",
            "Mapping",
            "Modeling",
            "UV Editing",
            "Code",
        ] {
            let name = format!("{:?}-{}.png", theme, ws.replace(' ', "_")).to_lowercase();
            if only.as_deref().is_some_and(|o| !name.contains(o)) {
                continue;
            }
            shot_with(&project, &out.join(name), theme, Some(ws), |_| {});
        }
    }
    let _ = fs::remove_dir_all(project.parent().unwrap());
}

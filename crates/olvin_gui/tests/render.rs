//! Offscreen check of the whole UI path: tree -> `DrawList` -> glyph atlas ->
//! `Overlay` -> GPU -> pixels. Skips when there is no adapter (llvmpipe:
//! `__EGL_VENDOR_LIBRARY_FILENAMES=.../50_mesa.json WGPU_BACKEND=gl`).
#![cfg(feature = "render")]

use glam::Vec2;
use olvin_gui::text::Font;
use olvin_gui::{Color, FontMeasure, GuiText, Node, Ui};
use olvin_render::{
    GpuMeshes, Overlay, OverlayRenderer, TextureHandle, Textures, begin_overlay_pass, wgpu,
};

const W: u32 = 192;
const H: u32 = 96;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits:
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .ok()
}

/// Draws `overlay` over `clear` and returns RGBA rows (W x H).
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    textures: &mut Textures,
    overlay: &Overlay,
    clear: [f64; 3],
) -> Vec<u8> {
    let core = olvin_render::RenderCore::new(
        device.clone(),
        queue.clone(),
        FORMAT,
        1,
        wgpu::TextureFormat::Depth24Plus,
    );
    let mut gpu = GpuMeshes::default();
    core.upload_assets(
        &mut gpu,
        textures,
        &mut Default::default(),
        &mut Default::default(),
    );
    let mut r = OverlayRenderer::new(device, queue, FORMAT);
    let draws = r.prepare(device, queue, &gpu, overlay, (W, H));
    assert!(draws > 0);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    // The 3D frame stand-in: a flat colour.
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: clear[0],
                    g: clear[1],
                    b: clear[2],
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    }));
    {
        let mut pass = begin_overlay_pass(&mut encoder, &view);
        r.draw(&mut pass);
    }
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (W * H * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(W * 4),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    readback.slice(..).get_mapped_range().expect("map").to_vec()
}

/// Pixels of `rows` that differ from `bg` in the rectangle.
fn changed(px: &[u8], bg: [u8; 3], x: std::ops::Range<u32>, y: std::ops::Range<u32>) -> usize {
    let mut n = 0;
    for yy in y {
        for xx in x.clone() {
            let i = ((yy * W + xx) * 4) as usize;
            if px[i..i + 3] != bg {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn text_and_rect_reach_the_screen() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let font = Font::builtin();
    let mut ui = Ui::new();
    let root = ui.root();
    // Plate at the top left, a Cyrillic label inside; the bottom stays empty.
    ui.add(
        root,
        Node::label("Привет, Hi")
            .text_size(20.0)
            .padding(4.0)
            .background(Color::rgba(0.0, 0.0, 0.5, 1.0))
            .color(Color::rgb(1.0, 1.0, 1.0)),
    );
    ui.layout(Vec2::new(W as f32, H as f32), &FontMeasure(font.clone()));
    let mut draw = Default::default();
    ui.paint(&Default::default(), &mut draw);

    let mut textures = Textures::default();
    let mut text = GuiText::new(font);
    let mut overlay = Overlay::default();
    text.paint(&draw, 1.0, &mut textures, &mut overlay);
    let tex: TextureHandle = text.texture().expect("atlas texture");
    assert!(overlay.quads.iter().all(|q| q.texture == Some(tex)));
    assert!(overlay.len() > 5, "plate and glyphs");

    let px = render(&device, &queue, &mut textures, &overlay, [0.0, 0.0, 0.0]);
    let plate = &overlay.quads[0];
    let (x1, y1) = (plate.max.x as u32, plate.max.y as u32);
    // The plate is blue; glyphs inside it are lighter than the plate.
    let at = |x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    let corner = at(1, 1);
    assert!(corner[2] > 100 && corner[0] < 20, "plate {corner:?}");
    let lit = (0..y1)
        .flat_map(|y| (0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| at(x, y)[0] > 100)
        .count();
    assert!(lit > 40, "white glyph pixels on the plate: {lit}");
    // Nothing below the plate.
    assert_eq!(changed(&px, [0, 0, 0], 0..W, (y1 + 2)..H), 0);
}

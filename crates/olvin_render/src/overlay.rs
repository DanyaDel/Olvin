//! Screen overlay: textured rectangles in window pixels, drawn last, after
//! post-processing and tonemapping, straight onto the presented image. The
//! renderer does not know who fills it: the in-game UI does (`olvin_gui` with
//! its `render` feature), a debug HUD could. Producers push every frame;
//! [`RenderPlugin`](crate::RenderPlugin) empties it in `Stage::First`.

use glam::Vec2;

use crate::core::GpuMeshes;
use crate::quad::{BatchKey, QuadBatcher, QuadBlend, QuadFilter, QuadInstance};
use crate::sprite::screen_view_proj;
use crate::{Color, TextureHandle};

/// One rectangle: physical pixels from the top-left (y down). Without a
/// texture it is a plain colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayQuad {
    pub min: Vec2,
    pub max: Vec2,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
    /// Linear RGBA, multiplies the texture.
    pub color: Color,
    pub texture: Option<TextureHandle>,
}

/// This frame's overlay, in paint order (later on top). Consecutive quads
/// with the same texture share a draw call, so a UI that keeps rectangles and
/// glyphs in one atlas is a single draw.
#[derive(Clone, Debug, Default)]
pub struct Overlay {
    pub quads: Vec<OverlayQuad>,
}

impl Overlay {
    /// Empties the list, keeping its capacity.
    pub fn clear(&mut self) {
        self.quads.clear();
    }

    pub fn len(&self) -> usize {
        self.quads.len()
    }

    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    pub fn push(&mut self, quad: OverlayQuad) {
        self.quads.push(quad);
    }

    /// A filled rectangle.
    pub fn rect(&mut self, min: Vec2, max: Vec2, color: Color) {
        self.push(OverlayQuad {
            min,
            max,
            uv_min: Vec2::ZERO,
            uv_max: Vec2::ONE,
            color,
            texture: None,
        });
    }

    /// A rectangle of `texture` (uvs from the top-left), tinted by `color`.
    pub fn image(
        &mut self,
        min: Vec2,
        max: Vec2,
        texture: TextureHandle,
        (uv_min, uv_max): (Vec2, Vec2),
        color: Color,
    ) {
        self.push(OverlayQuad {
            min,
            max,
            uv_min,
            uv_max,
            color,
            texture: Some(texture),
        });
    }
}

/// `Stage::First`: producers refill the overlay every frame.
pub(crate) fn clear(res: &mut olvin_core::Resources) {
    if let Some(mut overlay) = res.get_mut::<Overlay>() {
        overlay.clear();
    }
}

/// Draws an [`Overlay`] into a pass with one colour target and no depth
/// attachment. The window [`Renderer`](crate::Renderer) owns one; editors and
/// offscreen hosts can make their own.
pub struct OverlayRenderer {
    quads: QuadBatcher,
}

impl OverlayRenderer {
    /// `format` is the target's; an sRGB format encodes in hardware,
    /// any other in the shader.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self {
            quads: QuadBatcher::new(device, queue, format, 1, None, false),
        }
    }

    /// Uploads `overlay` for a `viewport` of physical pixels. Returns the
    /// number of draw calls [`Self::draw`] will make (0: skip the pass).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &GpuMeshes,
        overlay: &Overlay,
        viewport: (u32, u32),
    ) -> usize {
        self.quads.clear();
        for q in &overlay.quads {
            let size = q.max - q.min;
            if size.x <= 0.0 || size.y <= 0.0 || q.color.0[3] <= 0.0 {
                continue;
            }
            self.quads.push(
                BatchKey {
                    texture: q.texture,
                    filter: QuadFilter::Linear,
                    blend: QuadBlend::Alpha,
                },
                QuadInstance {
                    origin: [q.min.x, q.min.y, 0.0, 1.0],
                    axis_x: [size.x, 0.0, 0.0, 0.0],
                    axis_y: [0.0, size.y, 0.0, 0.0],
                    color: q.color.0,
                    uv: [q.uv_min.x, q.uv_min.y, q.uv_max.x, q.uv_max.y],
                },
            );
        }
        self.quads
            .upload(device, queue, gpu, screen_view_proj(viewport));
        self.quads.batch_count()
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.quads.draw(pass);
    }

    /// Draw calls of the last [`Self::prepare`].
    pub fn batch_count(&self) -> usize {
        self.quads.batch_count()
    }
}

/// A pass over `target` that keeps what is there (no depth): for the overlay.
pub fn begin_overlay_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("overlay"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    })
}

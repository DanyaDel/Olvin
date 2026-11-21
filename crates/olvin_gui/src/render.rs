//! Feature `render`: draws the UI. The [`DrawList`] becomes quads of
//! `olvin_render`'s [`Overlay`] (drawn after post-processing, on top of the
//! 3D frame) with glyphs from an `olvin_text` atlas. Rectangles sample the
//! atlas's white block, so a whole menu is one texture and one draw call.
//!
//! Only resources connect the crates: this module reads [`Gui`] and writes
//! `Textures` and `Overlay`; the renderer never learns about the UI.
//! Colours of the theme and nodes are taken as sRGB and converted to the
//! overlay's linear colours here.

use glam::Vec2;
use olvin_core::{App, Plugin, Resources, Stage};
use olvin_render::{Overlay, TextureData, TextureHandle, Textures};
use olvin_text::{Font, GlyphAtlas, GlyphQuad, HAlign};

use crate::draw::{DrawItem, DrawList, TextAlign};
use crate::geom::Color;
use crate::layout::TextMeasure;
use crate::plugin::{Gui, GuiPlugin};

/// [`TextMeasure`] from a real font: layout matches the drawn glyphs.
pub struct FontMeasure(pub Font);

impl TextMeasure for FontMeasure {
    fn measure(&self, text: &str, size: f32) -> Vec2 {
        self.0.measure(text, GlyphAtlas::quantize(size))
    }
}

/// The UI's font and glyph cache (resource), and the atlas texture in `Textures`.
pub struct GuiText {
    pub font: Font,
    pub atlas: GlyphAtlas,
    texture: Option<TextureHandle>,
    uploaded: u64,
    glyphs: Vec<GlyphQuad>,
}

impl GuiText {
    pub fn new(font: Font) -> Self {
        Self {
            font,
            atlas: GlyphAtlas::default(),
            texture: None,
            uploaded: 0,
            glyphs: Vec::new(),
        }
    }

    /// The atlas texture, once the first frame was painted.
    pub fn texture(&self) -> Option<TextureHandle> {
        self.texture
    }

    /// Appends `draw` to `overlay` at `scale` physical pixels per UI pixel,
    /// in paint order, and re-uploads the atlas into `textures` if glyphs were added.
    pub fn paint(
        &mut self,
        draw: &DrawList,
        scale: f32,
        textures: &mut Textures,
        overlay: &mut Overlay,
    ) {
        let texture = match self.texture {
            Some(t) => t,
            None => {
                let t = textures.add(self.atlas_texture());
                self.uploaded = self.atlas.version();
                self.texture = Some(t);
                t
            }
        };
        let start = overlay.len();
        // A second pass if the atlas grew or started over mid-frame (stale uvs).
        for _ in 0..2 {
            overlay.quads.truncate(start);
            self.emit(draw, scale, texture, overlay);
            if !self.atlas.take_stale() {
                break;
            }
        }
        if self.atlas.version() != self.uploaded {
            textures.set(texture, self.atlas_texture());
            self.uploaded = self.atlas.version();
        }
    }

    fn atlas_texture(&self) -> TextureData {
        let (w, h) = self.atlas.size();
        let mut rgba = Vec::new();
        self.atlas.rgba(&mut rgba);
        TextureData::rgba8(w, h, rgba, false)
    }

    fn emit(&mut self, draw: &DrawList, scale: f32, texture: TextureHandle, out: &mut Overlay) {
        let white = self.atlas.white_uv();
        for item in &draw.order {
            match *item {
                DrawItem::Quad(i) => {
                    let q = &draw.quads[i];
                    out.image(
                        q.rect.min * scale,
                        q.rect.max * scale,
                        texture,
                        (white, white),
                        linear(q.color),
                    );
                }
                DrawItem::Text(i) => {
                    let run = &draw.texts[i];
                    let size = run.size * scale;
                    let (min, max) = (run.rect.min * scale, run.rect.max * scale);
                    let block = self.font.measure(&run.text, GlyphAtlas::quantize(size));
                    // Vertically centred in the rect, as the layout assumes.
                    let top = (min.y + max.y - block.y) * 0.5;
                    let (x, align) = match run.align {
                        TextAlign::Left => (min.x, HAlign::Left),
                        TextAlign::Center => ((min.x + max.x) * 0.5, HAlign::Center),
                    };
                    self.glyphs.clear();
                    olvin_text::layout(
                        &self.font,
                        &mut self.atlas,
                        &run.text,
                        size,
                        Vec2::new(x, top),
                        align,
                        &mut self.glyphs,
                    );
                    let color = linear(run.color);
                    for g in &self.glyphs {
                        out.image(g.min, g.max, texture, (g.uv_min, g.uv_max), color);
                    }
                }
            }
        }
    }
}

/// sRGB UI colour to the overlay's linear one (alpha unchanged).
fn linear(c: Color) -> olvin_render::Color {
    let f = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b, a] = c.0;
    olvin_render::Color([f(r), f(g), f(b), a])
}

/// [`GuiPlugin`] plus drawing: inserts [`GuiText`] (the built-in Inter unless
/// `font` is set), puts a [`FontMeasure`] into [`Gui::measure`] and in
/// `PostUpdate`, after the layout, fills the [`Overlay`]. Add it with or after
/// `olvin_render::RenderPlugin` (it only creates the `Textures` / `Overlay`
/// resources if they are missing, so headless apps work too).
#[derive(Default)]
pub struct GuiRenderPlugin {
    pub font: Option<Font>,
}

impl Plugin for GuiRenderPlugin {
    fn build(&self, app: &mut App) {
        let font = self.font.clone().unwrap_or_else(Font::builtin);
        app.add_plugin(GuiPlugin)
            .init_resource::<Textures>()
            .init_resource::<Overlay>()
            .insert_resource(GuiText::new(font.clone()))
            // RenderPlugin clears it too; without a renderer nobody else would.
            .add_system(Stage::First, |res| res.fetch_mut::<Overlay>().clear())
            .add_system(Stage::PostUpdate, paint);
        app.resources.fetch_mut::<Gui>().measure = Box::new(FontMeasure(font));
    }
}

/// Swaps the UI font at runtime: the glyph cache and the layout measure.
pub fn set_font(res: &mut Resources, font: Font) {
    res.fetch_mut::<Gui>().measure = Box::new(FontMeasure(font.clone()));
    let mut text = res.fetch_mut::<GuiText>();
    text.font = font;
    text.atlas.clear();
}

fn paint(res: &mut Resources) {
    let (Some(mut text), Some(gui)) = (res.get_mut::<GuiText>(), res.get::<Gui>()) else {
        return;
    };
    text.paint(
        &gui.draw,
        gui.scale,
        &mut res.fetch_mut::<Textures>(),
        &mut res.fetch_mut::<Overlay>(),
    );
}

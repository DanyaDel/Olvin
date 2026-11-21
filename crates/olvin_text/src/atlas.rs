//! Glyph cache: rasterised glyphs packed into one coverage image by a shelf
//! packer. The image grows downwards (width stays) up to `max_height`; when
//! even that is full it starts over and tells the caller to lay out again.

use std::collections::HashMap;

use ab_glyph::{Font as _, GlyphId, PxScale, point};
use glam::Vec2;

use crate::Font;

/// Empty pixels around every glyph, so bilinear sampling never bleeds.
const PAD: u32 = 1;
/// Glyph sizes are cached in steps of 1/4 px.
const SIZE_STEPS: f32 = 4.0;
/// Side of the solid block at the top-left that untextured quads sample.
const WHITE: u32 = 2;

/// Where a cached glyph is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasGlyph {
    /// x, y, width, height in atlas pixels.
    pub rect: [u32; 4],
    /// From the pen position on the baseline to the bitmap's top-left (y down).
    pub offset: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    font: u32,
    glyph: u16,
    size: u32,
}

#[derive(Clone, Copy, Debug)]
struct Shelf {
    y: u32,
    height: u32,
    /// Next free x.
    x: u32,
}

/// One-channel coverage atlas (0 = empty, 255 = inside the glyph).
#[derive(Clone, Debug)]
pub struct GlyphAtlas {
    width: u32,
    height: u32,
    max_height: u32,
    coverage: Vec<u8>,
    shelves: Vec<Shelf>,
    /// `None`: the glyph has no pixels (a space).
    glyphs: HashMap<Key, Option<AtlasGlyph>>,
    version: u64,
    stale: bool,
}

impl Default for GlyphAtlas {
    /// 512 × 256, growing to 512 × 2048 (the WebGL2 texture limit).
    fn default() -> Self {
        Self::new(512, 256, 2048)
    }
}

impl GlyphAtlas {
    /// `width` × `height` pixels, may grow to `max_height`.
    pub fn new(width: u32, height: u32, max_height: u32) -> Self {
        let width = width.max(16);
        let height = height.max(16);
        let mut atlas = Self {
            width,
            height,
            max_height: max_height.max(height),
            coverage: Vec::new(),
            shelves: Vec::new(),
            glyphs: HashMap::new(),
            version: 0,
            stale: false,
        };
        atlas.clear();
        atlas.stale = false;
        atlas
    }

    /// Drops every glyph (keeps the size). Uvs handed out before are stale.
    pub fn clear(&mut self) {
        self.coverage.clear();
        self.coverage.resize((self.width * self.height) as usize, 0);
        self.shelves.clear();
        self.glyphs.clear();
        let [x, y, ..] = self
            .allocate(WHITE, WHITE)
            .expect("atlas fits its white block");
        for row in y..y + WHITE {
            let at = (row * self.width + x) as usize;
            self.coverage[at..at + WHITE as usize].fill(255);
        }
        self.version += 1;
        self.stale = true;
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Coverage, rows top to bottom, one byte per pixel.
    pub fn coverage(&self) -> &[u8] {
        &self.coverage
    }

    /// White RGB with the coverage as alpha (what a renderer uploads), into
    /// `out` (cleared first).
    pub fn rgba(&self, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(self.coverage.len() * 4);
        for &a in &self.coverage {
            out.extend_from_slice(&[255, 255, 255, a]);
        }
    }

    /// Bumped on every change of the pixels or the size: re-upload when it moves.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// True once after the atlas grew or started over: uvs computed since the
    /// previous call may point at the wrong place, lay the frame out again.
    pub fn take_stale(&mut self) -> bool {
        std::mem::take(&mut self.stale)
    }

    /// Cached glyphs, empty ones included.
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// A uv inside the solid block: a quad sampling only this is a plain
    /// rectangle, so rectangles and text can share one texture and draw call.
    pub fn white_uv(&self) -> Vec2 {
        // The centre of the 2×2 block: bilinear sampling sees only full texels.
        Vec2::new(WHITE as f32 * 0.5, WHITE as f32 * 0.5)
            / Vec2::new(self.width as f32, self.height as f32)
    }

    /// Uv rectangle (min, max) of a glyph at the current size.
    pub fn uv(&self, glyph: &AtlasGlyph) -> (Vec2, Vec2) {
        let size = Vec2::new(self.width as f32, self.height as f32);
        let [x, y, w, h] = glyph.rect.map(|v| v as f32);
        (Vec2::new(x, y) / size, Vec2::new(x + w, y + h) / size)
    }

    /// The size glyphs at `size` are really rasterised at.
    pub fn quantize(size: f32) -> f32 {
        (size.max(0.0) * SIZE_STEPS).round() / SIZE_STEPS
    }

    /// The glyph of `c` at em size `size` (quantised), rasterised on first use.
    /// `None` for glyphs without pixels and when the atlas had to start over
    /// and still could not fit it.
    pub fn glyph(&mut self, font: &Font, c: char, size: f32) -> Option<AtlasGlyph> {
        self.glyph_by_id(font, font.glyph_id(c), size)
    }

    pub(crate) fn glyph_by_id(
        &mut self,
        font: &Font,
        id: GlyphId,
        size: f32,
    ) -> Option<AtlasGlyph> {
        let key = Key {
            font: font.id,
            glyph: id.0,
            size: (Self::quantize(size) * SIZE_STEPS) as u32,
        };
        if let Some(cached) = self.glyphs.get(&key) {
            return *cached;
        }
        let size = key.size as f32 / SIZE_STEPS;
        // ab_glyph scales by the ascent-to-descent height, we by the em.
        let scale = font.scale(size) * font.inner.height_unscaled();
        let Some(outline) = font
            .inner
            .outline_glyph(id.with_scale_and_position(PxScale::from(scale), point(0.0, 0.0)))
        else {
            self.glyphs.insert(key, None);
            return None;
        };
        let bounds = outline.px_bounds();
        let (w, h) = (bounds.width() as u32, bounds.height() as u32);
        if w == 0 || h == 0 {
            self.glyphs.insert(key, None);
            return None;
        }
        let rect = match self.allocate(w, h) {
            Some(r) => r,
            None => {
                // Full at the largest size: start over. This frame's earlier
                // uvs are now wrong; `take_stale` tells the caller.
                self.clear();
                self.allocate(w, h)?
            }
        };
        let [x0, y0, ..] = rect;
        let stride = self.width;
        let coverage = &mut self.coverage;
        outline.draw(|x, y, c| {
            if x < w && y < h {
                let at = ((y0 + y) * stride + x0 + x) as usize;
                coverage[at] = (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        });
        self.version += 1;
        let glyph = AtlasGlyph {
            rect,
            offset: Vec2::new(bounds.min.x, bounds.min.y),
        };
        self.glyphs.insert(key, Some(glyph));
        Some(glyph)
    }

    /// Finds room for `w` × `h` (plus padding): the tightest shelf it fits,
    /// a new shelf, or a taller image. Returns x, y, w, h.
    fn allocate(&mut self, w: u32, h: u32) -> Option<[u32; 4]> {
        let (pw, ph) = (w + PAD, h + PAD);
        if pw > self.width {
            return None;
        }
        let best = self
            .shelves
            .iter_mut()
            .filter(|s| s.height >= ph && s.x + pw <= self.width)
            .min_by_key(|s| s.height - ph);
        if let Some(shelf) = best
            // Don't put a small glyph on a shelf twice its height.
            && shelf.height <= ph * 2
        {
            let x = shelf.x;
            shelf.x += pw;
            return Some([x, shelf.y, w, h]);
        }
        // Shelf heights in steps of 4 px: similar sizes share shelves.
        let height = ph.div_ceil(4) * 4;
        let y = self.shelves.last().map_or(0, |s| s.y + s.height);
        while y + height > self.height {
            if self.height >= self.max_height {
                return None;
            }
            self.height = (self.height * 2).min(self.max_height);
            self.coverage.resize((self.width * self.height) as usize, 0);
            self.version += 1;
            self.stale = true;
        }
        self.shelves.push(Shelf { y, height, x: pw });
        Some([0, y, w, h])
    }
}

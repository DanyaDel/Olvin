//! Line layout: characters to positioned glyph quads.

use glam::Vec2;

use crate::{Font, GlyphAtlas};

/// Horizontal anchor of every line: `origin.x` is its left edge, centre or right edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// One glyph to draw: pixel rectangle (y down) and its atlas uvs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphQuad {
    pub min: Vec2,
    pub max: Vec2,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
}

/// Lays `text` out at em size `size` and appends a quad per visible glyph to
/// `out`. `origin.y` is the top of the first line; lines split on `\n`, other
/// control characters are skipped. Glyphs land on whole pixels. Returns the
/// block size (as [`Font::measure`]).
///
/// Characters the font lacks show its "missing" box. If the atlas grows or
/// starts over meanwhile ([`GlyphAtlas::take_stale`]), lay out again: the
/// uvs of quads made before that are wrong.
pub fn layout(
    font: &Font,
    atlas: &mut GlyphAtlas,
    text: &str,
    size: f32,
    origin: Vec2,
    align: HAlign,
    out: &mut Vec<GlyphQuad>,
) -> Vec2 {
    let size = GlyphAtlas::quantize(size);
    let s = font.scale(size);
    let metrics = font.line_metrics(size);
    let line_height = metrics.line_height();
    let mut widest = 0.0f32;
    let mut lines = 0;
    for line in text.split('\n') {
        let width = font.measure(line, size).x;
        widest = widest.max(width);
        let left = match align {
            HAlign::Left => origin.x,
            HAlign::Center => origin.x - width * 0.5,
            HAlign::Right => origin.x - width,
        };
        let baseline = (origin.y + lines as f32 * line_height + metrics.ascent).round();
        lines += 1;
        let mut pen = left;
        let mut prev = None;
        for c in line.chars().filter(|c| !c.is_control()) {
            let id = font.glyph_id(c);
            let (advance, kern) = font.advance_units(prev, id);
            pen += kern * s;
            prev = Some(id);
            if let Some(g) = atlas.glyph_by_id(font, id, size) {
                let min = Vec2::new(pen.round(), baseline) + g.offset;
                let (uv_min, uv_max) = atlas.uv(&g);
                out.push(GlyphQuad {
                    min,
                    max: min + Vec2::new(g.rect[2] as f32, g.rect[3] as f32),
                    uv_min,
                    uv_max,
                });
            }
            pen += advance * s;
        }
    }
    Vec2::new(widest, lines as f32 * line_height)
}

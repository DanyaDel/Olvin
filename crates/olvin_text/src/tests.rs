use glam::Vec2;

use crate::*;

fn quads(atlas: &mut GlyphAtlas, text: &str, align: HAlign) -> (Vec<GlyphQuad>, Vec2) {
    let mut out = Vec::new();
    let size = layout(
        &Font::builtin(),
        atlas,
        text,
        20.0,
        Vec2::new(100.0, 10.0),
        align,
        &mut out,
    );
    (out, size)
}

fn overlaps(a: [u32; 4], b: [u32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

#[test]
fn builtin_font_covers_latin_and_cyrillic() {
    let font = Font::builtin();
    for c in "AZaz09.,!?—«»№ АБВГДЕЁЖЗИЙЯабвгдеёжзийяЇїЄєІіЎў‖→".chars()
    {
        assert!(font.has_glyph(c), "no glyph for {c:?}");
    }
    assert!(!font.has_glyph('中'), "the subset has no CJK");
    let m = font.line_metrics(20.0);
    assert!(m.ascent > 15.0 && m.descent < 0.0, "{m:?}");
    assert!((m.line_height() - 24.2).abs() < 0.5, "Inter is ~1.21 em");
}

#[test]
fn measure_adds_advances_and_lines() {
    let font = Font::builtin();
    let a = font.measure("A", 20.0);
    assert!((font.measure("AA", 20.0).x - 2.0 * a.x).abs() < 1e-3);
    assert!(
        (font.measure("A", 40.0).x - 2.0 * a.x).abs() < 1e-3,
        "linear in size"
    );
    let two = font.measure("Привет\nмир", 20.0);
    assert!((two.y - 2.0 * a.y).abs() < 1e-3);
    assert!(
        (two.x - font.measure("Привет", 20.0).x).abs() < 1e-3,
        "widest line"
    );
    let empty = font.measure("", 20.0);
    assert_eq!(empty.x, 0.0);
    assert!(
        (empty.y - a.y).abs() < 1e-3,
        "an empty text is still one line"
    );
    assert!(
        font.measure("i", 20.0).x < font.measure("Ш", 20.0).x,
        "proportional"
    );
}

#[test]
fn atlas_caches_glyphs_once() {
    let font = Font::builtin();
    let mut atlas = GlyphAtlas::default();
    let g = atlas.glyph(&font, 'Ж', 18.0).expect("pixels");
    let version = atlas.version();
    assert_eq!(atlas.glyph(&font, 'Ж', 18.0), Some(g));
    assert_eq!(
        atlas.glyph(&font, 'Ж', 18.1),
        Some(g),
        "sizes cached in 1/4 px steps"
    );
    assert_eq!(atlas.version(), version, "cache hit changes nothing");
    assert_eq!(atlas.len(), 1);
    assert!(
        atlas.glyph(&font, ' ', 18.0).is_none(),
        "a space has no pixels"
    );
    let big = atlas.glyph(&font, 'Ж', 36.0).unwrap();
    assert!(big.rect[3] > g.rect[3] + 8, "{big:?} vs {g:?}");
    assert!(atlas.version() > version);
    assert_eq!(atlas.len(), 3);
    // The glyph's bitmap is really there.
    let [x, y, w, h] = g.rect;
    let (aw, _) = atlas.size();
    let inside = (y..y + h)
        .flat_map(|r| (x..x + w).map(move |c| (r * aw + c) as usize))
        .filter(|&i| atlas.coverage()[i] > 128)
        .count();
    assert!(inside > 20, "{inside} covered pixels");
}

#[test]
fn packed_glyphs_never_overlap() {
    let font = Font::builtin();
    let mut atlas = GlyphAtlas::default();
    let mut rects = Vec::new();
    let text: String = ('А'..='я').chain('!'..='~').collect();
    for size in [12.0, 18.0, 30.0] {
        for c in text.chars() {
            if let Some(g) = atlas.glyph(&font, c, size) {
                rects.push(g.rect);
            }
        }
    }
    let (w, h) = atlas.size();
    assert!(rects.len() > 400);
    for (i, a) in rects.iter().enumerate() {
        assert!(
            a[0] + a[2] <= w && a[1] + a[3] <= h,
            "{a:?} outside {w}x{h}"
        );
        assert!(!overlaps(*a, [0, 0, 2, 2]), "{a:?} covers the white block");
        for b in &rects[i + 1..] {
            assert!(!overlaps(*a, *b), "{a:?} overlaps {b:?}");
        }
    }
}

#[test]
fn atlas_grows_then_starts_over() {
    let font = Font::builtin();
    let mut atlas = GlyphAtlas::new(64, 16, 128);
    assert!(!atlas.take_stale());
    let a = atlas.glyph(&font, 'A', 24.0).unwrap();
    for c in "BCDEFGH".chars() {
        atlas.glyph(&font, c, 24.0);
    }
    assert!(atlas.size().1 > 16, "grew: {:?}", atlas.size());
    assert!(atlas.take_stale());
    assert!(!atlas.take_stale(), "reported once");
    // Pixel rects survive growth; only the uvs change.
    assert_eq!(atlas.glyph(&font, 'A', 24.0), Some(a));
    let (uv_min, uv_max) = atlas.uv(&a);
    assert!(uv_min.cmpge(Vec2::ZERO).all() && uv_max.cmple(Vec2::ONE).all());

    for c in ('А'..='Я').chain('a'..='z') {
        atlas.glyph(&font, c, 24.0);
    }
    assert_eq!(atlas.size(), (64, 128), "never above max_height");
    assert!(atlas.take_stale(), "it started over");
    assert!(
        atlas.len() < 50,
        "only what came after the restart: {}",
        atlas.len()
    );
}

#[test]
fn layout_places_lines_on_whole_pixels() {
    let mut atlas = GlyphAtlas::default();
    let (one, size) = quads(&mut atlas, "Дверь открыта", HAlign::Left);
    assert_eq!(
        one.len(),
        12,
        "one quad per visible glyph, none for the space"
    );
    let font = Font::builtin();
    assert_eq!(size, font.measure("Дверь открыта", 20.0));
    for q in &one {
        assert_eq!(q.min, q.min.round(), "{q:?}");
        assert!(q.min.x >= 99.0 && q.max.x <= 100.0 + size.x + 2.0, "{q:?}");
        assert!(q.min.y >= 10.0 && q.max.y <= 10.0 + size.y, "{q:?}");
        assert!(q.uv_min.cmplt(q.uv_max).all());
    }
    // Left to right.
    assert!(one.windows(2).all(|w| w[0].min.x < w[1].min.x));

    let (two, size2) = quads(&mut atlas, "Да\nДа", HAlign::Left);
    assert_eq!(two.len(), 4);
    let step = two[2].min.y - two[0].min.y;
    assert!((step - font.line_metrics(20.0).line_height()).abs() <= 1.0);
    assert!((size2.y - 2.0 * size.y).abs() < 1e-3);

    let (centred, _) = quads(&mut atlas, "Пауза", HAlign::Center);
    let left = centred.first().unwrap().min.x - 100.0;
    let right = centred.last().unwrap().max.x - 100.0;
    assert!(
        (left + right).abs() <= 3.0,
        "centred on origin.x: {left}..{right}"
    );
}

#[test]
fn white_uv_samples_full_coverage() {
    let atlas = GlyphAtlas::default();
    let (w, h) = atlas.size();
    let uv = atlas.white_uv() * Vec2::new(w as f32, h as f32);
    // The four texels around the sample point.
    for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let (x, y) = (uv.x as u32 - 1 + x, uv.y as u32 - 1 + y);
        assert_eq!(atlas.coverage()[(y * w + x) as usize], 255);
    }
    let mut rgba = Vec::new();
    atlas.rgba(&mut rgba);
    assert_eq!(rgba.len(), (w * h * 4) as usize);
    assert_eq!(&rgba[..4], &[255, 255, 255, 255]);
    assert_eq!(&rgba[rgba.len() - 4..], &[255, 255, 255, 0]);
}

#[test]
fn absurd_sizes_do_not_panic_or_wipe_the_atlas() {
    let font = Font::builtin();
    let mut atlas = GlyphAtlas::default();
    let mut out = Vec::new();
    layout(
        &font,
        &mut atlas,
        "Wg",
        20.0,
        Vec2::ZERO,
        HAlign::Left,
        &mut out,
    );
    let _ = atlas.take_stale();
    let cached = atlas.len();
    for size in [0.0, -5.0, f32::NAN, f32::INFINITY, 1e4, 1e9] {
        out.clear();
        let s = layout(
            &font,
            &mut atlas,
            "Wg\nЯ",
            size,
            Vec2::ZERO,
            HAlign::Left,
            &mut out,
        );
        assert!(!s.x.is_nan(), "size {size}");
        // A glyph that can never fit is skipped; the others stay cached.
        assert!(!atlas.take_stale(), "size {size} restarted the atlas");
        assert!(atlas.len() >= cached);
    }
}

//! Fonts: metrics, advances and measuring. Rasterising is the atlas's job.

use std::fmt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use ab_glyph::{Font as _, FontArc, GlyphId};
use glam::Vec2;

/// Inter Regular, subset (see the crate docs).
static BUILTIN: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/Inter-Subset-Regular.ttf"
));

/// Ids keep glyphs of different fonts apart in a shared atlas.
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

#[derive(Debug)]
pub struct FontError(String);

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid font: {}", self.0)
    }
}

impl std::error::Error for FontError {}

/// Vertical metrics at one size, in pixels. `descent` is negative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

impl LineMetrics {
    /// Distance between baselines.
    pub fn line_height(&self) -> f32 {
        self.ascent - self.descent + self.line_gap
    }
}

/// A loaded font. Cheap to clone (shared data); thread-safe.
#[derive(Clone)]
pub struct Font {
    pub(crate) inner: FontArc,
    pub(crate) id: u32,
    /// Font units per em.
    units: f32,
}

impl fmt::Debug for Font {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Font").field("id", &self.id).finish()
    }
}

impl Font {
    /// A TrueType / OpenType file (`.ttf`, `.otf`).
    pub fn from_bytes(data: Vec<u8>) -> Result<Font, FontError> {
        let inner = FontArc::try_from_vec(data).map_err(|e| FontError(e.to_string()))?;
        Ok(Self::wrap(inner))
    }

    /// The embedded Inter subset. Parsed once; clones share it.
    pub fn builtin() -> Font {
        static FONT: OnceLock<Font> = OnceLock::new();
        FONT.get_or_init(|| {
            Self::wrap(FontArc::try_from_slice(BUILTIN).expect("built-in font is valid"))
        })
        .clone()
    }

    fn wrap(inner: FontArc) -> Font {
        let units = inner.units_per_em().unwrap_or(1000.0);
        Font {
            inner,
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            units,
        }
    }

    /// Pixels per font unit at em size `size`.
    pub(crate) fn scale(&self, size: f32) -> f32 {
        size / self.units
    }

    pub fn line_metrics(&self, size: f32) -> LineMetrics {
        let s = self.scale(size);
        LineMetrics {
            ascent: self.inner.ascent_unscaled() * s,
            descent: self.inner.descent_unscaled() * s,
            line_gap: self.inner.line_gap_unscaled() * s,
        }
    }

    /// Whether the font draws `c` (otherwise it shows as the "missing" box).
    pub fn has_glyph(&self, c: char) -> bool {
        self.inner.glyph_id(c).0 != 0
    }

    pub(crate) fn glyph_id(&self, c: char) -> GlyphId {
        self.inner.glyph_id(c)
    }

    /// Unscaled advance and kerning against the previous glyph, in font units.
    pub(crate) fn advance_units(&self, prev: Option<GlyphId>, id: GlyphId) -> (f32, f32) {
        let kern = prev.map_or(0.0, |p| self.inner.kern_unscaled(p, id));
        (self.inner.h_advance_unscaled(id), kern)
    }

    /// Horizontal advance of `c` at `size`, without kerning.
    pub fn advance(&self, c: char, size: f32) -> f32 {
        self.inner.h_advance_unscaled(self.glyph_id(c)) * self.scale(size)
    }

    /// Size of `text` (lines split on `\n`) at `size`: the widest line by the
    /// line height times the line count. Matches what [`layout`](crate::layout)
    /// draws, give or take the pixel snapping of each glyph.
    pub fn measure(&self, text: &str, size: f32) -> Vec2 {
        let s = self.scale(size);
        let mut widest = 0.0f32;
        let mut lines = 0;
        for line in text.split('\n') {
            lines += 1;
            let mut prev = None;
            let mut width = 0.0;
            for c in line.chars().filter(|c| !c.is_control()) {
                let id = self.glyph_id(c);
                let (advance, kern) = self.advance_units(prev, id);
                width += advance + kern;
                prev = Some(id);
            }
            widest = widest.max(width * s);
        }
        Vec2::new(widest, lines as f32 * self.line_metrics(size).line_height())
    }
}

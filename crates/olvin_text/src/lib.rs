//! Text for screen-space UI, without a GPU: a font ([`Font`], TrueType /
//! OpenType through `ab_glyph`), a glyph cache packed into one coverage
//! texture ([`GlyphAtlas`]) and line layout ([`layout`], [`Font::measure`]).
//! A renderer uploads [`GlyphAtlas::rgba`] when [`GlyphAtlas::version`]
//! changes and draws the [`GlyphQuad`]s as textured quads.
//!
//! Sizes are em sizes in pixels (like CSS `font-size`). Glyphs are placed on
//! whole pixels and sampled 1:1, so text is sharp with plain bilinear
//! filtering. Shaping is per character: no ligatures, no right-to-left, and
//! kerning only from a legacy `kern` table (the built-in font has none).
//!
//! [`Font::builtin`] is Inter 4.0 Regular cut down to Latin, Latin-1/Extended-A,
//! Cyrillic, punctuation, arrows and geometric shapes (41 KB, OFL 1.1, the
//! licence is `assets/fonts/Inter-OFL.txt`).

mod atlas;
mod font;
mod layout;

pub use atlas::{AtlasGlyph, GlyphAtlas};
pub use font::{Font, FontError, LineMetrics};
pub use layout::{GlyphQuad, HAlign, layout};

#[cfg(test)]
mod tests;

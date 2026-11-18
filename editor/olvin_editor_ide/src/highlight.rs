//! Tokens + search matches to an egui `LayoutJob`.

use std::ops::Range;

use olvin_editor::egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat};
use olvin_editor::egui::{Color32, FontId, Stroke};
use olvin_editor::theme;

use crate::lang::{Token, TokenKind};

pub fn color(kind: Option<TokenKind>) -> Color32 {
    match kind {
        None => theme::TEXT,
        Some(TokenKind::Keyword) => Color32::from_rgb(198, 146, 255),
        Some(TokenKind::Type) => Color32::from_rgb(86, 212, 190),
        Some(TokenKind::Function) => Color32::from_rgb(120, 170, 255),
        Some(TokenKind::Macro) => Color32::from_rgb(110, 200, 255),
        Some(TokenKind::Attribute) => Color32::from_rgb(229, 192, 123),
        Some(TokenKind::String) => Color32::from_rgb(152, 205, 121),
        Some(TokenKind::Number) => Color32::from_rgb(240, 160, 100),
        Some(TokenKind::Comment) => Color32::from_rgb(110, 120, 138),
        Some(TokenKind::Property) => Color32::from_rgb(140, 200, 255),
        Some(TokenKind::Heading) => theme::ACCENT,
        Some(TokenKind::Emphasis) => Color32::from_rgb(235, 200, 140),
        Some(TokenKind::Link) => Color32::from_rgb(130, 170, 255),
    }
}

pub const MATCH_BG: Color32 = Color32::from_rgba_premultiplied(90, 75, 20, 120);
pub const CURRENT_MATCH_BG: Color32 = Color32::from_rgba_premultiplied(160, 110, 20, 200);

/// Builds the job; `matches` are sorted byte ranges, `current` an index into them.
pub fn layout_job(
    text: &str,
    tokens: &[Token],
    matches: &[Range<usize>],
    current: Option<usize>,
    font: FontId,
) -> LayoutJob {
    let mut job = LayoutJob {
        text: text.to_owned(),
        ..Default::default()
    };
    job.wrap.max_width = f32::INFINITY;

    let mut cuts: Vec<usize> = Vec::with_capacity(2 + 2 * (tokens.len() + matches.len()));
    cuts.push(0);
    cuts.push(text.len());
    for t in tokens {
        cuts.push(t.start.min(text.len()));
        cuts.push(t.end.min(text.len()));
    }
    for m in matches {
        cuts.push(m.start.min(text.len()));
        cuts.push(m.end.min(text.len()));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let (mut ti, mut mi) = (0, 0);
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        while ti < tokens.len() && tokens[ti].end <= a {
            ti += 1;
        }
        while mi < matches.len() && matches[mi].end <= a {
            mi += 1;
        }
        let kind = tokens.get(ti).filter(|t| t.start <= a).map(|t| t.kind);
        let in_match = matches.get(mi).is_some_and(|m| m.start <= a);
        let mut format = TextFormat {
            font_id: font.clone(),
            color: color(kind),
            italics: kind == Some(TokenKind::Comment),
            ..Default::default()
        };
        if kind == Some(TokenKind::Link) {
            format.underline = Stroke::new(1.0, color(kind).gamma_multiply(0.5));
        }
        if in_match {
            format.background = if current == Some(mi) {
                CURRENT_MATCH_BG
            } else {
                MATCH_BG
            };
        }
        job.sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: ByteIndex(a)..ByteIndex(b),
            format,
        });
    }
    if job.sections.is_empty() {
        job.sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: ByteIndex(0)..ByteIndex(0),
            format: TextFormat::simple(font, theme::TEXT),
        });
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::{Lang, tokenize};

    #[test]
    fn sections_cover_text_with_colours() {
        let text = "fn main() { let s = \"x\"; } // done";
        let tokens = tokenize(Lang::Rust, text);
        let matches: Vec<Range<usize>> = std::iter::once(3..7).collect();
        let job = layout_job(text, &tokens, &matches, Some(0), FontId::monospace(12.0));
        let mut at = 0;
        for s in &job.sections {
            assert_eq!(s.byte_range.start.0, at);
            at = s.byte_range.end.0;
        }
        assert_eq!(at, text.len());
        let section = |needle: &str| {
            let start = text.find(needle).unwrap();
            job.sections
                .iter()
                .find(|s| s.byte_range.start.0 == start)
                .unwrap()
        };
        assert_eq!(section("fn").format.color, color(Some(TokenKind::Keyword)));
        assert_eq!(section("main").format.background, CURRENT_MATCH_BG);
        assert_eq!(
            section("\"x\"").format.color,
            color(Some(TokenKind::String))
        );
        assert!(section("// done").format.italics);
        assert_eq!(
            layout_job("", &[], &[], None, FontId::monospace(12.0))
                .sections
                .len(),
            1
        );
    }
}

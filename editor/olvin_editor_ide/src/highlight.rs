//! Tokens + search matches to an egui `LayoutJob`.

use std::ops::Range;

use olvin_editor::egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat};
use olvin_editor::egui::{Color32, FontId, Stroke};
use olvin_editor::theme;

use crate::lang::{Token, TokenKind};

/// Syntax colour on the dark or the light (`light`) editor surface.
pub fn color(kind: Option<TokenKind>, light: bool) -> Color32 {
    let pick = |dark: (u8, u8, u8), light_c: (u8, u8, u8)| {
        let (r, g, b) = if light { light_c } else { dark };
        Color32::from_rgb(r, g, b)
    };
    match kind {
        None if light => Color32::from_rgb(28, 30, 36),
        None => theme::TEXT,
        Some(TokenKind::Keyword) => pick((198, 146, 255), (130, 60, 200)),
        Some(TokenKind::Type) => pick((86, 212, 190), (0, 120, 104)),
        Some(TokenKind::Function) => pick((120, 170, 255), (30, 85, 200)),
        Some(TokenKind::Macro) => pick((110, 200, 255), (0, 105, 165)),
        Some(TokenKind::Attribute) => pick((229, 192, 123), (140, 92, 10)),
        Some(TokenKind::String) => pick((152, 205, 121), (50, 120, 30)),
        Some(TokenKind::Number) => pick((240, 160, 100), (190, 85, 15)),
        Some(TokenKind::Comment) => pick((110, 120, 138), (100, 108, 125)),
        Some(TokenKind::Property) => pick((140, 200, 255), (25, 95, 165)),
        Some(TokenKind::Heading) if light => Color32::from_rgb(0, 120, 85),
        Some(TokenKind::Heading) => theme::ACCENT,
        Some(TokenKind::Emphasis) => pick((235, 200, 140), (150, 95, 25)),
        Some(TokenKind::Link) => pick((130, 170, 255), (35, 85, 200)),
    }
}

pub const MATCH_BG: Color32 = Color32::from_rgba_premultiplied(90, 75, 20, 120);
pub const CURRENT_MATCH_BG: Color32 = Color32::from_rgba_premultiplied(160, 110, 20, 200);
const MATCH_BG_LIGHT: Color32 = Color32::from_rgba_premultiplied(120, 100, 0, 110);
const CURRENT_MATCH_BG_LIGHT: Color32 = Color32::from_rgba_premultiplied(200, 120, 0, 170);

/// Builds the job; `matches` are sorted byte ranges, `current` an index into them.
pub fn layout_job(
    text: &str,
    tokens: &[Token],
    matches: &[Range<usize>],
    current: Option<usize>,
    font: FontId,
    light: bool,
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
            color: color(kind, light),
            italics: kind == Some(TokenKind::Comment),
            ..Default::default()
        };
        if kind == Some(TokenKind::Link) {
            format.underline = Stroke::new(1.0, color(kind, light).gamma_multiply(0.5));
        }
        if in_match {
            format.background = match (current == Some(mi), light) {
                (true, false) => CURRENT_MATCH_BG,
                (false, false) => MATCH_BG,
                (true, true) => CURRENT_MATCH_BG_LIGHT,
                (false, true) => MATCH_BG_LIGHT,
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
            format: TextFormat::simple(font, color(None, light)),
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
        let job = layout_job(text, &tokens, &matches, Some(0), FontId::monospace(12.0), false);
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
        assert_eq!(section("fn").format.color, color(Some(TokenKind::Keyword), false));
        assert_eq!(section("main").format.background, CURRENT_MATCH_BG);
        assert_eq!(
            section("\"x\"").format.color,
            color(Some(TokenKind::String), false)
        );
        assert!(section("// done").format.italics);
        assert_eq!(
            layout_job("", &[], &[], None, FontId::monospace(12.0), false)
                .sections
                .len(),
            1
        );
    }
}

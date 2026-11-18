//! Pure text-editing operations used by the code editor. Selections are char
//! index ranges (what egui's `CCursor` counts), text ranges from search are
//! byte ranges.

use std::ops::Range;

pub const INDENT: &str = "    ";

/// Result of an edit: the new text and the new selection (char indices,
/// `start..end`, empty = a cursor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edited {
    pub text: String,
    pub sel: Range<usize>,
}

pub fn char_to_byte(text: &str, ci: usize) -> usize {
    text.char_indices().nth(ci).map_or(text.len(), |(b, _)| b)
}

pub fn byte_to_char(text: &str, bi: usize) -> usize {
    text[..bi.min(text.len())].chars().count()
}

fn sel_bytes(text: &str, sel: &Range<usize>) -> Range<usize> {
    let a = char_to_byte(text, sel.start.min(sel.end));
    let b = char_to_byte(text, sel.start.max(sel.end));
    a..b
}

fn line_start(text: &str, b: usize) -> usize {
    text[..b].rfind('\n').map_or(0, |p| p + 1)
}

fn line_end(text: &str, b: usize) -> usize {
    text[b..].find('\n').map_or(text.len(), |p| p + b)
}

/// Byte range of the whole lines a selection touches (without the final
/// newline). A selection ending at column 0 doesn't include that line.
fn touched_lines(text: &str, sel: &Range<usize>) -> Range<usize> {
    let r = sel_bytes(text, sel);
    let start = line_start(text, r.start);
    let mut end_at = r.end;
    if r.end > r.start && r.end > start && text.as_bytes()[r.end - 1] == b'\n' {
        end_at = r.end - 1;
    }
    start..line_end(text, end_at)
}

fn multi_line(text: &str, sel: &Range<usize>) -> bool {
    let r = sel_bytes(text, sel);
    text[r].contains('\n')
}

/// Rewrites every line in `lines` (byte range of whole lines) and returns
/// the new text with the rewritten block selected.
fn map_lines(
    text: &str,
    lines: Range<usize>,
    f: impl Fn(&str) -> String,
) -> (String, Range<usize>) {
    let block: Vec<String> = text[lines.clone()].split('\n').map(f).collect();
    let block = block.join("\n");
    let mut out = String::with_capacity(text.len() + block.len());
    out.push_str(&text[..lines.start]);
    out.push_str(&block);
    out.push_str(&text[lines.end..]);
    let start = byte_to_char(&out, lines.start);
    let end = start + block.chars().count();
    (out, start..end)
}

fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Tab: indents the selected lines, or inserts spaces up to the next tab
/// stop at the cursor.
pub fn indent(text: &str, sel: Range<usize>) -> Edited {
    if multi_line(text, &sel) {
        let lines = touched_lines(text, &sel);
        let (text, sel) = map_lines(text, lines, |l| {
            if l.trim().is_empty() {
                l.to_owned()
            } else {
                format!("{INDENT}{l}")
            }
        });
        return Edited { text, sel };
    }
    let r = sel_bytes(text, &sel);
    let col = text[line_start(text, r.start)..r.start].chars().count();
    let n = INDENT.len() - col % INDENT.len();
    let mut out = String::with_capacity(text.len() + n);
    out.push_str(&text[..r.start]);
    out.extend(std::iter::repeat_n(' ', n));
    out.push_str(&text[r.end..]);
    let c = sel.start.min(sel.end) + n;
    Edited {
        text: out,
        sel: c..c,
    }
}

/// Shift+Tab: removes one indentation level from the touched lines.
pub fn outdent(text: &str, sel: Range<usize>) -> Edited {
    let strip = |l: &str| -> usize {
        if l.starts_with('\t') {
            1
        } else {
            l.bytes()
                .take(INDENT.len())
                .take_while(|&b| b == b' ')
                .count()
        }
    };
    let lines = touched_lines(text, &sel);
    if multi_line(text, &sel) {
        let (text, sel) = map_lines(text, lines, |l| l[strip(l)..].to_owned());
        return Edited { text, sel };
    }
    let line = &text[lines.clone()];
    let n = strip(line);
    let out = format!(
        "{}{}{}",
        &text[..lines.start],
        &line[n..],
        &text[lines.end..]
    );
    let ls = byte_to_char(text, lines.start);
    let shift = |c: usize| if c >= ls + n { c - n } else { ls };
    Edited {
        text: out,
        sel: shift(sel.start.min(sel.end))..shift(sel.start.max(sel.end)),
    }
}

/// Ctrl+/: comments the touched lines out with `prefix`, or uncomments them
/// if every non-blank line already is.
pub fn toggle_comment(text: &str, sel: Range<usize>, prefix: &str) -> Edited {
    let lines = touched_lines(text, &sel);
    let block = &text[lines.clone()];
    let non_blank: Vec<&str> = block.split('\n').filter(|l| !l.trim().is_empty()).collect();
    if non_blank.is_empty() {
        return Edited {
            text: text.to_owned(),
            sel,
        };
    }
    let commented = non_blank.iter().all(|l| l.trim_start().starts_with(prefix));
    let min_indent = non_blank
        .iter()
        .map(|l| indent_of(l).len())
        .min()
        .unwrap_or(0);
    let rewrite = |l: &str| -> String {
        if l.trim().is_empty() {
            return l.to_owned();
        }
        if commented {
            let ind = indent_of(l);
            let rest = &l[ind.len() + prefix.len()..];
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            format!("{ind}{rest}")
        } else {
            format!("{}{prefix} {}", &l[..min_indent], &l[min_indent..])
        }
    };
    if sel.start == sel.end && !block.contains('\n') {
        // A cursor stays where it was relative to the text.
        let new_line = rewrite(block);
        let delta = new_line.chars().count() as isize - block.chars().count() as isize;
        let out = format!("{}{new_line}{}", &text[..lines.start], &text[lines.end..]);
        let ls = byte_to_char(text, lines.start) as isize;
        let c = (sel.start as isize + delta).max(ls) as usize;
        return Edited {
            text: out,
            sel: c..c,
        };
    }
    let (text, sel) = map_lines(text, lines, rewrite);
    Edited { text, sel }
}

pub fn closer(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        _ => None,
    }
}

/// Enter: newline keeping the current indentation, one level deeper after an
/// opening bracket; between a bracket pair the closer moves to its own line.
pub fn newline(text: &str, sel: Range<usize>) -> Edited {
    let r = sel_bytes(text, &sel);
    let ls = line_start(text, r.start);
    let before = &text[ls..r.start];
    let base = indent_of(before).to_owned();
    let last = before.trim_end().chars().last();
    let opens = matches!(last, Some('{' | '[' | '('));
    let next = text[r.end..].trim_start_matches([' ', '\t']).chars().next();
    let closes = opens && next.is_some_and(|n| Some(n) == last.and_then(closer));

    let mut insert = format!("\n{base}");
    if opens {
        insert.push_str(INDENT);
    }
    let cursor_in_insert = insert.chars().count();
    if closes {
        insert.push('\n');
        insert.push_str(&base);
    }
    // Drop trailing spaces before the cursor and leading ones after it.
    let cut_before = before.len() - before.trim_end_matches([' ', '\t']).len();
    let start = if before.trim().is_empty() {
        r.start
    } else {
        r.start - cut_before
    };
    let end = if closes || opens {
        r.end + (text[r.end..].len() - text[r.end..].trim_start_matches([' ', '\t']).len())
    } else {
        r.end
    };
    let out = format!("{}{insert}{}", &text[..start], &text[end..]);
    let c = byte_to_char(text, start) + cursor_in_insert;
    Edited {
        text: out,
        sel: c..c,
    }
}

/// Typing a bracket or quote: pairs it, wraps a selection or steps over an
/// existing closer. `None` = type the char normally.
pub fn type_char(text: &str, sel: Range<usize>, ch: char) -> Option<Edited> {
    let r = sel_bytes(text, &sel);
    let (lo, hi) = (sel.start.min(sel.end), sel.start.max(sel.end));
    let next = text[r.end..].chars().next();
    let prev = text[..r.start].chars().next_back();
    let wrap = |close: char| {
        let out = format!(
            "{}{ch}{}{close}{}",
            &text[..r.start],
            &text[r.clone()],
            &text[r.end..]
        );
        Edited {
            text: out,
            sel: lo + 1..hi + 1,
        }
    };
    let insert_pair = |close: char| Edited {
        text: format!("{}{ch}{close}{}", &text[..r.start], &text[r.end..]),
        sel: lo + 1..lo + 1,
    };
    let step_over = || Edited {
        text: text.to_owned(),
        sel: lo + 1..lo + 1,
    };
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    match ch {
        '(' | '[' | '{' => {
            let close = closer(ch)?;
            if lo != hi {
                Some(wrap(close))
            } else if next.is_none_or(|n| n.is_whitespace() || ")]};,".contains(n)) {
                Some(insert_pair(close))
            } else {
                None
            }
        }
        ')' | ']' | '}' => (lo == hi && next == Some(ch)).then(step_over),
        '"' => {
            if lo != hi {
                Some(wrap('"'))
            } else if next == Some('"') {
                Some(step_over())
            } else if is_word(prev) || prev == Some('\\') || is_word(next) {
                None
            } else {
                Some(insert_pair('"'))
            }
        }
        _ => None,
    }
}

/// Backspace between an empty pair `(|)` deletes both halves.
pub fn backspace_pair(text: &str, sel: Range<usize>) -> Option<Edited> {
    if sel.start != sel.end || sel.start == 0 {
        return None;
    }
    let b = char_to_byte(text, sel.start);
    let prev = text[..b].chars().next_back()?;
    let next = text[b..].chars().next()?;
    if closer(prev) != Some(next) {
        return None;
    }
    let out = format!(
        "{}{}",
        &text[..b - prev.len_utf8()],
        &text[b + next.len_utf8()..]
    );
    Some(Edited {
        text: out,
        sel: sel.start - 1..sel.start - 1,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
}

fn word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// Every match of `query` (byte ranges, non-overlapping). Case-insensitive
/// search folds ASCII only.
pub fn find_all(text: &str, query: &str, opts: FindOptions) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let (t, q) = (text.as_bytes(), query.as_bytes());
    let mut i = 0;
    while i + q.len() <= t.len() {
        let hit = if opts.case_sensitive {
            &t[i..i + q.len()] == q
        } else {
            t[i..i + q.len()].eq_ignore_ascii_case(q)
        };
        let boundaries = text.is_char_boundary(i) && text.is_char_boundary(i + q.len());
        let word_ok = !opts.whole_word
            || ((i == 0 || !word_byte(t[i - 1]))
                && t.get(i + q.len()).is_none_or(|&b| !word_byte(b)));
        if hit && boundaries && word_ok {
            out.push(i..i + q.len());
            i += q.len();
        } else {
            i += 1;
        }
    }
    out
}

/// Replaces every match; returns the new text and the number of replacements.
pub fn replace_all(
    text: &str,
    query: &str,
    replacement: &str,
    opts: FindOptions,
) -> (String, usize) {
    let matches = find_all(text, query, opts);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in &matches {
        out.push_str(&text[last..m.start]);
        out.push_str(replacement);
        last = m.end;
    }
    out.push_str(&text[last..]);
    (out, matches.len())
}

/// The match to select next: the first one starting at or after `from_byte`
/// (wrapping around), or the last one before it when going backwards.
pub fn next_match(matches: &[Range<usize>], from_byte: usize, backwards: bool) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    if backwards {
        Some(
            matches
                .iter()
                .rposition(|m| m.end <= from_byte && m.start < from_byte)
                .unwrap_or(matches.len() - 1),
        )
    } else {
        Some(
            matches
                .iter()
                .position(|m| m.start >= from_byte)
                .unwrap_or(0),
        )
    }
}

/// 1-based line and column to a char index (clamped).
pub fn line_col_to_char(text: &str, line: usize, col: usize) -> usize {
    let mut ci = 0;
    for (i, l) in text.split('\n').enumerate() {
        let len = l.chars().count();
        if i + 1 == line.max(1) {
            return ci + (col.max(1) - 1).min(len);
        }
        ci += len + 1;
    }
    text.chars().count()
}

/// Char index to 1-based (line, column).
pub fn char_to_line_col(text: &str, ci: usize) -> (usize, usize) {
    let b = char_to_byte(text, ci);
    let line = text[..b].matches('\n').count() + 1;
    let col = text[line_start(text, b)..b].chars().count() + 1;
    (line, col)
}

/// The identifier around a char index (byte range), for Ctrl+D.
pub fn word_at(text: &str, ci: usize) -> Option<Range<usize>> {
    let b = char_to_byte(text, ci);
    let s = text.as_bytes();
    let mut a = b;
    while a > 0 && word_byte(s[a - 1]) {
        a -= 1;
    }
    let mut e = b;
    while e < s.len() && word_byte(s[e]) {
        e += 1;
    }
    while !text.is_char_boundary(a) {
        a += 1;
    }
    while !text.is_char_boundary(e) {
        e -= 1;
    }
    (e > a).then_some(a..e)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `|` marks the cursor, `[` `]` a selection.
    fn parse(marked: &str) -> (String, Range<usize>) {
        let mut text = String::new();
        let (mut a, mut b) = (None, None);
        for ch in marked.chars() {
            match ch {
                '|' => {
                    a = Some(text.chars().count());
                    b = a;
                }
                '[' if a.is_none() => a = Some(text.chars().count()),
                ']' if b.is_none() => b = Some(text.chars().count()),
                c => text.push(c),
            }
        }
        (text, a.unwrap()..b.unwrap())
    }

    fn show(e: &Edited) -> String {
        let mut out = String::new();
        for (i, ch) in e.text.chars().chain(std::iter::once('\0')).enumerate() {
            if e.sel.start == e.sel.end && i == e.sel.start {
                out.push('|');
            } else if i == e.sel.start {
                out.push('[');
            } else if i == e.sel.end {
                out.push(']');
            }
            if ch != '\0' {
                out.push(ch);
            }
        }
        out
    }

    fn run(f: impl Fn(&str, Range<usize>) -> Edited, marked: &str) -> String {
        let (t, s) = parse(marked);
        show(&f(&t, s))
    }

    #[test]
    fn indent_and_outdent() {
        assert_eq!(run(indent, "ab|c"), "ab  |c");
        assert_eq!(run(indent, "|x"), "    |x");
        assert_eq!(
            run(indent, "a[a\nbb\n\ncc]\ndd"),
            "[    aa\n    bb\n\n    cc]\ndd"
        );
        // Ending at column 0 leaves that line alone.
        assert_eq!(run(indent, "[a\n]b"), "[    a]\nb");
        assert_eq!(run(outdent, "[    a\n  b\n\tc\nd]"), "[a\nb\nc\nd]");
        assert_eq!(run(outdent, "      x|y"), "  x|y");
        assert_eq!(run(outdent, "  |x"), "|x");
        assert_eq!(run(outdent, "x|"), "x|");
    }

    #[test]
    fn comment_toggle() {
        let c = |m: &str| run(|t, s| toggle_comment(t, s, "//"), m);
        assert_eq!(c("    let| x = 1;"), "    // let| x = 1;");
        assert_eq!(c("    // let| x = 1;"), "    let| x = 1;");
        assert_eq!(c("    //let x|"), "    let x|");
        assert_eq!(
            c("[fn a() {\n    b();\n\n}]"),
            "[// fn a() {\n//     b();\n\n// }]"
        );
        assert_eq!(c("[    // a\n    // b]"), "[    a\n    b]");
        // Mixed: comments everything (again).
        assert_eq!(c("[  // a\n  b]"), "[  // // a\n  // b]");
        assert_eq!(c("  |"), "  |");
        assert_eq!(
            run(|t, s| toggle_comment(t, s, "#"), "name = 1|"),
            "# name = 1|"
        );
    }

    #[test]
    fn enter_indents() {
        assert_eq!(run(newline, "    foo();|"), "    foo();\n    |");
        assert_eq!(run(newline, "fn a() {|}"), "fn a() {\n    |\n}");
        assert_eq!(run(newline, "    call(|"), "    call(\n        |");
        assert_eq!(run(newline, "  a|b"), "  a\n  |b");
        assert_eq!(run(newline, "  a[bc]d"), "  a\n  |d");
        assert_eq!(run(newline, "x {   |"), "x {\n    |");
    }

    #[test]
    fn bracket_pairs() {
        let t = |m: &str, ch| {
            let (text, s) = parse(m);
            type_char(&text, s, ch).map(|e| show(&e))
        };
        assert_eq!(t("foo|", '(').as_deref(), Some("foo(|)"));
        assert_eq!(t("|bar", '(').as_deref(), None);
        assert_eq!(t("a[bc]d", '[').as_deref(), Some("a[[bc]]d"));
        assert_eq!(t("f(|)", ')').as_deref(), Some("f()|"));
        assert_eq!(t("f(|x", ')').as_deref(), None);
        assert_eq!(t("let s = |;", '"').as_deref(), Some("let s = \"|\";"));
        assert_eq!(t("\"abc|\"", '"').as_deref(), Some("\"abc\"|"));
        assert_eq!(t("abc|", '"').as_deref(), None);
        assert_eq!(t("x|", 'y').as_deref(), None);
        let (text, s) = parse("f(|)");
        assert_eq!(show(&backspace_pair(&text, s).unwrap()), "f|");
        let (text, s) = parse("f(|x)");
        assert!(backspace_pair(&text, s).is_none());
    }

    #[test]
    fn find_and_replace() {
        let text = "let foo = Foo::new(); foo_bar(foo); фу foo";
        let ci = FindOptions::default();
        assert_eq!(find_all(text, "foo", ci).len(), 5);
        let cs = FindOptions {
            case_sensitive: true,
            ..ci
        };
        assert_eq!(find_all(text, "foo", cs).len(), 4);
        let ww = FindOptions {
            whole_word: true,
            case_sensitive: true,
        };
        let m = find_all(text, "foo", ww);
        assert_eq!(m.len(), 3);
        assert_eq!(&text[m[2].clone()], "foo");
        assert!(find_all(text, "", ci).is_empty());
        assert_eq!(find_all("aaaa", "aa", ci), vec![0..2, 2..4]);

        let (out, n) = replace_all(text, "foo", "bar", ww);
        assert_eq!(n, 3);
        assert_eq!(out, "let bar = Foo::new(); foo_bar(bar); фу bar");

        let m = find_all("x a x a x", "x", ci);
        assert_eq!(next_match(&m, 1, false), Some(1));
        assert_eq!(next_match(&m, 9, false), Some(0));
        assert_eq!(next_match(&m, 4, true), Some(0));
        assert_eq!(next_match(&m, 0, true), Some(2));
        assert_eq!(next_match(&[], 0, true), None);
    }

    #[test]
    fn positions() {
        let text = "ab\nцde\n\nf";
        assert_eq!(line_col_to_char(text, 1, 1), 0);
        assert_eq!(line_col_to_char(text, 2, 2), 4);
        assert_eq!(line_col_to_char(text, 2, 99), 6);
        assert_eq!(line_col_to_char(text, 4, 1), 8);
        assert_eq!(line_col_to_char(text, 99, 1), 9);
        assert_eq!(char_to_line_col(text, 4), (2, 2));
        assert_eq!(char_to_line_col(text, 8), (4, 1));
        assert_eq!(word_at("let foo_1 = 2", 6), Some(4..9));
        assert_eq!(word_at("a  b", 2), None);
        assert_eq!(byte_to_char("цx", 2), 1);
        assert_eq!(char_to_byte("цx", 1), 2);
    }
}

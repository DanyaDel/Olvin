//! Small hand-written tokenizers for syntax highlighting: Rust, WGSL, Lua,
//! RON, TOML and Markdown. They only find the interesting spans (keywords,
//! strings...); everything else is plain text. Spans are byte ranges that
//! always fall on char boundaries.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Wgsl,
    Lua,
    Ron,
    Toml,
    Markdown,
    Plain,
}

impl Lang {
    pub fn from_path(path: &Path) -> Self {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "rs" => Self::Rust,
            "wgsl" => Self::Wgsl,
            "lua" => Self::Lua,
            "ron" => Self::Ron,
            "toml" => Self::Toml,
            "md" | "markdown" => Self::Markdown,
            _ if path.file_name().is_some_and(|n| n == "Cargo.lock") => Self::Toml,
            _ => Self::Plain,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Wgsl => "WGSL",
            Self::Lua => "Lua",
            Self::Ron => "RON",
            Self::Toml => "TOML",
            Self::Markdown => "Markdown",
            Self::Plain => "Plain Text",
        }
    }

    /// Line comment marker used by "Toggle Comment".
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Self::Rust | Self::Wgsl | Self::Ron => Some("//"),
            Self::Lua => Some("--"),
            Self::Toml => Some("#"),
            Self::Markdown | Self::Plain => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Keyword,
    Type,
    Function,
    Macro,
    Attribute,
    String,
    Number,
    Comment,
    /// Struct fields in RON, keys in TOML.
    Property,
    /// TOML table headers, Markdown headings.
    Heading,
    /// Markdown emphasis.
    Emphasis,
    /// Markdown links.
    Link,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

pub fn tokenize(lang: Lang, text: &str) -> Vec<Token> {
    match lang {
        Lang::Rust => CLike::new(&RUST, text).run(),
        Lang::Wgsl => CLike::new(&WGSL, text).run(),
        Lang::Ron => CLike::new(&RON, text).run(),
        Lang::Lua => lua(text),
        Lang::Toml => toml(text),
        Lang::Markdown => markdown(text),
        Lang::Plain => Vec::new(),
    }
}

// --- C-like languages ---------------------------------------------------

struct Spec {
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    /// `r"..."`, `b"..."`, char literals vs lifetimes.
    rust_literals: bool,
    /// `#[...]` / `#![...]`.
    hash_attributes: bool,
    /// `@vertex`, `@location(0)`.
    at_attributes: bool,
    /// `name!(...)`.
    macros: bool,
    /// `name:` is a field (RON).
    fields: bool,
    /// WGSL builtin types like `vec3f`, `mat4x4`, `texture_2d`.
    wgsl_types: bool,
}

const RUST: Spec = Spec {
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true",
        "type", "unsafe", "use", "where", "while", "yield", "gen",
    ],
    types: &[
        "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32",
        "i64", "i128", "isize", "f32", "f64", "Self",
    ],
    rust_literals: true,
    hash_attributes: true,
    at_attributes: false,
    macros: true,
    fields: false,
    wgsl_types: false,
};

const WGSL: Spec = Spec {
    keywords: &[
        "alias",
        "break",
        "case",
        "const",
        "const_assert",
        "continue",
        "continuing",
        "default",
        "diagnostic",
        "discard",
        "else",
        "enable",
        "false",
        "fn",
        "for",
        "if",
        "let",
        "loop",
        "override",
        "requires",
        "return",
        "struct",
        "switch",
        "true",
        "var",
        "while",
        "function",
        "private",
        "workgroup",
        "uniform",
        "storage",
        "read",
        "write",
        "read_write",
    ],
    types: &[
        "bool",
        "f16",
        "f32",
        "i32",
        "u32",
        "array",
        "atomic",
        "ptr",
        "sampler",
        "sampler_comparison",
    ],
    rust_literals: false,
    hash_attributes: false,
    at_attributes: true,
    macros: false,
    fields: false,
    wgsl_types: true,
};

const RON: Spec = Spec {
    keywords: &["true", "false", "Some", "None"],
    types: &[],
    rust_literals: true,
    hash_attributes: true,
    at_attributes: false,
    macros: false,
    fields: true,
    wgsl_types: false,
};

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

fn wgsl_builtin_type(word: &str) -> bool {
    let vec_mat = |rest: &str| {
        let rest = rest.trim_end_matches(['f', 'h', 'i', 'u']);
        !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit() || b == b'x')
    };
    word.strip_prefix("vec").is_some_and(vec_mat)
        || word.strip_prefix("mat").is_some_and(vec_mat)
        || word.starts_with("texture_")
}

struct CLike<'a> {
    spec: &'a Spec,
    s: &'a [u8],
    text: &'a str,
    i: usize,
    out: Vec<Token>,
}

impl<'a> CLike<'a> {
    fn new(spec: &'a Spec, text: &'a str) -> Self {
        Self {
            spec,
            s: text.as_bytes(),
            text,
            i: 0,
            out: Vec::new(),
        }
    }

    fn peek(&self, off: usize) -> u8 {
        self.s.get(self.i + off).copied().unwrap_or(0)
    }

    fn push(&mut self, start: usize, kind: TokenKind) {
        if self.i > start {
            self.out.push(Token {
                start,
                end: self.i,
                kind,
            });
        }
    }

    fn run(mut self) -> Vec<Token> {
        while self.i < self.s.len() {
            let start = self.i;
            let c = self.s[self.i];
            match c {
                b'/' if self.peek(1) == b'/' => {
                    self.skip_line();
                    self.push(start, TokenKind::Comment);
                }
                b'/' if self.peek(1) == b'*' => {
                    self.block_comment();
                    self.push(start, TokenKind::Comment);
                }
                b'"' => {
                    self.i += 1;
                    self.quoted(b'"');
                    self.push(start, TokenKind::String);
                }
                b'\'' if self.spec.rust_literals => self.apostrophe(),
                b'#' if self.spec.hash_attributes
                    && (self.peek(1) == b'[' || (self.peek(1) == b'!' && self.peek(2) == b'[')) =>
                {
                    self.attribute();
                    self.push(start, TokenKind::Attribute);
                }
                b'@' if self.spec.at_attributes && is_ident_start(self.peek(1)) => {
                    self.i += 1;
                    self.ident_end();
                    self.push(start, TokenKind::Attribute);
                }
                b'0'..=b'9' => {
                    self.number();
                    self.push(start, TokenKind::Number);
                }
                c if is_ident_start(c) => self.word(),
                _ => self.i += 1,
            }
        }
        self.out
    }

    fn skip_line(&mut self) {
        while self.i < self.s.len() && self.s[self.i] != b'\n' {
            self.i += 1;
        }
    }

    fn block_comment(&mut self) {
        self.i += 2;
        let mut depth = 1;
        while self.i < self.s.len() && depth > 0 {
            if self.peek(0) == b'/' && self.peek(1) == b'*' {
                depth += 1;
                self.i += 2;
            } else if self.peek(0) == b'*' && self.peek(1) == b'/' {
                depth -= 1;
                self.i += 2;
            } else {
                self.i += 1;
            }
        }
    }

    /// After the opening quote: up to and including the closing one.
    fn quoted(&mut self, quote: u8) {
        while self.i < self.s.len() {
            let c = self.s[self.i];
            self.i += 1;
            if c == b'\\' {
                self.i = (self.i + 1).min(self.s.len());
                // Stay on a char boundary after escaping a multi-byte char.
                while self.i < self.s.len() && !self.text.is_char_boundary(self.i) {
                    self.i += 1;
                }
            } else if c == quote {
                return;
            }
        }
    }

    /// `r"..."`, `r#"..."#`, `br"..."`; `self.i` is on the `r`.
    fn raw_string(&mut self) -> bool {
        let mut j = self.i + 1;
        let mut hashes = 0;
        while self.s.get(j) == Some(&b'#') {
            hashes += 1;
            j += 1;
        }
        if self.s.get(j) != Some(&b'"') {
            return false;
        }
        j += 1;
        while j < self.s.len() {
            if self.s[j] == b'"'
                && self.s[j + 1..]
                    .iter()
                    .take(hashes)
                    .filter(|&&b| b == b'#')
                    .count()
                    == hashes
            {
                self.i = j + 1 + hashes;
                return true;
            }
            j += 1;
        }
        self.i = self.s.len();
        true
    }

    /// Char literal, byte literal or lifetime.
    fn apostrophe(&mut self) {
        let start = self.i;
        self.i += 1;
        if self.peek(0) == b'\\' {
            let limit = (self.i + 12).min(self.s.len());
            while self.i < limit && self.s[self.i] != b'\'' {
                self.i += 1;
            }
            self.i = (self.i + 1).min(self.s.len());
            self.push(start, TokenKind::String);
            return;
        }
        let ch_len = self.text[self.i..].chars().next().map_or(0, char::len_utf8);
        if ch_len > 0 && self.s.get(self.i + ch_len) == Some(&b'\'') {
            self.i += ch_len + 1;
            self.push(start, TokenKind::String);
        } else if is_ident_start(self.peek(0)) {
            self.ident_end();
            self.push(start, TokenKind::Keyword); // lifetime / label
        }
    }

    fn attribute(&mut self) {
        let mut depth = 0;
        while self.i < self.s.len() {
            match self.s[self.i] {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        self.i += 1;
                        return;
                    }
                }
                b'"' => {
                    self.i += 1;
                    self.quoted(b'"');
                    continue;
                }
                _ => {}
            }
            self.i += 1;
        }
    }

    fn number(&mut self) {
        let hex = self.peek(0) == b'0' && matches!(self.peek(1), b'x' | b'X');
        loop {
            let c = self.peek(0);
            if is_ident(c) && c < 0x80 {
                let exp = !hex && matches!(c, b'e' | b'E') && matches!(self.peek(1), b'+' | b'-');
                self.i += if exp { 2 } else { 1 };
            } else if c == b'.' && self.peek(1).is_ascii_digit() {
                self.i += 1;
            } else if c == b'.'
                && self.spec.wgsl_types
                && !is_ident_start(self.peek(1))
                && self.peek(1) != b'.'
            {
                self.i += 1; // WGSL `1.`
            } else {
                break;
            }
        }
    }

    fn ident_end(&mut self) {
        while self.i < self.s.len() && is_ident(self.s[self.i]) {
            self.i += 1;
        }
    }

    fn word(&mut self) {
        let start = self.i;
        if self.spec.rust_literals {
            let c = self.s[self.i];
            if (c == b'r' && matches!(self.peek(1), b'"' | b'#'))
                || (c == b'b' && self.peek(1) == b'r' && matches!(self.peek(2), b'"' | b'#'))
            {
                if c == b'b' {
                    self.i += 1;
                }
                if self.raw_string() {
                    self.push(start, TokenKind::String);
                    return;
                }
                self.i = start;
            }
            if c == b'b' && self.peek(1) == b'"' {
                self.i += 2;
                self.quoted(b'"');
                self.push(start, TokenKind::String);
                return;
            }
            if c == b'b' && self.peek(1) == b'\'' {
                self.i += 2;
                self.quoted(b'\'');
                self.push(start, TokenKind::String);
                return;
            }
        }
        self.ident_end();
        let word = &self.text[start..self.i];
        let next = self.peek(0);
        let kind = if self.spec.keywords.contains(&word) {
            Some(TokenKind::Keyword)
        } else if self.spec.macros && next == b'!' && self.peek(1) != b'=' {
            self.i += 1;
            Some(TokenKind::Macro)
        } else if self.spec.types.contains(&word)
            || (self.spec.wgsl_types && wgsl_builtin_type(word))
        {
            Some(TokenKind::Type)
        } else if self.spec.fields && self.field_colon() {
            Some(TokenKind::Property)
        } else if word.as_bytes()[0].is_ascii_uppercase() {
            Some(TokenKind::Type)
        } else if next == b'(' || (next == b':' && self.peek(1) == b':' && self.peek(2) == b'<') {
            Some(TokenKind::Function)
        } else {
            None
        };
        if let Some(kind) = kind {
            self.push(start, kind);
        }
    }

    /// RON field: `name:` but not `path::`.
    fn field_colon(&self) -> bool {
        let mut j = self.i;
        while matches!(self.s.get(j), Some(b' ' | b'\t')) {
            j += 1;
        }
        self.s.get(j) == Some(&b':') && self.s.get(j + 1) != Some(&b':')
    }
}

// --- Lua ------------------------------------------------------------------

const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// `[[`, `[=[`... at `i`: the level and the index after the opener.
fn lua_long_open(s: &[u8], i: usize) -> Option<(usize, usize)> {
    if s.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while s.get(j) == Some(&b'=') {
        j += 1;
    }
    (s.get(j) == Some(&b'[')).then_some((j - i - 1, j + 1))
}

/// End of a long bracket body that started with `level` `=`s.
fn lua_long_end(s: &[u8], mut i: usize, level: usize) -> usize {
    while i < s.len() {
        if s[i] == b']' {
            let mut j = i + 1;
            while s.get(j) == Some(&b'=') {
                j += 1;
            }
            if j - i - 1 == level && s.get(j) == Some(&b']') {
                return j + 1;
            }
        }
        i += 1;
    }
    s.len()
}

fn lua(text: &str) -> Vec<Token> {
    let s = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut push = |start: usize, end: usize, kind| {
        if end > start {
            out.push(Token { start, end, kind });
        }
    };
    while i < s.len() {
        let start = i;
        match s[i] {
            b'-' if s.get(i + 1) == Some(&b'-') => {
                i += 2;
                if let Some((level, body)) = lua_long_open(s, i) {
                    i = lua_long_end(s, body, level);
                } else {
                    while i < s.len() && s[i] != b'\n' {
                        i += 1;
                    }
                }
                push(start, i, TokenKind::Comment);
            }
            b'[' if lua_long_open(s, i).is_some() => {
                let (level, body) = lua_long_open(s, i).unwrap_or((0, i + 1));
                i = lua_long_end(s, body, level);
                push(start, i, TokenKind::String);
            }
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < s.len() && s[i] != q && s[i] != b'\n' {
                    i += if s[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(s.len());
                while i < s.len() && !text.is_char_boundary(i) {
                    i += 1;
                }
                push(start, i, TokenKind::String);
            }
            b'0'..=b'9' => {
                while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == b'.') {
                    i += 1;
                }
                push(start, i, TokenKind::Number);
            }
            c if is_ident_start(c) => {
                while i < s.len() && is_ident(s[i]) {
                    i += 1;
                }
                let word = &text[start..i];
                let kind = if LUA_KEYWORDS.contains(&word) {
                    Some(TokenKind::Keyword)
                } else if matches!(s.get(i), Some(b'(' | b'"' | b'\'' | b'{')) {
                    Some(TokenKind::Function)
                } else {
                    None
                };
                if let Some(kind) = kind {
                    push(start, i, kind);
                }
            }
            _ => i += 1,
        }
    }
    out
}

// --- TOML -----------------------------------------------------------------

fn toml(text: &str) -> Vec<Token> {
    let s = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    let push = |out: &mut Vec<Token>, start, end, kind| {
        if end > start {
            out.push(Token { start, end, kind });
        }
    };
    while i < s.len() {
        let c = s[i];
        let start = i;
        if c == b'\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            continue;
        }
        let at_line_start = std::mem::replace(&mut line_start, false);
        match c {
            b'#' => {
                while i < s.len() && s[i] != b'\n' {
                    i += 1;
                }
                push(&mut out, start, i, TokenKind::Comment);
            }
            b'[' if at_line_start => {
                while i < s.len() && s[i] != b'\n' && s[i] != b'#' {
                    i += 1;
                }
                let end = start + text[start..i].trim_end().len();
                push(&mut out, start, end, TokenKind::Heading);
            }
            b'"' | b'\'' => {
                let triple = s.get(i + 1) == Some(&c) && s.get(i + 2) == Some(&c);
                if triple {
                    i += 3;
                    while i < s.len()
                        && !(s[i] == c && s.get(i + 1) == Some(&c) && s.get(i + 2) == Some(&c))
                    {
                        if c == b'"' && s[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                    i = (i + 3).min(s.len());
                } else {
                    i += 1;
                    while i < s.len() && s[i] != c && s[i] != b'\n' {
                        if c == b'"' && s[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                    i = (i + 1).min(s.len());
                }
                while i < s.len() && !text.is_char_boundary(i) {
                    i += 1;
                }
                // A quoted key: `"a b" = 1`.
                let kind = if toml_eq_follows(s, i) {
                    TokenKind::Property
                } else {
                    TokenKind::String
                };
                push(&mut out, start, i, kind);
            }
            b'0'..=b'9' | b'+' | b'-'
                if c.is_ascii_digit() || s.get(i + 1).is_some_and(u8::is_ascii_digit) =>
            {
                i += 1;
                while i < s.len()
                    && (s[i].is_ascii_alphanumeric()
                        || matches!(s[i], b'_' | b'.' | b':' | b'-' | b'+'))
                {
                    i += 1;
                }
                push(&mut out, start, i, TokenKind::Number);
            }
            c if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' => {
                while i < s.len()
                    && (s[i].is_ascii_alphanumeric() || matches!(s[i], b'_' | b'-' | b'.'))
                {
                    i += 1;
                }
                let word = &text[start..i];
                let kind = if toml_eq_follows(s, i) {
                    Some(TokenKind::Property)
                } else if matches!(word, "true" | "false") {
                    Some(TokenKind::Keyword)
                } else if matches!(word, "inf" | "nan") {
                    Some(TokenKind::Number)
                } else {
                    None
                };
                if let Some(kind) = kind {
                    push(&mut out, start, i, kind);
                }
            }
            _ => i += 1,
        }
        while i < s.len() && !text.is_char_boundary(i) {
            i += 1;
        }
    }
    out
}

fn toml_eq_follows(s: &[u8], mut i: usize) -> bool {
    while matches!(s.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    s.get(i) == Some(&b'=') && s.get(i + 1) != Some(&b'=')
}

// --- Markdown -------------------------------------------------------------

fn markdown(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let body = line.trim_end_matches(['\n', '\r']);
        let end = start + body.len();
        let trimmed = body.trim_start();
        let indent = body.len() - trimmed.len();
        if let Some(f) = fence {
            out.push(Token {
                start,
                end,
                kind: TokenKind::String,
            });
            if trimmed.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(&trimmed[..3]);
            out.push(Token {
                start,
                end,
                kind: TokenKind::String,
            });
            continue;
        }
        let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
        let after = &trimmed[hashes..];
        if (1..=6).contains(&hashes) && (after.is_empty() || after.starts_with(' ')) {
            out.push(Token {
                start,
                end,
                kind: TokenKind::Heading,
            });
            continue;
        }
        if trimmed.starts_with('>') {
            out.push(Token {
                start,
                end,
                kind: TokenKind::Comment,
            });
            continue;
        }
        let mut inline_from = start + indent;
        let marker = if trimmed.starts_with(['-', '*', '+']) && trimmed[1..].starts_with(' ') {
            1
        } else {
            let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0 && trimmed[digits..].starts_with(". ") {
                digits + 1
            } else {
                0
            }
        };
        if marker > 0 {
            out.push(Token {
                start: inline_from,
                end: inline_from + marker,
                kind: TokenKind::Keyword,
            });
            inline_from += marker;
        }
        markdown_inline(text, inline_from, end, &mut out);
    }
    out
}

fn markdown_inline(text: &str, from: usize, to: usize, out: &mut Vec<Token>) {
    let s = text.as_bytes();
    let mut i = from;
    let find = |needle: &str, from: usize| text[from..to].find(needle).map(|p| p + from);
    while i < to {
        let c = s[i];
        let found = match c {
            b'`' => find("`", i + 1).map(|e| (e + 1, TokenKind::String)),
            b'*' | b'_' => {
                let double = s.get(i + 1) == Some(&c);
                let delim = if double {
                    &text[i..i + 2]
                } else {
                    &text[i..i + 1]
                };
                let open = i + delim.len();
                let left_ok = c == b'*' || i == 0 || !s[i - 1].is_ascii_alphanumeric();
                if left_ok && open < to && s[open] != b' ' {
                    find(delim, open)
                        .filter(|&e| e > open)
                        .map(|e| (e + delim.len(), TokenKind::Emphasis))
                } else {
                    None
                }
            }
            b'[' => find("](", i + 1)
                .and_then(|m| find(")", m + 2))
                .map(|e| (e + 1, TokenKind::Link)),
            b'<' if text[i..to].starts_with("<!--") => {
                Some((find("-->", i + 4).map_or(to, |e| e + 3), TokenKind::Comment))
            }
            _ => None,
        };
        match found {
            Some((end, kind)) => {
                out.push(Token {
                    start: i,
                    end,
                    kind,
                });
                i = end;
            }
            None => {
                i += 1;
                while i < to && !text.is_char_boundary(i) {
                    i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lang: Lang, text: &str) -> Vec<(&str, TokenKind)> {
        tokenize(lang, text)
            .into_iter()
            .map(|t| (&text[t.start..t.end], t.kind))
            .collect()
    }

    fn has(tokens: &[(&str, TokenKind)], text: &str, kind: TokenKind) -> bool {
        tokens.iter().any(|&(t, k)| t == text && k == kind)
    }

    #[test]
    fn lua_tokens() {
        assert_eq!(Lang::from_path(Path::new("scripts/player.lua")), Lang::Lua);
        let src = "-- note\nlocal speed = 4.5 --[[ block\nstill ]] \nfunction update(dt)\n  print(\"hi\\\"x\", [==[raw]]]==])\n  if not nil then return 'a' end\nend\n";
        let t = kinds(Lang::Lua, src);
        assert!(has(&t, "-- note", TokenKind::Comment));
        assert!(has(&t, "--[[ block\nstill ]]", TokenKind::Comment));
        assert!(has(&t, "local", TokenKind::Keyword));
        assert!(has(&t, "4.5", TokenKind::Number));
        assert!(has(&t, "update", TokenKind::Function));
        assert!(has(&t, "print", TokenKind::Function));
        assert!(has(&t, "\"hi\\\"x\"", TokenKind::String));
        assert!(has(&t, "[==[raw]]]==]", TokenKind::String));
        assert!(has(&t, "'a'", TokenKind::String));
        assert!(has(&t, "nil", TokenKind::Keyword));
        assert_eq!(Lang::Lua.line_comment(), Some("--"));
        // Never splits a char.
        tokenize(Lang::Lua, "x = \"é\\é\" -- ü [[ö");
    }

    #[test]
    fn rust_tokens() {
        let src = r##"#[derive(Debug)]
/// docs
pub struct Foo<'a> { x: &'a str } /* block /* nested */ still */
fn main() {
    let s = "hi \"there\""; let r = r#"raw "q""#;
    let c = 'x'; let b = b'\n'; let n = 0x1F_u32 + 1.5e-3 as f32;
    println!("{}", s.len()); Vec::<u8>::new();
    'outer: loop { break 'outer; }
}"##;
        let t = kinds(Lang::Rust, src);
        assert!(has(&t, "#[derive(Debug)]", TokenKind::Attribute));
        assert!(has(&t, "/// docs", TokenKind::Comment));
        assert!(has(
            &t,
            "/* block /* nested */ still */",
            TokenKind::Comment
        ));
        assert!(has(&t, "pub", TokenKind::Keyword));
        assert!(has(&t, "struct", TokenKind::Keyword));
        assert!(has(&t, "Foo", TokenKind::Type));
        assert!(has(&t, "'a", TokenKind::Keyword));
        assert!(has(&t, "str", TokenKind::Type));
        assert!(has(&t, "main", TokenKind::Function));
        assert!(has(&t, r#""hi \"there\"""#, TokenKind::String));
        assert!(has(&t, r##"r#"raw "q""#"##, TokenKind::String));
        assert!(has(&t, "'x'", TokenKind::String));
        assert!(has(&t, r"b'\n'", TokenKind::String));
        assert!(has(&t, "0x1F_u32", TokenKind::Number));
        assert!(has(&t, "1.5e-3", TokenKind::Number));
        assert!(has(&t, "f32", TokenKind::Type));
        assert!(has(&t, "println!", TokenKind::Macro));
        assert!(has(&t, "len", TokenKind::Function));
        assert!(has(&t, "Vec", TokenKind::Type));
        assert!(has(&t, "'outer", TokenKind::Keyword));
        // `s` and `x` are plain identifiers.
        assert!(!t.iter().any(|&(text, _)| text == "s" || text == "x"));
    }

    #[test]
    fn rust_ranges_are_sorted_and_on_char_boundaries() {
        let src = "let ü = \"ключ\"; // комментарий\nlet é = 'ж';";
        let tokens = tokenize(Lang::Rust, src);
        let mut last = 0;
        for t in &tokens {
            assert!(t.start >= last && t.end > t.start);
            assert!(src.is_char_boundary(t.start) && src.is_char_boundary(t.end));
            last = t.end;
        }
        let t = kinds(Lang::Rust, src);
        assert!(has(&t, "\"ключ\"", TokenKind::String));
        assert!(has(&t, "'ж'", TokenKind::String));
        assert!(has(&t, "// комментарий", TokenKind::Comment));
    }

    #[test]
    fn wgsl_tokens() {
        let src = "struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2f };\n\
                   @fragment fn fs(in: VsOut) -> @location(0) vec4<f32> {\n\
                   var c = textureSample(t, s, in.uv); // sample\n\
                   let k: mat4x4<f32> = mat4x4<f32>(); return c * 2.0 + vec4(1u, 0x10, 1.0f, 3.);\n}";
        let t = kinds(Lang::Wgsl, src);
        assert!(has(&t, "struct", TokenKind::Keyword));
        assert!(has(&t, "VsOut", TokenKind::Type));
        assert!(has(&t, "@builtin", TokenKind::Attribute));
        assert!(has(&t, "@fragment", TokenKind::Attribute));
        assert!(has(&t, "vec4", TokenKind::Type));
        assert!(has(&t, "vec2f", TokenKind::Type));
        assert!(has(&t, "mat4x4", TokenKind::Type));
        assert!(has(&t, "f32", TokenKind::Type));
        assert!(has(&t, "fn", TokenKind::Keyword));
        assert!(has(&t, "var", TokenKind::Keyword));
        assert!(has(&t, "textureSample", TokenKind::Function));
        assert!(has(&t, "// sample", TokenKind::Comment));
        assert!(has(&t, "2.0", TokenKind::Number));
        assert!(has(&t, "1u", TokenKind::Number));
        assert!(has(&t, "0x10", TokenKind::Number));
        assert!(has(&t, "1.0f", TokenKind::Number));
        assert!(has(&t, "3.", TokenKind::Number));
    }

    #[test]
    fn ron_tokens() {
        let src = "#![enable(implicit_some)]\n(\n  // comment\n  name: \"Cube\",\n  shape: Cube,\n  \
                   size: 1.5, tex: Some(\"a.png\"), on: true, raw: r#\"x\"#, ch: 'c',\n  path: a::b,\n)";
        let t = kinds(Lang::Ron, src);
        assert!(has(&t, "#![enable(implicit_some)]", TokenKind::Attribute));
        assert!(has(&t, "// comment", TokenKind::Comment));
        assert!(has(&t, "name", TokenKind::Property));
        assert!(has(&t, "\"Cube\"", TokenKind::String));
        assert!(has(&t, "Cube", TokenKind::Type));
        assert!(has(&t, "1.5", TokenKind::Number));
        assert!(has(&t, "Some", TokenKind::Keyword));
        assert!(has(&t, "true", TokenKind::Keyword));
        assert!(has(&t, "r#\"x\"#", TokenKind::String));
        assert!(has(&t, "'c'", TokenKind::String));
        assert!(has(&t, "path", TokenKind::Property));
        assert!(!has(&t, "a", TokenKind::Property));
    }

    #[test]
    fn toml_tokens() {
        let src = "# top\n[package]\nname = \"game\" # trailing\nversion = '0.1.0'\n\
                   opt-level = 3\n[[bin]]\ndesc = \"\"\"multi\nline\"\"\"\nflag = true\n\
                   when = 1979-05-27T07:32:00Z\ndep = { path = \"../x\", features = [\"a\"] }\n";
        let t = kinds(Lang::Toml, src);
        assert!(has(&t, "# top", TokenKind::Comment));
        assert!(has(&t, "[package]", TokenKind::Heading));
        assert!(has(&t, "[[bin]]", TokenKind::Heading));
        assert!(has(&t, "name", TokenKind::Property));
        assert!(has(&t, "\"game\"", TokenKind::String));
        assert!(has(&t, "# trailing", TokenKind::Comment));
        assert!(has(&t, "'0.1.0'", TokenKind::String));
        assert!(has(&t, "opt-level", TokenKind::Property));
        assert!(has(&t, "3", TokenKind::Number));
        assert!(has(&t, "\"\"\"multi\nline\"\"\"", TokenKind::String));
        assert!(has(&t, "true", TokenKind::Keyword));
        assert!(has(&t, "1979-05-27T07:32:00Z", TokenKind::Number));
        assert!(has(&t, "path", TokenKind::Property));
        assert!(has(&t, "\"a\"", TokenKind::String));
    }

    #[test]
    fn markdown_tokens() {
        let src = "# Title\nSome *em* and **strong** with `code` and [link](http://x).\n\
                   - item\n1. first\n> quote\n```rust\nlet x = 1;\n```\nafter_under_score\n";
        let t = kinds(Lang::Markdown, src);
        assert!(has(&t, "# Title", TokenKind::Heading));
        assert!(has(&t, "*em*", TokenKind::Emphasis));
        assert!(has(&t, "**strong**", TokenKind::Emphasis));
        assert!(has(&t, "`code`", TokenKind::String));
        assert!(has(&t, "[link](http://x)", TokenKind::Link));
        assert!(has(&t, "-", TokenKind::Keyword));
        assert!(has(&t, "1.", TokenKind::Keyword));
        assert!(has(&t, "> quote", TokenKind::Comment));
        assert!(has(&t, "```rust", TokenKind::String));
        assert!(has(&t, "let x = 1;", TokenKind::String));
        assert!(!has(&t, "_under_", TokenKind::Emphasis));
    }

    #[test]
    fn lang_from_path() {
        assert_eq!(Lang::from_path(Path::new("src/main.rs")), Lang::Rust);
        assert_eq!(Lang::from_path(Path::new("a/b.WGSL")), Lang::Wgsl);
        assert_eq!(Lang::from_path(Path::new("x.scene.ron")), Lang::Ron);
        assert_eq!(Lang::from_path(Path::new("Cargo.toml")), Lang::Toml);
        assert_eq!(Lang::from_path(Path::new("Cargo.lock")), Lang::Toml);
        assert_eq!(Lang::from_path(Path::new("README.md")), Lang::Markdown);
        assert_eq!(Lang::from_path(Path::new("a.txt")), Lang::Plain);
    }
}

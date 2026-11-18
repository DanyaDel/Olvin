//! Parsing `cargo --message-format=json-diagnostic-rendered-ansi` output.

use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    fn parse(level: &str) -> Option<Self> {
        match level {
            "error" | "error: internal compiler error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            "note" | "help" => Some(Self::Note),
            _ => None, // "failure-note" and friends
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Error => "⛔",
            Self::Warning => "⚠",
            Self::Note => "ℹ",
        }
    }
}

/// One problem with a location (1-based lines and columns, columns in chars).
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
    pub message: String,
    /// The primary span's label, e.g. "expected `u32`, found `&str`".
    pub label: Option<String>,
    pub code: Option<String>,
}

impl Diagnostic {
    pub fn summary(&self) -> String {
        match &self.label {
            Some(label) if !label.is_empty() => format!("{}: {label}", self.message),
            _ => self.message.clone(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum CargoMessage {
    /// A compiler message: its location (if it has one) and the text cargo
    /// would have printed (ANSI stripped).
    Diagnostic {
        diagnostic: Option<Diagnostic>,
        rendered: Option<String>,
    },
    Artifact,
    Finished {
        success: bool,
    },
    /// Some other JSON message (build-script-executed...).
    Other,
    /// Not JSON: the program's own output.
    Text(String),
}

/// Parses one stdout line. Relative file names are resolved against `root`
/// (cargo reports them relative to the workspace root).
pub fn parse_line(line: &str, root: &Path) -> CargoMessage {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return CargoMessage::Text(strip_ansi(line));
    }
    let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
        return CargoMessage::Text(strip_ansi(line));
    };
    match v["reason"].as_str() {
        Some("compiler-message") => {
            let msg = &v["message"];
            let manifest_dir = v["manifest_path"]
                .as_str()
                .map(Path::new)
                .and_then(Path::parent);
            CargoMessage::Diagnostic {
                diagnostic: diagnostic(msg, root, manifest_dir),
                rendered: msg["rendered"].as_str().map(strip_ansi),
            }
        }
        Some("compiler-artifact") => CargoMessage::Artifact,
        Some("build-finished") => CargoMessage::Finished {
            success: v["success"].as_bool().unwrap_or(false),
        },
        Some(_) => CargoMessage::Other,
        None => CargoMessage::Text(strip_ansi(line)),
    }
}

fn diagnostic(msg: &Value, root: &Path, manifest_dir: Option<&Path>) -> Option<Diagnostic> {
    let severity = Severity::parse(msg["level"].as_str()?)?;
    let spans = msg["spans"].as_array()?;
    let span = spans
        .iter()
        .find(|s| s["is_primary"].as_bool() == Some(true))
        .or_else(|| spans.first())?;
    // A span inside a macro from somewhere we can't open (std, a registry
    // crate that moved): use the call site instead.
    let mut span = span;
    let unreachable = |s: &Value| {
        s["file_name"].as_str().is_some_and(|f| {
            f.starts_with('<') || (Path::new(f).is_absolute() && !Path::new(f).exists())
        })
    };
    while unreachable(span)
        && let Some(exp) = span.get("expansion").filter(|e| !e.is_null())
    {
        match exp.get("span").filter(|s| !s.is_null()) {
            Some(s) => span = s,
            None => break,
        }
    }
    let file = span["file_name"].as_str()?;
    let num = |k: &str| span[k].as_u64().unwrap_or(1) as usize;
    Some(Diagnostic {
        severity,
        path: resolve(file, root, manifest_dir),
        line: num("line_start"),
        column: num("column_start"),
        end_line: num("line_end"),
        end_column: num("column_end"),
        message: msg["message"].as_str().unwrap_or_default().to_owned(),
        label: span["label"].as_str().map(str::to_owned),
        code: msg["code"]["code"].as_str().map(str::to_owned),
    })
}

fn resolve(file: &str, root: &Path, manifest_dir: Option<&Path>) -> PathBuf {
    let path = Path::new(file);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let in_root = root.join(path);
    if in_root.exists() {
        return in_root;
    }
    if let Some(dir) = manifest_dir {
        let in_manifest = dir.join(path);
        if in_manifest.exists() {
            return in_manifest;
        }
    }
    in_root
}

/// Removes ANSI escape sequences (CSI `ESC [ ... final` and OSC `ESC ] ... BEL/ST`).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("testdata/cargo_check.jsonl");

    #[test]
    fn parses_captured_cargo_output() {
        let root = Path::new("/work/game");
        let parsed: Vec<_> = SAMPLE.lines().map(|l| parse_line(l, root)).collect();
        assert_eq!(parsed.len(), 6);

        let CargoMessage::Diagnostic {
            diagnostic: Some(err),
            rendered: Some(rendered),
        } = &parsed[0]
        else {
            panic!("first line is the type error: {:?}", parsed[0]);
        };
        assert_eq!(err.severity, Severity::Error);
        assert_eq!(err.path, Path::new("/work/game/src/main.rs"));
        assert_eq!((err.line, err.column, err.end_column), (3, 18, 22));
        assert_eq!(err.message, "mismatched types");
        assert_eq!(err.label.as_deref(), Some("expected `u32`, found `&str`"));
        assert_eq!(err.code.as_deref(), Some("E0308"));
        assert_eq!(
            err.summary(),
            "mismatched types: expected `u32`, found `&str`"
        );
        assert!(rendered.starts_with("error[E0308]: mismatched types\n --> src/main.rs:3:18"));
        assert!(!rendered.contains('\u{1b}'));

        // "For more information..." has no location and is not a problem.
        assert!(matches!(
            &parsed[1],
            CargoMessage::Diagnostic {
                diagnostic: None,
                rendered: Some(_)
            }
        ));
        assert_eq!(parsed[2], CargoMessage::Finished { success: false });

        let CargoMessage::Diagnostic {
            diagnostic: Some(warn),
            ..
        } = &parsed[3]
        else {
            panic!("fourth line is the warning");
        };
        assert_eq!(warn.severity, Severity::Warning);
        assert_eq!((warn.line, warn.column), (2, 9));
        assert_eq!(warn.code.as_deref(), Some("unused_variables"));
        assert_eq!(parsed[4], CargoMessage::Artifact);
        assert_eq!(parsed[5], CargoMessage::Finished { success: true });
    }

    #[test]
    fn plain_lines_and_ansi() {
        let root = Path::new("/p");
        assert_eq!(
            parse_line("\u{1b}[1m\u{1b}[32mHello\u{1b}[0m game", root),
            CargoMessage::Text("Hello game".into())
        );
        assert_eq!(
            parse_line("{not json", root),
            CargoMessage::Text("{not json".into())
        );
        assert_eq!(
            strip_ansi("a\u{1b}]8;;http://x\u{7}b\u{1b}]8;;\u{1b}\\c"),
            "abc"
        );
        assert_eq!(strip_ansi("ключ\u{1b}[0m"), "ключ");
    }

    #[test]
    fn macro_spans_use_the_call_site() {
        let line = r#"{"reason":"compiler-message","manifest_path":"/p/Cargo.toml","message":{"message":"m","level":"error","code":null,"rendered":"x","spans":[{"file_name":"/rust/lib/fmt.rs","line_start":90,"line_end":90,"column_start":1,"column_end":2,"is_primary":true,"label":null,"expansion":{"span":{"file_name":"src/lib.rs","line_start":7,"line_end":7,"column_start":5,"column_end":20,"is_primary":false,"expansion":null}}}]}}"#;
        let CargoMessage::Diagnostic {
            diagnostic: Some(d),
            ..
        } = parse_line(line, Path::new("/p"))
        else {
            panic!()
        };
        assert_eq!(d.path, Path::new("/p/src/lib.rs"));
        assert_eq!(d.line, 7);
    }
}

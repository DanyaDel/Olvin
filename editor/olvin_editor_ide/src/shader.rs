//! Surface shader checks: the user's file is pasted between a stub with the
//! engine side of `SURFACE_SHADER_CONTRACT` and a call to `surface`, then
//! run through naga's WGSL front end and validator. Error lines are mapped
//! back to the user's file.

use std::error::Error;

use naga::valid::{Capabilities, ValidationFlags, Validator};

use olvin_editor::olvin_scene::material::SURFACE_SHADER_CONTRACT;

/// The engine side of the contract (see `olvin_scene::material`).
const PREFIX: &str = "\
struct SurfaceInput {
    world_position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    color: vec4<f32>,
    time: f32,
};
struct SurfaceOutput {
    color: vec4<f32>,
    normal: vec3<f32>,
    metallic: f32,
    roughness: f32,
    emissive: vec3<f32>,
};
fn default_surface(i: SurfaceInput) -> SurfaceOutput {
    var o: SurfaceOutput;
    o.color = i.color;
    o.normal = i.normal;
    o.metallic = 0.0;
    o.roughness = 0.6;
    o.emissive = vec3<f32>(0.0);
    return o;
}
";

/// Calls `surface` the way the engine does, so a wrong signature is an error.
const SUFFIX: &str = "
fn olvin_engine_calls_surface(i: SurfaceInput) -> SurfaceOutput {
    return surface(i);
}
";

pub const SIGNATURE: &str = "fn surface(in: SurfaceInput) -> SurfaceOutput";

/// Starting point for "New Surface Shader".
pub fn template() -> &'static str {
    SURFACE_SHADER_CONTRACT
}

pub fn is_surface_shader(text: &str) -> bool {
    text.contains("fn surface(")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderError {
    /// 1-based line in the user's file.
    pub line: usize,
    /// 1-based column (chars).
    pub column: usize,
    /// Length of the marked range in chars (at least 1).
    pub len: usize,
    pub message: String,
}

fn error_chain(e: &dyn Error) -> String {
    let mut text = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        let more = s.to_string();
        if !text.contains(&more) {
            text.push_str(": ");
            text.push_str(&more);
        }
        source = s.source();
    }
    text
}

/// Checks a surface shader. Empty = it compiles against the contract.
pub fn validate(source: &str) -> Vec<ShaderError> {
    let full = format!("{PREFIX}{source}\n{SUFFIX}");
    let mut found: Vec<(Option<naga::Span>, String)> = Vec::new();
    match naga::front::wgsl::parse_str(&full) {
        Err(e) => {
            let mut labels = e.labels();
            match labels.next() {
                Some((span, label)) => {
                    let msg = if label.is_empty() || e.message().contains(label) {
                        e.message().to_owned()
                    } else {
                        format!("{}: {label}", e.message())
                    };
                    found.push((Some(span), msg));
                }
                None => found.push((None, e.message().to_owned())),
            }
        }
        Ok(module) => {
            let mut validator = Validator::new(ValidationFlags::all(), Capabilities::all());
            if let Err(e) = validator.validate(&module) {
                let msg = error_chain(e.as_inner());
                let span = e.spans().map(|(s, _)| *s).find(|s| s.is_defined());
                found.push((span, msg));
            }
        }
    }
    found
        .into_iter()
        .map(|(span, message)| map_back(source, &full, span, message))
        .collect()
}

fn map_back(source: &str, full: &str, span: Option<naga::Span>, message: String) -> ShaderError {
    let prefix_lines = PREFIX.lines().count();
    let user_lines = source.lines().count().max(1);
    let surface_line = source
        .lines()
        .position(|l| l.contains("fn surface("))
        .map_or(1, |p| p + 1);
    let Some(span) = span else {
        return ShaderError {
            line: surface_line,
            column: 1,
            len: 1,
            message,
        };
    };
    let loc = span.location(full);
    let line = loc.line_number as usize;
    if line <= prefix_lines {
        return ShaderError {
            line: 1,
            column: 1,
            len: 1,
            message: format!("in the engine's contract stub: {message}"),
        };
    }
    let user_line = line - prefix_lines;
    if user_line > user_lines {
        return ShaderError {
            line: surface_line,
            column: 1,
            len: "fn surface".len(),
            message: format!("surface() must be `{SIGNATURE}`: {message}"),
        };
    }
    let text = source.lines().nth(user_line - 1).unwrap_or_default();
    let byte_col = (loc.line_position as usize)
        .saturating_sub(1)
        .min(text.len());
    let byte_col = (0..=byte_col)
        .rev()
        .find(|&b| text.is_char_boundary(b))
        .unwrap_or(0);
    let end = (byte_col + loc.length as usize).min(text.len());
    let end = (end..=text.len())
        .find(|&b| text.is_char_boundary(b))
        .unwrap_or(text.len());
    ShaderError {
        line: user_line,
        column: text[..byte_col].chars().count() + 1,
        len: text[byte_col..end].chars().count().max(1),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_valid() {
        assert!(is_surface_shader(template()));
        let errors = validate(template());
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn stripes_example_is_valid() {
        let src = "fn surface(in: SurfaceInput) -> SurfaceOutput {\n\
                   var out = default_surface(in);\n\
                   out.emissive = vec3(step(0.5, fract(in.uv.x * 8.0 + in.time))) * 0.5;\n\
                   return out;\n}\n";
        assert_eq!(validate(src), vec![]);
    }

    #[test]
    fn type_error_is_on_the_user_line() {
        let src = "// my shader\n\
                   fn surface(in: SurfaceInput) -> SurfaceOutput {\n\
                   \x20   var out = default_surface(in);\n\
                   \x20   let k: f32 = vec3<f32>(1.0);\n\
                   \x20   return out;\n\
                   }\n";
        let errors = validate(src);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].line, 4, "{errors:?}");
        assert!(errors[0].column >= 5, "{errors:?}");
    }

    #[test]
    fn syntax_error_and_bad_signature() {
        let src =
            "fn surface(in: SurfaceInput) -> SurfaceOutput {\n    return default_surface(in)\n}\n";
        let errors = validate(src);
        assert_eq!(errors.len(), 1);
        assert!((2..=3).contains(&errors[0].line), "{errors:?}");

        let src =
            "\n\nfn surface(uv: vec2<f32>) -> vec4<f32> {\n    return vec4(uv, 0.0, 1.0);\n}\n";
        let errors = validate(src);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 3, "{errors:?}");
        assert!(
            errors[0].message.starts_with("surface() must be"),
            "{errors:?}"
        );
    }
}

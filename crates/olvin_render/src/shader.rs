//! Custom surface shaders: user WGSL pasted in front of the engine's mesh
//! shader template (so error line numbers match the user's file), validated
//! with naga before any GPU work, compiled per shader by `upload_assets`.

/// Group 0 (per view): globals, shadow map, sky; sky / tonemap helpers.
pub(crate) const GLOBALS_WGSL: &str = include_str!("globals.wgsl");
/// The mesh shader template; calls the user's `surface`.
pub(crate) const MESH_WGSL: &str = include_str!("mesh.wgsl");
pub(crate) const SKY_WGSL: &str = include_str!("sky.wgsl");
/// The built-in material's `surface`.
pub const DEFAULT_SURFACE_SHADER: &str =
    "fn surface(in: SurfaceInput) -> SurfaceOutput {\n    return default_surface(in);\n}\n";

/// The full mesh shader with `surface_source` defining `surface`.
pub(crate) fn mesh_source(surface_source: &str) -> String {
    format!("{surface_source}\n{GLOBALS_WGSL}\n{MESH_WGSL}")
}

/// The line of `shadow_depth_at` that reads a raw depth: GLSL (the Compat
/// tier's GL / WebGL2 backends) has no `textureLoad` from depth textures, so
/// there it becomes a stub and PCSS stays off (`RenderCore::pcss_shader`).
const DEPTH_READ: &str = "return textureLoad(shadow_map, texel, 0);";

/// Adapts a full mesh shader to the device: keeps the raw depth reads PCSS
/// needs only where they compile.
pub(crate) fn for_device(source: &str, depth_reads: bool) -> std::borrow::Cow<'_, str> {
    if depth_reads {
        source.into()
    } else {
        source.replace(DEPTH_READ, "return 1.0;").into()
    }
}

pub(crate) fn sky_source() -> String {
    format!("{GLOBALS_WGSL}\n{SKY_WGSL}")
}

/// Checks a surface shader (see `olvin_scene::SURFACE_SHADER_CONTRACT`)
/// against the engine's template without a GPU. The error is a readable
/// report; line numbers are the user's file's.
pub fn validate_surface_shader(source: &str) -> Result<(), String> {
    let full = mesh_source(source);
    let user_lines = source.lines().count() as u32;
    let hint = |line: Option<u32>| match line {
        Some(l) if l > user_lines => {
            "\n(the error is in the engine's template: does the file define \
             `fn surface(in: SurfaceInput) -> SurfaceOutput` and avoid the \
             engine's names?)"
        }
        _ => "",
    };
    let module = naga::front::wgsl::parse_str(&full).map_err(|e| {
        let line = e.location(&full).map(|l| l.line_number);
        format!(
            "{}{}",
            e.emit_to_string_with_path(&full, "surface.wgsl"),
            hint(line)
        )
    })?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| {
        let line = e.location(&full).map(|l| l.line_number);
        format!(
            "{}{}",
            e.emit_to_string_with_path(&full, "surface.wgsl"),
            hint(line)
        )
    })?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShaderHandle(pub(crate) u32);

/// Surface shader registry. Sources are validated on `add` / `set`; a broken
/// source keeps the last good pipeline (or the built-in one) and reports
/// through [`Shaders::error`].
#[derive(Default)]
pub struct Shaders {
    next: u32,
    /// Valid sources waiting for a pipeline (full template already applied).
    pub(crate) pending: Vec<(ShaderHandle, String)>,
    errors: Vec<Option<String>>,
}

impl Shaders {
    pub fn add(&mut self, source: String) -> ShaderHandle {
        let handle = ShaderHandle(self.next);
        self.next += 1;
        self.set(handle, source);
        handle
    }

    /// Replaces a shader's source (hot reload).
    pub fn set(&mut self, handle: ShaderHandle, source: String) {
        self.pending.retain(|(h, _)| *h != handle);
        match validate_surface_shader(&source) {
            Ok(()) => {
                self.set_error(handle, None);
                self.pending.push((handle, mesh_source(&source)));
            }
            Err(e) => {
                log::warn!("surface shader {}: {e}", handle.0);
                self.set_error(handle, Some(e));
            }
        }
    }

    /// The last compile error of this shader's current source, if any.
    pub fn error(&self, handle: ShaderHandle) -> Option<String> {
        self.errors.get(handle.0 as usize).cloned().flatten()
    }

    pub(crate) fn set_error(&mut self, handle: ShaderHandle, error: Option<String>) {
        let i = handle.0 as usize;
        if self.errors.len() <= i {
            self.errors.resize(i + 1, None);
        }
        self.errors[i] = error;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_and_sky_shaders_validate() {
        validate_surface_shader(DEFAULT_SURFACE_SHADER).unwrap();
        let sky = naga::front::wgsl::parse_str(&sky_source()).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&sky)
        .unwrap();
    }

    #[test]
    fn the_contract_example_validates() {
        // SURFACE_SHADER_CONTRACT's example with its comment uncommented
        // (olvin_scene::material needs the serde feature, not used here).
        let example = "fn surface(in: SurfaceInput) -> SurfaceOutput {\n    var out = default_surface(in);\n    out.emissive = vec3(step(0.5, fract(in.uv.x * 8.0 + in.time))) * 0.5;\n    return out;\n}\n";
        validate_surface_shader(example).unwrap();
    }

    #[test]
    fn errors_point_at_the_users_line() {
        let src = "fn surface(in: SurfaceInput) -> SurfaceOutput {\n    var out = default_surface(in);\n    out.emissive = 1.0;\n    return out;\n}\n";
        let err = validate_surface_shader(src).unwrap_err();
        // The report quotes the user's own line 3 (numbered as in their file).
        assert!(
            err.lines()
                .any(|l| l.starts_with("3 ") && l.contains("out.emissive = 1.0;"))
                && !err.contains("template"),
            "{err}"
        );
        let err = validate_surface_shader("fn nope() {}").unwrap_err();
        assert!(err.contains("template"), "{err}");
        let mut shaders = Shaders::default();
        let h = shaders.add("fn surface(".into());
        assert!(shaders.error(h).is_some() && shaders.pending.is_empty());
        shaders.set(h, DEFAULT_SURFACE_SHADER.into());
        assert!(shaders.error(h).is_none() && shaders.pending.len() == 1);
    }
}

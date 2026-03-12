use std::path::Path;

/// Number of lines in the GLSL template before user code is inserted.
/// Used to adjust error line numbers back to the user's .art file.
const TEMPLATE_PREFIX_LINES: usize = 9;

/// Reads a .art file and translates Shadertoy-compatible GLSL to WGSL for wgpu.
pub fn translate_shader(path: &Path) -> Result<String, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read shader file: {e}"))?;

    let glsl = wrap_shadertoy_glsl(&source);
    glsl_to_wgsl(&glsl)
}

/// Wraps user Shadertoy GLSL in a GLSL 450 template with uniform buffer.
fn wrap_shadertoy_glsl(user_code: &str) -> String {
    // Strip GL_ES precision qualifiers that aren't valid in GLSL 450
    let filtered: Vec<&str> = user_code
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with("#ifdef GL_ES")
                && !trimmed.starts_with("#endif")
                && !trimmed.starts_with("precision ")
        })
        .collect();

    format!(
        r#"#version 450
layout(std140, set=0, binding=0) uniform Uniforms {{
    vec3 iResolution;
    float iTime;
    vec4 iMouse;
    int iFrame;
}};
layout(location=0) out vec4 fragColor;

{user_code}

void main() {{
    vec2 coord = gl_FragCoord.xy;
    coord.y = iResolution.y - coord.y;
    mainImage(fragColor, coord);
}}
"#,
        user_code = filtered.join("\n")
    )
}

/// Translates GLSL 450 source to WGSL using naga.
fn glsl_to_wgsl(glsl_source: &str) -> Result<String, String> {
    let mut parser = naga::front::glsl::Frontend::default();
    let options = naga::front::glsl::Options::from(naga::ShaderStage::Fragment);

    let module = parser.parse(&options, glsl_source).map_err(|errors| {
        let mut msg = String::from("GLSL compile errors:\n");
        // Use emit_to_string for formatted error output
        let formatted = errors.emit_to_string(glsl_source);
        msg.push_str(&formatted);
        msg.push_str(&format!(
            "\n(Note: subtract {TEMPLATE_PREFIX_LINES} from line numbers to get .art file lines)\n"
        ));
        msg
    })?;

    // Validate the module
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| format!("Shader validation error: {e}"))?;

    // Write WGSL
    let wgsl = naga::back::wgsl::write_string(
        &module,
        &info,
        naga::back::wgsl::WriterFlags::empty(),
    )
    .map_err(|e| format!("WGSL generation error: {e}"))?;

    // naga translates fragment output but we need to inject our fullscreen vertex shader
    // and ensure the entry points are named correctly
    let full_wgsl = inject_vertex_shader(&wgsl);
    Ok(full_wgsl)
}

/// Injects a fullscreen triangle vertex shader into the WGSL module.
/// naga only translates the fragment shader; we need a vertex shader too.
fn inject_vertex_shader(fragment_wgsl: &str) -> String {
    let vertex_shader = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Fullscreen triangle: 3 vertices cover the entire screen
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VertexOutput;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    return out;
}
"#;

    // naga generates `@fragment fn main(...)` as the entry point.
    // Rename ONLY that specific occurrence to `fs_main` for our pipeline.
    // Be precise to avoid mangling `main_1`, `mainImage`, etc.
    let mut patched = fragment_wgsl.to_string();

    // Replace "@fragment \nfn main(" with "@fragment \nfn fs_main("
    // naga may put whitespace/newline between @fragment and fn
    if let Some(frag_pos) = patched.find("@fragment") {
        let after_fragment = &patched[frag_pos..];
        if let Some(fn_pos) = after_fragment.find("fn main(") {
            let abs_pos = frag_pos + fn_pos;
            // Replace just "fn main(" with "fn fs_main("
            patched.replace_range(abs_pos..abs_pos + 8, "fn fs_main(");
        }
    }

    format!("{vertex_shader}\n{patched}")
}

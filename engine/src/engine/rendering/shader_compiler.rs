//! Editor-only runtime shader compiler using shaderc.
//!
//! Compiles GLSL source to SPIR-V with include resolution and structured error reporting.

use std::path::{Path, PathBuf};

/// Structured shader compilation error.
#[derive(Debug, Clone)]
pub struct ShaderError {
    pub path: PathBuf,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
}

impl std::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(l), Some(c)) => write!(f, "{}:{}:{}: {}", self.path.display(), l, c, self.message),
            (Some(l), None) => write!(f, "{}:{}: {}", self.path.display(), l, self.message),
            _ => write!(f, "{}: {}", self.path.display(), self.message),
        }
    }
}

impl std::error::Error for ShaderError {}

/// The kind of shader being compiled.
#[derive(Debug, Clone, Copy)]
pub enum ShaderKind {
    Vertex,
    Fragment,
    Compute,
}

impl ShaderKind {
    fn to_shaderc(self) -> shaderc::ShaderKind {
        match self {
            ShaderKind::Vertex => shaderc::ShaderKind::Vertex,
            ShaderKind::Fragment => shaderc::ShaderKind::Fragment,
            ShaderKind::Compute => shaderc::ShaderKind::Compute,
        }
    }
}

/// Runtime shader compiler with include resolution and cycle detection.
pub struct ShaderCompiler {
    compiler: shaderc::Compiler,
}

impl ShaderCompiler {
    /// Create a new shader compiler instance.
    pub fn new() -> Result<Self, ShaderError> {
        let compiler = shaderc::Compiler::new().ok_or_else(|| ShaderError {
            path: PathBuf::new(),
            line: None,
            column: None,
            message: "Failed to create shaderc compiler".to_string(),
        })?;

        Ok(Self { compiler })
    }

    /// Compile a GLSL file at `path` to SPIR-V.
    ///
    /// Resolves `#include "..."` directives relative to the including file's directory.
    /// Detects include cycles and returns an error on revisit.
    pub fn compile(&self, path: &Path, kind: ShaderKind) -> Result<Vec<u32>, ShaderError> {
        let source = std::fs::read_to_string(path).map_err(|e| ShaderError {
            path: path.to_path_buf(),
            line: None,
            column: None,
            message: format!("Failed to read shader file: {}", e),
        })?;

        let mut options = shaderc::CompileOptions::new().ok_or_else(|| ShaderError {
            path: path.to_path_buf(),
            line: None,
            column: None,
            message: "Failed to create compile options".to_string(),
        })?;

        options.set_target_env(
            shaderc::TargetEnv::Vulkan,
            shaderc::EnvVersion::Vulkan1_2 as u32,
        );
        options.set_target_spirv(shaderc::SpirvVersion::V1_5);
        options.set_source_language(shaderc::SourceLanguage::GLSL);

        // Include callback with cycle guard via depth limit.
        let base_dir = path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        const MAX_INCLUDE_DEPTH: usize = 32;

        options.set_include_callback(move |requested, _type, requesting_source, depth| {
            if depth > MAX_INCLUDE_DEPTH {
                return Err(format!(
                    "Include depth limit ({}) exceeded — possible cycle from '{}'",
                    MAX_INCLUDE_DEPTH, requested
                ));
            }

            let requesting_path = Path::new(requesting_source);
            let requesting_dir = if requesting_source.is_empty() {
                base_dir.clone()
            } else if requesting_path.parent().is_none_or(|p| p.as_os_str().is_empty()) {
                // Requesting source has no directory component — use base dir
                base_dir.clone()
            } else {
                requesting_path.parent().unwrap().to_path_buf()
            };

            let resolved = requesting_dir.join(requested);
            let canonical = std::fs::canonicalize(&resolved)
                .unwrap_or_else(|_| resolved.clone());

            let content = std::fs::read_to_string(&canonical)
                .map_err(|e| format!("Failed to read include '{}': {}", canonical.display(), e))?;

            Ok(shaderc::ResolvedInclude {
                resolved_name: canonical.to_string_lossy().into_owned(),
                content,
            })
        });

        let file_name = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();

        let result = self.compiler.compile_into_spirv(
            &source,
            kind.to_shaderc(),
            &file_name,
            "main",
            Some(&options),
        );

        match result {
            Ok(artifact) => Ok(artifact.as_binary().to_vec()),
            Err(shaderc::Error::CompilationError(_, msg)) => {
                Err(parse_shaderc_error(path, &msg))
            }
            Err(e) => Err(ShaderError {
                path: path.to_path_buf(),
                line: None,
                column: None,
                message: e.to_string(),
            }),
        }
    }
}

/// Parse shaderc's error string (format: `file:line:col: error: msg`) into structured fields.
fn parse_shaderc_error(path: &Path, msg: &str) -> ShaderError {
    // shaderc errors typically look like:
    // filename:line: error: message
    // or: filename:line:col: error: message
    for line in msg.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Try to parse "file:line:col: error: msg" or "file:line: error: msg"
        if let Some(rest) = trimmed.strip_prefix(&format!("{}:", path.file_name().unwrap_or_default().to_string_lossy())) {
            if let Some((line_part, remainder)) = rest.split_once(':') {
                if let Ok(line_num) = line_part.trim().parse::<u32>() {
                    // Check if next part is column or error message
                    if let Some((col_part, msg_part)) = remainder.split_once(':') {
                        if let Ok(col_num) = col_part.trim().parse::<u32>() {
                            return ShaderError {
                                path: path.to_path_buf(),
                                line: Some(line_num),
                                column: Some(col_num),
                                message: msg_part.trim().to_string(),
                            };
                        }
                    }
                    return ShaderError {
                        path: path.to_path_buf(),
                        line: Some(line_num),
                        column: None,
                        message: remainder.trim().to_string(),
                    };
                }
            }
        }
    }

    // Fallback: put the whole message in
    ShaderError {
        path: path.to_path_buf(),
        line: None,
        column: None,
        message: msg.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_gbuffer_frag() {
        let compiler = ShaderCompiler::new().expect("compiler creation");
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/engine/rendering/shaders/deferred/gbuffer.frag");
        let spirv = compiler
            .compile(&path, ShaderKind::Fragment)
            .expect("compile gbuffer.frag");
        assert!(!spirv.is_empty());
    }

    #[test]
    fn compile_gbuffer_vert() {
        let compiler = ShaderCompiler::new().expect("compiler creation");
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/engine/rendering/shaders/deferred/gbuffer.vert");
        let spirv = compiler
            .compile(&path, ShaderKind::Vertex)
            .expect("compile gbuffer.vert");
        assert!(!spirv.is_empty());
    }

    #[test]
    fn compile_syntax_error() {
        let compiler = ShaderCompiler::new().expect("compiler creation");

        let temp_dir = std::env::temp_dir().join("rust_engine_shader_test");
        let _ = std::fs::create_dir_all(&temp_dir);
        let bad_path = temp_dir.join("bad.frag");
        std::fs::write(
            &bad_path,
            "#version 460\nvoid main() { int x = }\n",
        )
        .expect("write temp file");

        let result = compiler.compile(&bad_path, ShaderKind::Fragment);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.line.is_some(), "should have a line number");

        let _ = std::fs::remove_file(&bad_path);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    /// Verifies the error-recovery workflow: a broken shader fails with structured
    /// error info, then the same file — once fixed — compiles successfully.
    #[test]
    fn compile_error_then_fix() {
        let compiler = ShaderCompiler::new().expect("compiler creation");

        let temp_dir = std::env::temp_dir().join("rust_engine_error_recovery_test");
        let _ = std::fs::create_dir_all(&temp_dir);
        let shader_path = temp_dir.join("recover.frag");

        // Write a broken shader
        std::fs::write(
            &shader_path,
            "#version 460\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(1.0 }\n",
        )
        .expect("write broken shader");

        let bad_result = compiler.compile(&shader_path, ShaderKind::Fragment);
        assert!(bad_result.is_err(), "broken shader must fail");
        let err = bad_result.unwrap_err();
        assert!(err.line.is_some(), "error should carry a line number");

        // Fix the shader
        std::fs::write(
            &shader_path,
            "#version 460\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(1.0); }\n",
        )
        .expect("write fixed shader");

        let good_result = compiler.compile(&shader_path, ShaderKind::Fragment);
        assert!(good_result.is_ok(), "fixed shader must compile: {:?}", good_result.err());

        let _ = std::fs::remove_file(&shader_path);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    #[test]
    fn compile_include_resolution() {
        let compiler = ShaderCompiler::new().expect("compiler creation");

        let temp_dir = std::env::temp_dir().join("rust_engine_include_test");
        let _ = std::fs::create_dir_all(&temp_dir);

        std::fs::write(
            temp_dir.join("common.glsl"),
            "vec3 get_color() { return vec3(1.0, 0.0, 0.0); }\n",
        )
        .expect("write common.glsl");

        std::fs::write(
            temp_dir.join("host.frag"),
            r#"#version 460
#include "common.glsl"
layout(location = 0) out vec4 out_color;
void main() { out_color = vec4(get_color(), 1.0); }
"#,
        )
        .expect("write host.frag");

        let result = compiler.compile(&temp_dir.join("host.frag"), ShaderKind::Fragment);
        assert!(result.is_ok(), "include resolution should succeed: {:?}", result.err());

        let _ = std::fs::remove_file(temp_dir.join("common.glsl"));
        let _ = std::fs::remove_file(temp_dir.join("host.frag"));
        let _ = std::fs::remove_dir(&temp_dir);
    }
}

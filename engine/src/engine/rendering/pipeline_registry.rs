//! Central pipeline registry for hot-reloadable graphics pipelines.
//!
//! Only Geometry and Lighting pipelines are registered here — other passes
//! keep their own `Arc<GraphicsPipeline>` fields unchanged.

use std::collections::HashMap;
use std::sync::Arc;
use vulkano::pipeline::GraphicsPipeline;

#[cfg(feature = "editor")]
use std::path::PathBuf;

/// Identifies a registered pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineId {
    Geometry,
    Lighting,
}

/// Error during pipeline rebuild.
#[derive(Debug)]
pub enum PipelineError {
    #[cfg(feature = "editor")]
    Shader(super::shader_compiler::ShaderError),
    Vulkan(String),
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "editor")]
            PipelineError::Shader(e) => write!(f, "{}", e),
            PipelineError::Vulkan(msg) => write!(f, "{}", msg),
        }
    }
}

/// Result of a single pipeline rebuild attempt.
pub struct RebuildResult {
    pub id: PipelineId,
    pub outcome: Result<(), PipelineError>,
}

struct RegistryEntry {
    current: arc_swap::ArcSwap<GraphicsPipeline>,
    #[cfg(feature = "editor")]
    shader_paths: Vec<PathBuf>,
    #[cfg(feature = "editor")]
    rebuild: Box<
        dyn Fn(
                &super::shader_compiler::ShaderCompiler,
                &Arc<vulkano::device::Device>,
            ) -> Result<Arc<GraphicsPipeline>, PipelineError>
            + Send
            + Sync,
    >,
}

/// Central registry that owns Geometry and Lighting pipelines behind `ArcSwap`.
///
/// On editor builds, each entry also stores the shader paths and a rebuild closure
/// that recompiles from disk and constructs a fresh pipeline.
pub struct PipelineRegistry {
    entries: HashMap<PipelineId, RegistryEntry>,
}

impl PipelineRegistry {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Register a pipeline. Called during `DeferredRenderer::new`.
    #[allow(unused_variables)]
    pub fn register(
        &mut self,
        id: PipelineId,
        pipeline: Arc<GraphicsPipeline>,
        #[cfg(feature = "editor")] shader_paths: Vec<PathBuf>,
        #[cfg(feature = "editor")] rebuild: Box<
            dyn Fn(
                    &super::shader_compiler::ShaderCompiler,
                    &Arc<vulkano::device::Device>,
                ) -> Result<Arc<GraphicsPipeline>, PipelineError>
                + Send
                + Sync,
        >,
    ) {
        self.entries.insert(
            id,
            RegistryEntry {
                current: arc_swap::ArcSwap::new(pipeline),
                #[cfg(feature = "editor")]
                shader_paths,
                #[cfg(feature = "editor")]
                rebuild,
            },
        );
    }

    /// Load the current pipeline for a given ID.
    ///
    /// Panics if the ID is not registered (the registry is internally consistent).
    pub fn get(&self, id: PipelineId) -> Arc<GraphicsPipeline> {
        self.entries
            .get(&id)
            .unwrap_or_else(|| panic!("Pipeline {:?} not registered", id))
            .current
            .load_full()
    }

    /// Rebuild all registered pipelines from their shader source files.
    ///
    /// Returns a result for each pipeline. On success, the `ArcSwap::store` atomically
    /// replaces the pipeline. On failure, the old pipeline remains in place.
    #[cfg(feature = "editor")]
    pub fn rebuild_all(
        &self,
        compiler: &super::shader_compiler::ShaderCompiler,
        device: &Arc<vulkano::device::Device>,
    ) -> Vec<RebuildResult> {
        let mut results = Vec::new();
        for (&id, entry) in &self.entries {
            let outcome = match (entry.rebuild)(compiler, device) {
                Ok(new_pipeline) => {
                    // Atomic swap: the old pipeline stays alive until all in-flight frames
                    // that reference it have completed.
                    entry.current.store(new_pipeline);
                    Ok(())
                }
                Err(e) => Err(e),
            };
            results.push(RebuildResult { id, outcome });
        }
        results
    }

    /// Rebuild only the pipelines that use the given shader path.
    #[cfg(feature = "editor")]
    pub fn rebuild_for_shader(
        &self,
        path: &std::path::Path,
        compiler: &super::shader_compiler::ShaderCompiler,
        device: &Arc<vulkano::device::Device>,
    ) -> Vec<RebuildResult> {
        let mut results = Vec::new();

        // Canonicalize the changed path for comparison
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        for (&id, entry) in &self.entries {
            let uses_shader = entry.shader_paths.iter().any(|sp| {
                let sp_canonical = std::fs::canonicalize(sp).unwrap_or_else(|_| sp.clone());
                sp_canonical == canonical
            });

            if uses_shader {
                let outcome = match (entry.rebuild)(compiler, device) {
                    Ok(new_pipeline) => {
                        entry.current.store(new_pipeline);
                        Ok(())
                    }
                    Err(e) => Err(e),
                };
                results.push(RebuildResult { id, outcome });
            }
        }
        results
    }
}

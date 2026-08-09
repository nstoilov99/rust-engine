//! Per-pass resize/rebind contract for the deferred pipeline.
//!
//! `DeferredRenderer::resize` used to be a hand-maintained chain (gbuffer →
//! SSAO → bloom → luminance → composite → plankton) where one missed
//! recreation meant a stale-descriptor crash on window resize. Every pass now
//! implements [`DeferredPass`] and the renderer loops over its registered
//! passes (`DeferredRenderer::passes_mut`) in two phases:
//!
//! 1. **`resize`** — the pass recreates its *own* size-dependent GPU targets
//!    (images, framebuffers, mip chains). No pass reads another pass's output
//!    here.
//! 2. **`rebind`** — the pass recreates descriptor sets / framebuffers that
//!    reference *shared* resources or other passes' outputs, from a
//!    [`PassInputs`] snapshot taken after phase 1 completes for all passes.
//!
//! Because inputs are snapshotted between the phases, rebind order cannot go
//! stale; the registration order in `passes_mut` mirrors frame execution
//! order (shadow → geometry → ssao → lighting → plankton → bloom → luminance
//! → composite → grid → debug_draw) purely for readability.

use crate::engine::rendering::error::RenderError;
use std::sync::Arc;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::image::sampler::Sampler;
use vulkano::image::view::ImageView;
use vulkano::memory::allocator::StandardMemoryAllocator;

/// Everything a pass may need to recreate its own size-dependent targets.
pub struct PassResizeContext {
    pub allocator: Arc<StandardMemoryAllocator>,
    pub width: u32,
    pub height: u32,
}

/// Snapshot of the shared resources and cross-pass outputs that descriptor
/// sets may sample. Cloned `Arc`s — taken after every pass has resized, so
/// all views point at the freshly created images.
pub struct PassInputs {
    pub descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    // G-buffer attachments (owned by `DeferredRenderer`, recreated first).
    pub gbuffer_position: Arc<ImageView>,
    pub gbuffer_normal: Arc<ImageView>,
    pub gbuffer_albedo: Arc<ImageView>,
    pub gbuffer_material: Arc<ImageView>,
    pub gbuffer_emissive: Arc<ImageView>,
    pub gbuffer_depth: Arc<ImageView>,
    /// HDR scene color (owned by `DeferredRenderer`, recreated first).
    pub hdr_target: Arc<ImageView>,
    // Cross-pass outputs.
    pub shadow_map: Arc<ImageView>,
    pub shadow_sampler: Arc<Sampler>,
    pub ssao_blurred: Arc<ImageView>,
    /// 1x1 white fallback bound when SSAO is disabled.
    pub ssao_fallback: Arc<ImageView>,
    pub ssao_sampler: Arc<Sampler>,
    pub bloom_result: Arc<ImageView>,
    pub luminance_1x1: Arc<ImageView>,
}

/// Resize/rebind contract implemented by every deferred pass.
///
/// Both methods default to no-ops so passes without size-dependent state
/// (shadow: fixed-size map; geometry/grid/debug_draw: render into
/// renderer-owned framebuffers) only implement `name`.
pub trait DeferredPass {
    /// Stable pass name, used to attribute resize/rebind failures.
    fn name(&self) -> &'static str;

    /// Recreate this pass's own size-dependent GPU targets.
    fn resize(&mut self, _ctx: &PassResizeContext) -> Result<(), RenderError> {
        Ok(())
    }

    /// Recreate descriptor sets / framebuffers referencing shared inputs.
    fn rebind(&mut self, _inputs: &PassInputs) -> Result<(), RenderError> {
        Ok(())
    }
}

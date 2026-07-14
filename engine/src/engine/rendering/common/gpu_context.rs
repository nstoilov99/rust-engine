//! Shared GPU context for cross-thread Vulkan resource access.
//!
//! `GpuContext` holds the core Vulkan objects that are needed by both
//! the main thread and the render thread. All fields are already
//! `Arc`-wrapped, making the struct `Send + Sync` automatically.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::device::{Device, Queue};
use vulkano::memory::allocator::StandardMemoryAllocator;

/// Serializes every queue-submitting call (`flush()` /
/// `then_signal_fence_and_flush()`) across threads.
///
/// vulkano 0.35's `queue_submit` locks per-resource state mutexes in
/// command-buffer iteration order (`States::from_submit_infos`). Two threads
/// flushing concurrently with overlapping resources (glyph atlas, mesh
/// buffers, viewport images) can acquire those mutexes in opposite orders and
/// deadlock permanently — observed between the render thread and a crusty
/// float window pumped from an OS modal resize loop. Hold this guard around
/// the whole flush/present expression; do NOT hold it across fence waits.
static QUEUE_SUBMIT_LOCK: Mutex<()> = Mutex::new(());

pub fn lock_queue_submit() -> MutexGuard<'static, ()> {
    QUEUE_SUBMIT_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub struct GpuContext {
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub memory_allocator: Arc<StandardMemoryAllocator>,
    pub command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    pub descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
}

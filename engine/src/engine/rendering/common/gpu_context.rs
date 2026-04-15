//! Shared GPU context for cross-thread Vulkan resource access.
//!
//! `GpuContext` holds the core Vulkan objects that are needed by both
//! the main thread and the render thread. All fields are already
//! `Arc`-wrapped, making the struct `Send + Sync` automatically.

use std::sync::Arc;
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::device::{Device, Queue};
use vulkano::memory::allocator::StandardMemoryAllocator;

pub struct GpuContext {
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub memory_allocator: Arc<StandardMemoryAllocator>,
    pub command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    pub descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
}

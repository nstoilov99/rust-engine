//! Skinning backend for GPU bone palette upload and binding.
//!
//! Phase 1 implements the `FixedUbo` backend: a fixed-capacity UBO
//! with `MAX_PALETTE_BONES` slots. The animation system produces
//! `Vec<Mat4>` palette data; this module converts it into a GPU
//! binding (descriptor set). A future `LargeSsbo` backend can be
//! added without changing animation sampling or ECS components.

use std::sync::Arc;

use glam::Mat4;
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer};
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::descriptor_set::layout::DescriptorSetLayout;
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::pipeline::Pipeline;
use vulkano::pipeline::GraphicsPipeline;

use super::pipeline_3d::{BonePaletteData, MAX_PALETTE_BONES};

/// FixedUbo skinning backend.
///
/// Manages bone palette UBO allocation and descriptor set creation
/// for a specific pipeline layout. The identity binding is shared
/// across all static meshes.
pub struct SkinningBackend {
    allocator: Arc<StandardMemoryAllocator>,
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    #[allow(dead_code)] // Kept alive — referenced by identity_set's descriptor
    identity_buffer: Subbuffer<BonePaletteData>,
    identity_set: Arc<DescriptorSet>,
    set_layout: Arc<DescriptorSetLayout>,
}

impl SkinningBackend {
    /// Create a new skinning backend for the given pipeline.
    ///
    /// The pipeline must have a descriptor set layout at set 0 with
    /// a uniform buffer binding at binding 0 (the bone palette UBO).
    pub fn new(
        allocator: Arc<StandardMemoryAllocator>,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        pipeline: &Arc<GraphicsPipeline>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let set_layout = pipeline.layout().set_layouts()[0].clone();

        let identity_buffer = Buffer::from_data(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::UNIFORM_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            BonePaletteData::identity(),
        )?;

        let identity_set = DescriptorSet::new(
            descriptor_set_allocator.clone(),
            set_layout.clone(),
            [WriteDescriptorSet::buffer(0, identity_buffer.clone())],
            [],
        )?;

        Ok(Self {
            allocator,
            descriptor_set_allocator,
            identity_buffer,
            identity_set,
            set_layout,
        })
    }

    /// Returns the shared identity bone palette descriptor set.
    /// Used for all static (non-skinned) meshes.
    pub fn identity_set(&self) -> &Arc<DescriptorSet> {
        &self.identity_set
    }

    /// Create a descriptor set from a bone palette (Vec<Mat4>).
    ///
    /// The palette must not exceed `MAX_PALETTE_BONES`. Panics if it does.
    /// The animation system should validate bone counts before calling this.
    pub fn create_palette_set(
        &self,
        palette: &[Mat4],
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        debug_assert!(
            palette.len() <= MAX_PALETTE_BONES,
            "Bone palette ({}) exceeds FixedUbo cap ({})",
            palette.len(),
            MAX_PALETTE_BONES,
        );

        let mut data = BonePaletteData::identity();
        data.bone_count = palette.len() as u32;
        for (i, mat) in palette.iter().enumerate() {
            data.matrices[i] = mat.to_cols_array();
        }

        let buffer = Buffer::from_data(
            self.allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::UNIFORM_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            data,
        )?;

        let set = DescriptorSet::new(
            self.descriptor_set_allocator.clone(),
            self.set_layout.clone(),
            [WriteDescriptorSet::buffer(0, buffer)],
            [],
        )?;

        Ok(set)
    }

    /// Create an identity descriptor set for a different pipeline layout.
    ///
    /// Used by pipelines (shadow, thumbnail, mesh editor) that have their
    /// own layout but need an identity bone palette at set 0.
    pub fn create_identity_set_for_layout(
        allocator: &Arc<StandardMemoryAllocator>,
        descriptor_set_allocator: &Arc<StandardDescriptorSetAllocator>,
        set_layout: Arc<DescriptorSetLayout>,
    ) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
        let buffer = Buffer::from_data(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::UNIFORM_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            BonePaletteData::identity(),
        )?;

        let set = DescriptorSet::new(
            descriptor_set_allocator.clone(),
            set_layout,
            [WriteDescriptorSet::buffer(0, buffer)],
            [],
        )?;

        Ok(set)
    }
}

use super::pool::PlanktonPool;
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::command_buffer::AutoCommandBufferBuilder;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::Device;
use vulkano::pipeline::compute::ComputePipelineCreateInfo;
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::pipeline::{
    ComputePipeline, PipelineBindPoint, PipelineLayout, PipelineShaderStageCreateInfo,
};

mod shader {
    vulkano_shaders::shader! {
        ty: "compute",
        path: "src/engine/rendering/shaders/plankton/plankton_emit.comp",
    }
}

/// Push constants for the emit compute shader.
/// Must match the layout in plankton_emit.comp exactly.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EmitPushConstants {
    pub emitter_transform: [[f32; 4]; 4], // mat4 = 64 bytes
    pub velocity_base_variance: [f32; 4], // vec4 = 16 bytes
    pub color_start: [f32; 4],            // vec4 = 16 bytes
    pub color_end: [f32; 4],              // vec4 = 16 bytes
    pub size_lifetime: [f32; 4],          // vec4 = 16 bytes
    pub shape_params: [f32; 4],           // vec4 = 16 bytes
    pub shape_type: u32,
    pub random_seed: u32,
    pub dt: f32,
    pub emit_count: u32,
}

unsafe impl bytemuck::Pod for EmitPushConstants {}
unsafe impl bytemuck::Zeroable for EmitPushConstants {}

pub struct PlanktonEmitPass {
    pipeline: Arc<ComputePipeline>,
    layout: Arc<PipelineLayout>,
    descriptor_sets: HashMap<uuid::Uuid, Arc<DescriptorSet>>,
}

impl PlanktonEmitPass {
    pub fn new(device: Arc<Device>) -> Result<Self, Box<dyn std::error::Error>> {
        let cs = shader::load(device.clone())?
            .entry_point("main")
            .ok_or("missing main entry point in plankton_emit.comp")?;

        let stage = PipelineShaderStageCreateInfo::new(cs);
        let layout = PipelineLayout::new(
            device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(std::slice::from_ref(&stage))
                .into_pipeline_layout_create_info(device.clone())?,
        )?;

        let pipeline = ComputePipeline::new(
            device,
            None,
            ComputePipelineCreateInfo::stage_layout(stage, layout.clone()),
        )?;

        Ok(Self {
            pipeline,
            layout,
            descriptor_sets: HashMap::new(),
        })
    }

    pub fn prepare_pool(
        &mut self,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        pool_guid: uuid::Uuid,
        pool: &PlanktonPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let set_layout = self
            .layout
            .set_layouts()
            .first()
            .ok_or("missing set 0 layout in emit pass")?
            .clone();

        let set = DescriptorSet::new(
            descriptor_set_allocator,
            set_layout,
            [
                WriteDescriptorSet::buffer(0, pool.particle_buffer.clone()),
                WriteDescriptorSet::buffer(1, pool.dead_list_buffer.clone()),
                WriteDescriptorSet::buffer(2, pool.counters_buffer.clone()),
            ],
            [],
        )?;

        self.descriptor_sets.insert(pool_guid, set);
        Ok(())
    }

    pub fn dispatch<L>(
        &self,
        builder: &mut AutoCommandBufferBuilder<L>,
        pool_guid: uuid::Uuid,
        params: EmitPushConstants,
        emit_count: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if emit_count == 0 {
            return Ok(());
        }

        let set = self
            .descriptor_sets
            .get(&pool_guid)
            .ok_or("emit pass: no descriptor set for pool")?;

        builder
            .bind_pipeline_compute(self.pipeline.clone())?
            .bind_descriptor_sets(
                PipelineBindPoint::Compute,
                self.layout.clone(),
                0,
                set.clone(),
            )?
            .push_constants(self.layout.clone(), 0, params)?;

        let workgroups = emit_count.div_ceil(64);
        unsafe {
            builder.dispatch([workgroups, 1, 1])?;
        }

        Ok(())
    }

    pub fn remove_pool(&mut self, pool_guid: &uuid::Uuid) {
        self.descriptor_sets.remove(pool_guid);
    }
}

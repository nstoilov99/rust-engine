use super::pool::PlanktonPool;
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::command_buffer::AutoCommandBufferBuilder;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::Device;
use vulkano::pipeline::compute::ComputePipelineCreateInfo;
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::pipeline::{ComputePipeline, PipelineBindPoint, PipelineLayout, PipelineShaderStageCreateInfo};

mod shader {
    vulkano_shaders::shader! {
        ty: "compute",
        path: "src/engine/rendering/shaders/plankton/plankton_init.comp",
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InitPushConstants {
    pub capacity: u32,
}

unsafe impl bytemuck::Pod for InitPushConstants {}
unsafe impl bytemuck::Zeroable for InitPushConstants {}

pub struct PlanktonInitPass {
    pipeline: Arc<ComputePipeline>,
    layout: Arc<PipelineLayout>,
    descriptor_sets: HashMap<uuid::Uuid, Arc<DescriptorSet>>,
}

impl PlanktonInitPass {
    pub fn new(device: Arc<Device>) -> Result<Self, Box<dyn std::error::Error>> {
        let cs = shader::load(device.clone())?
            .entry_point("main")
            .ok_or("missing main entry point in plankton_init.comp")?;

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
            .ok_or("missing set 0 layout in init pass")?
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
        capacity: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let set = self
            .descriptor_sets
            .get(&pool_guid)
            .ok_or("init pass: no descriptor set for pool")?;

        builder
            .bind_pipeline_compute(self.pipeline.clone())?
            .bind_descriptor_sets(
                PipelineBindPoint::Compute,
                self.layout.clone(),
                0,
                set.clone(),
            )?
            .push_constants(
                self.layout.clone(),
                0,
                InitPushConstants { capacity },
            )?;

        let workgroups = capacity.div_ceil(64);
        unsafe {
            builder.dispatch([workgroups, 1, 1])?;
        }

        Ok(())
    }

    pub fn remove_pool(&mut self, pool_guid: &uuid::Uuid) {
        self.descriptor_sets.remove(pool_guid);
    }
}

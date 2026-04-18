use super::pool::PlanktonPool;
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::command_buffer::AutoCommandBufferBuilder;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::Device;
use vulkano::format::Format;
use vulkano::image::sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo};
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::pipeline::compute::ComputePipelineCreateInfo;
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::pipeline::{
    ComputePipeline, PipelineBindPoint, PipelineLayout, PipelineShaderStageCreateInfo,
};

mod shader {
    vulkano_shaders::shader! {
        ty: "compute",
        path: "src/engine/rendering/shaders/plankton/plankton_simulate.comp",
    }
}

/// Push constants for the simulate compute shader.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SimulatePushConstants {
    pub gravity_and_drag: [f32; 4],       // .xyz = gravity, .w = drag
    pub wind_and_turb_strength: [f32; 4], // .xyz = wind, .w = turbulence_strength
    pub turb_scale_speed_pad: [f32; 4],   // .x = scale, .y = speed, .zw = 0
    pub color_start: [f32; 4],
    pub color_end: [f32; 4],
    pub size_start: f32,
    pub size_end: f32,
    pub delta_time: f32,
    pub time: f32,
    pub capacity: u32,
    pub _sim_pad0: u32,
    pub _sim_pad1: u32,
    pub _sim_pad2: u32,
}

unsafe impl bytemuck::Pod for SimulatePushConstants {}
unsafe impl bytemuck::Zeroable for SimulatePushConstants {}

pub struct PlanktonSimulatePass {
    pipeline: Arc<ComputePipeline>,
    layout: Arc<PipelineLayout>,
    descriptor_sets: HashMap<uuid::Uuid, Arc<DescriptorSet>>,
    noise_texture: Arc<ImageView>,
    noise_sampler: Arc<Sampler>,
}

impl PlanktonSimulatePass {
    pub fn new(
        device: Arc<Device>,
        allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let cs = shader::load(device.clone())?
            .entry_point("main")
            .ok_or("missing main entry point in plankton_simulate.comp")?;

        let stage = PipelineShaderStageCreateInfo::new(cs);
        let layout = PipelineLayout::new(
            device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(std::slice::from_ref(&stage))
                .into_pipeline_layout_create_info(device.clone())?,
        )?;

        let pipeline = ComputePipeline::new(
            device.clone(),
            None,
            ComputePipelineCreateInfo::stage_layout(stage, layout.clone()),
        )?;

        // Create 1x1x1 zero fallback 3D noise texture
        let noise_texture = Self::create_fallback_noise(allocator)?;

        let noise_sampler = Sampler::new(
            device,
            SamplerCreateInfo {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: [SamplerAddressMode::Repeat; 3],
                ..Default::default()
            },
        )?;

        Ok(Self {
            pipeline,
            layout,
            descriptor_sets: HashMap::new(),
            noise_texture,
            noise_sampler,
        })
    }

    fn create_fallback_noise(
        allocator: Arc<StandardMemoryAllocator>,
    ) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
        let image = Image::new(
            allocator,
            ImageCreateInfo {
                image_type: ImageType::Dim3d,
                format: Format::R16G16B16A16_SFLOAT,
                extent: [1, 1, 1],
                usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
        )?;

        ImageView::new_default(image).map_err(|e| e.into())
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
            .ok_or("missing set 0 layout in simulate pass")?
            .clone();

        let set = DescriptorSet::new(
            descriptor_set_allocator,
            set_layout,
            [
                WriteDescriptorSet::buffer(0, pool.particle_buffer.clone()),
                WriteDescriptorSet::buffer(1, pool.dead_list_buffer.clone()),
                WriteDescriptorSet::buffer(2, pool.counters_buffer.clone()),
                WriteDescriptorSet::image_view_sampler(
                    3,
                    self.noise_texture.clone(),
                    self.noise_sampler.clone(),
                ),
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
        params: SimulatePushConstants,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let set = self
            .descriptor_sets
            .get(&pool_guid)
            .ok_or("simulate pass: no descriptor set for pool")?;

        builder
            .bind_pipeline_compute(self.pipeline.clone())?
            .bind_descriptor_sets(
                PipelineBindPoint::Compute,
                self.layout.clone(),
                0,
                set.clone(),
            )?
            .push_constants(self.layout.clone(), 0, params)?;

        let workgroups = params.capacity.div_ceil(64);
        unsafe {
            builder.dispatch([workgroups, 1, 1])?;
        }

        Ok(())
    }

    /// Replace the noise texture (used in Step 7 when real curl-noise is available).
    #[allow(dead_code)]
    pub fn set_noise_texture(&mut self, texture: Arc<ImageView>) {
        self.noise_texture = texture;
        // Descriptor sets will need to be rebuilt for all pools
        self.descriptor_sets.clear();
    }

    pub fn remove_pool(&mut self, pool_guid: &uuid::Uuid) {
        self.descriptor_sets.remove(pool_guid);
    }
}

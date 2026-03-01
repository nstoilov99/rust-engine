use std::sync::Arc;
use vulkano::device::Device;
use vulkano::format::Format;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::image::view::ImageView;
use vulkano::memory::allocator::{AllocationCreateInfo, StandardMemoryAllocator};
use vulkano::render_pass::RenderPass;
use vulkano::image::sampler::{Sampler, SamplerCreateInfo, Filter, SamplerAddressMode, BorderColor};
use vulkano::pipeline::graphics::depth_stencil::CompareOp;

/// Creates a depth-only render pass for shadow mapping
pub fn create_shadow_render_pass(
    device: Arc<Device>,
) -> Result<Arc<RenderPass>, Box<dyn std::error::Error>> {
    vulkano::single_pass_renderpass!(
        device,
        attachments: {
            depth: {
                format: Format::D32_SFLOAT,
                samples: 1,
                load_op: Clear,
                store_op: Store,
            }
        },
        pass: {
            color: [],
            depth_stencil: {depth}
        }
    )
    .map_err(|e| e.into())
}

/// Creates a shadow map (depth texture) - typically 2048x2048
pub fn create_shadow_map(
    _device: Arc<Device>,
    allocator: Arc<StandardMemoryAllocator>,
    size: u32,
) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
    let image = Image::new(
        allocator,
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::D32_SFLOAT,
            extent: [size, size, 1],
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
            ..Default::default()
        },
        AllocationCreateInfo::default(),
    )?;

    ImageView::new_default(image).map_err(|e| e.into())
}

/// Creates a sampler for shadow map with depth comparison
pub fn create_shadow_sampler(
    device: Arc<Device>,
) -> Result<Arc<Sampler>, Box<dyn std::error::Error>> {
    Sampler::new(
        device,
        SamplerCreateInfo {
            mag_filter: Filter::Linear,
            min_filter: Filter::Linear,
            address_mode: [SamplerAddressMode::ClampToBorder; 3],
            border_color: BorderColor::FloatOpaqueWhite,
            compare: Some(CompareOp::LessOrEqual), // Enable shadow comparison
            ..Default::default()
        },
    ).map_err(|e| e.into())
}
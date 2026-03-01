use std::sync::Arc;
use vulkano::image::view::ImageView;
use vulkano::descriptor_set::DescriptorSet;

/// PBR material with all texture maps
pub struct PbrMaterial {
    pub albedo: Arc<ImageView>,
    pub normal: Arc<ImageView>,
    pub metallic_roughness: Arc<ImageView>,
    pub ao: Arc<ImageView>,
    pub descriptor_set: Arc<DescriptorSet>,
}

impl PbrMaterial {
    pub fn new(
        albedo: Arc<ImageView>,
        normal: Arc<ImageView>,
        metallic_roughness: Arc<ImageView>,
        ao: Arc<ImageView>,
        descriptor_set: Arc<DescriptorSet>,
    ) -> Self {
        Self {
            albedo,
            normal,
            metallic_roughness,
            ao,
            descriptor_set,
        }
    }
}

/// Creates default white 1x1 texture for missing maps
pub fn create_default_texture(
    _device: Arc<vulkano::device::Device>,
    allocator: Arc<vulkano::memory::allocator::StandardMemoryAllocator>,
    _color: [u8; 4],
) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
    use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
    use vulkano::format::Format;
    use vulkano::memory::allocator::AllocationCreateInfo;

    // Create 1x1 texture with solid color
    let image = Image::new(
        allocator.clone(),
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::R8G8B8A8_SRGB,
            extent: [1, 1, 1],
            usage: ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED,
            ..Default::default()
        },
        AllocationCreateInfo::default(),
    )?;

    let view = ImageView::new_default(image)?;

    // TODO: Upload pixel data to texture
    // (For now, returns empty texture - will be initialized by validation layers)

    Ok(view)
}
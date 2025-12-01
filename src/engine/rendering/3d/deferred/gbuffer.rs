//! G-Buffer setup for deferred rendering

use std::sync::Arc;
use vulkano::device::Device;
use vulkano::format::Format;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::image::view::ImageView;
use vulkano::memory::allocator::{AllocationCreateInfo, StandardMemoryAllocator};
use vulkano::render_pass::{RenderPass, Framebuffer, FramebufferCreateInfo};

/// G-Buffer attachments
pub struct GBuffer {
    pub position: Arc<ImageView>,    // RT0: RGB16F (world position)
    pub normal: Arc<ImageView>,      // RT1: RGB16F (world normal)
    pub albedo: Arc<ImageView>,      // RT2: RGBA8 (albedo + roughness)
    pub material: Arc<ImageView>,    // RT3: RGBA8 (metallic, AO, etc.)
    pub depth: Arc<ImageView>,       // Depth buffer
    pub framebuffer: Arc<Framebuffer>,
    pub render_pass: Arc<RenderPass>,
}

impl GBuffer {
    pub fn new(
        device: Arc<Device>,
        allocator: Arc<StandardMemoryAllocator>,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Create render pass for G-Buffer
        let render_pass = create_gbuffer_render_pass(device.clone())?;

        // Create position attachment (RGB16F)
        let position_image = Image::new(
            allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R16G16B16A16_SFLOAT,
                extent: [width, height, 1],
                usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;
        let position = ImageView::new_default(position_image)?;

        // Create normal attachment (RGB16F)
        let normal_image = Image::new(
            allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R16G16B16A16_SFLOAT,
                extent: [width, height, 1],
                usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;
        let normal = ImageView::new_default(normal_image)?;

        // Create albedo attachment (RGBA8)
        let albedo_image = Image::new(
            allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_UNORM,
                extent: [width, height, 1],
                usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;
        let albedo = ImageView::new_default(albedo_image)?;

        // Create material attachment (RGBA8)
        let material_image = Image::new(
            allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_UNORM,
                extent: [width, height, 1],
                usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;
        let material = ImageView::new_default(material_image)?;

        // Create depth attachment (D32F)
        let depth_image = Image::new(
            allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::D32_SFLOAT,
                extent: [width, height, 1],
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;
        let depth = ImageView::new_default(depth_image)?;

        // Create framebuffer
        let framebuffer = Framebuffer::new(
            render_pass.clone(),
            FramebufferCreateInfo {
                attachments: vec![
                    position.clone(),
                    normal.clone(),
                    albedo.clone(),
                    material.clone(),
                    depth.clone(),
                ],
                ..Default::default()
            },
        )?;

        Ok(Self {
            position,
            normal,
            albedo,
            material,
            depth,
            framebuffer,
            render_pass,
        })
    }
}

/// Create G-Buffer render pass with 4 color attachments + depth
fn create_gbuffer_render_pass(
    device: Arc<Device>,
) -> Result<Arc<RenderPass>, Box<dyn std::error::Error>> {
    use vulkano::render_pass::{
        AttachmentDescription, AttachmentReference,
        SubpassDescription, RenderPassCreateInfo,
    };
    use vulkano::render_pass::{AttachmentLoadOp, AttachmentStoreOp};
    use vulkano::image::SampleCount;

    let attachments = vec![
        // Attachment 0: Position (RGB16F)
        AttachmentDescription {
            format: Format::R16G16B16A16_SFLOAT,
            samples: SampleCount::Sample1,
            load_op: AttachmentLoadOp::Clear,
            store_op: AttachmentStoreOp::Store,
            initial_layout: vulkano::image::ImageLayout::Undefined,
            final_layout: vulkano::image::ImageLayout::ShaderReadOnlyOptimal,
            ..Default::default()
        },
        // Attachment 1: Normal (RGB16F)
        AttachmentDescription {
            format: Format::R16G16B16A16_SFLOAT,
            samples: SampleCount::Sample1,
            load_op: AttachmentLoadOp::Clear,
            store_op: AttachmentStoreOp::Store,
            initial_layout: vulkano::image::ImageLayout::Undefined,
            final_layout: vulkano::image::ImageLayout::ShaderReadOnlyOptimal,
            ..Default::default()
        },
        // Attachment 2: Albedo (RGBA8)
        AttachmentDescription {
            format: Format::R8G8B8A8_UNORM,
            samples: SampleCount::Sample1,
            load_op: AttachmentLoadOp::Clear,
            store_op: AttachmentStoreOp::Store,
            initial_layout: vulkano::image::ImageLayout::Undefined,
            final_layout: vulkano::image::ImageLayout::ShaderReadOnlyOptimal,
            ..Default::default()
        },
        // Attachment 3: Material (RGBA8)
        AttachmentDescription {
            format: Format::R8G8B8A8_UNORM,
            samples: SampleCount::Sample1,
            load_op: AttachmentLoadOp::Clear,
            store_op: AttachmentStoreOp::Store,
            initial_layout: vulkano::image::ImageLayout::Undefined,
            final_layout: vulkano::image::ImageLayout::ShaderReadOnlyOptimal,
            ..Default::default()
        },
        // Attachment 4: Depth (D32F)
        AttachmentDescription {
            format: Format::D32_SFLOAT,
            samples: SampleCount::Sample1,
            load_op: AttachmentLoadOp::Clear,
            store_op: AttachmentStoreOp::Store,
            initial_layout: vulkano::image::ImageLayout::Undefined,
            final_layout: vulkano::image::ImageLayout::DepthStencilAttachmentOptimal,
            ..Default::default()
        },
    ];

    let color_attachment_refs = vec![
        Some(AttachmentReference {
            attachment: 0,
            layout: vulkano::image::ImageLayout::ColorAttachmentOptimal,
            ..Default::default()
        }),
        Some(AttachmentReference {
            attachment: 1,
            layout: vulkano::image::ImageLayout::ColorAttachmentOptimal,
            ..Default::default()
        }),
        Some(AttachmentReference {
            attachment: 2,
            layout: vulkano::image::ImageLayout::ColorAttachmentOptimal,
            ..Default::default()
        }),
        Some(AttachmentReference {
            attachment: 3,
            layout: vulkano::image::ImageLayout::ColorAttachmentOptimal,
            ..Default::default()
        }),
    ];

    let depth_attachment_ref = Some(AttachmentReference {
        attachment: 4,
        layout: vulkano::image::ImageLayout::DepthStencilAttachmentOptimal,
        ..Default::default()
    });

    let subpass = SubpassDescription {
        color_attachments: color_attachment_refs,
        depth_stencil_attachment: depth_attachment_ref,
        ..Default::default()
    };

    Ok(RenderPass::new(
        device,
        RenderPassCreateInfo {
            attachments,
            subpasses: vec![subpass],
            ..Default::default()
        },
    )?)
}
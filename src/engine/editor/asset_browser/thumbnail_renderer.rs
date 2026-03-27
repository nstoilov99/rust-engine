//! GPU-based 3D thumbnail renderer for model assets.
//!
//! Renders models to an offscreen framebuffer using a simplified forward shader
//! with directional lighting. Long-lived resources (pipeline, render pass,
//! framebuffer) are cached; per-thumbnail vertex/index buffers are temporary.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, CommandBufferUsage, CopyImageToBufferInfo,
    PrimaryCommandBufferAbstract, RenderPassBeginInfo, SubpassBeginInfo, SubpassContents,
    SubpassEndInfo,
};
use vulkano::device::{Device, Queue};
use vulkano::format::Format;
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::pipeline::graphics::color_blend::{ColorBlendAttachmentState, ColorBlendState};
use vulkano::pipeline::graphics::depth_stencil::{DepthState, DepthStencilState};
use vulkano::pipeline::graphics::input_assembly::InputAssemblyState;
use vulkano::pipeline::graphics::multisample::MultisampleState;
use vulkano::pipeline::graphics::rasterization::RasterizationState;
use vulkano::pipeline::graphics::vertex_input::{Vertex as VertexTrait, VertexDefinition};
use vulkano::pipeline::graphics::viewport::{Viewport, ViewportState};
use vulkano::pipeline::graphics::GraphicsPipelineCreateInfo;
use vulkano::pipeline::layout::PipelineDescriptorSetLayoutCreateInfo;
use vulkano::pipeline::{
    DynamicState, GraphicsPipeline, Pipeline, PipelineLayout, PipelineShaderStageCreateInfo,
};
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};
use vulkano::sync::GpuFuture;

use crate::engine::assets::model_loader::{LoadedMesh, Model};
use crate::engine::rendering::rendering_3d::pipeline_3d::Vertex3D;

mod thumbnail_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/3d/thumbnail_vs.glsl",
    }
}

mod thumbnail_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/3d/thumbnail_fs.glsl",
    }
}

const RENDER_SIZE: u32 = 256;

/// GPU context needed for thumbnail rendering. Cloned from the main Renderer.
pub struct GpuThumbnailContext {
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub memory_allocator: Arc<StandardMemoryAllocator>,
    pub command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
}

impl GpuThumbnailContext {
    /// Clone all Arc references to create a new context for the renderer.
    pub fn clone_context(&self) -> Self {
        Self {
            device: self.device.clone(),
            queue: self.queue.clone(),
            memory_allocator: self.memory_allocator.clone(),
            command_buffer_allocator: self.command_buffer_allocator.clone(),
        }
    }
}

/// Offscreen 3D thumbnail renderer with cached GPU resources.
pub struct ThumbnailRenderer {
    _device: Arc<Device>,
    queue: Arc<Queue>,
    memory_allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    _render_pass: Arc<RenderPass>,
    pipeline: Arc<GraphicsPipeline>,
    color_image: Arc<Image>,
    _color_view: Arc<ImageView>,
    _depth_view: Arc<ImageView>,
    framebuffer: Arc<Framebuffer>,
    readback_buffer: vulkano::buffer::Subbuffer<[u8]>,
}

impl ThumbnailRenderer {
    /// Create a new thumbnail renderer. Call once, reuse for all thumbnails.
    pub fn new(ctx: GpuThumbnailContext) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Render pass: single color + depth
        let render_pass = vulkano::single_pass_renderpass!(
            ctx.device.clone(),
            attachments: {
                color: {
                    format: Format::R8G8B8A8_UNORM,
                    samples: 1,
                    load_op: Clear,
                    store_op: Store,
                },
                depth: {
                    format: Format::D32_SFLOAT,
                    samples: 1,
                    load_op: Clear,
                    store_op: DontCare,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {depth},
            }
        )?;

        // Pipeline
        let vs = thumbnail_vs::load(ctx.device.clone())?
            .entry_point("main")
            .ok_or("Missing vertex shader entry point")?;
        let fs = thumbnail_fs::load(ctx.device.clone())?
            .entry_point("main")
            .ok_or("Missing fragment shader entry point")?;

        let vertex_input_state = Vertex3D::per_vertex().definition(&vs)?;

        let stages = [
            PipelineShaderStageCreateInfo::new(vs),
            PipelineShaderStageCreateInfo::new(fs),
        ];

        let layout = PipelineLayout::new(
            ctx.device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                .into_pipeline_layout_create_info(ctx.device.clone())?,
        )?;

        let pipeline = GraphicsPipeline::new(
            ctx.device.clone(),
            None,
            GraphicsPipelineCreateInfo {
                stages: stages.into_iter().collect(),
                vertex_input_state: Some(vertex_input_state),
                input_assembly_state: Some(InputAssemblyState::default()),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(RasterizationState::default()),
                multisample_state: Some(MultisampleState::default()),
                depth_stencil_state: Some(DepthStencilState {
                    depth: Some(DepthState::simple()),
                    ..Default::default()
                }),
                color_blend_state: Some(ColorBlendState::with_attachment_states(
                    1,
                    ColorBlendAttachmentState::default(),
                )),
                dynamic_state: [DynamicState::Viewport].into_iter().collect(),
                subpass: Some(render_pass.clone().first_subpass().into()),
                ..GraphicsPipelineCreateInfo::layout(layout)
            },
        )?;

        // Offscreen attachments
        let color_image = Image::new(
            ctx.memory_allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_UNORM,
                extent: [RENDER_SIZE, RENDER_SIZE, 1],
                usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;
        let color_view = ImageView::new_default(color_image.clone())?;

        let depth_image = Image::new(
            ctx.memory_allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::D32_SFLOAT,
                extent: [RENDER_SIZE, RENDER_SIZE, 1],
                usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;
        let depth_view = ImageView::new_default(depth_image)?;

        let framebuffer = Framebuffer::new(
            render_pass.clone(),
            FramebufferCreateInfo {
                attachments: vec![color_view.clone(), depth_view.clone()],
                ..Default::default()
            },
        )?;

        // Readback staging buffer (RENDER_SIZE * RENDER_SIZE * 4 bytes RGBA)
        let readback_buffer = Buffer::new_slice::<u8>(
            ctx.memory_allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::TRANSFER_DST,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_HOST
                    | MemoryTypeFilter::HOST_RANDOM_ACCESS,
                ..Default::default()
            },
            (RENDER_SIZE * RENDER_SIZE * 4) as u64,
        )?;

        Ok(Self {
            _device: ctx.device,
            queue: ctx.queue,
            memory_allocator: ctx.memory_allocator,
            command_buffer_allocator: ctx.command_buffer_allocator,
            _render_pass: render_pass,
            pipeline,
            color_image,
            _color_view: color_view,
            _depth_view: depth_view,
            framebuffer,
            readback_buffer,
        })
    }

    /// Render a model and return the thumbnail as an RGBA image.
    pub fn render_model(
        &self,
        model: &Model,
        thumbnail_size: u32,
    ) -> Result<egui::ColorImage, Box<dyn std::error::Error + Send + Sync>> {
        if model.meshes.is_empty() {
            return Err("Model has no meshes".into());
        }

        // Compute combined bounding sphere
        let (center, radius) = combined_bounding_sphere(&model.meshes);
        let radius = if radius < 0.001 { 1.0 } else { radius };

        // Camera matrices
        let (view, proj) = compute_camera_matrices(center, radius);
        let view_projection = proj * view;

        // Upload all mesh geometry
        let mut gpu_meshes = Vec::with_capacity(model.meshes.len());
        for mesh in &model.meshes {
            if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                continue;
            }

            let vertex_buffer = Buffer::from_iter(
                self.memory_allocator.clone(),
                BufferCreateInfo {
                    usage: BufferUsage::VERTEX_BUFFER,
                    ..Default::default()
                },
                AllocationCreateInfo {
                    memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                        | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                    ..Default::default()
                },
                mesh.vertices.iter().copied(),
            )?;

            let index_buffer = Buffer::from_iter(
                self.memory_allocator.clone(),
                BufferCreateInfo {
                    usage: BufferUsage::INDEX_BUFFER,
                    ..Default::default()
                },
                AllocationCreateInfo {
                    memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                        | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                    ..Default::default()
                },
                mesh.indices.iter().copied(),
            )?;

            gpu_meshes.push((vertex_buffer, index_buffer, mesh.indices.len() as u32));
        }

        if gpu_meshes.is_empty() {
            return Err("Model has no renderable geometry".into());
        }

        // Record command buffer
        let mut builder = AutoCommandBufferBuilder::primary(
            self.command_buffer_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        // Background color: dark gray
        builder.begin_render_pass(
            RenderPassBeginInfo {
                clear_values: vec![
                    Some([0.16, 0.16, 0.18, 1.0].into()), // color
                    Some(1.0f32.into()),                    // depth
                ],
                ..RenderPassBeginInfo::framebuffer(self.framebuffer.clone())
            },
            SubpassBeginInfo {
                contents: SubpassContents::Inline,
                ..Default::default()
            },
        )?;

        builder.set_viewport(
            0,
            [Viewport {
                offset: [0.0, 0.0],
                extent: [RENDER_SIZE as f32, RENDER_SIZE as f32],
                depth_range: 0.0..=1.0,
            }]
            .into_iter()
            .collect(),
        )?;

        builder.bind_pipeline_graphics(self.pipeline.clone())?;

        // Push constants: model (identity) + view_projection
        let model_matrix = Mat4::IDENTITY;
        let push_data = thumbnail_vs::PushConstants {
            model: model_matrix.to_cols_array_2d(),
            view_projection: view_projection.to_cols_array_2d(),
        };
        builder.push_constants(self.pipeline.layout().clone(), 0, push_data)?;

        // Draw all meshes
        for (vb, ib, index_count) in &gpu_meshes {
            builder.bind_vertex_buffers(0, vb.clone())?;
            builder.bind_index_buffer(ib.clone())?;
            // SAFETY: vertex/index buffers are valid and match the pipeline's vertex input state.
            unsafe { builder.draw_indexed(*index_count, 1, 0, 0, 0)? };
        }

        builder.end_render_pass(SubpassEndInfo::default())?;

        // Copy rendered image to readback buffer
        builder.copy_image_to_buffer(CopyImageToBufferInfo::image_buffer(
            self.color_image.clone(),
            self.readback_buffer.clone(),
        ))?;

        let command_buffer = builder.build()?;

        // Submit and wait
        command_buffer
            .execute(self.queue.clone())?
            .then_signal_fence_and_flush()?
            .wait(None)?;

        // Read back pixels
        let data = self.readback_buffer.read()?;
        let rgba_image = image::RgbaImage::from_raw(RENDER_SIZE, RENDER_SIZE, data.to_vec())
            .ok_or("Failed to create image from readback data")?;

        // Vulkan clip-space Y is inverted relative to image convention — flip vertically
        let rgba_image = image::imageops::flip_vertical(&rgba_image);

        // Resize to thumbnail size
        let resized = image::imageops::resize(
            &rgba_image,
            thumbnail_size,
            thumbnail_size,
            image::imageops::FilterType::Triangle,
        );

        let size = [thumbnail_size as usize, thumbnail_size as usize];
        Ok(egui::ColorImage::from_rgba_unmultiplied(
            size,
            resized.as_raw(),
        ))
    }
}

/// Compute combined bounding sphere from multiple meshes.
fn combined_bounding_sphere(meshes: &[LoadedMesh]) -> (Vec3, f32) {
    if meshes.is_empty() {
        return (Vec3::ZERO, 1.0);
    }
    if meshes.len() == 1 {
        return (meshes[0].center, meshes[0].radius);
    }

    // Use average center, then find max extent
    let center: Vec3 =
        meshes.iter().map(|m| m.center).sum::<Vec3>() / meshes.len() as f32;

    let radius = meshes
        .iter()
        .map(|m| (m.center - center).length() + m.radius)
        .fold(0.0f32, f32::max);

    (center, radius)
}

/// Compute view and projection matrices for thumbnail camera.
///
/// Camera orbits at 45 deg azimuth, 30 deg elevation, looking at the
/// bounding sphere center. Y-up coordinate system.
fn compute_camera_matrices(center: Vec3, radius: f32) -> (Mat4, Mat4) {
    let fov = 45.0_f32.to_radians();
    let azimuth = 45.0_f32.to_radians();
    let elevation = 30.0_f32.to_radians();

    // Distance so the bounding sphere fills the view
    let distance = radius / (fov / 2.0).sin() * 1.3;

    let x = distance * elevation.cos() * azimuth.sin();
    let y = distance * elevation.sin();
    let z = distance * elevation.cos() * azimuth.cos();

    let camera_pos = center + Vec3::new(x, y, z);

    let near = distance * 0.01;
    let far = distance * 10.0;

    let view = Mat4::look_at_rh(camera_pos, center, Vec3::Y);
    let proj = Mat4::perspective_rh(fov, 1.0, near, far);

    (view, proj)
}

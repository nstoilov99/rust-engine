//! Vulkan renderer for egui using Vulkano 0.34

use egui::{ClippedPrimitive, Rect, TexturesDelta};
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::{
    buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer},
    command_buffer::{
        allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, CommandBufferUsage,
        PrimaryAutoCommandBuffer, RenderPassBeginInfo, SubpassBeginInfo, SubpassEndInfo,
    },
    descriptor_set::{
        allocator::StandardDescriptorSetAllocator, PersistentDescriptorSet,
        WriteDescriptorSet,
    },
    device::{Device, Queue},
    format::Format,
    image::{
        sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo},
        view::ImageView,
        Image, ImageCreateInfo, ImageType, ImageUsage,
    },
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator},
    pipeline::{
        graphics::{
            color_blend::{ColorBlendAttachmentState, ColorBlendState},
            input_assembly::InputAssemblyState,
            multisample::MultisampleState,
            rasterization::RasterizationState,
            vertex_input::{Vertex, VertexDefinition},
            viewport::{Viewport, ViewportState},
            GraphicsPipelineCreateInfo,
        },
        layout::PipelineDescriptorSetLayoutCreateInfo,
        DynamicState, GraphicsPipeline, PipelineLayout, PipelineShaderStageCreateInfo,
    },
    render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass, Subpass},
    pipeline::{PipelineBindPoint, Pipeline},
    command_buffer::{CopyBufferToImageInfo, PrimaryCommandBufferAbstract},
    sync::GpuFuture,
};

/// egui vertex format
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct EguiVertex {
    pub position: [f32; 2],
    pub tex_coords: [f32; 2],
    pub color: [f32; 4],
}

vulkano::impl_vertex!(EguiVertex, position, tex_coords, color);

/// Push constants for screen size
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct PushConstants {
    screen_size: [f32; 2],
}

pub struct EguiRenderer {
    device: Arc<Device>,
    queue: Arc<Queue>,
    memory_allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    pipeline: Arc<GraphicsPipeline>,
    render_pass: Arc<RenderPass>,
    sampler: Arc<Sampler>,
    textures: HashMap<egui::TextureId, Arc<ImageView>>,
}

impl EguiRenderer {
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        swapchain_format: Format,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));
        let command_buffer_allocator = Arc::new(StandardCommandBufferAllocator::new(
            device.clone(),
            Default::default(),
        ));
        let descriptor_set_allocator = Arc::new(StandardDescriptorSetAllocator::new(
            device.clone(),
            Default::default(),
        ));

        // Create render pass
        let render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: swapchain_format,
                    samples: 1,
                    load_op: Load,
                    store_op: Store,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {}
            }
        )?;

        // Create pipeline
        let pipeline = Self::create_pipeline(device.clone(), render_pass.clone())?;

        // Create sampler for textures
        let sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                ..Default::default()
            },
        )?;

        Ok(Self {
            device,
            queue,
            memory_allocator,
            command_buffer_allocator,
            descriptor_set_allocator,
            pipeline,
            render_pass,
            sampler,
            textures: HashMap::new(),
        })
    }

    fn create_pipeline(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
        // Load shaders
        let vs = vs::load(device.clone())?;
        let fs = fs::load(device.clone())?;

        let vertex_input_state = EguiVertex::per_vertex()
            .definition(&vs.entry_point("main").unwrap().info().input_interface)?;

        let stages = [
            PipelineShaderStageCreateInfo::new(vs.entry_point("main").unwrap()),
            PipelineShaderStageCreateInfo::new(fs.entry_point("main").unwrap()),
        ];

        let layout = PipelineLayout::new(
            device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                .into_pipeline_layout_create_info(device.clone())
                .map_err(|e| format!("Pipeline layout error: {:?}", e))?,
        )?;

        let subpass = Subpass::from(render_pass, 0).unwrap();

        Ok(GraphicsPipeline::new(
            device,
            None,
            GraphicsPipelineCreateInfo {
                stages: stages.into_iter().collect(),
                vertex_input_state: Some(vertex_input_state),
                input_assembly_state: Some(InputAssemblyState::default()),
                viewport_state: Some(ViewportState::default()),
                rasterization_state: Some(RasterizationState::default()),
                multisample_state: Some(MultisampleState::default()),
                color_blend_state: Some(ColorBlendState::with_attachment_states(
                    subpass.num_color_attachments(),
                    ColorBlendAttachmentState {
                        blend: Some(vulkano::pipeline::graphics::color_blend::AttachmentBlend::alpha()),
                        ..Default::default()
                    },
                )),
                dynamic_state: [DynamicState::Viewport, DynamicState::Scissor].into_iter().collect(),
                subpass: Some(subpass.into()),
                ..GraphicsPipelineCreateInfo::layout(layout)
            },
        )?)
    }

    /// Upload or update a texture
    fn upload_texture(&mut self, texture_id: egui::TextureId, image_delta: &egui::epaint::ImageDelta) -> Result<(), Box<dyn std::error::Error>> {
        let delta_size = image_delta.image.size();

        // Handle both Color and Font image types
        let pixels: Vec<u8> = match &image_delta.image {
            egui::ImageData::Color(img) => {
                img.pixels.iter()
                    .flat_map(|c| [c.r(), c.g(), c.b(), c.a()])
                    .collect()
            }
            egui::ImageData::Font(font_img) => {
                // Font images are single-channel coverage values, expand to RGBA
                font_img.srgba_pixels(None)
                    .flat_map(|c| [c.r(), c.g(), c.b(), c.a()])
                    .collect()
            }
        };

        // Check if this is a partial update or a full texture creation
        if let Some(pos) = image_delta.pos {
            // Partial update - update existing texture at offset
            if let Some(existing_view) = self.textures.get(&texture_id) {
                let existing_image = existing_view.image();

                // Upload pixels via staging buffer
                let buffer = Buffer::from_iter(
                    self.memory_allocator.clone(),
                    BufferCreateInfo {
                        usage: BufferUsage::TRANSFER_SRC,
                        ..Default::default()
                    },
                    AllocationCreateInfo {
                        memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                        ..Default::default()
                    },
                    pixels,
                )?;

                let mut builder = AutoCommandBufferBuilder::primary(
                    &self.command_buffer_allocator,
                    self.queue.queue_family_index(),
                    CommandBufferUsage::OneTimeSubmit,
                )?;

                // Copy to region of existing texture
                use vulkano::command_buffer::CopyBufferToImageInfo;
                use vulkano::image::ImageSubresourceLayers;

                builder.copy_buffer_to_image(CopyBufferToImageInfo {
                    regions: [vulkano::command_buffer::BufferImageCopy {
                        image_subresource: ImageSubresourceLayers {
                            aspects: vulkano::image::ImageAspects::COLOR,
                            mip_level: 0,
                            array_layers: 0..1,
                        },
                        image_offset: [pos[0] as u32, pos[1] as u32, 0],
                        image_extent: [delta_size[0] as u32, delta_size[1] as u32, 1],
                        ..Default::default()
                    }]
                    .into(),
                    ..CopyBufferToImageInfo::buffer_image(buffer, existing_image.clone())
                })?;

                let command_buffer = builder.build()?;
                command_buffer
                    .execute(self.queue.clone())?
                    .then_signal_fence_and_flush()?
                    .wait(None)?;
            }
        } else {
            // Full texture creation
            let image = Image::new(
                self.memory_allocator.clone(),
                ImageCreateInfo {
                    image_type: ImageType::Dim2d,
                    format: Format::R8G8B8A8_SRGB,
                    extent: [delta_size[0] as u32, delta_size[1] as u32, 1],
                    usage: ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED,
                    ..Default::default()
                },
                AllocationCreateInfo::default(),
            )?;

            // Upload pixels via staging buffer
            let buffer = Buffer::from_iter(
                self.memory_allocator.clone(),
                BufferCreateInfo {
                    usage: BufferUsage::TRANSFER_SRC,
                    ..Default::default()
                },
                AllocationCreateInfo {
                    memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                    ..Default::default()
                },
                pixels,
            )?;

            let mut builder = AutoCommandBufferBuilder::primary(
                &self.command_buffer_allocator,
                self.queue.queue_family_index(),
                CommandBufferUsage::OneTimeSubmit,
            )?;

            builder.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(buffer, image.clone()))?;

            let command_buffer = builder.build()?;
            command_buffer
                .execute(self.queue.clone())?
                .then_signal_fence_and_flush()?
                .wait(None)?;

            // Create image view and store
            let view = ImageView::new_default(image)?;
            self.textures.insert(texture_id, view);
        }

        Ok(())
    }

    pub fn render(
        &mut self,
        target_image: Arc<Image>,
        clipped_primitives: Vec<ClippedPrimitive>,
        textures_delta: TexturesDelta,
        screen_rect: Rect,
    ) -> Result<Arc<vulkano::command_buffer::PrimaryAutoCommandBuffer<Arc<vulkano::command_buffer::allocator::StandardCommandBufferAllocator>>>, Box<dyn std::error::Error>> {
        // Handle texture updates
        for (texture_id, image_delta) in &textures_delta.set {
            self.upload_texture(*texture_id, image_delta)?;
        }

        // Create framebuffer
        let view = ImageView::new_default(target_image.clone())?;
        let framebuffer = Framebuffer::new(
            self.render_pass.clone(),
            FramebufferCreateInfo {
                attachments: vec![view],
                ..Default::default()
            },
        )?;

        // Build command buffer
        let mut builder = AutoCommandBufferBuilder::primary(
            &self.command_buffer_allocator,
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        let extent = target_image.extent();
        let screen_size = [screen_rect.width(), screen_rect.height()];

        builder.begin_render_pass(
            RenderPassBeginInfo {
                clear_values: vec![None],
                ..RenderPassBeginInfo::framebuffer(framebuffer)
            },
            SubpassBeginInfo {
                contents: vulkano::command_buffer::SubpassContents::Inline,
                ..Default::default()
            },
        )?;

        builder.bind_pipeline_graphics(self.pipeline.clone())?;

        // Set viewport
        let viewport = Viewport {
            offset: [0.0, 0.0],
            extent: [extent[0] as f32, extent[1] as f32],
            depth_range: 0.0..=1.0,
        };
        builder.set_viewport(0, [viewport].into_iter().collect())?;

        // Render each primitive
        for clipped_primitive in clipped_primitives {
            let mesh = match &clipped_primitive.primitive {
                egui::epaint::Primitive::Mesh(mesh) => mesh,
                egui::epaint::Primitive::Callback(_) => continue, // Skip paint callbacks
            };

            if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                continue;
            }

            // Convert egui vertices to our format
            let vertices: Vec<EguiVertex> = mesh.vertices.iter().map(|v| {
                EguiVertex {
                    position: [v.pos.x, v.pos.y],
                    tex_coords: [v.uv.x, v.uv.y],
                    color: [
                        v.color.r() as f32 / 255.0,
                        v.color.g() as f32 / 255.0,
                        v.color.b() as f32 / 255.0,
                        v.color.a() as f32 / 255.0,
                    ],
                }
            }).collect();

            // Create vertex buffer
            let vertex_buffer: Subbuffer<[EguiVertex]> = Buffer::from_iter(
                self.memory_allocator.clone(),
                BufferCreateInfo {
                    usage: BufferUsage::VERTEX_BUFFER,
                    ..Default::default()
                },
                AllocationCreateInfo {
                    memory_type_filter: MemoryTypeFilter::PREFER_DEVICE | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                    ..Default::default()
                },
                vertices,
            )?;

            // Create index buffer
            let index_buffer: Subbuffer<[u32]> = Buffer::from_iter(
                self.memory_allocator.clone(),
                BufferCreateInfo {
                    usage: BufferUsage::INDEX_BUFFER,
                    ..Default::default()
                },
                AllocationCreateInfo {
                    memory_type_filter: MemoryTypeFilter::PREFER_DEVICE | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                    ..Default::default()
                },
                mesh.indices.iter().copied(),
            )?;

            // Get texture for this primitive
            let texture_view = self.textures.get(&mesh.texture_id)
                .ok_or("Texture not found")?;

            // Create descriptor set for this texture
            let layout = self.pipeline.layout().set_layouts().get(0)
                .ok_or("No descriptor set layout")?;

            let descriptor_set = PersistentDescriptorSet::new(
                &self.descriptor_set_allocator,
                layout.clone(),
                [
                    WriteDescriptorSet::image_view_sampler(0, texture_view.clone(), self.sampler.clone()),
                ],
                [],
            )?;

            // Push constants for screen size
            let push_constants = PushConstants {
                screen_size,
            };

            // Set scissor rectangle for clipping
            let clip_rect = clipped_primitive.clip_rect;
            let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [
                    clip_rect.min.x.max(0.0) as u32,
                    clip_rect.min.y.max(0.0) as u32,
                ],
                extent: [
                    (clip_rect.max.x - clip_rect.min.x).max(0.0) as u32,
                    (clip_rect.max.y - clip_rect.min.y).max(0.0) as u32,
                ],
            };

            builder.set_scissor(0, [scissor].into_iter().collect())?;

            // Bind descriptor set
            builder.bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.pipeline.layout().clone(),
                0,
                descriptor_set,
            )?;

            // Push constants
            builder.push_constants(
                self.pipeline.layout().clone(),
                0,
                push_constants,
            )?;

            // Bind buffers and draw
            builder.bind_vertex_buffers(0, vertex_buffer)?;
            builder.bind_index_buffer(index_buffer)?;
            builder.draw_indexed(mesh.indices.len() as u32, 1, 0, 0, 0)?;
        }

        builder.end_render_pass(SubpassEndInfo::default())?;

        // Handle texture deletions
        for texture_id in &textures_delta.free {
            self.textures.remove(texture_id);
        }

        Ok(builder.build()?)
    }
}

// Vertex shader
mod vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        src: r"
            #version 450

            layout(location = 0) in vec2 position;
            layout(location = 1) in vec2 tex_coords;
            layout(location = 2) in vec4 color;

            layout(location = 0) out vec4 v_color;
            layout(location = 1) out vec2 v_tex_coords;

            layout(push_constant) uniform PushConstants {
                vec2 screen_size;
            } push_constants;

            void main() {
                gl_Position = vec4(
                    2.0 * position.x / push_constants.screen_size.x - 1.0,
                    2.0 * position.y / push_constants.screen_size.y - 1.0,
                    0.0, 1.0
                );
                v_color = color;
                v_tex_coords = tex_coords;
            }
        "
    }
}

// Fragment shader
mod fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        src: r"
            #version 450

            layout(location = 0) in vec4 v_color;
            layout(location = 1) in vec2 v_tex_coords;

            layout(location = 0) out vec4 f_color;

            layout(binding = 0, set = 0) uniform sampler2D font_texture;

            void main() {
                f_color = v_color * texture(font_texture, v_tex_coords);
            }
        "
    }
}

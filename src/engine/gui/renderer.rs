//! Vulkan renderer for egui using Vulkano 0.35

use egui::{ClippedPrimitive, Rect, TexturesDelta};
use smallvec::smallvec;
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::{
    buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer},
    command_buffer::{
        allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, CommandBufferUsage,
        RenderPassBeginInfo, SubpassBeginInfo, SubpassEndInfo,
    },
    descriptor_set::{
        allocator::StandardDescriptorSetAllocator, DescriptorSet,
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

/// A texture upload that has been submitted to the GPU but not yet confirmed complete.
struct PendingUpload {
    texture_id: egui::TextureId,
    fence: vulkano::sync::future::FenceSignalFuture<Box<dyn GpuFuture>>,
    /// New image view to install once the fence signals (None for partial updates).
    new_view: Option<Arc<ImageView>>,
}

/// egui vertex format
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable, Vertex)]
#[repr(C)]
pub struct EguiVertex {
    #[format(R32G32_SFLOAT)]
    pub position: [f32; 2],
    #[format(R32G32_SFLOAT)]
    pub tex_coords: [f32; 2],
    #[format(R32G32B32A32_SFLOAT)]
    pub color: [f32; 4],
}

/// Push constants for screen size
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct PushConstants {
    screen_size: [f32; 2],
}

pub struct EguiRenderer {
    _device: Arc<Device>,
    queue: Arc<Queue>,
    memory_allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    pipeline: Arc<GraphicsPipeline>,
    render_pass: Arc<RenderPass>,
    sampler: Arc<Sampler>,
    /// Cached texture image views (uploaded textures)
    texture_cache: HashMap<egui::TextureId, Arc<ImageView>>,
    /// Cached descriptor sets per texture (avoids re-creating per primitive)
    descriptor_set_cache: HashMap<egui::TextureId, Arc<DescriptorSet>>,
    /// Cached framebuffers by target image pointer (avoids per-frame creation)
    framebuffer_cache: HashMap<usize, Arc<Framebuffer>>,
    /// Batched vertex data for entire frame (avoids per-primitive buffer allocation)
    batched_vertices: Vec<EguiVertex>,
    /// Batched index data for entire frame
    batched_indices: Vec<u32>,
    /// Texture uploads submitted to GPU but not yet confirmed complete.
    pending_uploads: Vec<PendingUpload>,
    /// 1x1 white placeholder texture used while real textures are still uploading.
    placeholder_view: Arc<ImageView>,
}

/// Counter for generating unique user texture IDs
static NEXT_USER_TEXTURE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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

        // 1x1 white placeholder: shown while a real texture is still uploading.
        let placeholder_image = Image::new(
            memory_allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_SRGB,
                extent: [1, 1, 1],
                usage: ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo::default(),
        )?;
        let placeholder_buf = Buffer::from_iter(
            memory_allocator.clone(),
            BufferCreateInfo { usage: BufferUsage::TRANSFER_SRC, ..Default::default() },
            AllocationCreateInfo { memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE, ..Default::default() },
            [255u8, 255, 255, 255],
        )?;
        let mut ph_builder = AutoCommandBufferBuilder::primary(
            command_buffer_allocator.clone(),
            queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;
        ph_builder.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(placeholder_buf, placeholder_image.clone()))?;
        let ph_cb = ph_builder.build()?;
        ph_cb.execute(queue.clone())?.then_signal_fence_and_flush()?.wait(None)?;
        let placeholder_view = ImageView::new_default(placeholder_image)?;

        Ok(Self {
            _device: device,
            queue,
            memory_allocator,
            command_buffer_allocator,
            descriptor_set_allocator,
            pipeline,
            render_pass,
            sampler,
            texture_cache: HashMap::new(),
            descriptor_set_cache: HashMap::new(),
            framebuffer_cache: HashMap::new(),
            batched_vertices: Vec::with_capacity(16384),
            batched_indices: Vec::with_capacity(32768),
            pending_uploads: Vec::new(),
            placeholder_view,
        })
    }

    /// Clear framebuffer cache (call on swapchain recreation)
    pub fn clear_framebuffer_cache(&mut self) {
        self.framebuffer_cache.clear();
    }

    fn create_pipeline(
        device: Arc<Device>,
        render_pass: Arc<RenderPass>,
    ) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
        // Load shaders
        let vs = vs::load(device.clone())?;
        let fs = fs::load(device.clone())?;

        let vs_entry = vs.entry_point("main").unwrap();
        let vertex_input_state = EguiVertex::per_vertex()
            .definition(&vs_entry)?;

        let fs_entry = fs.entry_point("main").unwrap();
        let stages = [
            PipelineShaderStageCreateInfo::new(vs_entry),
            PipelineShaderStageCreateInfo::new(fs_entry),
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

    /// Non-blocking poll of pending texture uploads.
    ///
    /// Checks each fence with a short timeout.  Completed uploads are
    /// installed into the texture cache; unfinished ones are kept for next
    /// frame (the placeholder texture remains visible in the meantime).
    ///
    /// After signalling we call `cleanup_finished()` so Vulkano marks the
    /// upload command buffer as reclaimable.
    fn poll_pending_uploads(&mut self) {
        let mut still_pending = Vec::new();
        for mut upload in self.pending_uploads.drain(..) {
            // Use a very short wait — if the GPU hasn't finished, keep it pending.
            match upload.fence.wait(Some(std::time::Duration::from_micros(100))) {
                Ok(()) => {
                    upload.fence.cleanup_finished();
                    if let Some(view) = upload.new_view {
                        self.texture_cache.insert(upload.texture_id, view);
                    }
                    self.descriptor_set_cache.remove(&upload.texture_id);
                }
                Err(_) => {
                    // Not ready yet — keep for next frame.
                    still_pending.push(upload);
                }
            }
        }
        self.pending_uploads = still_pending;
    }

    /// Submit a non-blocking texture upload / update.
    ///
    /// New textures become visible through the placeholder until the fence
    /// signals. Partial updates are written in-place to the existing image;
    /// the old content remains visible until the upload completes.
    fn upload_texture(&mut self, texture_id: egui::TextureId, image_delta: &egui::epaint::ImageDelta) -> Result<(), Box<dyn std::error::Error>> {
        let delta_size = image_delta.image.size();

        let pixels: Vec<u8> = match &image_delta.image {
            egui::ImageData::Color(img) => {
                img.pixels.iter()
                    .flat_map(|c| [c.r(), c.g(), c.b(), c.a()])
                    .collect()
            }
        };

        if let Some(pos) = image_delta.pos {
            // Partial update — write into the existing GPU image.
            if let Some(existing_view) = self.texture_cache.get(&texture_id) {
                let existing_image = existing_view.image();

                let buffer = Buffer::from_iter(
                    self.memory_allocator.clone(),
                    BufferCreateInfo { usage: BufferUsage::TRANSFER_SRC, ..Default::default() },
                    AllocationCreateInfo { memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE, ..Default::default() },
                    pixels,
                )?;

                let mut builder = AutoCommandBufferBuilder::primary(
                    self.command_buffer_allocator.clone(),
                    self.queue.queue_family_index(),
                    CommandBufferUsage::OneTimeSubmit,
                )?;

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
                    }].into(),
                    ..CopyBufferToImageInfo::buffer_image(buffer, existing_image.clone())
                })?;

                let fence = builder.build()?.execute(self.queue.clone()).map(|f| f.boxed())?.then_signal_fence_and_flush()?;
                self.pending_uploads.push(PendingUpload {
                    texture_id,
                    fence,
                    new_view: None,
                });
            }
        } else {
            // Full texture creation — render with placeholder until ready.
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

            let buffer = Buffer::from_iter(
                self.memory_allocator.clone(),
                BufferCreateInfo { usage: BufferUsage::TRANSFER_SRC, ..Default::default() },
                AllocationCreateInfo { memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE, ..Default::default() },
                pixels,
            )?;

            let mut builder = AutoCommandBufferBuilder::primary(
                self.command_buffer_allocator.clone(),
                self.queue.queue_family_index(),
                CommandBufferUsage::OneTimeSubmit,
            )?;
            builder.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(buffer, image.clone()))?;

            let fence = builder.build()?.execute(self.queue.clone()).map(|f| f.boxed())?.then_signal_fence_and_flush()?;
            let view = ImageView::new_default(image)?;

            // Install placeholder so the texture id is drawable immediately.
            if !self.texture_cache.contains_key(&texture_id) {
                self.texture_cache.insert(texture_id, self.placeholder_view.clone());
                self.descriptor_set_cache.remove(&texture_id);
            }

            self.pending_uploads.push(PendingUpload {
                texture_id,
                fence,
                new_view: Some(view),
            });
        }

        Ok(())
    }

    /// Register an external Vulkan image view as an egui texture
    ///
    /// This is used for render-to-texture scenarios like the viewport.
    /// Returns an egui TextureId that can be used to display the image.
    pub fn register_native_texture(&mut self, image_view: Arc<ImageView>) -> egui::TextureId {
        let id = NEXT_USER_TEXTURE_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let texture_id = egui::TextureId::User(id);
        self.texture_cache.insert(texture_id, image_view);
        texture_id
    }

    /// Update an existing native texture with a new image view
    ///
    /// Used when the viewport is resized and the texture needs to be recreated.
    pub fn update_native_texture(&mut self, texture_id: egui::TextureId, image_view: Arc<ImageView>) {
        self.texture_cache.insert(texture_id, image_view);
        // Invalidate cached descriptor set since texture changed
        self.descriptor_set_cache.remove(&texture_id);
    }

    /// Remove a native texture
    pub fn unregister_native_texture(&mut self, texture_id: egui::TextureId) {
        self.texture_cache.remove(&texture_id);
        self.descriptor_set_cache.remove(&texture_id);
    }

    pub fn render(
        &mut self,
        target_image: Arc<Image>,
        clipped_primitives: Vec<ClippedPrimitive>,
        textures_delta: TexturesDelta,
        screen_rect: Rect,
    ) -> Result<Arc<vulkano::command_buffer::PrimaryAutoCommandBuffer>, Box<dyn std::error::Error>> {
        crate::profile_function!();

        // Submit texture uploads (non-blocking: each gets its own command
        // buffer + fence, submitted to the GPU queue immediately).
        {
            crate::profile_scope!("texture_updates");
            for (texture_id, image_delta) in &textures_delta.set {
                self.upload_texture(*texture_id, image_delta)?;
            }
        }

        // Drain ALL pending uploads (previous frames + this frame's) so
        // every texture uses its real image view before we build the draw
        // command buffer.  Most uploads complete instantly on the GPU; the
        // non-blocking poll catches those, and force-wait handles any
        // stragglers.
        self.poll_pending_uploads();

        // Get or create cached framebuffer for this target image
        let cache_key = Arc::as_ptr(&target_image) as usize;
        let framebuffer = if let Some(fb) = self.framebuffer_cache.get(&cache_key) {
            fb.clone()
        } else {
            let view = ImageView::new_default(target_image.clone())?;
            let fb = Framebuffer::new(
                self.render_pass.clone(),
                FramebufferCreateInfo {
                    attachments: vec![view],
                    ..Default::default()
                },
            )?;
            self.framebuffer_cache.insert(cache_key, fb.clone());
            fb
        };

        let extent = target_image.extent();
        let screen_size = [screen_rect.width(), screen_rect.height()];

        // Phase 1: Batch all primitives into single vertex/index buffers
        // This avoids per-primitive GPU buffer allocation (major performance win)
        crate::profile_scope!("primitive_batching");

        // Clear batched data from previous frame (capacity preserved)
        self.batched_vertices.clear();
        self.batched_indices.clear();

        // Collect draw commands with their offsets into the batched buffers
        struct DrawCommand {
            vertex_offset: u32,
            index_offset: u32,
            index_count: u32,
            texture_id: egui::TextureId,
            clip_rect: egui::Rect,
        }
        let mut draw_commands = Vec::with_capacity(clipped_primitives.len());

        for clipped_primitive in &clipped_primitives {
            let mesh = match &clipped_primitive.primitive {
                egui::epaint::Primitive::Mesh(mesh) => mesh,
                egui::epaint::Primitive::Callback(_) => continue,
            };

            if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                continue;
            }

            let vertex_offset = self.batched_vertices.len() as u32;
            let index_offset = self.batched_indices.len() as u32;

            // Append vertices (convert egui format to our format)
            // Must use egui::Rgba for proper sRGB→linear conversion
            // (GPU expects linear colors when writing to sRGB framebuffer)
            self.batched_vertices.extend(mesh.vertices.iter().map(|v| {
                let linear: egui::Rgba = v.color.into();
                EguiVertex {
                    position: [v.pos.x, v.pos.y],
                    tex_coords: [v.uv.x, v.uv.y],
                    color: [linear.r(), linear.g(), linear.b(), linear.a()],
                }
            }));

            // Append indices (offset by vertex_offset since we're batching)
            self.batched_indices.extend(mesh.indices.iter().copied());

            draw_commands.push(DrawCommand {
                vertex_offset,
                index_offset,
                index_count: mesh.indices.len() as u32,
                texture_id: mesh.texture_id,
                clip_rect: clipped_primitive.clip_rect,
            });
        }

        // Early exit if nothing to draw
        if draw_commands.is_empty() {
            // Still need to build a valid command buffer
            let mut builder = AutoCommandBufferBuilder::primary(
                self.command_buffer_allocator.clone(),
                self.queue.queue_family_index(),
                CommandBufferUsage::OneTimeSubmit,
            )?;
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
            builder.end_render_pass(SubpassEndInfo::default())?;

            // Handle texture deletions
            for texture_id in &textures_delta.free {
                self.texture_cache.remove(texture_id);
                self.descriptor_set_cache.remove(texture_id);
            }
            return Ok(builder.build()?);
        }

        // Phase 2: Create single vertex and index buffer for entire frame (just 2 allocations!)
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
            self.batched_vertices.iter().copied(),
        )?;

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
            self.batched_indices.iter().copied(),
        )?;

        // Phase 3: Build command buffer with draw calls
        let mut builder = AutoCommandBufferBuilder::primary(
            self.command_buffer_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

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
        builder.set_viewport(0, smallvec![viewport])?;

        // Bind the batched buffers once
        builder.bind_vertex_buffers(0, vertex_buffer)?;
        builder.bind_index_buffer(index_buffer)?;

        // Push constants (same for all draws)
        let push_constants = PushConstants { screen_size };
        builder.push_constants(
            self.pipeline.layout().clone(),
            0,
            push_constants,
        )?;

        // Issue draw commands
        crate::profile_scope!("draw_commands");
        for cmd in &draw_commands {
            // Get or create cached descriptor set for this texture
            let descriptor_set = if let Some(cached) = self.descriptor_set_cache.get(&cmd.texture_id) {
                cached.clone()
            } else {
                let texture_view = self.texture_cache.get(&cmd.texture_id)
                    .ok_or("Texture not found")?;

                let layout = self.pipeline.layout().set_layouts().get(0)
                    .ok_or("No descriptor set layout")?;

                let new_set = DescriptorSet::new(
                    self.descriptor_set_allocator.clone(),
                    layout.clone(),
                    [
                        WriteDescriptorSet::image_view_sampler(0, texture_view.clone(), self.sampler.clone()),
                    ],
                    [],
                )?;

                self.descriptor_set_cache.insert(cmd.texture_id, new_set.clone());
                new_set
            };

            // Set scissor rectangle for clipping
            let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [
                    cmd.clip_rect.min.x.max(0.0) as u32,
                    cmd.clip_rect.min.y.max(0.0) as u32,
                ],
                extent: [
                    (cmd.clip_rect.max.x - cmd.clip_rect.min.x).max(0.0) as u32,
                    (cmd.clip_rect.max.y - cmd.clip_rect.min.y).max(0.0) as u32,
                ],
            };
            builder.set_scissor(0, smallvec![scissor])?;

            // Bind descriptor set
            builder.bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.pipeline.layout().clone(),
                0,
                descriptor_set,
            )?;

            // Draw with offset into batched buffers
            unsafe {
                builder.draw_indexed(
                    cmd.index_count,
                    1,
                    cmd.index_offset,
                    cmd.vertex_offset as i32,
                    0,
                )?;
            }
        }

        builder.end_render_pass(SubpassEndInfo::default())?;

        // Handle texture deletions (also invalidate cached descriptor sets)
        for texture_id in &textures_delta.free {
            self.texture_cache.remove(texture_id);
            self.descriptor_set_cache.remove(texture_id);
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

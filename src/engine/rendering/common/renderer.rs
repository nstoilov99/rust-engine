use crate::engine::assets::texture::load_texture;
use crate::engine::camera::{Camera2D, Camera3D};
use crate::engine::core::swapchain::{
    create_swapchain, create_swapchain_with_present_mode, recreate_swapchain,
    SwapchainPresentModePreference,
};
use crate::engine::core::{create_logical_device, select_physical_device, VulkanContext};
use crate::engine::ecs::components::{Camera, MeshRenderer, Transform};
use crate::engine::rendering::rendering_2d::SpriteBatch;
use crate::engine::rendering::rendering_3d::light::{AmbientLight, DirectionalLight, PointLight};
use crate::engine::rendering::rendering_3d::mesh_manager::GpuMesh;
use crate::engine::scene::Transform2D;
use crate::rendering::common::framebuffer::create_framebuffers;
use crate::rendering::common::render_pass::create_render_pass;
use crate::rendering::rendering_2d::pipeline_2d::{
    camera_vs, create_camera_pipeline, create_quad_indices, create_quad_vertices,
    create_texture_descriptor_set, transform_vs, TexturedVertex,
};
use crate::rendering::rendering_3d::pipeline_3d::{
    create_lit_mesh_pipeline, mesh_vs, LightingUniformData,
};
// use egui_winit_vulkano::Gui; // No compatible version for Vulkano 0.34
use glam::Mat4;
use hecs::World;
use smallvec::smallvec;
use std::sync::Arc;
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer};
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, CommandBufferUsage, RenderPassBeginInfo, SubpassBeginInfo,
    SubpassEndInfo,
};
use vulkano::descriptor_set::{
    allocator::StandardDescriptorSetAllocator, DescriptorSet, WriteDescriptorSet,
};
use vulkano::device::{Device, Queue};
use vulkano::image::view::ImageView;
use vulkano::image::Image;
use vulkano::instance::Instance;
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::pipeline::graphics::viewport::Viewport;
use vulkano::pipeline::{GraphicsPipeline, Pipeline, PipelineBindPoint};
use vulkano::render_pass::{Framebuffer, RenderPass};
use vulkano::swapchain::{self as vk_swapchain, Surface, Swapchain};
use vulkano::sync::GpuFuture;
use vulkano::{Validated, VulkanError};
use winit::window::Window;

pub struct Renderer {
    #[allow(dead_code)] // Kept alive for Vulkan instance lifetime
    instance: Arc<Instance>,
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub surface: Arc<Surface>,
    pub swapchain: Arc<Swapchain>,
    pub images: Vec<Arc<Image>>,
    pub command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    pub memory_allocator: Arc<StandardMemoryAllocator>,
    pub vertex_buffer: Subbuffer<[TexturedVertex]>,
    pub index_buffer: Subbuffer<[u32]>,
    pub render_pass: Arc<vulkano::render_pass::RenderPass>,
    pub framebuffers: Vec<Arc<Framebuffer>>,
    pub pipeline: Arc<GraphicsPipeline>,
    pub descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    pub descriptor_set: Arc<DescriptorSet>,
    _frames_in_flight: usize,
    _current_frame: usize,
    previous_frame_end: Option<Box<dyn GpuFuture>>,
    pub recreate_swapchain: bool,
    /// Tracks if we've already logged the minimized state (prevents log spam)
    was_minimized: bool,
    pub camera: Camera2D,
    pub camera_3d: Camera3D,
    pub render_pass_3d: Arc<RenderPass>,
    pub framebuffers_3d: Vec<Arc<Framebuffer>>,
    pub pipeline_3d: Arc<GraphicsPipeline>,
    pub depth_buffer: Arc<ImageView>,
    // Lit mesh pipeline (uses lighting uniform data)
    pub lit_mesh_pipeline: Arc<GraphicsPipeline>,
    pub ambient_light: AmbientLight,
    pub directional_light: Option<DirectionalLight>,
    pub point_lights: Vec<PointLight>,
    pub lighting_buffer: Subbuffer<LightingUniformData>,
    pub lighting_descriptor_set: Arc<DescriptorSet>,
    // pub gui: Gui, // No compatible version for Vulkano 0.34
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_with_present_mode(window, SwapchainPresentModePreference::Default)
    }

    pub fn new_with_present_mode(
        window: Arc<Window>,
        present_mode_preference: SwapchainPresentModePreference,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let vulkan_context = VulkanContext::new("Rust Engine")?;

        let surface = Surface::from_window(vulkan_context.instance.clone(), window.clone())?;

        let physical_device = select_physical_device(vulkan_context.instance.clone())?;
        let device_context = create_logical_device(physical_device, surface.clone())?;
        let (swapchain, images) = if matches!(
            present_mode_preference,
            SwapchainPresentModePreference::Default
        ) {
            create_swapchain(device_context.device.clone(), surface.clone())?
        } else {
            create_swapchain_with_present_mode(
                device_context.device.clone(),
                surface.clone(),
                present_mode_preference,
            )?
        };

        let command_buffer_allocator = Arc::new(StandardCommandBufferAllocator::new(
            device_context.device.clone(),
            Default::default(),
        ));

        // Create memory allocator (MOVED UP - needed by depth buffer)
        let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(
            device_context.device.clone(),
        ));

        let window_size = window.inner_size();

        // === 3D RENDERING SETUP ===

        // Create 3D camera (Z-up coordinates)
        let camera_3d = Camera3D::new(window_size.width as f32, window_size.height as f32);

        // Create depth buffer for 3D
        let depth_buffer = crate::engine::depth_buffer::create_depth_buffer(
            device_context.device.clone(),
            memory_allocator.clone(),
            images[0].extent()[0],
            images[0].extent()[1],
        )?;

        // Create 3D render pass
        let render_pass_3d = crate::engine::render_pass::create_render_pass_3d(
            device_context.device.clone(),
            swapchain.image_format(),
        )?;

        // Create 3D framebuffers
        let framebuffers_3d = crate::engine::framebuffer::create_framebuffers_3d(
            &images,
            render_pass_3d.clone(),
            depth_buffer.clone(),
        )?;

        // Create 3D pipeline
        let pipeline_3d =
            crate::engine::rendering::rendering_3d::pipeline_3d::create_mesh_pipeline(
                device_context.device.clone(),
                render_pass_3d.clone(),
            )?;


        // Create lit mesh pipeline
        let pipeline_lit =
            create_lit_mesh_pipeline(device_context.device.clone(), render_pass_3d.clone())?;

        // Create descriptor set allocator (needed for lighting descriptor set)
        let descriptor_set_allocator = Arc::new(StandardDescriptorSetAllocator::new(
            device_context.device.clone(),
            Default::default(),
        ));

        // Create default lighting
        let ambient_light = AmbientLight::default();
        let directional_light = Some(DirectionalLight::sun());
        let point_lights = Vec::new();

        // Create lighting uniform buffer
        let lighting_data = LightingUniformData::default();
        let lighting_buffer = Buffer::from_data(
            memory_allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::UNIFORM_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            lighting_data,
        )?;

        // Create descriptor set for lighting
        let lighting_descriptor_set = DescriptorSet::new(
            descriptor_set_allocator.clone(),
            pipeline_lit.layout().set_layouts()[1].clone(), // Set 1 = lighting
            [WriteDescriptorSet::buffer(0, lighting_buffer.clone())],
            [],
        )?;

        // === 2D RENDERING SETUP (for UI/sprites) ===

        // Create 2D camera
        let camera = Camera2D::new(window_size.width as f32, window_size.height as f32);

        // Create 2D quad vertices (for sprites)
        let vertices = create_quad_vertices();
        let vertex_buffer = Buffer::from_iter(
            memory_allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::VERTEX_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            vertices.iter().cloned(),
        )?;

        // Create index buffer
        let indices = create_quad_indices();
        let index_buffer = Buffer::from_iter(
            memory_allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::INDEX_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            indices.iter().cloned(),
        )?;

        // Create 2D render pass
        let render_pass = create_render_pass(device_context.device.clone(), swapchain.clone())?;

        // Create 2D framebuffers
        let framebuffers = create_framebuffers(&images, render_pass.clone())?;

        // Create 2D pipeline with camera support
        let viewport = Viewport {
            offset: [0.0, 0.0],
            extent: window.inner_size().into(),
            depth_range: 0.0..=1.0,
        };

        let pipeline =
            create_camera_pipeline(device_context.device.clone(), render_pass.clone(), viewport)?;


        // Load texture (for both 2D and 3D)
        let (texture_view, sampler) = load_texture(
            device_context.device.clone(),
            device_context.queue.clone(),
            command_buffer_allocator.clone(),
            memory_allocator.clone(),
            "textures/idle_animation.png",
        )?;

        // Create descriptor set for texture (descriptor_set_allocator already created earlier)
        let descriptor_set = create_texture_descriptor_set(
            descriptor_set_allocator.clone(),
            pipeline.clone(),
            texture_view,
            sampler,
        )?;

        Ok(Self {
            instance: vulkan_context.instance,
            device: device_context.device,
            queue: device_context.queue,
            surface,
            swapchain,
            images,
            command_buffer_allocator,
            memory_allocator,
            vertex_buffer,
            index_buffer,
            render_pass,
            framebuffers,
            pipeline,
            descriptor_set_allocator,
            descriptor_set,
            _frames_in_flight: 2,
            _current_frame: 0,
            previous_frame_end: None,
            recreate_swapchain: false,
            was_minimized: false,
            camera,
            camera_3d,
            render_pass_3d,
            framebuffers_3d,
            pipeline_3d,
            depth_buffer,
            lit_mesh_pipeline: pipeline_lit,
            ambient_light,
            directional_light,
            point_lights,
            lighting_buffer,
            lighting_descriptor_set,
            // gui, // No compatible version for Vulkano 0.34
        })
    }

    /// Begin a render frame: cleanup the previous frame future, handle
    /// minimized windows, recreate the swapchain when flagged, and acquire
    /// the next swapchain image.
    ///
    /// Returns `Ok(None)` when rendering should be skipped this frame
    /// (minimized window or out-of-date swapchain queued for recreation).
    fn begin_render_frame(
        &mut self,
    ) -> Result<Option<(u32, vk_swapchain::SwapchainAcquireFuture)>, Box<dyn std::error::Error>> {
        if let Some(mut previous) = self.previous_frame_end.take() {
            previous.cleanup_finished();
        }

        let extent = self.swapchain.image_extent();
        if extent[0] == 0 || extent[1] == 0 {
            if !self.was_minimized {
                println!("Window minimized, pausing render");
                self.was_minimized = true;
            }
            self.previous_frame_end = Some(vulkano::sync::now(self.device.clone()).boxed());
            return Ok(None);
        } else if self.was_minimized {
            println!("Window restored, resuming render");
            self.was_minimized = false;
        }

        if self.recreate_swapchain {
            if !self.recreate_swapchain_resources()? {
                return Ok(None);
            }
        }

        let (image_index, suboptimal, acquire_future) =
            match vk_swapchain::acquire_next_image(self.swapchain.clone(), None) {
                Ok(r) => r,
                Err(Validated::Error(VulkanError::OutOfDate)) => {
                    self.recreate_swapchain = true;
                    return Ok(None);
                }
                Err(e) => return Err(e.into()),
            };

        if suboptimal {
            self.recreate_swapchain = true;
        }

        Ok(Some((image_index, acquire_future)))
    }

    /// Recreate all swapchain-dependent resources (framebuffers, depth buffer,
    /// camera viewports). Returns `false` when the window is minimized
    /// (zero-sized images) and rendering should be skipped.
    fn recreate_swapchain_resources(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        let (new_swapchain, new_images) = recreate_swapchain(
            self.device.clone(),
            self.surface.clone(),
            self.swapchain.clone(),
        )?;

        if new_images.is_empty() {
            self.recreate_swapchain = false;
            self.previous_frame_end = Some(vulkano::sync::now(self.device.clone()).boxed());
            return Ok(false);
        }

        self.swapchain = new_swapchain;

        self.depth_buffer = crate::engine::depth_buffer::create_depth_buffer(
            self.device.clone(),
            self.memory_allocator.clone(),
            new_images[0].extent()[0],
            new_images[0].extent()[1],
        )?;

        self.images = new_images;

        self.framebuffers = create_framebuffers(&self.images, self.render_pass.clone())?;
        self.framebuffers_3d = crate::engine::framebuffer::create_framebuffers_3d(
            &self.images,
            self.render_pass_3d.clone(),
            self.depth_buffer.clone(),
        )?;

        let extent = self.images[0].extent();
        self.camera
            .set_viewport_size(extent[0] as f32, extent[1] as f32);
        self.camera_3d
            .set_viewport_size(extent[0] as f32, extent[1] as f32);

        self.recreate_swapchain = false;
        Ok(true)
    }

    /// Submit a command buffer, present to the swapchain, and store the
    /// resulting fence future for the next frame's cleanup.
    fn submit_and_present(
        &mut self,
        acquire_future: impl GpuFuture + 'static,
        command_buffer: Arc<impl vulkano::command_buffer::PrimaryCommandBufferAbstract + 'static>,
        image_index: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let future = acquire_future
            .then_execute(self.queue.clone(), command_buffer)?
            .then_swapchain_present(
                self.queue.clone(),
                vk_swapchain::SwapchainPresentInfo::swapchain_image_index(
                    self.swapchain.clone(),
                    image_index,
                ),
            )
            .then_signal_fence_and_flush();

        match future {
            Ok(future) => {
                self.previous_frame_end = Some(future.boxed());
            }
            Err(Validated::Error(VulkanError::DeviceLost)) => {
                return Err("GPU device lost".into());
            }
            Err(Validated::Error(VulkanError::OutOfDate)) => {
                self.recreate_swapchain = true;
                self.previous_frame_end = Some(vulkano::sync::now(self.device.clone()).boxed());
            }
            Err(e) => {
                eprintln!("Failed to flush future: {:?}", e);
                self.previous_frame_end = Some(vulkano::sync::now(self.device.clone()).boxed());
            }
        }

        Ok(())
    }

    pub fn render(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (image_index, acquire_future) = match self.begin_render_frame()? {
            Some(frame) => frame,
            None => return Ok(()),
        };

        let mut builder = AutoCommandBufferBuilder::primary(
            self.command_buffer_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        // Begin render pass
        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![Some([0.0, 0.0, 0.0, 1.0].into())], // Black background
                    ..RenderPassBeginInfo::framebuffer(
                        self.framebuffers[image_index as usize].clone(),
                    )
                },
                SubpassBeginInfo::default(),
            )?
            // Bind pipeline
            .bind_pipeline_graphics(self.pipeline.clone())?
            .bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.pipeline.layout().clone(),
                0,
                self.descriptor_set.clone(),
            )?
            // Bind vertex buffer
            .bind_vertex_buffers(0, self.vertex_buffer.clone())?
            .bind_index_buffer(self.index_buffer.clone())?;
        unsafe { builder.draw_indexed(6, 1, 0, 0, 0)?; }
        // End render pass
        builder.end_render_pass(SubpassEndInfo::default())?;

        let command_buffer = builder.build()?;
        self.submit_and_present(acquire_future, command_buffer, image_index)
    }

    /// Renders a sprite with 2D transform
    pub fn render_sprite(
        &mut self,
        transform: Transform2D,
        descriptor_set: Arc<DescriptorSet>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // 1. Acquire image
        let (image_index, _, acquire_future) =
            vulkano::swapchain::acquire_next_image(self.swapchain.clone(), None)?;

        // 2. Build command buffer
        let mut builder = AutoCommandBufferBuilder::primary(
            self.command_buffer_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        // 3. Create push constants data
        let push_constants = transform_vs::PushConstants {
            pos: transform.position,
            rotation: transform.rotation.into(),
            scale: transform.scale,
        };

        // 4. Begin render pass and draw
        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![Some([0.0, 0.0, 0.0, 1.0].into())],
                    ..RenderPassBeginInfo::framebuffer(
                        self.framebuffers[image_index as usize].clone(),
                    )
                },
                SubpassBeginInfo::default(),
            )?
            .bind_pipeline_graphics(self.pipeline.clone())?
            .bind_descriptor_sets(
                vulkano::pipeline::PipelineBindPoint::Graphics,
                self.pipeline.layout().clone(),
                0,
                descriptor_set.clone(),
            )?
            .push_constants(self.pipeline.layout().clone(), 0, push_constants)?
            .bind_vertex_buffers(0, self.vertex_buffer.clone())?
            .bind_index_buffer(self.index_buffer.clone())?;
        unsafe { builder.draw_indexed(6, 1, 0, 0, 0)?; }
        builder.end_render_pass(SubpassEndInfo::default())?;

        // 5. Build and submit
        let command_buffer = builder.build()?;

        let future = acquire_future
            .then_execute(self.queue.clone(), command_buffer)?
            .then_swapchain_present(
                self.queue.clone(),
                vulkano::swapchain::SwapchainPresentInfo::swapchain_image_index(
                    self.swapchain.clone(),
                    image_index,
                ),
            )
            .then_signal_fence_and_flush()?;

        future.wait(None)?;

        Ok(())
    }

    /// Renders multiple sprites in one frame
    pub fn render_sprites(
        &mut self,
        sprites: &[(Transform2D, Arc<DescriptorSet>)],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (image_index, acquire_future) = match self.begin_render_frame()? {
            Some(frame) => frame,
            None => return Ok(()),
        };

        let mut builder = AutoCommandBufferBuilder::primary(
            self.command_buffer_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        // Begin render pass
        builder.begin_render_pass(
            RenderPassBeginInfo {
                clear_values: vec![Some([0.1, 0.1, 0.1, 1.0].into())],
                ..RenderPassBeginInfo::framebuffer(self.framebuffers[image_index as usize].clone())
            },
            SubpassBeginInfo::default(),
        )?;

        builder.bind_pipeline_graphics(self.pipeline.clone())?;

        // Set dynamic viewport (updates with window resize)
        let extent = self.swapchain.image_extent();
        builder.set_viewport(
            0,
            smallvec![Viewport {
                offset: [0.0, 0.0],
                extent: [extent[0] as f32, extent[1] as f32],
                depth_range: 0.0..=1.0,
            }],
        )?;

        builder.bind_vertex_buffers(0, self.vertex_buffer.clone())?;
        builder.bind_index_buffer(self.index_buffer.clone())?;

        // Get camera view-projection matrix
        let camera_vp = self.camera.view_projection_matrix();

        // Draw each sprite
        for (transform, texture_descriptor) in sprites {
            let push_constants = camera_vs::PushConstants {
                view_projection: camera_vp.to_cols_array_2d(),
                pos: transform.position,
                rotation: transform.rotation.into(),
                scale: transform.scale.into(),
                uv_rect: [0.0, 0.0, 0.0, 0.0], // Use full texture
            };

            builder
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.pipeline.layout().clone(),
                    0,
                    texture_descriptor.clone(),
                )?
                .push_constants(self.pipeline.layout().clone(), 0, push_constants)?;
            unsafe { builder.draw_indexed(6, 1, 0, 0, 0)?; }
        }

        // End render pass
        builder.end_render_pass(SubpassEndInfo::default())?;

        let command_buffer = builder.build()?;
        self.submit_and_present(acquire_future, command_buffer, image_index)
    }

    /// Renders sprites using batching (more efficient)
    pub fn render_sprite_batch(
        &mut self,
        batch: &SpriteBatch,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (image_index, acquire_future) = match self.begin_render_frame()? {
            Some(frame) => frame,
            None => return Ok(()),
        };

        let mut builder = AutoCommandBufferBuilder::primary(
            self.command_buffer_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        // Begin render pass
        builder.begin_render_pass(
            RenderPassBeginInfo {
                clear_values: vec![Some([0.1, 0.1, 0.1, 1.0].into())],
                ..RenderPassBeginInfo::framebuffer(self.framebuffers[image_index as usize].clone())
            },
            SubpassBeginInfo::default(),
        )?;

        // Bind pipeline and buffers (once for all sprites)
        builder.bind_pipeline_graphics(self.pipeline.clone())?;

        // Set dynamic viewport (updates with window resize)
        let extent = self.swapchain.image_extent();
        builder.set_viewport(
            0,
            smallvec![Viewport {
                offset: [0.0, 0.0],
                extent: [extent[0] as f32, extent[1] as f32],
                depth_range: 0.0..=1.0,
            }],
        )?;

        builder.bind_vertex_buffers(0, self.vertex_buffer.clone())?;
        builder.bind_index_buffer(self.index_buffer.clone())?;

        // Get camera view-projection matrix
        let camera_vp = self.camera.view_projection_matrix();

        // Draw batched sprites
        for (descriptor_set, transforms) in batch.iter_batches() {
            // Bind texture once per batch
            builder.bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.pipeline.layout().clone(),
                0,
                descriptor_set,
            )?;

            // Draw all sprites with this texture
            for transform in transforms {
                let push_constants = camera_vs::PushConstants {
                    view_projection: camera_vp.to_cols_array_2d(),
                    pos: transform.position.into(),
                    rotation: transform.rotation.into(),
                    scale: transform.scale.into(),
                    uv_rect: [0.0, 0.0, 0.0, 0.0].into(),
                };

                builder.push_constants(self.pipeline.layout().clone(), 0, push_constants)?;
                unsafe { builder.draw_indexed(6, 1, 0, 0, 0)?; }
            }
        }

        // Draw animated sprites
        for (descriptor_set, animated_sprites) in batch.iter_animated_batches() {
            builder.bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.pipeline.layout().clone(),
                0,
                descriptor_set,
            )?;

            for sprite in animated_sprites {
                let push_constants = camera_vs::PushConstants {
                    view_projection: camera_vp.to_cols_array_2d(),
                    pos: sprite.transform.position,
                    rotation: sprite.transform.rotation.into(),
                    scale: sprite.transform.scale.into(),
                    uv_rect: sprite.uv_rect, // Use sprite sheet frame UVs
                };

                builder.push_constants(self.pipeline.layout().clone(), 0, push_constants)?;
                unsafe { builder.draw_indexed(6, 1, 0, 0, 0)?; }
            }
        }

        // End render pass
        builder.end_render_pass(SubpassEndInfo::default())?;

        let command_buffer = builder.build()?;
        self.submit_and_present(acquire_future, command_buffer, image_index)
    }

    /// Renders a GPU mesh (from mesh manager)
    pub fn render_gpu_mesh(
        &mut self,
        mesh: &GpuMesh,
        model_matrix: Mat4,
        texture_descriptor: Arc<DescriptorSet>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (image_index, acquire_future) = match self.begin_render_frame()? {
            Some(frame) => frame,
            None => return Ok(()),
        };

        let mut builder = AutoCommandBufferBuilder::primary(
            self.command_buffer_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;

        // Begin render pass
        builder.begin_render_pass(
            RenderPassBeginInfo {
                clear_values: vec![Some([0.1, 0.1, 0.15, 1.0].into()), Some(1.0.into())],
                ..RenderPassBeginInfo::framebuffer(
                    self.framebuffers_3d[image_index as usize].clone(),
                )
            },
            SubpassBeginInfo::default(),
        )?;

        builder.bind_pipeline_graphics(self.pipeline_3d.clone())?;

        // Set viewport
        let extent = self.swapchain.image_extent();
        builder.set_viewport(
            0,
            smallvec![Viewport {
                offset: [0.0, 0.0],
                extent: [extent[0] as f32, extent[1] as f32],
                depth_range: 0.0..=1.0,
            }],
        )?;

        // Bind texture
        builder.bind_descriptor_sets(
            PipelineBindPoint::Graphics,
            self.pipeline_3d.layout().clone(),
            0,
            texture_descriptor,
        )?;

        // Bind mesh buffers (no need to create them - already on GPU!)
        builder
            .bind_vertex_buffers(0, mesh.vertex_buffer.clone())?
            .bind_index_buffer(mesh.index_buffer.clone())?;

        // Push constants
        let view_projection = self.camera_3d.view_projection_matrix();
        let push_constants = mesh_vs::PushConstants {
            model: model_matrix.to_cols_array_2d(),
            view_projection: view_projection.to_cols_array_2d(),
        };

        // Draw
        builder.push_constants(self.pipeline_3d.layout().clone(), 0, push_constants)?;
        unsafe { builder.draw_indexed(mesh.index_count, 1, 0, 0, 0)?; }

        // End render pass
        builder.end_render_pass(SubpassEndInfo::default())?;

        let command_buffer = builder.build()?;
        self.submit_and_present(acquire_future, command_buffer, image_index)
    }

    /// Render all mesh entities from ECS world
    pub fn render_ecs_meshes(
        &mut self,
        world: &World,
        mesh_manager: &crate::MeshManager,
        texture_descriptor: Arc<DescriptorSet>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Query all entities with Transform + MeshRenderer
        for (_entity, (transform, mesh_renderer)) in
            world.query::<(&Transform, &MeshRenderer)>().iter()
        {
            // Get mesh from manager
            if let Some(gpu_mesh) = mesh_manager.get(mesh_renderer.mesh_index) {
                // Get model matrix from ECS transform (nalgebra-glm)
                let model_glm = transform.model_matrix();

                // Convert nalgebra-glm Mat4 to glam Mat4
                use crate::engine::utils::math_convert::mat4_from_glm;
                let model = mat4_from_glm(&model_glm);

                // Render mesh using existing method
                self.render_gpu_mesh(gpu_mesh, model, texture_descriptor.clone())?;
            }
        }

        Ok(())
    }

    /// Get active camera from ECS world
    pub fn get_active_camera(
        world: &World,
    ) -> Option<(nalgebra_glm::Vec3, nalgebra_glm::Quat, Camera)> {
        for (_entity, (transform, camera)) in world.query::<(&Transform, &Camera)>().iter() {
            if camera.active {
                return Some((transform.position, transform.rotation, camera.clone()));
            }
        }
        None
    }
}

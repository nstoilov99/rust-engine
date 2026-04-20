use super::bloom_pass::{BloomPass, BloomSamplePush, BloomThresholdPush};
use super::composite_pass::{CompositePass, CompositePushConstants};
use super::gbuffer::GBuffer;
use super::geometry_pass::GeometryPass;
use super::grid_pass::{GridPass, GridPushConstants};
use super::lighting_pass::LightingPass;
use super::luminance_pass::{LuminancePass, LuminancePush};
use super::shadow_pass::ShadowPass;
use super::plankton::PlanktonSystem;
use super::ssao_pass::{SsaoPass, SsaoPushConstants};
use crate::engine::debug_draw::{DebugDrawData, DebugDrawPass, DebugLinePushConstants};
use crate::engine::rendering::counters::RenderCounters;
use crate::engine::rendering::graph::RenderGraph;
use crate::engine::rendering::pipeline_registry::PipelineRegistry;
use crate::engine::rendering::render_target::RenderTarget;
use crate::engine::rendering::rendering_3d::material::{
    create_default_texture, create_default_texture_with_format, PbrMaterial,
    DEFAULT_ALBEDO_RGBA, DEFAULT_AO_RGBA, DEFAULT_METALLIC_ROUGHNESS_RGBA, DEFAULT_NORMAL_RGBA,
};
use glam::{Mat4, Vec3, Vec4};
use smallvec::smallvec;
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::buffer::Subbuffer;
use vulkano::command_buffer::{
    allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, CommandBufferUsage,
    PrimaryAutoCommandBuffer, RenderPassBeginInfo, SubpassBeginInfo, SubpassContents,
    SubpassEndInfo,
};
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::DescriptorSet;
use vulkano::device::{Device, Queue};
use vulkano::format::Format;
use vulkano::image::sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo};
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::pipeline::{Pipeline, PipelineBindPoint};
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};

pub struct DeferredRenderer {
    gbuffer: GBuffer,
    geometry_pass: GeometryPass,
    lighting_pass: LightingPass,
    pipeline_registry: PipelineRegistry,
    shadow_pass: ShadowPass,
    ssao_pass: SsaoPass,
    bloom_pass: BloomPass,
    luminance_pass: LuminancePass,
    composite_pass: CompositePass,
    grid_pass: GridPass,
    debug_draw_pass: DebugDrawPass,
    device: Arc<Device>,
    queue: Arc<Queue>,
    allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    debug_view: DebugView,
    render_counters: RenderCounters,
    gbuffer_descriptor_set: Arc<DescriptorSet>,
    shadow_descriptor_set: Arc<DescriptorSet>,
    ssao_descriptor_set: Arc<DescriptorSet>,
    ssao_fallback_descriptor_set: Arc<DescriptorSet>,
    ssao_sampler: Arc<Sampler>,
    #[allow(dead_code)]
    ssao_fallback: Arc<ImageView>,
    default_material_set: Arc<DescriptorSet>,
    hdr_target: Arc<ImageView>,
    hdr_framebuffer: Arc<Framebuffer>,
    composite_descriptor_set: Arc<DescriptorSet>,
    composite_render_pass: Arc<RenderPass>,
    framebuffer_cache: HashMap<usize, Arc<Framebuffer>>,
    grid_framebuffer_cache: HashMap<usize, Arc<Framebuffer>>,
    grid_render_pass: Arc<RenderPass>,
    plankton_system: PlanktonSystem,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DebugView {
    None,
    Position,
    Normal,
    Albedo,
    Material,
    Depth,
}

fn create_hdr_target(
    allocator: Arc<StandardMemoryAllocator>,
    width: u32,
    height: u32,
) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
    let image = Image::new(
        allocator,
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::R16G16B16A16_SFLOAT,
            extent: [width, height, 1],
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
            ..Default::default()
        },
        AllocationCreateInfo::default(),
    )?;
    ImageView::new_default(image).map_err(|e| e.into())
}

fn create_ssao_fallback(
    allocator: Arc<StandardMemoryAllocator>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    queue: Arc<Queue>,
) -> Result<Arc<ImageView>, Box<dyn std::error::Error>> {
    use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
    use vulkano::command_buffer::CopyBufferToImageInfo;

    let image = Image::new(
        allocator.clone(),
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::R8_UNORM,
            extent: [1, 1, 1],
            usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
            ..Default::default()
        },
    )?;

    // Upload a single white pixel (0xFF) so ambient multiplication passes through
    // unchanged when SSAO is disabled and this fallback is bound.
    let staging = Buffer::from_iter(
        allocator,
        BufferCreateInfo {
            usage: BufferUsage::TRANSFER_SRC,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
            ..Default::default()
        },
        [255u8],
    )?;

    let mut builder = AutoCommandBufferBuilder::primary(
        command_buffer_allocator,
        queue.queue_family_index(),
        CommandBufferUsage::OneTimeSubmit,
    )?;
    builder.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(staging, image.clone()))?;
    use vulkano::command_buffer::PrimaryCommandBufferAbstract;
    use vulkano::sync::GpuFuture;
    builder
        .build()?
        .execute(queue)?
        .then_signal_fence_and_flush()?
        .wait(None)?;

    ImageView::new_default(image).map_err(|e| e.into())
}

impl DeferredRenderer {
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        allocator: Arc<StandardMemoryAllocator>,
        command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let gbuffer = GBuffer::new(device.clone(), allocator.clone(), width, height)?;
        let (geometry_pass, geometry_pipeline) =
            GeometryPass::new(device.clone(), gbuffer.render_pass.clone())?;

        let hdr_render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::R16G16B16A16_SFLOAT,
                    samples: 1,
                    load_op: Clear,
                    store_op: Store,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {}
            }
        )?;

        let (lighting_pass, lighting_pipeline) =
            LightingPass::new(device.clone(), hdr_render_pass)?;

        let shadow_pass = ShadowPass::new(
            device.clone(),
            allocator.clone(),
            descriptor_set_allocator.clone(),
        )?;

        let composite_render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::B8G8R8A8_SRGB,
                    samples: 1,
                    load_op: Clear,
                    store_op: Store,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {}
            }
        )?;

        let composite_pass = CompositePass::new(device.clone(), composite_render_pass.clone())?;

        let mut bloom_pass = BloomPass::new(device.clone(), allocator.clone(), width, height)?;

        let mut luminance_pass = LuminancePass::new(device.clone(), allocator.clone())?;

        let hdr_target = create_hdr_target(allocator.clone(), width, height)?;

        bloom_pass.prepare_sets(descriptor_set_allocator.clone(), hdr_target.clone())?;
        luminance_pass.prepare_sets(descriptor_set_allocator.clone(), hdr_target.clone())?;

        let hdr_framebuffer = Framebuffer::new(
            lighting_pass.render_pass(),
            FramebufferCreateInfo {
                attachments: vec![hdr_target.clone()],
                ..Default::default()
            },
        )?;

        let grid_render_pass = vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    format: Format::B8G8R8A8_SRGB,
                    samples: 1,
                    load_op: Load,
                    store_op: Store,
                },
                depth: {
                    format: Format::D32_SFLOAT,
                    samples: 1,
                    load_op: Load,
                    store_op: DontCare,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {depth}
            }
        )?;

        let grid_pass = GridPass::new(device.clone(), grid_render_pass.clone())?;
        let debug_draw_pass = DebugDrawPass::new(device.clone(), grid_render_pass.clone())?;

        let gbuffer_descriptor_set = lighting_pass.create_descriptor_set(
            descriptor_set_allocator.clone(),
            gbuffer.position.clone(),
            gbuffer.normal.clone(),
            gbuffer.albedo.clone(),
            gbuffer.material.clone(),
            gbuffer.emissive.clone(),
        )?;

        let shadow_descriptor_set = lighting_pass.create_shadow_descriptor_set(
            descriptor_set_allocator.clone(),
            shadow_pass.shadow_map(),
            shadow_pass.shadow_sampler(),
        )?;

        let mut ssao_pass = SsaoPass::new(
            device.clone(),
            allocator.clone(),
            descriptor_set_allocator.clone(),
            width,
            height,
        )?;
        ssao_pass.prepare_sets(
            descriptor_set_allocator.clone(),
            gbuffer.position.clone(),
            gbuffer.normal.clone(),
        )?;

        let ssao_sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Nearest,
                min_filter: Filter::Nearest,
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                ..Default::default()
            },
        )?;

        let ssao_fallback = create_ssao_fallback(
            allocator.clone(),
            command_buffer_allocator.clone(),
            queue.clone(),
        )?;

        let ssao_descriptor_set = lighting_pass.create_ssao_descriptor_set(
            descriptor_set_allocator.clone(),
            ssao_pass.ssao_blurred(),
            ssao_sampler.clone(),
        )?;

        let ssao_fallback_descriptor_set = lighting_pass.create_ssao_descriptor_set(
            descriptor_set_allocator.clone(),
            ssao_fallback.clone(),
            ssao_sampler.clone(),
        )?;

        let composite_descriptor_set = composite_pass.create_descriptor_set(
            descriptor_set_allocator.clone(),
            hdr_target.clone(),
            bloom_pass.bloom_result(),
            luminance_pass.persistent_1x1(),
        )?;

        let mut plankton_system = PlanktonSystem::new(
            device.clone(),
            queue.clone(),
            allocator.clone(),
            command_buffer_allocator.clone(),
            descriptor_set_allocator.clone(),
        )?;
        plankton_system.set_gbuffer_depth(gbuffer.depth.clone())?;
        plankton_system.set_hdr_target(hdr_target.clone())?;

        // --- Pipeline Registry ---
        let mut pipeline_registry = PipelineRegistry::new();

        {
            #[cfg(feature = "editor")]
            let gbuffer_rp = gbuffer.render_pass.clone();

            pipeline_registry.register(
                crate::engine::rendering::pipeline_registry::PipelineId::Geometry,
                geometry_pipeline,
                #[cfg(feature = "editor")]
                vec![
                    std::path::PathBuf::from("src/engine/rendering/shaders/deferred/gbuffer.vert"),
                    std::path::PathBuf::from("src/engine/rendering/shaders/deferred/gbuffer.frag"),
                ],
                #[cfg(feature = "editor")]
                Box::new(move |compiler, device| {
                    use crate::engine::rendering::pipeline_registry::PipelineError;
                    use crate::engine::rendering::shader_compiler::ShaderKind;
                    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
                    let vs_spirv = compiler
                        .compile(
                            &base.join("src/engine/rendering/shaders/deferred/gbuffer.vert"),
                            ShaderKind::Vertex,
                        )
                        .map_err(PipelineError::Shader)?;
                    let fs_spirv = compiler
                        .compile(
                            &base.join("src/engine/rendering/shaders/deferred/gbuffer.frag"),
                            ShaderKind::Fragment,
                        )
                        .map_err(PipelineError::Shader)?;
                    GeometryPass::create_pipeline_from_spirv(
                        device.clone(),
                        gbuffer_rp.clone(),
                        &vs_spirv,
                        &fs_spirv,
                    )
                    .map_err(|e| PipelineError::Vulkan(e.to_string()))
                }),
            );
        }

        {
            #[cfg(feature = "editor")]
            let lighting_rp = lighting_pass.render_pass();

            pipeline_registry.register(
                crate::engine::rendering::pipeline_registry::PipelineId::Lighting,
                lighting_pipeline,
                #[cfg(feature = "editor")]
                vec![
                    std::path::PathBuf::from("src/engine/rendering/shaders/deferred/lighting.vert"),
                    std::path::PathBuf::from("src/engine/rendering/shaders/deferred/lighting.frag"),
                ],
                #[cfg(feature = "editor")]
                Box::new(move |compiler, device| {
                    use crate::engine::rendering::pipeline_registry::PipelineError;
                    use crate::engine::rendering::shader_compiler::ShaderKind;
                    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
                    let vs_spirv = compiler
                        .compile(
                            &base.join("src/engine/rendering/shaders/deferred/lighting.vert"),
                            ShaderKind::Vertex,
                        )
                        .map_err(PipelineError::Shader)?;
                    let fs_spirv = compiler
                        .compile(
                            &base.join("src/engine/rendering/shaders/deferred/lighting.frag"),
                            ShaderKind::Fragment,
                        )
                        .map_err(PipelineError::Shader)?;
                    LightingPass::create_pipeline_from_spirv(
                        device.clone(),
                        lighting_rp.clone(),
                        &vs_spirv,
                        &fs_spirv,
                    )
                    .map_err(|e| PipelineError::Vulkan(e.to_string()))
                }),
            );
        }

        // --- Default material descriptor set (fallback for meshes without a material) ---
        let mat_sampler = Sampler::new(
            device.clone(),
            SamplerCreateInfo {
                mag_filter: Filter::Linear,
                min_filter: Filter::Linear,
                address_mode: [SamplerAddressMode::Repeat; 3],
                ..Default::default()
            },
        )?;
        // Albedo is a color texture → SRGB for automatic gamma decoding.
        let default_albedo = create_default_texture(
            device.clone(), allocator.clone(), command_buffer_allocator.clone(), queue.clone(), DEFAULT_ALBEDO_RGBA,
        )?;
        // Normal, metallic-roughness, and AO are data textures → UNORM (no gamma).
        let default_normal = create_default_texture_with_format(
            allocator.clone(), command_buffer_allocator.clone(), queue.clone(), DEFAULT_NORMAL_RGBA,
            Format::R8G8B8A8_UNORM,
        )?;
        let default_mr = create_default_texture_with_format(
            allocator.clone(), command_buffer_allocator.clone(), queue.clone(), DEFAULT_METALLIC_ROUGHNESS_RGBA,
            Format::R8G8B8A8_UNORM,
        )?;
        let default_ao = create_default_texture_with_format(
            allocator.clone(), command_buffer_allocator.clone(), queue.clone(), DEFAULT_AO_RGBA,
            Format::R8G8B8A8_UNORM,
        )?;
        let geom_pipeline_layout = pipeline_registry
            .get(crate::engine::rendering::pipeline_registry::PipelineId::Geometry)
            .layout()
            .clone();
        let default_material = PbrMaterial::new(
            default_albedo,
            default_normal,
            default_mr,
            default_ao,
            mat_sampler,
            [1.0, 1.0, 1.0, 1.0],
            1.0,
            0.5,
            [0.0, 0.0, 0.0],
            allocator.clone(),
            descriptor_set_allocator.clone(),
            geom_pipeline_layout,
        )?;
        let default_material_set = default_material.descriptor_set.clone();

        Ok(Self {
            gbuffer,
            geometry_pass,
            lighting_pass,
            pipeline_registry,
            shadow_pass,
            ssao_pass,
            bloom_pass,
            luminance_pass,
            composite_pass,
            grid_pass,
            debug_draw_pass,
            device,
            queue,
            allocator,
            command_buffer_allocator,
            descriptor_set_allocator,
            debug_view: DebugView::None,
            render_counters: RenderCounters::default(),
            gbuffer_descriptor_set,
            shadow_descriptor_set,
            ssao_descriptor_set,
            ssao_fallback_descriptor_set,
            ssao_sampler,
            ssao_fallback,
            default_material_set,
            hdr_target,
            hdr_framebuffer,
            composite_descriptor_set,
            composite_render_pass,
            framebuffer_cache: HashMap::new(),
            grid_framebuffer_cache: HashMap::new(),
            grid_render_pass,
            plankton_system,
        })
    }

    fn get_or_create_framebuffer(
        &mut self,
        target_image: Arc<Image>,
    ) -> Result<Arc<Framebuffer>, Box<dyn std::error::Error>> {
        let cache_key = Arc::as_ptr(&target_image) as usize;

        if let Some(fb) = self.framebuffer_cache.get(&cache_key) {
            return Ok(fb.clone());
        }

        let target_view = ImageView::new_default(target_image)?;
        let framebuffer = Framebuffer::new(
            self.composite_render_pass.clone(),
            FramebufferCreateInfo {
                attachments: vec![target_view],
                ..Default::default()
            },
        )?;

        self.framebuffer_cache
            .insert(cache_key, framebuffer.clone());
        Ok(framebuffer)
    }

    fn get_or_create_grid_framebuffer(
        &mut self,
        target_image: Arc<Image>,
    ) -> Result<Arc<Framebuffer>, Box<dyn std::error::Error>> {
        let cache_key = Arc::as_ptr(&target_image) as usize;

        if let Some(fb) = self.grid_framebuffer_cache.get(&cache_key) {
            return Ok(fb.clone());
        }

        let target_view = ImageView::new_default(target_image)?;
        let framebuffer = Framebuffer::new(
            self.grid_render_pass.clone(),
            FramebufferCreateInfo {
                attachments: vec![target_view, self.gbuffer.depth.clone()],
                ..Default::default()
            },
        )?;

        self.grid_framebuffer_cache
            .insert(cache_key, framebuffer.clone());
        Ok(framebuffer)
    }

    pub fn clear_framebuffer_cache(&mut self) {
        self.framebuffer_cache.clear();
        self.grid_framebuffer_cache.clear();
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
        if width == 0 || height == 0 {
            return Ok(());
        }

        let current_extent = self.gbuffer.position.image().extent();
        if current_extent[0] == width && current_extent[1] == height {
            return Ok(());
        }

        self.gbuffer = GBuffer::new(self.device.clone(), self.allocator.clone(), width, height)?;

        self.gbuffer_descriptor_set = self.lighting_pass.create_descriptor_set(
            self.descriptor_set_allocator.clone(),
            self.gbuffer.position.clone(),
            self.gbuffer.normal.clone(),
            self.gbuffer.albedo.clone(),
            self.gbuffer.material.clone(),
            self.gbuffer.emissive.clone(),
        )?;

        self.hdr_target = create_hdr_target(self.allocator.clone(), width, height)?;

        self.hdr_framebuffer = Framebuffer::new(
            self.lighting_pass.render_pass(),
            FramebufferCreateInfo {
                attachments: vec![self.hdr_target.clone()],
                ..Default::default()
            },
        )?;

        self.ssao_pass
            .resize(self.allocator.clone(), width, height)?;
        self.ssao_pass.prepare_sets(
            self.descriptor_set_allocator.clone(),
            self.gbuffer.position.clone(),
            self.gbuffer.normal.clone(),
        )?;

        self.ssao_descriptor_set = self.lighting_pass.create_ssao_descriptor_set(
            self.descriptor_set_allocator.clone(),
            self.ssao_pass.ssao_blurred(),
            self.ssao_sampler.clone(),
        )?;

        self.bloom_pass
            .resize(self.allocator.clone(), width, height)?;
        self.bloom_pass.prepare_sets(
            self.descriptor_set_allocator.clone(),
            self.hdr_target.clone(),
        )?;

        self.luminance_pass
            .resize(self.allocator.clone())?;
        self.luminance_pass.prepare_sets(
            self.descriptor_set_allocator.clone(),
            self.hdr_target.clone(),
        )?;

        self.composite_descriptor_set = self.composite_pass.create_descriptor_set(
            self.descriptor_set_allocator.clone(),
            self.hdr_target.clone(),
            self.bloom_pass.bloom_result(),
            self.luminance_pass.persistent_1x1(),
        )?;

        self.plankton_system
            .set_gbuffer_depth(self.gbuffer.depth.clone())?;
        self.plankton_system
            .set_hdr_target(self.hdr_target.clone())?;

        self.framebuffer_cache.clear();
        self.grid_framebuffer_cache.clear();

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        mesh_data: &[MeshRenderData],
        shadow_caster_data: &[MeshRenderData],
        light_data: &LightUniformData,
        target: RenderTarget,
        grid_visible: bool,
        view_proj: Mat4,
        camera_pos: Vec3,
        debug_draw: &DebugDrawData,
        settings: &PostProcessingSettings,
        plankton_emitters: &[crate::engine::rendering::frame_packet::PlanktonEmitterFrameData],
    ) -> Result<Arc<PrimaryAutoCommandBuffer>, Box<dyn std::error::Error>> {
        crate::profile_function!();

        self.render_counters.reset();

        let needs_depth_framebuffer = grid_visible || !debug_draw.is_empty();

        let (target_framebuffer, depth_framebuffer, mut builder) = {
            crate::profile_scope!("command_buffer_setup");
            let target_image = target.image().clone();
            let target_framebuffer = self.get_or_create_framebuffer(target_image.clone())?;
            let depth_framebuffer = if needs_depth_framebuffer {
                Some(self.get_or_create_grid_framebuffer(target_image)?)
            } else {
                None
            };
            let builder = AutoCommandBufferBuilder::primary(
                self.command_buffer_allocator.clone(),
                self.queue.queue_family_index(),
                CommandBufferUsage::OneTimeSubmit,
            )?;
            (target_framebuffer, depth_framebuffer, builder)
        };

        // Run plankton compute passes (init/emit/simulate) before graph execution.
        // Skip expensive camera-vector math when no emitters are active this frame —
        // update_frame still runs so pool eviction/absence counters keep progressing.
        if !plankton_emitters.is_empty() {
            let vp_inverse = view_proj.inverse();
            let cam_right = (vp_inverse * Vec4::new(1.0, 0.0, 0.0, 0.0)).truncate().normalize();
            let cam_up = (vp_inverse * Vec4::new(0.0, 1.0, 0.0, 0.0)).truncate().normalize();
            let vp_cols = view_proj.to_cols_array_2d();
            self.plankton_system.update_frame(
                &mut builder,
                plankton_emitters,
                &vp_cols,
                cam_right.into(),
                cam_up.into(),
                0.1,  // near plane (TODO: pass from camera)
                1000.0, // far plane (TODO: pass from camera)
            )?;
        } else {
            self.plankton_system.update_frame(
                &mut builder,
                &[],
                &[[0.0; 4]; 4],
                [0.0; 3],
                [0.0; 3],
                0.1,
                1000.0,
            )?;
        }

        let graph = {
            crate::profile_scope!("graph_setup");

            let mut graph = RenderGraph::new();

            let gbuffer_position = graph.declare_virtual("gbuffer_position");
            let gbuffer_normal = graph.declare_virtual("gbuffer_normal");
            let gbuffer_albedo = graph.declare_virtual("gbuffer_albedo");
            let gbuffer_material = graph.declare_virtual("gbuffer_material");
            let gbuffer_emissive = graph.declare_virtual("gbuffer_emissive");
            let gbuffer_depth = graph.declare_virtual("gbuffer_depth");
            let target_res = graph.declare_virtual("target");

            let shadow_map_res = graph.import_image("shadow_map", self.shadow_pass.shadow_map());
            let hdr_res = graph.import_image("hdr_target", self.hdr_target.clone());

            graph.add_pass("geometry", |b| {
                b.write(gbuffer_position);
                b.write(gbuffer_normal);
                b.write(gbuffer_albedo);
                b.write(gbuffer_material);
                b.write(gbuffer_emissive);
                b.write(gbuffer_depth);
            });

            if light_data.shadow_enabled > 0.5 {
                graph.add_pass("shadow", |b| {
                    b.write(shadow_map_res);
                });
            }

            if settings.ssao_enabled {
                let ssao_raw_res = graph.declare_virtual("ssao_raw");
                let ssao_blurred_res = graph.declare_virtual("ssao_blurred");

                graph.add_pass("ssao", |b| {
                    b.read(gbuffer_position);
                    b.read(gbuffer_normal);
                    b.write(ssao_raw_res);
                });

                graph.add_pass("ssao_blur", |b| {
                    b.read(ssao_raw_res);
                    b.write(ssao_blurred_res);
                });
            }

            graph.add_pass("lighting", |b| {
                b.read(gbuffer_position);
                b.read(gbuffer_normal);
                b.read(gbuffer_albedo);
                b.read(gbuffer_material);
                b.read(gbuffer_emissive);
                b.read(shadow_map_res);
                b.write(hdr_res);
            });

            if self.plankton_system.has_pending_draws() {
                graph.add_pass("plankton", |b| {
                    b.read(gbuffer_depth);
                    b.modify(hdr_res);
                });
            }

            let bloom_res = graph.declare_virtual("bloom_result");
            let lum_res = graph.declare_virtual("luminance_result");

            if settings.bloom_enabled {
                graph.add_pass("bloom", |b| {
                    b.read(hdr_res);
                    b.write(bloom_res);
                });
            }

            if matches!(settings.exposure_mode, ExposureMode::Auto) {
                graph.add_pass("luminance", |b| {
                    b.read(hdr_res);
                    b.write(lum_res);
                });
            }

            graph.add_pass("composite", |b| {
                b.read(hdr_res);
                b.read(bloom_res);
                b.read(lum_res);
                b.write(target_res);
            });

            if grid_visible {
                graph.add_pass("grid", |b| {
                    b.read(gbuffer_depth);
                    b.modify(target_res);
                });
            }

            if !debug_draw.is_empty() {
                graph.add_pass("debug_draw", |b| {
                    b.read(gbuffer_depth);
                    b.modify(target_res);
                });
            }

            graph.mark_output(target_res);
            graph.enable_culling();
            graph.compile()?;
            graph
        };

        for &pass_idx in graph.compiled_order() {
            let pass_name = graph.pass_name(pass_idx);
            match pass_name {
                "shadow" => {
                    self.render_shadow_pass(&mut builder, shadow_caster_data, light_data)?;
                }
                "geometry" => {
                    crate::profile_scope!("geometry_pass");

                    let gbuffer_extent = self.gbuffer.framebuffer.extent();
                    let viewport = vulkano::pipeline::graphics::viewport::Viewport {
                        offset: [0.0, 0.0],
                        extent: [gbuffer_extent[0] as f32, gbuffer_extent[1] as f32],
                        depth_range: 0.0..=1.0,
                    };
                    let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                        offset: [0, 0],
                        extent: [gbuffer_extent[0], gbuffer_extent[1]],
                    };

                    builder
                        .begin_render_pass(
                            RenderPassBeginInfo {
                                clear_values: vec![
                                    Some([0.0, 0.0, 0.0, 1.0].into()),
                                    Some([0.0, 0.0, 0.0, 1.0].into()),
                                    Some([0.0, 0.0, 0.0, 1.0].into()),
                                    Some([0.0, 0.0, 0.0, 1.0].into()),
                                    Some([0.0, 0.0, 0.0, 0.0].into()), // emissive
                                    Some(1.0.into()),
                                ],
                                ..RenderPassBeginInfo::framebuffer(
                                    self.gbuffer.framebuffer.clone(),
                                )
                            },
                            SubpassBeginInfo {
                                contents: SubpassContents::Inline,
                                ..Default::default()
                            },
                        )?
                        .bind_pipeline_graphics(self.geometry_pass.pipeline(&self.pipeline_registry))?
                        .set_viewport(0, smallvec![viewport.clone()])?
                        .set_scissor(0, smallvec![scissor])?;

                    {
                        crate::profile_scope!("mesh_loop");
                        let mut last_material_ptr: Option<usize> = None;
                        let mut last_palette: Option<usize> = None;
                        let geom_layout = self.geometry_pass.layout();
                        for mesh in mesh_data {
                            self.render_counters.visible_entities += 1;
                            self.render_counters.draw_calls += 1;
                            self.render_counters.triangles += mesh.index_count / 3;

                            let mat_set = mesh
                                .material_descriptor_set
                                .as_ref()
                                .unwrap_or(&self.default_material_set);
                            let mat_ptr = Arc::as_ptr(mat_set) as usize;
                            if last_material_ptr != Some(mat_ptr) {
                                builder.bind_descriptor_sets(
                                    PipelineBindPoint::Graphics,
                                    geom_layout.clone(),
                                    1,
                                    mat_set.clone(),
                                )?;
                                last_material_ptr = Some(mat_ptr);
                                self.render_counters.material_changes += 1;
                            }

                            let palette_ptr = Arc::as_ptr(&mesh.bone_palette_set) as usize;
                            if last_palette != Some(palette_ptr) {
                                builder.bind_descriptor_sets(
                                    PipelineBindPoint::Graphics,
                                    geom_layout.clone(),
                                    0,
                                    mesh.bone_palette_set.clone(),
                                )?;
                                last_palette = Some(palette_ptr);
                            }

                            builder
                                .bind_vertex_buffers(0, mesh.vertex_buffer.clone())?
                                .bind_index_buffer(mesh.index_buffer.clone())?
                                .push_constants(
                                    geom_layout.clone(),
                                    0,
                                    mesh.push_constants,
                                )?;
                            unsafe {
                                builder.draw_indexed(mesh.index_count, 1, 0, 0, 0)?;
                            }
                        }
                    }

                    builder.end_render_pass(SubpassEndInfo::default())?;
                }
                "ssao" => {
                    self.render_ssao_pass(&mut builder, view_proj, settings)?;
                }
                "ssao_blur" => {
                    self.render_ssao_blur_pass(&mut builder)?;
                }
                "luminance" => {
                    self.render_luminance_pass(&mut builder)?;
                }
                "lighting" => {
                    crate::profile_scope!("lighting_pass");

                    let hdr_extent = self.hdr_framebuffer.extent();
                    let hdr_viewport = vulkano::pipeline::graphics::viewport::Viewport {
                        offset: [0.0, 0.0],
                        extent: [hdr_extent[0] as f32, hdr_extent[1] as f32],
                        depth_range: 0.0..=1.0,
                    };
                    let hdr_scissor = vulkano::pipeline::graphics::viewport::Scissor {
                        offset: [0, 0],
                        extent: [hdr_extent[0], hdr_extent[1]],
                    };

                    builder
                        .begin_render_pass(
                            RenderPassBeginInfo {
                                clear_values: vec![Some([0.0, 0.0, 0.0, 1.0].into())],
                                ..RenderPassBeginInfo::framebuffer(self.hdr_framebuffer.clone())
                            },
                            SubpassBeginInfo {
                                contents: SubpassContents::Inline,
                                ..Default::default()
                            },
                        )?
                        .bind_pipeline_graphics(self.lighting_pass.pipeline(&self.pipeline_registry))?
                        .set_viewport(0, smallvec![hdr_viewport])?
                        .set_scissor(0, smallvec![hdr_scissor])?
                        .bind_descriptor_sets(
                            PipelineBindPoint::Graphics,
                            self.lighting_pass.layout(),
                            0,
                            self.gbuffer_descriptor_set.clone(),
                        )?
                        .bind_descriptor_sets(
                            PipelineBindPoint::Graphics,
                            self.lighting_pass.layout(),
                            1,
                            self.shadow_descriptor_set.clone(),
                        )?
                        .bind_descriptor_sets(
                            PipelineBindPoint::Graphics,
                            self.lighting_pass.layout(),
                            2,
                            if settings.ssao_enabled {
                                self.ssao_descriptor_set.clone()
                            } else {
                                self.ssao_fallback_descriptor_set.clone()
                            },
                        )?
                        .push_constants(self.lighting_pass.layout(), 0, *light_data)?;
                    unsafe {
                        builder.draw(3, 1, 0, 0)?;
                    }
                    builder.end_render_pass(SubpassEndInfo::default())?;
                }
                "plankton" => {
                    crate::profile_scope!("plankton_pass");
                    self.plankton_system.render_particles(&mut builder)?;
                }
                "bloom" => {
                    self.render_bloom_pass(&mut builder, settings)?;
                }
                "composite" => {
                    crate::profile_scope!("composite_pass");

                    let target_extent = target_framebuffer.extent();
                    let target_viewport = vulkano::pipeline::graphics::viewport::Viewport {
                        offset: [0.0, 0.0],
                        extent: [target_extent[0] as f32, target_extent[1] as f32],
                        depth_range: 0.0..=1.0,
                    };
                    let target_scissor = vulkano::pipeline::graphics::viewport::Scissor {
                        offset: [0, 0],
                        extent: [target_extent[0], target_extent[1]],
                    };

                    let exposure_val = match settings.exposure_mode {
                        ExposureMode::Manual(v) => v,
                        ExposureMode::Auto => 1.0,
                    };
                    let composite_push = CompositePushConstants {
                        exposure: exposure_val,
                        bloom_intensity: if settings.bloom_enabled {
                            settings.bloom_intensity
                        } else {
                            0.0
                        },
                        vignette_intensity: settings.vignette_intensity,
                        tone_map_mode: match settings.tone_map_mode {
                            ToneMapMode::Reinhard => 0.0,
                            ToneMapMode::AcesFilmic => 1.0,
                        },
                        exposure_mode: match settings.exposure_mode {
                            ExposureMode::Auto => 0.0,
                            ExposureMode::Manual(_) => 1.0,
                        },
                        _pad0: 0.0,
                        _pad1: 0.0,
                        _pad2: 0.0,
                    };

                    builder
                        .begin_render_pass(
                            RenderPassBeginInfo {
                                clear_values: vec![Some([0.0, 0.0, 0.0, 1.0].into())],
                                ..RenderPassBeginInfo::framebuffer(target_framebuffer.clone())
                            },
                            SubpassBeginInfo {
                                contents: SubpassContents::Inline,
                                ..Default::default()
                            },
                        )?
                        .bind_pipeline_graphics(self.composite_pass.pipeline())?
                        .set_viewport(0, smallvec![target_viewport])?
                        .set_scissor(0, smallvec![target_scissor])?
                        .bind_descriptor_sets(
                            PipelineBindPoint::Graphics,
                            self.composite_pass.layout(),
                            0,
                            self.composite_descriptor_set.clone(),
                        )?
                        .push_constants(self.composite_pass.layout(), 0, composite_push)?;
                    unsafe {
                        builder.draw(3, 1, 0, 0)?;
                    }
                    builder.end_render_pass(SubpassEndInfo::default())?;
                }
                "grid" => {
                    if let Some(ref grid_fb) = depth_framebuffer {
                        crate::profile_scope!("grid_pass");

                        let grid_extent = grid_fb.extent();
                        let grid_viewport = vulkano::pipeline::graphics::viewport::Viewport {
                            offset: [0.0, 0.0],
                            extent: [grid_extent[0] as f32, grid_extent[1] as f32],
                            depth_range: 0.0..=1.0,
                        };
                        let grid_scissor = vulkano::pipeline::graphics::viewport::Scissor {
                            offset: [0, 0],
                            extent: [grid_extent[0], grid_extent[1]],
                        };

                        let grid_extent_size = 500.0;
                        let grid_push = GridPushConstants::new(
                            view_proj,
                            camera_pos,
                            grid_extent_size,
                            100.0,
                        );

                        builder
                            .begin_render_pass(
                                RenderPassBeginInfo {
                                    clear_values: vec![None, None],
                                    ..RenderPassBeginInfo::framebuffer(grid_fb.clone())
                                },
                                SubpassBeginInfo {
                                    contents: SubpassContents::Inline,
                                    ..Default::default()
                                },
                            )?
                            .bind_pipeline_graphics(self.grid_pass.pipeline())?
                            .set_viewport(0, smallvec![grid_viewport])?
                            .set_scissor(0, smallvec![grid_scissor])?
                            .push_constants(self.grid_pass.layout(), 0, grid_push)?;

                        unsafe {
                            builder.draw(4, 1, 0, 0)?;
                        }
                        builder.end_render_pass(SubpassEndInfo::default())?;
                    }
                }
                "debug_draw" => {
                    if let Some(ref debug_fb) = depth_framebuffer {
                        crate::profile_scope!("debug_draw_pass");

                        let debug_extent = debug_fb.extent();
                        let debug_viewport = vulkano::pipeline::graphics::viewport::Viewport {
                            offset: [0.0, 0.0],
                            extent: [debug_extent[0] as f32, debug_extent[1] as f32],
                            depth_range: 0.0..=1.0,
                        };
                        let debug_scissor = vulkano::pipeline::graphics::viewport::Scissor {
                            offset: [0, 0],
                            extent: [debug_extent[0], debug_extent[1]],
                        };

                        let debug_push = DebugLinePushConstants {
                            view_proj: view_proj.to_cols_array_2d(),
                        };

                        builder.begin_render_pass(
                            RenderPassBeginInfo {
                                clear_values: vec![None, None],
                                ..RenderPassBeginInfo::framebuffer(debug_fb.clone())
                            },
                            SubpassBeginInfo {
                                contents: SubpassContents::Inline,
                                ..Default::default()
                            },
                        )?;

                        if let Some(ref depth_buf) = debug_draw.depth_buffer {
                            builder
                                .bind_pipeline_graphics(
                                    self.debug_draw_pass.depth_pipeline(),
                                )?
                                .set_viewport(0, smallvec![debug_viewport.clone()])?
                                .set_scissor(0, smallvec![debug_scissor])?
                                .push_constants(
                                    self.debug_draw_pass.layout(),
                                    0,
                                    debug_push,
                                )?
                                .bind_vertex_buffers(0, depth_buf.clone())?;
                            unsafe {
                                builder.draw(debug_draw.depth_vertex_count, 1, 0, 0)?;
                            }
                        }

                        if let Some(ref overlay_buf) = debug_draw.overlay_buffer {
                            builder
                                .bind_pipeline_graphics(
                                    self.debug_draw_pass.overlay_pipeline(),
                                )?
                                .set_viewport(0, smallvec![debug_viewport])?
                                .set_scissor(0, smallvec![debug_scissor])?
                                .push_constants(
                                    self.debug_draw_pass.layout(),
                                    0,
                                    debug_push,
                                )?
                                .bind_vertex_buffers(0, overlay_buf.clone())?;
                            unsafe {
                                builder.draw(debug_draw.overlay_vertex_count, 1, 0, 0)?;
                            }
                        }

                        builder.end_render_pass(SubpassEndInfo::default())?;
                    }
                }
                _ => {}
            }
        }

        let command_buffer = {
            crate::profile_scope!("command_buffer_build");
            builder.build()?
        };

        Ok(command_buffer)
    }

    pub fn set_debug_view(&mut self, view: DebugView) {
        self.debug_view = view;
    }

    pub fn render_counters(&self) -> &RenderCounters {
        &self.render_counters
    }

    pub fn geometry_pipeline(&self) -> Arc<vulkano::pipeline::GraphicsPipeline> {
        self.geometry_pass.pipeline(&self.pipeline_registry)
    }

    /// Access the pipeline registry (for debug menu rebuild triggers).
    pub fn pipeline_registry(&self) -> &PipelineRegistry {
        &self.pipeline_registry
    }

    /// Default material descriptor set (Set 1) for meshes without a material.
    pub fn default_material_set(&self) -> &Arc<DescriptorSet> {
        &self.default_material_set
    }

    fn render_shadow_pass(
        &mut self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        mesh_data: &[MeshRenderData],
        light_data: &LightUniformData,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("shadow_pass");

        let shadow_extent = self.shadow_pass.shadow_map().image().extent();
        let shadow_w = shadow_extent[0];
        let shadow_h = shadow_extent[1];
        let shadow_viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [shadow_w as f32, shadow_h as f32],
            depth_range: 0.0..=1.0,
        };
        let shadow_scissor = vulkano::pipeline::graphics::viewport::Scissor {
            offset: [0, 0],
            extent: [shadow_w, shadow_h],
        };

        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![Some(1.0.into())],
                    ..RenderPassBeginInfo::framebuffer(self.shadow_pass.framebuffer())
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )?
            .bind_pipeline_graphics(self.shadow_pass.pipeline())?
            .set_viewport(0, smallvec![shadow_viewport])?
            .set_scissor(0, smallvec![shadow_scissor])?;

        let shadow_layout = self.shadow_pass.layout();
        for mesh in mesh_data {
            builder
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    shadow_layout.clone(),
                    0,
                    mesh.bone_palette_set.clone(),
                )?
                .bind_vertex_buffers(0, mesh.vertex_buffer.clone())?
                .bind_index_buffer(mesh.index_buffer.clone())?
                .push_constants(
                    shadow_layout.clone(),
                    0,
                    PushConstantData {
                        model: mesh.push_constants.model,
                        view_projection: light_data.light_vp,
                    },
                )?;
            unsafe {
                builder.draw_indexed(mesh.index_count, 1, 0, 0, 0)?;
            }
            self.render_counters.draw_calls += 1;
        }

        builder.end_render_pass(SubpassEndInfo::default())?;
        Ok(())
    }

    fn render_ssao_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        view_proj: Mat4,
        settings: &PostProcessingSettings,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("ssao_pass");

        let fb = self.ssao_pass.ssao_raw_framebuffer();
        let extent = fb.extent();
        let viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [extent[0] as f32, extent[1] as f32],
            depth_range: 0.0..=1.0,
        };
        let scissor = vulkano::pipeline::graphics::viewport::Scissor {
            offset: [0, 0],
            extent: [extent[0], extent[1]],
        };

        let ssao_desc = self
            .ssao_pass
            .ssao_gbuffer_set()
            .ok_or("ssao gbuffer set not prepared")?
            .clone();
        let kernel_desc = self
            .ssao_pass
            .ssao_kernel_set()
            .ok_or("ssao kernel set not prepared")?
            .clone();

        let push = SsaoPushConstants {
            view_projection: view_proj.to_cols_array_2d(),
            screen_size: [extent[0] as f32, extent[1] as f32],
            radius: settings.ssao_radius,
            bias: 0.025,
        };

        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![None],
                    ..RenderPassBeginInfo::framebuffer(fb)
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )?
            .bind_pipeline_graphics(self.ssao_pass.ssao_pipeline())?
            .set_viewport(0, smallvec![viewport])?
            .set_scissor(0, smallvec![scissor])?
            .bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.ssao_pass.ssao_layout().clone(),
                0,
                ssao_desc,
            )?
            .bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.ssao_pass.ssao_layout().clone(),
                1,
                kernel_desc,
            )?
            .push_constants(self.ssao_pass.ssao_layout().clone(), 0, push)?;
        unsafe {
            builder.draw(3, 1, 0, 0)?;
        }
        builder.end_render_pass(SubpassEndInfo::default())?;
        Ok(())
    }

    fn render_ssao_blur_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("ssao_blur_pass");

        let fb = self.ssao_pass.ssao_blur_framebuffer();
        let extent = fb.extent();
        let viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [extent[0] as f32, extent[1] as f32],
            depth_range: 0.0..=1.0,
        };
        let scissor = vulkano::pipeline::graphics::viewport::Scissor {
            offset: [0, 0],
            extent: [extent[0], extent[1]],
        };

        let blur_desc = self
            .ssao_pass
            .blur_set()
            .ok_or("ssao blur set not prepared")?
            .clone();

        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![None],
                    ..RenderPassBeginInfo::framebuffer(fb)
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )?
            .bind_pipeline_graphics(self.ssao_pass.blur_pipeline())?
            .set_viewport(0, smallvec![viewport])?
            .set_scissor(0, smallvec![scissor])?
            .bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.ssao_pass.blur_layout().clone(),
                0,
                blur_desc,
            )?;
        unsafe {
            builder.draw(3, 1, 0, 0)?;
        }
        builder.end_render_pass(SubpassEndInfo::default())?;
        Ok(())
    }

    fn render_luminance_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("luminance_pass");

        let level_count = self.luminance_pass.level_count();
        let level_fbs = self.luminance_pass.level_framebuffers();
        let level_sizes = self.luminance_pass.level_sizes();

        for i in 0..level_count {
            let size = level_sizes[i];
            let viewport = vulkano::pipeline::graphics::viewport::Viewport {
                offset: [0.0, 0.0],
                extent: [size as f32, size as f32],
                depth_range: 0.0..=1.0,
            };
            let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [0, 0],
                extent: [size, size],
            };

            let desc = self
                .luminance_pass
                .level_set(i)
                .ok_or("luminance level set not prepared")?
                .clone();

            let push = LuminancePush {
                is_first_pass: if i == 0 { 1.0 } else { 0.0 },
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            };

            builder
                .begin_render_pass(
                    RenderPassBeginInfo {
                        clear_values: vec![None],
                        ..RenderPassBeginInfo::framebuffer(level_fbs[i].clone())
                    },
                    SubpassBeginInfo {
                        contents: SubpassContents::Inline,
                        ..Default::default()
                    },
                )?
                .bind_pipeline_graphics(self.luminance_pass.pipeline())?
                .set_viewport(0, smallvec![viewport])?
                .set_scissor(0, smallvec![scissor])?
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.luminance_pass.layout().clone(),
                    0,
                    desc,
                )?
                .push_constants(self.luminance_pass.layout().clone(), 0, push)?;
            unsafe {
                builder.draw(3, 1, 0, 0)?;
            }
            builder.end_render_pass(SubpassEndInfo::default())?;
        }

        Ok(())
    }

    fn render_bloom_pass(
        &self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        settings: &PostProcessingSettings,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::profile_scope!("bloom_pass");

        let mip_count = self.bloom_pass.mip_count();
        let mip_sizes = self.bloom_pass.mip_sizes();
        let mip_fbs = self.bloom_pass.mip_framebuffers();
        let additive_fbs = self.bloom_pass.additive_framebuffers();

        // Threshold: HDR → mip[0]
        {
            let size = mip_sizes[0];
            let viewport = vulkano::pipeline::graphics::viewport::Viewport {
                offset: [0.0, 0.0],
                extent: [size[0] as f32, size[1] as f32],
                depth_range: 0.0..=1.0,
            };
            let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [0, 0],
                extent: [size[0], size[1]],
            };

            let desc = self
                .bloom_pass
                .threshold_set()
                .ok_or("bloom threshold set not prepared")?
                .clone();

            builder
                .begin_render_pass(
                    RenderPassBeginInfo {
                        clear_values: vec![None],
                        ..RenderPassBeginInfo::framebuffer(mip_fbs[0].clone())
                    },
                    SubpassBeginInfo {
                        contents: SubpassContents::Inline,
                        ..Default::default()
                    },
                )?
                .bind_pipeline_graphics(self.bloom_pass.threshold_pipeline())?
                .set_viewport(0, smallvec![viewport])?
                .set_scissor(0, smallvec![scissor])?
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.bloom_pass.threshold_layout().clone(),
                    0,
                    desc,
                )?
                .push_constants(
                    self.bloom_pass.threshold_layout().clone(),
                    0,
                    BloomThresholdPush {
                        threshold: settings.bloom_threshold,
                        _pad0: 0.0,
                        _pad1: 0.0,
                        _pad2: 0.0,
                    },
                )?;
            unsafe {
                builder.draw(3, 1, 0, 0)?;
            }
            builder.end_render_pass(SubpassEndInfo::default())?;
        }

        // Downsample: mip[i-1] → mip[i]
        for i in 1..mip_count {
            let size = mip_sizes[i];
            let src_size = mip_sizes[i - 1];
            let viewport = vulkano::pipeline::graphics::viewport::Viewport {
                offset: [0.0, 0.0],
                extent: [size[0] as f32, size[1] as f32],
                depth_range: 0.0..=1.0,
            };
            let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [0, 0],
                extent: [size[0], size[1]],
            };

            let desc = self
                .bloom_pass
                .downsample_set(i - 1)
                .ok_or("bloom downsample set not prepared")?
                .clone();

            builder
                .begin_render_pass(
                    RenderPassBeginInfo {
                        clear_values: vec![None],
                        ..RenderPassBeginInfo::framebuffer(mip_fbs[i].clone())
                    },
                    SubpassBeginInfo {
                        contents: SubpassContents::Inline,
                        ..Default::default()
                    },
                )?
                .bind_pipeline_graphics(self.bloom_pass.downsample_pipeline())?
                .set_viewport(0, smallvec![viewport])?
                .set_scissor(0, smallvec![scissor])?
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.bloom_pass.downsample_layout().clone(),
                    0,
                    desc,
                )?
                .push_constants(
                    self.bloom_pass.downsample_layout().clone(),
                    0,
                    BloomSamplePush {
                        texel_size: [1.0 / src_size[0] as f32, 1.0 / src_size[1] as f32],
                        is_first_pass: if i == 1 { 1.0 } else { 0.0 },
                        _pad: 0.0,
                    },
                )?;
            unsafe {
                builder.draw(3, 1, 0, 0)?;
            }
            builder.end_render_pass(SubpassEndInfo::default())?;
        }

        // Upsample: mip[i] → mip[i-1] (additive)
        for i in (0..mip_count - 1).rev() {
            let size = mip_sizes[i];
            let src_size = mip_sizes[i + 1];
            let viewport = vulkano::pipeline::graphics::viewport::Viewport {
                offset: [0.0, 0.0],
                extent: [size[0] as f32, size[1] as f32],
                depth_range: 0.0..=1.0,
            };
            let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                offset: [0, 0],
                extent: [size[0], size[1]],
            };

            let desc = self
                .bloom_pass
                .upsample_set(i)
                .ok_or("bloom upsample set not prepared")?
                .clone();

            builder
                .begin_render_pass(
                    RenderPassBeginInfo {
                        clear_values: vec![None],
                        ..RenderPassBeginInfo::framebuffer(additive_fbs[i].clone())
                    },
                    SubpassBeginInfo {
                        contents: SubpassContents::Inline,
                        ..Default::default()
                    },
                )?
                .bind_pipeline_graphics(self.bloom_pass.upsample_pipeline())?
                .set_viewport(0, smallvec![viewport])?
                .set_scissor(0, smallvec![scissor])?
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.bloom_pass.upsample_layout().clone(),
                    0,
                    desc,
                )?
                .push_constants(
                    self.bloom_pass.upsample_layout().clone(),
                    0,
                    BloomSamplePush {
                        texel_size: [1.0 / src_size[0] as f32, 1.0 / src_size[1] as f32],
                        is_first_pass: 0.0,
                        _pad: 0.0,
                    },
                )?;
            unsafe {
                builder.draw(3, 1, 0, 0)?;
            }
            builder.end_render_pass(SubpassEndInfo::default())?;
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct MeshRenderData {
    pub vertex_buffer: Subbuffer<[crate::engine::rendering::rendering_3d::Vertex3D]>,
    pub index_buffer: Subbuffer<[u32]>,
    pub index_count: u32,
    pub mesh_index: usize,
    pub material_index: usize,
    pub push_constants: PushConstantData,
    pub bone_palette_set: Arc<DescriptorSet>,
    /// Pre-resolved material descriptor set (Set 1 for geometry pass).
    /// Resolved at `prepare_mesh_data` time — the render thread does no manager lookups.
    pub material_descriptor_set: Option<Arc<DescriptorSet>>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PushConstantData {
    pub model: [[f32; 4]; 4],
    pub view_projection: [[f32; 4]; 4],
}

unsafe impl bytemuck::Pod for PushConstantData {}
unsafe impl bytemuck::Zeroable for PushConstantData {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LightUniformData {
    pub camera_position: [f32; 3],
    pub shadow_bias: f32,
    pub directional_light_dir: [f32; 3],
    pub shadow_enabled: f32,
    pub directional_light_color: [f32; 3],
    pub directional_light_intensity: f32,
    pub ambient_color: [f32; 3],
    pub ambient_intensity: f32,
    pub light_vp: [[f32; 4]; 4],
}

unsafe impl bytemuck::Pod for LightUniformData {}
unsafe impl bytemuck::Zeroable for LightUniformData {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExposureMode {
    Auto,
    Manual(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToneMapMode {
    Reinhard,
    AcesFilmic,
}

pub struct PostProcessingSettings {
    pub bloom_enabled: bool,
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub ssao_enabled: bool,
    pub ssao_radius: f32,
    pub ssao_intensity: f32,
    pub exposure_mode: ExposureMode,
    pub vignette_intensity: f32,
    pub tone_map_mode: ToneMapMode,
}

impl Default for PostProcessingSettings {
    fn default() -> Self {
        Self {
            bloom_enabled: true,
            bloom_intensity: 0.04,
            bloom_threshold: 1.0,
            ssao_enabled: true,
            ssao_radius: 0.5,
            ssao_intensity: 1.0,
            // Manual default avoids the "darker when close" snap from the
            // instant auto-exposure path. Scene brightness should be driven by
            // light intensity; users can opt into Auto for adaptive behavior.
            exposure_mode: ExposureMode::Manual(1.0),
            vignette_intensity: 0.0,
            tone_map_mode: ToneMapMode::Reinhard,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_uniform_data_size() {
        assert_eq!(std::mem::size_of::<LightUniformData>(), 128);
    }
}

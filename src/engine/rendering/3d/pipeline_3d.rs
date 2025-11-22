// Graphics pipeline - combines shaders, vertex format, and rendering settings
use std::mem::size_of;
use vulkano::descriptor_set::layout::DescriptorSetLayout;
use std::sync::Arc;
use vulkano::device::Device;
use vulkano::pipeline::graphics::{
    color_blend::{ColorBlendAttachmentState, ColorBlendState, AttachmentBlend},
    input_assembly::InputAssemblyState,
    multisample::MultisampleState,
    rasterization::RasterizationState,
    vertex_input::{Vertex as VertexTrait, VertexDefinition},
    viewport::{ViewportState},
    GraphicsPipelineCreateInfo,
    depth_stencil::DepthStencilState,
    depth_stencil::DepthState,
    depth_stencil::CompareOp,
};
use vulkano::shader::ShaderStages;
use vulkano::pipeline::{DynamicState, Pipeline};
use vulkano::pipeline::layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout, PipelineLayoutCreateInfo, PushConstantRange};
use vulkano::pipeline::{GraphicsPipeline, PipelineShaderStageCreateInfo};
use vulkano::render_pass::RenderPass;
use vulkano::descriptor_set::{layout::{DescriptorSetLayoutBinding, DescriptorSetLayoutCreateInfo, DescriptorType}, PersistentDescriptorSet, WriteDescriptorSet};
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::image::sampler::Sampler;
use vulkano::image::view::ImageView;
use vulkano::pipeline::graphics::rasterization::DepthBiasState;
use smallvec::smallvec;


// 3D mesh shaders
pub mod mesh_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/3d/mesh_vs.glsl",
    }
}

pub mod mesh_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/3d/mesh_fs.glsl",
    }
}

pub mod lit_mesh_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/3d/lit_mesh_vs.glsl",
    }
}

pub mod lit_mesh_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/3d/lit_mesh_fs.glsl",
    }
}

// Shadow shaders
pub mod shadow_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/3d/shadow_vs.glsl",
    }
}

pub mod shadow_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/3d/shadow_fs.glsl",
    }
}

// PBR shaders
pub mod pbr_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/3d/pbr_vs.glsl",
    }
}

pub mod pbr_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/3d/pbr_fs.glsl",
    }
}

/// Vertex format for 3D meshes with lighting support
#[derive(Clone, Copy, Debug, Default, vulkano::buffer::BufferContents, vulkano::pipeline::graphics::vertex_input::Vertex)]
#[repr(C)]
pub struct Vertex3D {
    #[format(R32G32B32_SFLOAT)]
    pub position: [f32; 3],  // X, Y, Z position
    #[format(R32G32B32_SFLOAT)]
    pub normal: [f32; 3],    // Surface normal for lighting
    #[format(R32G32_SFLOAT)]
    pub uv: [f32; 2],        // Texture coordinates

    #[format(R32G32B32A32_SFLOAT)]  // W component = bitangent handedness
    pub tangent: [f32; 4],
}

/// Lighting data passed to fragment shader
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct LightingUniformData {
    pub camera_position: [f32; 3],
    pub _padding1: f32,

    pub ambient_color: [f32; 3],
    pub ambient_intensity: f32,

    pub directional_light_dir: [f32; 3],
    pub _padding2: f32,

    pub directional_light_color: [f32; 3],
    pub directional_light_intensity: f32,

    pub metallic: f32,
    pub roughness: f32,
    pub _padding3: f32,
    pub _padding4: f32,
}

unsafe impl bytemuck::Pod for LightingUniformData {}
unsafe impl bytemuck::Zeroable for LightingUniformData {}

impl Default for LightingUniformData {
    fn default() -> Self {
        Self {
            camera_position: [0.0, 0.0, 5.0],
            _padding1: 0.0,

            ambient_color: [1.0, 1.0, 1.0],
            ambient_intensity: 0.2,

            directional_light_dir: [0.3, -1.0, 0.2],
            _padding2: 0.0,

            directional_light_color: [1.0, 0.95, 0.8],
            directional_light_intensity: 1.0,

            metallic: 0.0,
            roughness: 0.5,
            _padding3: 0.0,
            _padding4: 0.0,
        }
    }
}

/// Creates graphics pipeline for 3D mesh rendering
pub fn create_mesh_pipeline(
    device: Arc<Device>,
    render_pass: Arc<RenderPass>,
) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
    // Load shaders
    let vs = mesh_vs::load(device.clone())?.entry_point("main").unwrap();
    let fs = mesh_fs::load(device.clone())?.entry_point("main").unwrap();

    let stages = [
        PipelineShaderStageCreateInfo::new(vs),
        PipelineShaderStageCreateInfo::new(fs),
    ];

    // Vertex input: Vertex3D format
    let vertex_input_state = Vertex3D::per_vertex()
        .definition(&stages[0].entry_point.info().input_interface)?;

    // Pipeline layout (push constants + descriptor set)
    let layout = PipelineLayout::new(
        device.clone(),
        PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
            .into_pipeline_layout_create_info(device.clone())?,
    )?;

    let pipeline = GraphicsPipeline::new(
        device.clone(),
        None,
        GraphicsPipelineCreateInfo {
            stages: stages.into_iter().collect(),
            vertex_input_state: Some(vertex_input_state),
            input_assembly_state: Some(InputAssemblyState::default()),
            viewport_state: Some(ViewportState::default()),
            rasterization_state: Some(RasterizationState::default()),
            multisample_state: Some(MultisampleState::default()),

            // NEW: Enable depth testing!
            depth_stencil_state: Some(DepthStencilState {
                depth: Some(DepthState {
                    compare_op: CompareOp::Less,  // Closer pixels win
                    write_enable: true,            // Update depth buffer
                }),
                ..Default::default()
            }),

            // Alpha blending (same as 2D)
            color_blend_state: Some(ColorBlendState::with_attachment_states(
                1,
                ColorBlendAttachmentState {
                    blend: Some(AttachmentBlend::alpha()),
                    ..Default::default()
                },
            )),

            // Dynamic viewport
            dynamic_state: [DynamicState::Viewport].into_iter().collect(),
            subpass: Some(render_pass.clone().first_subpass().into()),
            ..GraphicsPipelineCreateInfo::layout(layout)
        },
    )?;

    Ok(pipeline)
}

/// Creates graphics pipeline for lit 3D meshes
pub fn create_lit_mesh_pipeline(
    device: Arc<Device>,
    render_pass: Arc<RenderPass>,
) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
    // Load shaders
    let vs = lit_mesh_vs::load(device.clone())?;
    let fs = lit_mesh_fs::load(device.clone())?;

    let vs_entry_point = vs.entry_point("main").unwrap();
    let fs_entry_point = fs.entry_point("main").unwrap();

    let vertex_input_state = Vertex3D::per_vertex()
        .definition(&vs_entry_point.info().input_interface)?;

    // Create pipeline layout with two descriptor sets:
    // - Set 0: Texture (albedo)
    // - Set 1: Lighting uniforms
    let pipeline = GraphicsPipeline::new(
        device.clone(),
        None,
        GraphicsPipelineCreateInfo {
            stages: smallvec![
                PipelineShaderStageCreateInfo::new(vs_entry_point),
                PipelineShaderStageCreateInfo::new(fs_entry_point),
            ],
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
            ..GraphicsPipelineCreateInfo::layout(PipelineLayout::new(
                device.clone(),
                PipelineLayoutCreateInfo {
                    set_layouts: vec![
                        // Set 0: Texture
                        DescriptorSetLayout::new(
                            device.clone(),
                            DescriptorSetLayoutCreateInfo {
                                bindings: [(
                                    0,
                                    DescriptorSetLayoutBinding {
                                        stages: ShaderStages::FRAGMENT,
                                        ..DescriptorSetLayoutBinding::descriptor_type(
                                            DescriptorType::CombinedImageSampler
                                        )
                                    },
                                )]
                                .into(),
                                ..Default::default()
                            },
                        )?,
                        // Set 1: Lighting data
                        DescriptorSetLayout::new(
                            device.clone(),
                            DescriptorSetLayoutCreateInfo {
                                bindings: [(
                                    0,
                                    DescriptorSetLayoutBinding {
                                        stages: ShaderStages::FRAGMENT,
                                        ..DescriptorSetLayoutBinding::descriptor_type(
                                            DescriptorType::UniformBuffer
                                        )
                                    },
                                )]
                                .into(),
                                ..Default::default()
                            },
                        )?,
                    ],
                    push_constant_ranges: vec![PushConstantRange {
                        stages: ShaderStages::VERTEX,
                        offset: 0,
                        size: size_of::<lit_mesh_vs::PushConstants>() as u32,
                    }],
                    ..Default::default()
                },
            )?)
        },
    )?;

    Ok(pipeline)
}

/// Creates PBR graphics pipeline with 4 texture slots
pub fn create_pbr_pipeline(
    device: Arc<Device>,
    render_pass: Arc<RenderPass>,
) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
    let vs = pbr_vs::load(device.clone())?;
    let fs = pbr_fs::load(device.clone())?;

    let vs_entry = vs.entry_point("main").ok_or("Vertex shader missing 'main' entry point")?;
    let fs_entry = fs.entry_point("main").ok_or("Fragment shader missing 'main' entry point")?;

    let vertex_input_state = Vertex3D::per_vertex()
        .definition(&vs_entry.info().input_interface)?;

    let pipeline = GraphicsPipeline::new(
        device.clone(),
        None,
        GraphicsPipelineCreateInfo {
            stages: smallvec![
                PipelineShaderStageCreateInfo::new(vs_entry),
                PipelineShaderStageCreateInfo::new(fs_entry),
            ],
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
            ..GraphicsPipelineCreateInfo::layout(PipelineLayout::new(
                device.clone(),
                PipelineLayoutCreateInfo {
                    set_layouts: vec![
                        // Set 0: Material textures (4 slots)
                        DescriptorSetLayout::new(
                            device.clone(),
                            DescriptorSetLayoutCreateInfo {
                                bindings: [
                                    (0, DescriptorSetLayoutBinding {
                                        stages: ShaderStages::FRAGMENT,
                                        ..DescriptorSetLayoutBinding::descriptor_type(
                                            DescriptorType::CombinedImageSampler
                                        )
                                    }),
                                    (1, DescriptorSetLayoutBinding {
                                        stages: ShaderStages::FRAGMENT,
                                        ..DescriptorSetLayoutBinding::descriptor_type(
                                            DescriptorType::CombinedImageSampler
                                        )
                                    }),
                                    (2, DescriptorSetLayoutBinding {
                                        stages: ShaderStages::FRAGMENT,
                                        ..DescriptorSetLayoutBinding::descriptor_type(
                                            DescriptorType::CombinedImageSampler
                                        )
                                    }),
                                    (3, DescriptorSetLayoutBinding {
                                        stages: ShaderStages::FRAGMENT,
                                        ..DescriptorSetLayoutBinding::descriptor_type(
                                            DescriptorType::CombinedImageSampler
                                        )
                                    }),
                                ].into(),
                                ..Default::default()
                            },
                        )?,
                        // Set 1: Lighting
                        DescriptorSetLayout::new(
                            device.clone(),
                            DescriptorSetLayoutCreateInfo {
                                bindings: [(
                                    0,
                                    DescriptorSetLayoutBinding {
                                        stages: ShaderStages::FRAGMENT,
                                        ..DescriptorSetLayoutBinding::descriptor_type(
                                            DescriptorType::UniformBuffer
                                        )
                                    },
                                )].into(),
                                ..Default::default()
                            },
                        )?,
                        // Set 2: Shadow map
                        DescriptorSetLayout::new(
                            device.clone(),
                            DescriptorSetLayoutCreateInfo {
                                bindings: [(
                                    0,
                                    DescriptorSetLayoutBinding {
                                        stages: ShaderStages::FRAGMENT,
                                        ..DescriptorSetLayoutBinding::descriptor_type(
                                            DescriptorType::CombinedImageSampler
                                        )
                                    },
                                )].into(),
                                ..Default::default()
                            },
                        )?,
                    ],
                    push_constant_ranges: vec![PushConstantRange {
                        stages: ShaderStages::VERTEX,
                        offset: 0,
                        size: size_of::<pbr_vs::PushConstants>() as u32,
                    }],
                    ..Default::default()
                },
            )?)
        },
    )?;

    Ok(pipeline)
}

/// Creates shadow rendering pipeline (depth-only)
pub fn create_shadow_pipeline(
    device: Arc<Device>,
    render_pass: Arc<RenderPass>,
) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
    let vs = shadow_vs::load(device.clone())?;
    let fs = shadow_fs::load(device.clone())?;

    let vs_entry = vs.entry_point("main").ok_or("Vertex shader missing 'main' entry point")?;
    let fs_entry = fs.entry_point("main").ok_or("Fragment shader missing 'main' entry point")?;

    let vertex_input_state = Vertex3D::per_vertex()
        .definition(&vs_entry.info().input_interface)?;

    let pipeline = GraphicsPipeline::new(
        device.clone(),
        None,
        GraphicsPipelineCreateInfo {
            stages: smallvec![
                PipelineShaderStageCreateInfo::new(vs_entry),
                PipelineShaderStageCreateInfo::new(fs_entry),
            ],
            vertex_input_state: Some(vertex_input_state),
            input_assembly_state: Some(InputAssemblyState::default()),
            viewport_state: Some(ViewportState::default()),
            rasterization_state: Some(RasterizationState {
                depth_bias: Some(DepthBiasState {
                    constant_factor: 1.25,  // Prevents shadow acne
                    clamp: 0.0,
                    slope_factor: 1.75,
                }),
                ..Default::default()
            }),
            multisample_state: Some(MultisampleState::default()),
            depth_stencil_state: Some(DepthStencilState {
                depth: Some(DepthState {
                    write_enable: true,
                    compare_op: CompareOp::Less,
                }),
                ..Default::default()
            }),
            color_blend_state: None, // No color attachment
            dynamic_state: [DynamicState::Viewport].into_iter().collect(),
            subpass: Some(render_pass.clone().first_subpass().into()),
            ..GraphicsPipelineCreateInfo::layout(PipelineLayout::new(
                device.clone(),
                PipelineLayoutCreateInfo {
                    set_layouts: vec![
                        
                    ],
                    push_constant_ranges: vec![PushConstantRange {
                        stages: ShaderStages::VERTEX,
                        offset: 0,
                        size: size_of::<shadow_vs::PushConstants>() as u32,
                    }],
                    ..Default::default()
                },
            )?)
        },
    )?;

    Ok(pipeline)
}

/// Creates PBR material descriptor set
pub fn create_pbr_material_descriptor_set(
    allocator: Arc<StandardDescriptorSetAllocator>,
    pipeline: Arc<GraphicsPipeline>,
    albedo: Arc<ImageView>,
    normal: Arc<ImageView>,
    metallic_roughness: Arc<ImageView>,
    ao: Arc<ImageView>,
    sampler: Arc<Sampler>,
) -> Result<Arc<PersistentDescriptorSet>, Box<dyn std::error::Error>> {
    let layout = pipeline.layout().set_layouts()[0].clone();

    let descriptor_set = PersistentDescriptorSet::new(
        &allocator,
        layout,
        [
            WriteDescriptorSet::image_view_sampler(0, albedo, sampler.clone()),
            WriteDescriptorSet::image_view_sampler(1, normal, sampler.clone()),
            WriteDescriptorSet::image_view_sampler(2, metallic_roughness, sampler.clone()),
            WriteDescriptorSet::image_view_sampler(3, ao, sampler),
        ],
        [],
    )?;

    Ok(descriptor_set)
}
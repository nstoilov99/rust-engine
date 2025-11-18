// Graphics pipeline - combines shaders, vertex format, and rendering settings

use std::sync::Arc;
use vulkano::device::Device;
use vulkano::pipeline::graphics::{
    color_blend::{ColorBlendAttachmentState, ColorBlendState, AttachmentBlend},
    input_assembly::InputAssemblyState,
    multisample::MultisampleState,
    rasterization::RasterizationState,
    vertex_input::{Vertex as VertexTrait, VertexDefinition},
    viewport::{Viewport, ViewportState},
    GraphicsPipelineCreateInfo,
    depth_stencil::DepthStencilState,
    depth_stencil::DepthState,
    depth_stencil::CompareOp,
};
use vulkano::shader::ShaderStages;
use vulkano::pipeline::DynamicState;
use vulkano::pipeline::layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout, PipelineLayoutCreateInfo, PushConstantRange};
use vulkano::pipeline::{Pipeline, GraphicsPipeline, PipelineShaderStageCreateInfo};
use vulkano::render_pass::RenderPass;
use vulkano::descriptor_set::{PersistentDescriptorSet, WriteDescriptorSet,
    layout::{DescriptorSetLayoutBinding, DescriptorSetLayoutCreateInfo, DescriptorType}};
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::image::sampler::Sampler;
use vulkano::image::view::ImageView;
use smallvec::smallvec;

// Vertex shader module
mod vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        src: "
            #version 450
            layout(location = 0) in vec2 position;
            layout(location = 1) in vec3 color;
            layout(location = 0) out vec3 fragColor;
            void main() {
                gl_Position = vec4(position, 0.0, 1.0);
                fragColor = color;
            }
        "
    }
}

// Fragment shader module
mod fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        src: "
            #version 450
            layout(location = 0) in vec3 fragColor;
            layout(location = 0) out vec4 outColor;
            void main() {
                outColor = vec4(fragColor, 1.0);
            }
        "
    }
}

/// Vertex shader for textured sprites
mod textured_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        src: "
            #version 450

            layout(location = 0) in vec2 position;
            layout(location = 1) in vec2 uv;

            layout(location = 0) out vec2 fragUV;

            void main() {
                gl_Position = vec4(position, 0.0, 1.0);
                fragUV = uv;  // Pass UV to fragment shader
            }
        "
    }
}

/// Fragment shader for textured sprites
mod textured_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        src: "
            #version 450

            layout(location = 0) in vec2 fragUV;

            layout(location = 0) out vec4 outColor;

            layout(set = 0, binding = 0) uniform sampler2D texSampler;  // Texture binding

            void main() {
                outColor = texture(texSampler, fragUV);  // Sample texture at UV
            }
        "
    }
}

/// Vertex shader with 2D transform support
pub mod transform_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        src: "
            #version 450

            // Vertex inputs
            layout(location = 0) in vec2 position;
            layout(location = 1) in vec2 uv;

            // Push constants (transform data)
            layout(push_constant) uniform PushConstants {
                vec2 pos;       // Position
                float rotation; // Rotation (radians)
                vec2 scale;     // Scale
            } transform;

            // Output to fragment shader
            layout(location = 0) out vec2 fragUV;

            void main() {
                // Apply scale
                vec2 scaled = position * transform.scale;

                // Apply rotation
                float c = cos(transform.rotation);
                float s = sin(transform.rotation);
                vec2 rotated = vec2(
                    scaled.x * c - scaled.y * s,
                    scaled.x * s + scaled.y * c
                );

                // Apply position
                vec2 final_pos = rotated + transform.pos;

                gl_Position = vec4(final_pos, 0.0, 1.0);
                fragUV = uv;
            }
        "
    }
}

// Fragment shader stays the same
mod transform_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        src: "
            #version 450

            layout(location = 0) in vec2 fragUV;
            layout(location = 0) out vec4 outColor;
            layout(set = 0, binding = 0) uniform sampler2D texSampler;

            void main() {
                outColor = texture(texSampler, fragUV);
            }
        "
    }
}

/// Vertex shader with camera view-projection matrix
pub mod camera_vs {
    vulkano_shaders::shader! {
              ty: "vertex",
        src: "
            #version 450

            // Vertex inputs
            layout(location = 0) in vec2 position;
            layout(location = 1) in vec2 uv;  // We have UVs but will override them

            // Push constants (per-sprite data)
            layout(push_constant) uniform PushConstants {
                mat4 view_projection;  // Camera matrix
                vec2 pos;              // Sprite position
                float rotation;        // Sprite rotation
                vec2 scale;            // Sprite scale
                vec4 uv_rect;          // UV coordinates (u_min, v_min, u_max, v_max) - NEW!
            } constants;

            // Output to fragment shader
            layout(location = 0) out vec2 fragUV;

            void main() {
                // Apply sprite transform (scale, rotate, translate)
                vec2 scaled = position * constants.scale;

                float c = cos(constants.rotation);
                float s = sin(constants.rotation);
                vec2 rotated = vec2(
                    scaled.x * c - scaled.y * s,
                    scaled.x * s + scaled.y * c
                );

                vec2 world_pos = rotated + constants.pos;

                // Apply camera view-projection
                gl_Position = constants.view_projection * vec4(world_pos, 0.0, 1.0);

                // Calculate UV from position and uv_rect
                // If uv_rect is (0,0,0,0), use default UVs from vertex
                // Otherwise, map position to sprite sheet UV rectangle
                if (constants.uv_rect == vec4(0.0, 0.0, 0.0, 0.0)) {
                    // Default: use full texture (for non-animated sprites)
                    fragUV = uv;
                } else {
                    // Animation: map position to UV rectangle
                    // position ranges from -0.5 to 0.5, convert to 0-1
                    vec2 uv_local = position + 0.5;

                    // Map to sprite sheet UV rectangle
                    fragUV = mix(
                        constants.uv_rect.xy,  // (u_min, v_min)
                        constants.uv_rect.zw,  // (u_max, v_max)
                        uv_local
                    );
                }
            }
        "
    }
}

// Fragment shader
mod camera_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        src: "
            #version 450

            layout(location = 0) in vec2 fragUV;
            layout(location = 0) out vec4 outColor;
            layout(set = 0, binding = 0) uniform sampler2D texSampler;

            void main() {
                outColor = texture(texSampler, fragUV);
            }
        "
    }
}

/// Vertex shader for 3D meshes with MVP transformation
pub mod mesh_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        src: "
            #version 450

            // Vertex inputs (from Vertex3D struct)
            layout(location = 0) in vec3 position;
            layout(location = 1) in vec3 normal;
            layout(location = 2) in vec2 uv;

            // Push constants (per-draw data)
            layout(push_constant) uniform PushConstants {
                mat4 model;             // Model matrix (local → world)
                mat4 view_projection;   // Combined view + projection
            } constants;

            // Outputs to fragment shader
            layout(location = 0) out vec3 fragNormal;  // Normal in world space
            layout(location = 1) out vec2 fragUV;
            layout(location = 2) out vec3 fragWorldPos; // Position in world space

            void main() {
                // Transform position to world space
                vec4 worldPos = constants.model * vec4(position, 1.0);
                fragWorldPos = worldPos.xyz;

                // Transform normal to world space (important for lighting!)
                // Note: Use transpose(inverse(model)) for non-uniform scaling
                fragNormal = mat3(constants.model) * normal;

                // Transform to clip space for GPU
                gl_Position = constants.view_projection * worldPos;

                // Pass through UV
                fragUV = uv;
            }
        "
    }
}

/// Fragment shader for 3D meshes (simple textured version)
pub mod mesh_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        src: "
            #version 450

            // Inputs from vertex shader
            layout(location = 0) in vec3 fragNormal;
            layout(location = 1) in vec2 fragUV;
            layout(location = 2) in vec3 fragWorldPos;

            // Texture sampler
            layout(set = 0, binding = 0) uniform sampler2D texSampler;

            // Output color
            layout(location = 0) out vec4 outColor;

            void main() {
                // Simple textured output (no lighting yet)
                vec4 texColor = texture(texSampler, fragUV);

                // Debug: Use normal as color (remove after testing)
                // vec3 normalColor = normalize(fragNormal) * 0.5 + 0.5;
                // outColor = vec4(normalColor, 1.0);

                outColor = texColor;
            }
        "
    }
}

pub mod lit_mesh_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/shaders/lit_mesh_vs.glsl",
    }
}

pub mod lit_mesh_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/shaders/lit_mesh_fs.glsl",
    }
}

/// Vertex structure matching shader inputs
#[derive(Clone, Copy, Debug, Default, vulkano::buffer::BufferContents, vulkano::pipeline::graphics::vertex_input::Vertex)]
#[repr(C)]
pub struct Vertex {
    #[format(R32G32_SFLOAT)]
    pub position: [f32; 2],
    #[format(R32G32B32_SFLOAT)]
    pub color: [f32; 3],
}

#[derive(Clone, Copy, Debug, Default, vulkano::buffer::BufferContents, vulkano::pipeline::graphics::vertex_input::Vertex)]
#[repr(C)]
pub struct TexturedVertex {
    #[format(R32G32_SFLOAT)]
    pub position: [f32; 2],  // Screen position
    #[format(R32G32_SFLOAT)]
    pub uv: [f32; 2],        // Texture coordinate
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

/// Creates graphics pipeline with shaders and rendering settings
pub fn create_pipeline(
    device: Arc<Device>,
    render_pass: Arc<RenderPass>,
    viewport: Viewport,
) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
    // Load shaders
    let vs = vs::load(device.clone())?;
    let fs = fs::load(device.clone())?;

    let vs_entry_point = vs.entry_point("main").unwrap();
    let fs_entry_point = fs.entry_point("main").unwrap();

    let vertex_input_state = Vertex::per_vertex()
        .definition(&vs_entry_point.info().input_interface)?;

    let stages = [
        PipelineShaderStageCreateInfo::new(vs_entry_point),
        PipelineShaderStageCreateInfo::new(fs_entry_point),
    ];

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
            viewport_state: Some(ViewportState {
                viewports: [viewport].into_iter().collect(),
                ..Default::default()
            }),
            rasterization_state: Some(RasterizationState::default()),
            multisample_state: Some(MultisampleState::default()),
            color_blend_state: Some(ColorBlendState::with_attachment_states(
                1,
                ColorBlendAttachmentState::default(),
            )),
            subpass: Some(render_pass.clone().first_subpass().into()),
            ..GraphicsPipelineCreateInfo::layout(layout)
        },
    )?;

    println!("✓ Graphics pipeline created");

    Ok(pipeline)
}

/// Creates graphics pipeline for textured sprite rendering
pub fn create_textured_pipeline(
    device: Arc<Device>,
    render_pass: Arc<RenderPass>,
    viewport: Viewport,
) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
    // Load textured shaders
    let vs = textured_vs::load(device.clone())?;
    let fs = textured_fs::load(device.clone())?;

    let vs_entry_point = vs.entry_point("main").unwrap();
    let fs_entry_point = fs.entry_point("main").unwrap();

    let vertex_input_state = TexturedVertex::per_vertex()
        .definition(&vs_entry_point.info().input_interface)?;

    let stages = [
        PipelineShaderStageCreateInfo::new(vs_entry_point),
        PipelineShaderStageCreateInfo::new(fs_entry_point),
    ];

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
            viewport_state: Some(ViewportState {
                viewports: [viewport].into_iter().collect(),
                ..Default::default()
            }),
            rasterization_state: Some(RasterizationState::default()),
            multisample_state: Some(MultisampleState::default()),
            color_blend_state: Some(ColorBlendState::with_attachment_states(
                1,
                ColorBlendAttachmentState::default(),
            )),
            subpass: Some(render_pass.clone().first_subpass().into()),
            ..GraphicsPipelineCreateInfo::layout(layout)
        },
    )?;

    println!("✓ Textured pipeline created");

    Ok(pipeline)
}

/// Creates graphics pipeline with 2D transform support
pub fn create_transform_pipeline(
    device: Arc<Device>,
    render_pass: Arc<RenderPass>,
    viewport: Viewport,
) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
    // Load transform shaders
    let vs = transform_vs::load(device.clone())?;
    let fs = transform_fs::load(device.clone())?;

    let vs_entry_point = vs.entry_point("main").unwrap();
    let fs_entry_point = fs.entry_point("main").unwrap();

    let vertex_input_state = TexturedVertex::per_vertex()
        .definition(&vs_entry_point.info().input_interface)?;

    let stages = [
        PipelineShaderStageCreateInfo::new(vs_entry_point),
        PipelineShaderStageCreateInfo::new(fs_entry_point),
    ];

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
            viewport_state: Some(ViewportState {
                viewports: [viewport].into_iter().collect(),
                ..Default::default()
            }),
            rasterization_state: Some(RasterizationState::default()),
            multisample_state: Some(MultisampleState::default()),
            color_blend_state: Some(ColorBlendState::with_attachment_states(
                1,
                ColorBlendAttachmentState::default(),
            )),
            subpass: Some(render_pass.clone().first_subpass().into()),
            ..GraphicsPipelineCreateInfo::layout(layout)
        },
    )?;

    println!("✓ Transform pipeline created");

    Ok(pipeline)
}

/// Creates graphics pipeline with camera support (view-projection matrix)
pub fn create_camera_pipeline(
    device: Arc<Device>,
    render_pass: Arc<RenderPass>,
    viewport: Viewport,
) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
    // Load camera shaders
    let vs = camera_vs::load(device.clone())?;
    let fs = camera_fs::load(device.clone())?;

    let vs_entry_point = vs.entry_point("main").unwrap();
    let fs_entry_point = fs.entry_point("main").unwrap();

    let vertex_input_state = TexturedVertex::per_vertex()
        .definition(&vs_entry_point.info().input_interface)?;

    let stages = [
        PipelineShaderStageCreateInfo::new(vs_entry_point),
        PipelineShaderStageCreateInfo::new(fs_entry_point),
    ];

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
            viewport_state: Some(ViewportState::default()),  // Dynamic viewport - will be set per-frame
            rasterization_state: Some(RasterizationState::default()),
            multisample_state: Some(MultisampleState::default()),
            color_blend_state: Some(ColorBlendState::with_attachment_states(
                1,
                ColorBlendAttachmentState {
                    blend: Some(AttachmentBlend::alpha()),  // Enable alpha blending for transparency
                    ..Default::default()
                },
            )),
            dynamic_state: [DynamicState::Viewport].into_iter().collect(),  // Enable dynamic viewport
            subpass: Some(render_pass.clone().first_subpass().into()),
            ..GraphicsPipelineCreateInfo::layout(layout)
        },
    )?;

    println!("✓ Camera pipeline created (with dynamic viewport)");

    Ok(pipeline)
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

use vulkano::descriptor_set::layout::DescriptorSetLayout;

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

/// Creates descriptor set binding texture to shader
pub fn create_texture_descriptor_set(
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    pipeline: Arc<GraphicsPipeline>,
    texture_view: Arc<ImageView>,
    sampler: Arc<Sampler>,
) -> Result<Arc<PersistentDescriptorSet>, Box<dyn std::error::Error>> {
    let layout = pipeline.layout().set_layouts().get(0).unwrap();

    let descriptor_set = PersistentDescriptorSet::new(
        &descriptor_set_allocator,
        layout.clone(),
        [WriteDescriptorSet::image_view_sampler(0, texture_view, sampler)],
        [],
    )?;

    println!("✓ Descriptor set created");

    Ok(descriptor_set)
}

/// Creates vertices for a textured quad (sprite)
pub fn create_quad_vertices() -> [TexturedVertex; 4] {
    [
        TexturedVertex { position: [-0.5, -0.5], uv: [0.0, 0.0] },  // Top-left
        TexturedVertex { position: [ 0.5, -0.5], uv: [1.0, 0.0] },  // Top-right
        TexturedVertex { position: [-0.5,  0.5], uv: [0.0, 1.0] },  // Bottom-left
        TexturedVertex { position: [ 0.5,  0.5], uv: [1.0, 1.0] },  // Bottom-right
    ]
}

/// Creates indices for a quad (2 triangles)
pub fn create_quad_indices() -> [u32; 6] {
    [
        0, 1, 2,  // First triangle: top-left, top-right, bottom-left
        1, 3, 2,  // Second triangle: top-right, bottom-right, bottom-left
    ]
}
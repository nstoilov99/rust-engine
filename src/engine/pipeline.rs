// Graphics pipeline - combines shaders, vertex format, and rendering settings

use std::sync::Arc;
use vulkano::device::Device;
use vulkano::pipeline::graphics::{
    color_blend::{ColorBlendAttachmentState, ColorBlendState},
    input_assembly::InputAssemblyState,
    multisample::MultisampleState,
    rasterization::RasterizationState,
    vertex_input::{Vertex as VertexTrait, VertexDefinition},
    viewport::{Viewport, ViewportState},
    GraphicsPipelineCreateInfo,
};
use vulkano::pipeline::layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout};
use vulkano::pipeline::{GraphicsPipeline, PipelineShaderStageCreateInfo};
use vulkano::render_pass::RenderPass;
use vulkano::descriptor_set::{PersistentDescriptorSet, WriteDescriptorSet};
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::pipeline::Pipeline;
use vulkano::image::sampler::Sampler;
use vulkano::image::view::ImageView;

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
pub fn create_quad_vertices() -> [TexturedVertex; 6] {
    [
        // Triangle 1
        TexturedVertex { position: [-0.5, -0.5], uv: [0.0, 0.0] },  // Top-left
        TexturedVertex { position: [ 0.5, -0.5], uv: [1.0, 0.0] },  // Top-right
        TexturedVertex { position: [-0.5,  0.5], uv: [0.0, 1.0] },  // Bottom-left

        // Triangle 2
        TexturedVertex { position: [ 0.5, -0.5], uv: [1.0, 0.0] },  // Top-right
        TexturedVertex { position: [ 0.5,  0.5], uv: [1.0, 1.0] },  // Bottom-right
        TexturedVertex { position: [-0.5,  0.5], uv: [0.0, 1.0] },  // Bottom-left
    ]
}
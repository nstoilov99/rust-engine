use std::sync::Arc;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::descriptor_set::{DescriptorSet, WriteDescriptorSet};
use vulkano::device::Device;
use vulkano::image::sampler::Sampler;
use vulkano::image::view::ImageView;
use vulkano::pipeline::graphics::{
    color_blend::{AttachmentBlend, ColorBlendAttachmentState, ColorBlendState},
    input_assembly::InputAssemblyState,
    multisample::MultisampleState,
    rasterization::RasterizationState,
    vertex_input::{Vertex as VertexTrait, VertexDefinition},
    viewport::{Viewport, ViewportState},
    GraphicsPipelineCreateInfo,
};
use vulkano::pipeline::layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout};
use vulkano::pipeline::DynamicState;
use vulkano::pipeline::{GraphicsPipeline, Pipeline, PipelineShaderStageCreateInfo};
use vulkano::render_pass::RenderPass;

// Basic colored vertex shader
mod vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/2d/basic_vs.glsl",
    }
}

mod fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/2d/basic_fs.glsl",
    }
}

mod textured_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/2d/textured_vs.glsl",
    }
}

mod textured_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/2d/textured_fs.glsl",
    }
}

pub mod transform_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/2d/transform_vs.glsl",
    }
}

mod transform_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/2d/transform_fs.glsl",
    }
}

pub mod camera_vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/engine/rendering/shaders/2d/camera_vs.glsl",
    }
}

mod camera_fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/engine/rendering/shaders/2d/camera_fs.glsl",
    }
}

/// Vertex structure matching shader inputs
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    vulkano::buffer::BufferContents,
    vulkano::pipeline::graphics::vertex_input::Vertex,
)]
#[repr(C)]
pub struct Vertex {
    #[format(R32G32_SFLOAT)]
    pub position: [f32; 2],
    #[format(R32G32B32_SFLOAT)]
    pub color: [f32; 3],
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    vulkano::buffer::BufferContents,
    vulkano::pipeline::graphics::vertex_input::Vertex,
)]
#[repr(C)]
pub struct TexturedVertex {
    #[format(R32G32_SFLOAT)]
    pub position: [f32; 2], // Screen position
    #[format(R32G32_SFLOAT)]
    pub uv: [f32; 2], // Texture coordinate
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

    let vertex_input_state = Vertex::per_vertex().definition(&vs_entry_point)?;

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

    let vertex_input_state = TexturedVertex::per_vertex().definition(&vs_entry_point)?;

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

    let vertex_input_state = TexturedVertex::per_vertex().definition(&vs_entry_point)?;

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
    _viewport: Viewport,
) -> Result<Arc<GraphicsPipeline>, Box<dyn std::error::Error>> {
    // Load camera shaders
    let vs = camera_vs::load(device.clone())?;
    let fs = camera_fs::load(device.clone())?;

    let vs_entry_point = vs.entry_point("main").unwrap();
    let fs_entry_point = fs.entry_point("main").unwrap();

    let vertex_input_state = TexturedVertex::per_vertex().definition(&vs_entry_point)?;

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
            viewport_state: Some(ViewportState::default()), // Dynamic viewport - will be set per-frame
            rasterization_state: Some(RasterizationState::default()),
            multisample_state: Some(MultisampleState::default()),
            color_blend_state: Some(ColorBlendState::with_attachment_states(
                1,
                ColorBlendAttachmentState {
                    blend: Some(AttachmentBlend::alpha()), // Enable alpha blending for transparency
                    ..Default::default()
                },
            )),
            dynamic_state: [DynamicState::Viewport].into_iter().collect(), // Enable dynamic viewport
            subpass: Some(render_pass.clone().first_subpass().into()),
            ..GraphicsPipelineCreateInfo::layout(layout)
        },
    )?;

    println!("✓ Camera pipeline created (with dynamic viewport)");

    Ok(pipeline)
}

/// Creates descriptor set binding texture to shader
pub fn create_texture_descriptor_set(
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    pipeline: Arc<GraphicsPipeline>,
    texture_view: Arc<ImageView>,
    sampler: Arc<Sampler>,
) -> Result<Arc<DescriptorSet>, Box<dyn std::error::Error>> {
    let layout = pipeline.layout().set_layouts().first().unwrap();

    let descriptor_set = DescriptorSet::new(
        descriptor_set_allocator,
        layout.clone(),
        [WriteDescriptorSet::image_view_sampler(
            0,
            texture_view,
            sampler,
        )],
        [],
    )?;

    println!("✓ Descriptor set created");

    Ok(descriptor_set)
}

/// Creates vertices for a textured quad (sprite)
pub fn create_quad_vertices() -> [TexturedVertex; 4] {
    [
        TexturedVertex {
            position: [-0.5, -0.5],
            uv: [0.0, 0.0],
        }, // Top-left
        TexturedVertex {
            position: [0.5, -0.5],
            uv: [1.0, 0.0],
        }, // Top-right
        TexturedVertex {
            position: [-0.5, 0.5],
            uv: [0.0, 1.0],
        }, // Bottom-left
        TexturedVertex {
            position: [0.5, 0.5],
            uv: [1.0, 1.0],
        }, // Bottom-right
    ]
}

/// Creates indices for a quad (2 triangles)
pub fn create_quad_indices() -> [u32; 6] {
    [
        0, 1, 2, // First triangle: top-left, top-right, bottom-left
        1, 3, 2, // Second triangle: top-right, bottom-right, bottom-left
    ]
}

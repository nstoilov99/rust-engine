//! Rendering orchestration for the game loop
//!
//! Handles mesh/light data preparation, swapchain management, and frame rendering.

use hecs::World;
use rust_engine::assets::AssetManager;
use rust_engine::engine::ecs::components::DirectionalLight as EcsDirectionalLight;
use rust_engine::engine::ecs::components::{MeshRenderer, Transform};
use rust_engine::engine::rendering::rendering_3d::{
    DeferredRenderer, LightUniformData, MeshRenderData, PushConstantData,
};
use rust_engine::Renderer;
use std::sync::Arc;
use vulkano::image::Image;
use vulkano::swapchain::acquire_next_image;
use vulkano::sync::{self, GpuFuture};
use vulkano::{Validated, VulkanError};

/// Prepare mesh render data from ECS world
///
/// Performance optimized: view_projection is calculated once per frame,
/// not per mesh.
pub fn prepare_mesh_data(
    world: &World,
    asset_manager: &Arc<AssetManager>,
    renderer: &Renderer,
) -> Vec<MeshRenderData> {
    puffin::profile_function!();

    let mut mesh_data_vec = Vec::new();
    let meshes = asset_manager.meshes.read();

    // Calculate view_projection ONCE per frame (same for all meshes)
    let view_matrix = renderer.camera_3d.view_matrix();
    let projection_matrix = renderer.camera_3d.projection_matrix();
    let view_projection = projection_matrix * view_matrix;
    let vp_array: [[f32; 4]; 4] = unsafe { std::mem::transmute(view_projection) };

    for (_entity, (transform, mesh_renderer)) in
        world.query::<(&Transform, &MeshRenderer)>().iter()
    {
        if let Some(gpu_mesh) = meshes.get(mesh_renderer.mesh_index) {
            // Only model matrix is per-mesh
            let model_matrix = transform.model_matrix();
            let model_array: [[f32; 4]; 4] = unsafe { std::mem::transmute(model_matrix) };

            mesh_data_vec.push(MeshRenderData {
                vertex_buffer: gpu_mesh.vertex_buffer.clone(),
                index_buffer: gpu_mesh.index_buffer.clone(),
                index_count: gpu_mesh.index_count,
                push_constants: PushConstantData {
                    model: model_array,
                    view_projection: vp_array,
                },
            });
        }
    }

    mesh_data_vec
}

/// Prepare light uniform data from ECS world
pub fn prepare_light_data(world: &World, renderer: &Renderer) -> LightUniformData {
    puffin::profile_function!();

    let camera_pos = renderer.camera_3d.position;
    let mut light_data = LightUniformData {
        camera_position: [camera_pos.x, camera_pos.y, camera_pos.z],
        _pad0: 0.0,
        directional_light_dir: [0.0, -1.0, -1.0],
        _pad1: 0.0,
        directional_light_color: [1.0, 1.0, 1.0],
        directional_light_intensity: 1.0,
        ambient_color: [0.1, 0.1, 0.15],
        ambient_intensity: 0.3,
    };

    // Query ECS for directional light (use first one found)
    if let Some((_entity, dir_light)) = world.query::<&EcsDirectionalLight>().iter().next() {
        light_data.directional_light_dir = [
            dir_light.direction.x,
            dir_light.direction.y,
            dir_light.direction.z,
        ];
        light_data.directional_light_color =
            [dir_light.color.x, dir_light.color.y, dir_light.color.z];
        light_data.directional_light_intensity = dir_light.intensity;
    }

    light_data
}

/// Handle swapchain recreation when window is resized
pub fn handle_swapchain_recreation(
    renderer: &mut Renderer,
    deferred_renderer: &mut DeferredRenderer,
    current_debug_view: rust_engine::engine::rendering::rendering_3d::DebugView,
) -> Result<bool, Box<dyn std::error::Error>> {
    use rust_engine::engine::core::swapchain::recreate_swapchain;

    match recreate_swapchain(
        renderer.device.clone(),
        renderer.surface.clone(),
        renderer.swapchain.clone(),
    ) {
        Ok((new_swapchain, new_images)) => {
            // Check if window is minimized
            if new_images.is_empty() {
                renderer.recreate_swapchain = false;
                return Ok(false);
            }

            renderer.swapchain = new_swapchain;
            renderer.images = new_images.clone();

            // Update camera aspect ratio
            let extent = new_images[0].extent();
            renderer
                .camera_3d
                .set_viewport_size(extent[0] as f32, extent[1] as f32);

            // Recreate deferred renderer with new dimensions
            *deferred_renderer = DeferredRenderer::new(
                renderer.device.clone(),
                renderer.queue.clone(),
                renderer.memory_allocator.clone(),
                renderer.command_buffer_allocator.clone(),
                renderer.descriptor_set_allocator.clone(),
                extent[0],
                extent[1],
            )?;
            deferred_renderer.set_debug_view(current_debug_view);

            println!(
                "Swapchain and deferred renderer recreated: {}x{}",
                extent[0], extent[1]
            );

            renderer.recreate_swapchain = false;
            Ok(true)
        }
        Err(e) => {
            eprintln!("Failed to recreate swapchain: {}", e);
            Err(e.into())
        }
    }
}

/// Acquire next swapchain image
pub fn acquire_swapchain_image(
    renderer: &mut Renderer,
) -> Result<
    (
        u32,
        Arc<Image>,
        Box<dyn GpuFuture>,
    ),
    SwapchainError,
> {
    puffin::profile_function!();

    match acquire_next_image(renderer.swapchain.clone(), None) {
        Ok((image_index, suboptimal, acquire_future)) => {
            if suboptimal {
                renderer.recreate_swapchain = true;
            }
            let target_image = renderer.images[image_index as usize].clone();
            Ok((image_index, target_image, acquire_future.boxed()))
        }
        Err(e) => match e {
            Validated::Error(VulkanError::OutOfDate) => {
                renderer.recreate_swapchain = true;
                Err(SwapchainError::OutOfDate)
            }
            _ => Err(SwapchainError::AcquireFailed(format!("{:?}", e))),
        },
    }
}

/// Swapchain acquisition errors
pub enum SwapchainError {
    OutOfDate,
    AcquireFailed(String),
}

/// Create a "now" future for synchronization after errors
pub fn create_now_future(renderer: &Renderer) -> Box<dyn GpuFuture> {
    sync::now(renderer.device.clone()).boxed()
}

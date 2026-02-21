//! Rendering orchestration for the game loop
//!
//! Handles mesh/light data preparation, swapchain management, and frame rendering.

use glam::Vec3;
use hecs::World;
use nalgebra_glm as glm;
use rust_engine::assets::AssetManager;
use rust_engine::engine::adapters::render_adapter::world_matrix_to_render;
use rust_engine::engine::ecs::components::DirectionalLight as EcsDirectionalLight;
use rust_engine::engine::ecs::components::{MeshRenderer, Transform};
use rust_engine::engine::ecs::hierarchy::get_world_transform;
use rust_engine::engine::math::Frustum;
use rust_engine::engine::rendering::rendering_3d::{
    DeferredRenderer, LightUniformData, MeshRenderData, PushConstantData,
};
use rust_engine::Renderer;
use std::sync::Arc;
use vulkano::image::Image;
use vulkano::swapchain::acquire_next_image;
use vulkano::sync::{self, GpuFuture};
use vulkano::{Validated, VulkanError};

/// Prepare mesh render data from ECS world into a reusable buffer
///
/// Performance optimized:
/// - view_projection is calculated once per frame, not per mesh
/// - Reuses the provided Vec buffer (clears and refills) to avoid per-frame allocation
/// - Frustum culling: skips meshes outside camera view
/// - Properly handles hierarchical transforms with non-uniform scaling
pub fn prepare_mesh_data(
    world: &World,
    asset_manager: &Arc<AssetManager>,
    renderer: &Renderer,
    mesh_data_buffer: &mut Vec<MeshRenderData>,
) {
    rust_engine::profile_function!();

    // Clear buffer but retain capacity (no allocation)
    mesh_data_buffer.clear();

    let meshes = asset_manager.meshes.read();

    // Calculate view_projection ONCE per frame (same for all meshes)
    let view_matrix = renderer.camera_3d.view_matrix();
    let projection_matrix = renderer.camera_3d.projection_matrix();
    let view_projection = projection_matrix * view_matrix;

    // Compute frustum planes once per frame for culling
    let frustum = Frustum::from_view_projection(view_projection);

    let vp_array: [[f32; 4]; 4] = unsafe { std::mem::transmute(view_projection) };

    for (entity, (_transform, mesh_renderer)) in
        world.query::<(&Transform, &MeshRenderer)>().iter()
    {
        if !mesh_renderer.visible {
            continue;
        }
        if let Some(gpu_mesh) = meshes.get(mesh_renderer.mesh_index) {
            // Get WORLD transform in Z-up space (includes parent hierarchy)
            let world_matrix_zup = get_world_transform(world, entity);

            // Convert to Y-up for rendering
            let model_matrix = world_matrix_to_render(&world_matrix_zup);

            // FRUSTUM CULLING: Transform bounding sphere to world space
            // Use the render matrix (Y-up) since frustum is in render space
            let c = gpu_mesh.center;
            let m = &model_matrix;
            let world_center = Vec3::new(
                m[(0, 0)] * c.x + m[(0, 1)] * c.y + m[(0, 2)] * c.z + m[(0, 3)],
                m[(1, 0)] * c.x + m[(1, 1)] * c.y + m[(1, 2)] * c.z + m[(1, 3)],
                m[(2, 0)] * c.x + m[(2, 1)] * c.y + m[(2, 2)] * c.z + m[(2, 3)],
            );

            // Extract scale from Z-up world matrix for correct bounding radius
            // Scale magnitude is preserved regardless of coordinate system
            let scale_x = glm::length(&glm::vec3(
                world_matrix_zup[(0, 0)],
                world_matrix_zup[(1, 0)],
                world_matrix_zup[(2, 0)],
            ));
            let scale_y = glm::length(&glm::vec3(
                world_matrix_zup[(0, 1)],
                world_matrix_zup[(1, 1)],
                world_matrix_zup[(2, 1)],
            ));
            let scale_z = glm::length(&glm::vec3(
                world_matrix_zup[(0, 2)],
                world_matrix_zup[(1, 2)],
                world_matrix_zup[(2, 2)],
            ));
            let max_scale = scale_x.max(scale_y).max(scale_z);
            let world_radius = gpu_mesh.radius * max_scale;

            // Skip mesh if completely outside frustum
            if !frustum.contains_sphere(world_center, world_radius) {
                continue;
            }

            let model_array: [[f32; 4]; 4] = unsafe { std::mem::transmute(model_matrix) };

            mesh_data_buffer.push(MeshRenderData {
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
}

/// Prepare light uniform data from ECS world
pub fn prepare_light_data(world: &World, renderer: &Renderer) -> LightUniformData {
    rust_engine::profile_function!();

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

            // NOTE: Do NOT update camera aspect ratio here!
            // The camera should use VIEWPORT PANEL dimensions, not window dimensions.
            // Camera aspect ratio is updated in app.rs when viewport_size changes.

            // Clear the deferred renderer's framebuffer cache (output framebuffers changed)
            // NOTE: We do NOT recreate the DeferredRenderer here because:
            // - The G-Buffer should match the VIEWPORT size, not the window size
            // - The viewport resize logic in app.rs handles G-Buffer resizing
            // - Recreating at window size caused stretching after minimize/restore
            deferred_renderer.clear_framebuffer_cache();

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
    rust_engine::profile_function!();

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

//! Rendering orchestration for the game loop
//!
//! Handles mesh/light data preparation, swapchain management, and frame rendering.

use hecs::World;
use nalgebra_glm as glm;
use rust_engine::assets::AssetManager;
use rust_engine::engine::adapters::render_adapter;
use rust_engine::engine::ecs::components::DirectionalLight as EcsDirectionalLight;
use rust_engine::engine::animation::SkeletonInstance;
use rust_engine::engine::ecs::components::{MeshRenderer, Transform};
use rust_engine::engine::ecs::hierarchy::TransformCache;
use rust_engine::engine::math::{Aabb, Frustum};
use rust_engine::engine::rendering::rendering_3d::{
    DeferredRenderer, LightUniformData, MeshRenderData, PushConstantData, SkinningBackend,
};
use rust_engine::Renderer;
use std::sync::Arc;
use vulkano::image::Image;
use vulkano::swapchain::acquire_next_image;
use vulkano::sync::{self, GpuFuture};
use vulkano::{Validated, VulkanError};

/// Result type for swapchain image acquisition.
type AcquireResult = Result<(u32, Arc<Image>, Box<dyn GpuFuture>), SwapchainError>;

/// Prepare mesh render data from ECS world into a reusable buffer.
///
/// Reads pre-computed transforms from `transform_cache` (populated by
/// `TransformCache::propagate` earlier in the frame).  No recursive
/// hierarchy traversal happens here.
pub fn prepare_mesh_data(
    world: &World,
    asset_manager: &Arc<AssetManager>,
    renderer: &Renderer,
    mesh_data_buffer: &mut Vec<MeshRenderData>,
    transform_cache: &TransformCache,
    skinning: &SkinningBackend,
) {
    rust_engine::profile_scope!("prepare_mesh_data");

    mesh_data_buffer.clear();

    let meshes = asset_manager.meshes.read();
    let identity_set = skinning.identity_set();

    let view_matrix = renderer.camera_3d.view_matrix();
    let projection_matrix = renderer.camera_3d.projection_matrix();
    let view_projection = projection_matrix * view_matrix;

    let frustum = Frustum::from_view_projection(view_projection);

    let vp_array: [[f32; 4]; 4] = view_projection.to_cols_array_2d();

    for (entity, (_transform, mesh_renderer, skeleton)) in
        world.query::<(&Transform, &MeshRenderer, Option<&SkeletonInstance>)>().iter()
    {
        if !mesh_renderer.visible {
            continue;
        }

        // Resolve mesh_path → submesh indices (multi-submesh support)
        let submesh_indices: &[usize] = if !mesh_renderer.mesh_path.is_empty() {
            if let Some(indices) = meshes.indices_for_path(&mesh_renderer.mesh_path) {
                indices
            } else {
                std::slice::from_ref(&mesh_renderer.mesh_index)
            }
        } else {
            std::slice::from_ref(&mesh_renderer.mesh_index)
        };

        let model_matrix = transform_cache.get_render(entity);
        let glam_model = glam::Mat4::from_cols_array_2d(&unsafe {
            std::mem::transmute::<nalgebra_glm::Mat4, [[f32; 4]; 4]>(model_matrix)
        });
        let model_array: [[f32; 4]; 4] = unsafe { std::mem::transmute(model_matrix) };

        // GPU bone palette: use skeleton's palette if present, else identity
        let palette_set = if let Some(skel) = skeleton {
            if !skel.palette.is_empty() {
                match skinning.create_palette_set(&skel.palette) {
                    Ok(set) => set,
                    Err(_) => identity_set.clone(),
                }
            } else {
                identity_set.clone()
            }
        } else {
            identity_set.clone()
        };

        for &mesh_idx in submesh_indices {
            if let Some(gpu_mesh) = meshes.get(mesh_idx) {
                // AABB frustum culling per submesh
                let local_aabb = Aabb::new(gpu_mesh.aabb_min, gpu_mesh.aabb_max);
                let world_aabb = local_aabb.transformed(&glam_model);
                if !frustum.contains_aabb(world_aabb.min, world_aabb.max) {
                    continue;
                }

                mesh_data_buffer.push(MeshRenderData {
                    vertex_buffer: gpu_mesh.vertex_buffer.clone(),
                    index_buffer: gpu_mesh.index_buffer.clone(),
                    index_count: gpu_mesh.index_count,
                    mesh_index: mesh_idx,
                    material_index: mesh_renderer.material_index,
                    push_constants: PushConstantData {
                        model: model_array,
                        view_projection: vp_array,
                    },
                    bone_palette_set: palette_set.clone(),
                });
            }
        }
    }

    mesh_data_buffer.sort_by_key(|mesh| (mesh.material_index, mesh.mesh_index));
}

/// Prepare light uniform data from ECS world
pub fn prepare_light_data(world: &World, renderer: &Renderer) -> LightUniformData {
    rust_engine::profile_scope!("prepare_light_data");

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
        let direction = glm::normalize(&render_adapter::direction_to_render(&dir_light.direction));
        light_data.directional_light_dir = [direction.x, direction.y, direction.z];
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
            Err(e)
        }
    }
}

/// Acquire next swapchain image
pub fn acquire_swapchain_image(renderer: &mut Renderer) -> AcquireResult {
    rust_engine::profile_scope!("acquire_swapchain_image");

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

#[allow(dead_code)]
pub enum SwapchainError {
    OutOfDate,
    AcquireFailed(String),
}

/// Create a "now" future for synchronization after errors
pub fn create_now_future(renderer: &Renderer) -> Box<dyn GpuFuture> {
    sync::now(renderer.device.clone()).boxed()
}

/// Prepare debug draw GPU data from the debug draw buffer.
///
/// Drains lines from the buffer, converts Z-up positions to Y-up render space,
/// and uploads to GPU vertex buffers.
#[cfg(debug_assertions)]
#[allow(dead_code)]
pub fn prepare_debug_draw_data(
    debug_draw_buffer: &mut rust_engine::engine::debug_draw::DebugDrawBuffer,
    renderer: &Renderer,
) -> rust_engine::engine::debug_draw::DebugDrawData {
    use rust_engine::engine::debug_draw::{DebugDrawData, DebugLineVertex};
    use rust_engine::engine::utils::coords::convert_position_zup_to_yup;
    use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
    use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter};

    rust_engine::profile_scope!("prepare_debug_draw_data");

    let (depth_lines, overlay_lines) = debug_draw_buffer.drain();

    let convert_lines = |lines: &[rust_engine::engine::debug_draw::DebugLineData]| -> Option<(vulkano::buffer::Subbuffer<[DebugLineVertex]>, u32)> {
        if lines.is_empty() {
            return None;
        }

        let mut vertices = Vec::with_capacity(lines.len() * 2);
        for line in lines {
            let start_yup = convert_position_zup_to_yup(rust_engine::Vec3::from(line.start));
            let end_yup = convert_position_zup_to_yup(rust_engine::Vec3::from(line.end));
            vertices.push(DebugLineVertex {
                position: start_yup.into(),
                color: line.color,
            });
            vertices.push(DebugLineVertex {
                position: end_yup.into(),
                color: line.color,
            });
        }

        let vertex_count = vertices.len() as u32;
        let buffer = Buffer::from_iter(
            renderer.memory_allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::VERTEX_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            vertices,
        );

        match buffer {
            Ok(buf) => Some((buf, vertex_count)),
            Err(e) => {
                log::warn!("Failed to create debug draw vertex buffer: {}", e);
                None
            }
        }
    };

    let (depth_buffer, depth_vertex_count) = convert_lines(&depth_lines)
        .map(|(b, c)| (Some(b), c))
        .unwrap_or((None, 0));
    let (overlay_buffer, overlay_vertex_count) = convert_lines(&overlay_lines)
        .map(|(b, c)| (Some(b), c))
        .unwrap_or((None, 0));

    DebugDrawData {
        depth_buffer,
        depth_vertex_count,
        overlay_buffer,
        overlay_vertex_count,
    }
}

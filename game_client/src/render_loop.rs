//! Rendering orchestration for the game loop
//!
//! Handles mesh/light data preparation, swapchain management, and frame rendering.

use hecs::World;
use nalgebra_glm as glm;
use rust_engine::assets::AssetManager;
use rust_engine::engine::adapters::render_adapter;
use rust_engine::engine::animation::SkeletonInstance;
use rust_engine::engine::ecs::components::DirectionalLight as EcsDirectionalLight;
use rust_engine::engine::ecs::components::{
    EditorVisibility, EntityGuid, ParticleEffect, SpawnShape,
};
use rust_engine::engine::ecs::components::{MeshRenderer, Transform};
use rust_engine::engine::ecs::hierarchy::Parent;
use rust_engine::engine::ecs::hierarchy::TransformCache;
use rust_engine::engine::math::{Aabb, Frustum};
use rust_engine::engine::rendering::frame_packet::{
    EmissionParameters, EmitterFlags, ForceParameters, PlanktonEmitterFrameData, VisualParameters,
};
use rust_engine::engine::rendering::rendering_3d::{
    DeferredRenderer, LightUniformData, MeshRenderData, SkinnedPaletteFrame, SkinningBackend,
};
use rust_engine::Renderer;
use std::sync::Arc;
use vulkano::descriptor_set::DescriptorSet;
use vulkano::image::Image;
use vulkano::swapchain::acquire_next_image;
use vulkano::sync::{self, GpuFuture};
use vulkano::{Validated, VulkanError};

/// Result type for swapchain image acquisition.
type AcquireResult = Result<(u32, Arc<Image>, Box<dyn GpuFuture>), SwapchainError>;

/// Warn once per offending asset path, then stay silent. `prepare_mesh_data`
/// runs every frame — without the seen-set a single broken path would spam
/// the console at frame rate. (No logger is installed in `game_client`;
/// stderr is the visible channel for these main-thread warnings.)
fn warn_once_per_path(kind: &str, path: &str) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let mut seen = SEEN
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if seen.insert(format!("{kind}:{path}")) {
        eprintln!("render: {kind} '{path}' failed to resolve (skipping/using fallback); further warnings for this path suppressed");
    }
}

/// Walk the parent chain to resolve effective editor visibility. Returns
/// `true` unless `entity` or any ancestor carries `EditorVisibility { visible:
/// false }`. Visibility-component absence is treated as "visible" so the
/// flag is opt-in.
fn is_editor_visible(world: &World, entity: hecs::Entity) -> bool {
    let mut current = entity;
    loop {
        if let Ok(v) = world.get::<&EditorVisibility>(current) {
            if !v.visible {
                return false;
            }
        }
        match world.get::<&Parent>(current) {
            Ok(p) => current = p.0,
            Err(_) => return true,
        }
    }
}

/// One submesh draw awaiting batching (Task 41.5 P7): sort-key fields plus
/// the per-instance metadata. Records are sorted so equal batch keys are
/// adjacent with camera-visible instances first — a batch's camera draw is
/// then a prefix (`visible` instances) of its shadow draw's span (`all`
/// instances), over one contiguous run in the instance-metadata ring.
struct DrawRecord {
    material_index: usize,
    mesh_idx: usize,
    mat_ptr: usize,
    in_camera: bool,
    is_skinned: bool,
    /// Push order — sort tiebreak so `sort_unstable_by_key` stays
    /// deterministic (entity iteration order within a batch).
    seq: u32,
    model: [[f32; 4]; 4],
    palette_base: u32,
    mat_set: Option<Arc<DescriptorSet>>,
}

impl DrawRecord {
    /// Batch identity — one draw call per distinct key. `mesh_idx` names the
    /// GPU submesh (its vertex/index buffers); the material set's Arc pointer
    /// is stable per cached material, so it splits batches by material
    /// cheaply (same discriminator the geometry pass uses for rebinds).
    fn batch_key(&self) -> (usize, usize, usize) {
        (self.material_index, self.mesh_idx, self.mat_ptr)
    }
}

/// Batch keys adjacent, visible instances first within a batch, entity
/// iteration order (`seq`) breaking ties — a total order, so the unstable
/// sort is deterministic frame to frame and allocates no merge scratch
/// (the stable `sort_by_key` would, every frame, on the hot path).
fn sort_draw_records(records: &mut [DrawRecord]) {
    records.sort_unstable_by_key(|r| (r.material_index, r.mesh_idx, r.mat_ptr, !r.in_camera, r.seq));
}

thread_local! {
    /// Reusable phase-1 scratch — `prepare_mesh_data` runs on one thread per
    /// host (main thread, or `benchmark_runner`'s); a fresh Vec per frame
    /// would be a steady-state allocation on the hot path (review F1).
    static DRAW_RECORDS: std::cell::RefCell<Vec<DrawRecord>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Prepare mesh render data from ECS world into a reusable buffer.
///
/// Reads pre-computed transforms from `transform_cache` (populated by
/// `TransformCache::propagate` earlier in the frame).  No recursive
/// hierarchy traversal happens here.
///
/// Writes every visible skeleton's palette into the SSBO ring's region for
/// `frame_number` (one write per skeleton, shared by all its submesh draws),
/// batches submesh draws by (submesh, material) into instanced draws — one
/// `MeshRenderData` per batch, instances contiguous in the frame's
/// instance-metadata ring region — and returns the frame's ring regions for
/// the `FramePacket`. The caller's fence ring must release ring slots via
/// `skinning.sync()` (the render thread does; `benchmark_runner` does it
/// inline).
#[allow(clippy::too_many_arguments)]
pub fn prepare_mesh_data(
    world: &World,
    asset_manager: &Arc<AssetManager>,
    renderer: &Renderer,
    mesh_data_buffer: &mut Vec<MeshRenderData>,
    shadow_caster_buffer: &mut Vec<MeshRenderData>,
    transform_cache: &TransformCache,
    skinning: &mut SkinningBackend,
    frame_number: u64,
    default_material_set: &Arc<vulkano::descriptor_set::DescriptorSet>,
    material_cache: &std::collections::HashMap<String, Arc<vulkano::descriptor_set::DescriptorSet>>,
) -> SkinnedPaletteFrame {
    rust_engine::profile_scope!("prepare_mesh_data");

    // Task 41.5 P0 bench hooks — inert (one atomic load) unless --bench-secs
    // armed them in the standalone build.
    let bench = crate::bench::render_hooks_enabled();

    mesh_data_buffer.clear();
    shadow_caster_buffer.clear();

    let meshes = asset_manager.meshes.read();
    // Claim this frame's ring region (blocks until frame N-4 — the region's
    // previous occupant — is marked done via fence reclaim or guard).
    skinning.begin_frame(frame_number);

    let view_matrix = renderer.camera_3d.view_matrix();
    let projection_matrix = renderer.camera_3d.projection_matrix();
    let view_projection = projection_matrix * view_matrix;

    let camera_frustum = Frustum::from_view_projection(view_projection);

    // Phase 1: one DrawRecord per (entity, submesh). Palette writes happen
    // here, in entity iteration order — the ring's upload gate (R6) keys on
    // that write sequence and is untouched by batching.
    let mut records = DRAW_RECORDS.with(|r| std::mem::take(&mut *r.borrow_mut()));

    for (entity, (_transform, mesh_renderer, skeleton)) in world
        .query::<(&Transform, &MeshRenderer, Option<&SkeletonInstance>)>()
        .iter()
    {
        if !mesh_renderer.visible || mesh_renderer.mesh_path.is_empty() {
            continue;
        }
        // Editor visibility cascades — hiding a parent hides its descendants.
        // Walks up the Parent chain; cheap because depths are typically small,
        // and entities without an `EditorVisibility` component default to visible.
        if !is_editor_visible(world, entity) {
            continue;
        }

        let submesh_indices: &[usize] =
            if let Some(indices) = meshes.indices_for_path(&mesh_renderer.mesh_path) {
                indices
            } else {
                warn_once_per_path("mesh path", &mesh_renderer.mesh_path);
                continue;
            };

        let model_matrix = transform_cache.get_render(entity);
        let glam_model = glam::Mat4::from_cols_array_2d(&unsafe {
            std::mem::transmute::<nalgebra_glm::Mat4, [[f32; 4]; 4]>(model_matrix)
        });
        let model_array: [[f32; 4]; 4] = unsafe { std::mem::transmute(model_matrix) };

        // One ring write per skeleton per frame; every submesh draw of the
        // entity shares the returned palette_base. Base 0 = identity (static
        // meshes, or a failed write — mesh renders in bind pose). The entity
        // key + palette revision let the backend skip re-copying a palette
        // the region already holds (P4 upload gate for throttled skeletons).
        let is_skinned = skeleton.is_some_and(|s| !s.palette.is_empty());
        let palette_base = if let Some(skel) = skeleton.filter(|s| !s.palette.is_empty()) {
            // Task 41.5 bench hook (moved here from the old per-entity
            // descriptor upload): count = skeletons written this frame,
            // ms = time writing palettes into the ring.
            let t0 = bench.then(std::time::Instant::now);
            let base = match skinning.write_palette(entity.to_bits().get(), skel.revision, &skel.palette) {
                Ok(base) => base,
                Err(_) => {
                    warn_once_per_path("palette ring write for", &mesh_renderer.mesh_path);
                    0
                }
            };
            if let Some(t0) = t0 {
                crate::bench::palette_upload(t0.elapsed().as_nanos() as u64);
            }
            base
        } else {
            0
        };

        for (sub_i, &mesh_idx) in submesh_indices.iter().enumerate() {
            if let Some(gpu_mesh) = meshes.get(mesh_idx) {
                let local_aabb = Aabb::new(gpu_mesh.aabb_min, gpu_mesh.aabb_max);
                let world_aabb = local_aabb.transformed(&glam_model);
                let in_camera = camera_frustum.contains_aabb(world_aabb.min, world_aabb.max);

                // Resolve material descriptor set from cache, falling back to
                // default. An unset/empty path is normal; a set path missing
                // from the cache is a broken reference worth one warning.
                let mat_set = match mesh_renderer.material_paths.get(sub_i) {
                    Some(p) if !p.is_empty() => match material_cache.get(p) {
                        Some(set) => set.clone(),
                        None => {
                            warn_once_per_path("material path", p);
                            default_material_set.clone()
                        }
                    },
                    _ => default_material_set.clone(),
                };

                records.push(DrawRecord {
                    material_index: mesh_renderer.material_index,
                    mesh_idx,
                    mat_ptr: Arc::as_ptr(&mat_set) as usize,
                    in_camera,
                    is_skinned,
                    seq: records.len() as u32,
                    model: model_array,
                    palette_base,
                    mat_set: Some(mat_set),
                });
            }
        }
    }

    // Phase 2: batch (Task 41.5 P7). Sort so equal batch keys are adjacent
    // (camera-visible first), write instance metadata in draw order —
    // contiguous per batch — and emit one shadow entry per batch (all
    // instances: the shadow pass is not camera-frustum culled, an off-screen
    // instance can still cast into the visible region) plus one camera entry
    // over the visible prefix when it is non-empty.
    sort_draw_records(&mut records);
    let mut skinned_draws = 0u32;
    let mut skinned_instances = 0u32;
    for group in records.chunk_by(|a, b| a.batch_key() == b.batch_key()) {
        let Some(gpu_mesh) = meshes.get(group[0].mesh_idx) else {
            continue;
        };
        let mut first_instance = 0u32;
        let mut written = 0u32;
        let mut visible = 0u32;
        let mut any_skinned = false;
        for rec in group {
            match skinning.write_instance(rec.model, rec.palette_base) {
                Ok(idx) => {
                    if written == 0 {
                        first_instance = idx;
                    }
                    written += 1;
                    visible += rec.in_camera as u32;
                    any_skinned |= rec.is_skinned;
                }
                Err(_) => {
                    // Host-write failure (effectively never). Truncate the
                    // batch: the written records are a prefix of the group,
                    // so the visible-first accounting stays consistent.
                    warn_once_per_path("instance ring write for submesh", &group[0].mesh_idx.to_string());
                    break;
                }
            }
        }
        if written == 0 {
            continue;
        }
        let batch = MeshRenderData {
            vertex_buffer: gpu_mesh.vertex_buffer.clone(),
            index_buffer: gpu_mesh.index_buffer.clone(),
            index_count: gpu_mesh.index_count,
            mesh_index: group[0].mesh_idx,
            material_index: group[0].material_index,
            first_instance,
            instance_count: written,
            material_descriptor_set: group[0].mat_set.clone(),
        };
        if visible > 0 {
            mesh_data_buffer.push(MeshRenderData {
                instance_count: visible,
                ..batch.clone()
            });
        }
        shadow_caster_buffer.push(batch);
        if any_skinned {
            // Draw calls this batch submits (shadow + camera-if-visible) and
            // the instances they cover — the bench's collapse metrics.
            skinned_draws += 1 + (visible > 0) as u32;
            skinned_instances += written + visible;
        }
    }

    if bench {
        crate::bench::add_skinned_draws(skinned_draws);
        crate::bench::add_skinned_instances(skinned_instances);
    }

    // Return the scratch (cleared so material-set Arcs drop now, capacity kept).
    records.clear();
    DRAW_RECORDS.with(|r| *r.borrow_mut() = records);

    skinning.end_frame()
}

fn compute_light_vp(light_dir_render: glm::Vec3) -> glam::Mat4 {
    let dir =
        glam::Vec3::new(light_dir_render.x, light_dir_render.y, light_dir_render.z).normalize();
    let distance = 100.0;
    let half_size = 50.0;
    let light_pos = glam::Vec3::ZERO - dir * distance;

    let up = if dir.y.abs() > 0.99 {
        glam::Vec3::X
    } else {
        glam::Vec3::Y
    };

    let view = glam::Mat4::look_at_rh(light_pos, glam::Vec3::ZERO, up);
    let proj =
        glam::Mat4::orthographic_rh(-half_size, half_size, -half_size, half_size, 0.1, 200.0);
    proj * view
}

/// Prepare light uniform data from ECS world
pub fn prepare_light_data(world: &World, renderer: &Renderer) -> LightUniformData {
    rust_engine::profile_scope!("prepare_light_data");

    let identity: [[f32; 4]; 4] = glam::Mat4::IDENTITY.to_cols_array_2d();
    let camera_pos = renderer.camera_3d.position;
    let mut light_data = LightUniformData {
        camera_position: [camera_pos.x, camera_pos.y, camera_pos.z],
        shadow_bias: 0.005,
        directional_light_dir: [0.0, -1.0, -1.0],
        shadow_enabled: 0.0,
        directional_light_color: [1.0, 1.0, 1.0],
        directional_light_intensity: 1.0,
        ambient_color: [0.1, 0.1, 0.15],
        ambient_intensity: 0.3,
        light_vp: identity,
    };

    if let Some((_entity, dir_light)) = world.query::<&EcsDirectionalLight>().iter().next() {
        let direction = glm::normalize(&render_adapter::direction_to_render(&dir_light.direction));
        light_data.directional_light_dir = [direction.x, direction.y, direction.z];
        light_data.directional_light_color =
            [dir_light.color.x, dir_light.color.y, dir_light.color.z];
        light_data.directional_light_intensity = dir_light.intensity;
        light_data.shadow_bias = dir_light.shadow_bias;
        light_data.shadow_enabled = if dir_light.shadow_enabled { 1.0 } else { 0.0 };
        light_data.light_vp = compute_light_vp(direction).to_cols_array_2d();
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
        renderer.gpu.device.clone(),
        renderer.swapchain_state.surface.clone(),
        renderer.swapchain_state.swapchain.clone(),
    ) {
        Ok((new_swapchain, new_images)) => {
            // Check if window is minimized
            if new_images.is_empty() {
                renderer.swapchain_state.recreate_swapchain = false;
                return Ok(false);
            }

            renderer.swapchain_state.swapchain = new_swapchain;
            renderer.swapchain_state.images = new_images.clone();

            // NOTE: Do NOT update camera aspect ratio here!
            // The camera should use VIEWPORT PANEL dimensions, not window dimensions.
            // Camera aspect ratio is updated in app.rs when viewport_size changes.

            // Clear the deferred renderer's framebuffer cache (output framebuffers changed)
            // NOTE: We do NOT recreate the DeferredRenderer here because:
            // - The G-Buffer should match the VIEWPORT size, not the window size
            // - The viewport resize logic in app.rs handles G-Buffer resizing
            // - Recreating at window size caused stretching after minimize/restore
            deferred_renderer.clear_framebuffer_cache();

            renderer.swapchain_state.recreate_swapchain = false;
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

    match acquire_next_image(renderer.swapchain_state.swapchain.clone(), None) {
        Ok((image_index, suboptimal, acquire_future)) => {
            if suboptimal {
                renderer.swapchain_state.recreate_swapchain = true;
            }
            let target_image = renderer.swapchain_state.images[image_index as usize].clone();
            Ok((image_index, target_image, acquire_future.boxed()))
        }
        Err(e) => match e {
            Validated::Error(VulkanError::OutOfDate) => {
                renderer.swapchain_state.recreate_swapchain = true;
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
    sync::now(renderer.gpu.device.clone()).boxed()
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
    use rust_engine::engine::debug_draw::DebugDrawData;

    rust_engine::profile_scope!("prepare_debug_draw_data");

    let (depth_lines, overlay_lines) = debug_draw_buffer.drain();

    let (depth_buffer, depth_vertex_count) = upload_debug_lines(&depth_lines, renderer)
        .map(|(b, c)| (Some(b), c))
        .unwrap_or((None, 0));
    let (overlay_buffer, overlay_vertex_count) = upload_debug_lines(&overlay_lines, renderer)
        .map(|(b, c)| (Some(b), c))
        .unwrap_or((None, 0));

    DebugDrawData {
        depth_buffer,
        depth_vertex_count,
        overlay_buffer,
        overlay_vertex_count,
        static_depth_buffer: None,
        static_depth_vertex_count: 0,
    }
}

/// Convert Z-up debug lines to Y-up render space and upload a GPU vertex
/// buffer. Callers may hold the returned buffer across frames (used to cache
/// static line sets like collision wireframes).
#[cfg(debug_assertions)]
pub fn upload_debug_lines(
    lines: &[rust_engine::engine::debug_draw::DebugLineData],
    renderer: &Renderer,
) -> Option<(
    vulkano::buffer::Subbuffer<[rust_engine::engine::debug_draw::DebugLineVertex]>,
    u32,
)> {
    use rust_engine::engine::debug_draw::DebugLineVertex;
    use rust_engine::engine::utils::coords::convert_position_zup_to_yup;
    use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
    use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter};

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
        renderer.gpu.memory_allocator.clone(),
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
}

/// Prepare plankton emitter frame data from ECS world.
///
/// Extracts enabled emitters with EntityGuids, converts transforms and
/// force vectors from Z-up game space to Y-up render space.
pub fn prepare_plankton_data(
    world: &World,
    frame_buffer: &mut Vec<PlanktonEmitterFrameData>,
    transform_cache: &TransformCache,
    delta_time: f32,
) {
    rust_engine::profile_scope!("prepare_plankton_data");
    frame_buffer.clear();

    for (entity, (effect, guid)) in world.query::<(&ParticleEffect, &EntityGuid)>().iter() {
        if !effect.enabled {
            continue;
        }

        let world_matrix_zup = transform_cache.get_world(entity);
        let world_matrix_yup = render_adapter::world_matrix_to_render(&world_matrix_zup);
        let model_array: [[f32; 4]; 4] =
            unsafe { std::mem::transmute::<nalgebra_glm::Mat4, [[f32; 4]; 4]>(world_matrix_yup) };

        // Extract module values with defaults
        let gravity_raw = effect.gravity().unwrap_or([0.0, 0.0, 0.0]);
        let wind_raw = effect.wind().unwrap_or([0.0, 0.0, 0.0]);
        let drag_val = effect.drag().unwrap_or(0.0);
        let (turb_strength, turb_scale, turb_speed) =
            effect.curl_noise().unwrap_or((0.0, 1.0, 0.0));
        let (color_start, color_end) = effect
            .color_over_life()
            .unwrap_or(([1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 0.0]));
        let (size_start, size_end) = effect.size_over_life().unwrap_or((0.1, 0.0));

        // Convert Z-up force vectors to Y-up render space
        let gravity_yup = render_adapter::direction_to_render(&glm::vec3(
            gravity_raw[0],
            gravity_raw[1],
            gravity_raw[2],
        ));
        let wind_yup =
            render_adapter::direction_to_render(&glm::vec3(wind_raw[0], wind_raw[1], wind_raw[2]));
        let vel_yup = render_adapter::direction_to_render(&glm::vec3(
            effect.initial_velocity[0],
            effect.initial_velocity[1],
            effect.initial_velocity[2],
        ));

        let (shape_type, shape_params) = match effect.spawn_shape {
            SpawnShape::Point => (0u32, [0.0f32; 4]),
            SpawnShape::Sphere { radius } => (1, [radius, 0.0, 0.0, 0.0]),
            SpawnShape::Cone { angle_rad, radius } => (2, [angle_rad, radius, 0.0, 0.0]),
            SpawnShape::Box { half_extents } => {
                (3, [half_extents[0], half_extents[1], half_extents[2], 0.0])
            }
        };

        frame_buffer.push(PlanktonEmitterFrameData {
            entity_guid: guid.0,
            world_transform: model_array,
            emission: EmissionParameters {
                shape_type,
                shape_params,
                emission_rate: effect.emission_rate,
                burst_count: effect.burst_count,
                burst_interval: effect.burst_interval,
                velocity_base: [vel_yup.x, vel_yup.y, vel_yup.z],
                velocity_variance: effect.velocity_variance,
                lifetime_min: effect.lifetime_min,
                lifetime_max: effect.lifetime_max,
            },
            forces: ForceParameters {
                gravity: [gravity_yup.x, gravity_yup.y, gravity_yup.z],
                drag: drag_val,
                wind: [wind_yup.x, wind_yup.y, wind_yup.z],
                turbulence_strength: turb_strength,
                turbulence_scale: turb_scale,
                turbulence_speed: turb_speed,
            },
            visual: VisualParameters {
                size_start,
                size_end,
                color_start,
                color_end,
                texture_path: effect.texture_path.clone(),
                soft_fade_distance: effect.soft_fade_distance,
            },
            flags: EmitterFlags {
                blend_mode: 0, // Additive
            },
            delta_time,
            capacity: effect.capacity,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(material_index: usize, mesh_idx: usize, mat_ptr: usize, in_camera: bool) -> DrawRecord {
        DrawRecord {
            material_index,
            mesh_idx,
            mat_ptr,
            in_camera,
            is_skinned: false,
            seq: 0,
            model: [[0.0; 4]; 4],
            palette_base: 0,
            mat_set: None,
        }
    }

    /// Batching invariants (P7): equal batch keys adjacent after the sort,
    /// camera-visible records form each batch's prefix (the camera draw's
    /// instance count is a prefix of the shadow draw's span).
    #[test]
    fn draw_records_group_with_visible_prefix() {
        let mut records = vec![
            rec(0, 2, 10, false),
            rec(0, 1, 10, true),
            rec(0, 2, 10, true),
            rec(1, 2, 11, true),
            rec(0, 2, 10, true),
            rec(0, 1, 10, false),
        ];
        for (i, r) in records.iter_mut().enumerate() {
            r.seq = i as u32;
        }
        sort_draw_records(&mut records);
        let groups: Vec<(usize, usize, usize, usize)> = records
            .chunk_by(|a, b| a.batch_key() == b.batch_key())
            .map(|g| {
                let visible = g.iter().filter(|r| r.in_camera).count();
                assert!(
                    g.iter().take(visible).all(|r| r.in_camera)
                        && g.iter().skip(visible).all(|r| !r.in_camera),
                    "camera-visible records must be the batch prefix"
                );
                let (mat, mesh, _) = g[0].batch_key();
                (mat, mesh, g.len(), visible)
            })
            .collect();
        assert_eq!(groups, vec![(0, 1, 2, 1), (0, 2, 3, 2), (1, 2, 1, 1)]);
    }
}

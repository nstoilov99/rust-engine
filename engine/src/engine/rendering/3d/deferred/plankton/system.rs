use super::emit_pass::{EmitPushConstants, PlanktonEmitPass};
use super::init_pass::PlanktonInitPass;
use super::pool::PlanktonPool;
use super::render_pass::{PlanktonRenderPass, PlanktonRenderPushConstants};
use super::simulate_pass::{PlanktonSimulatePass, SimulatePushConstants};
use crate::engine::rendering::frame_packet::PlanktonEmitterFrameData;
use std::collections::HashMap;
use std::sync::Arc;
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::command_buffer::AutoCommandBufferBuilder;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::device::{Device, Queue};
use vulkano::image::view::ImageView;
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass};

/// Maximum frames an emitter can be absent before its pool is evicted.
const ABSENCE_EVICTION_THRESHOLD: u32 = 60;

pub struct PlanktonSystem {
    #[allow(dead_code)]
    device: Arc<Device>,
    allocator: Arc<StandardMemoryAllocator>,
    descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    init_pass: PlanktonInitPass,
    emit_pass: PlanktonEmitPass,
    simulate_pass: PlanktonSimulatePass,
    render_pass: PlanktonRenderPass,
    pools: HashMap<uuid::Uuid, PlanktonPool>,
    absence_counter: HashMap<uuid::Uuid, u32>,
    /// Accumulated fractional emissions per emitter (for sub-frame accumulation).
    emission_accumulators: HashMap<uuid::Uuid, f32>,
    /// Frame counter used as part of the random seed.
    frame_counter: u32,
    /// Elapsed time since system creation (for turbulence animation).
    elapsed_time: f32,
    /// Cached framebuffer for the plankton HDR render pass.
    hdr_framebuffer: Option<Arc<Framebuffer>>,
    /// Per-frame render data collected during update_frame for later draw.
    pending_draws: Vec<(uuid::Uuid, u32, PlanktonRenderPushConstants)>,
}

impl PlanktonSystem {
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        allocator: Arc<StandardMemoryAllocator>,
        command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
        descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let init_pass = PlanktonInitPass::new(device.clone())?;
        let emit_pass = PlanktonEmitPass::new(device.clone())?;
        let mut simulate_pass = PlanktonSimulatePass::new(device.clone(), allocator.clone())?;
        let render_pass = PlanktonRenderPass::new(
            device.clone(),
            allocator.clone(),
            command_buffer_allocator.clone(),
            queue.clone(),
        )?;

        // Generate the 3D curl-noise texture and wire it into the simulate pass
        let noise_texture = super::noise::generate_curl_noise_texture(
            allocator.clone(),
            command_buffer_allocator,
            queue,
        )?;
        simulate_pass.set_noise_texture(noise_texture);

        Ok(Self {
            device,
            allocator,
            descriptor_set_allocator,
            init_pass,
            emit_pass,
            simulate_pass,
            render_pass,
            pools: HashMap::new(),
            absence_counter: HashMap::new(),
            emission_accumulators: HashMap::new(),
            frame_counter: 0,
            elapsed_time: 0.0,
            hdr_framebuffer: None,
            pending_draws: Vec::new(),
        })
    }

    /// Get the plankton HDR render pass (for framebuffer compatibility).
    pub fn hdr_render_pass(&self) -> Arc<RenderPass> {
        self.render_pass.render_pass()
    }

    /// Update the gbuffer depth reference (call on init and resize).
    pub fn set_gbuffer_depth(
        &mut self,
        depth_view: Arc<ImageView>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.render_pass.set_gbuffer_depth(
            self.descriptor_set_allocator.clone(),
            depth_view,
        )
    }

    /// Update the HDR framebuffer (call on init and resize).
    pub fn set_hdr_target(
        &mut self,
        hdr_target: Arc<ImageView>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let fb = Framebuffer::new(
            self.render_pass.render_pass(),
            FramebufferCreateInfo {
                attachments: vec![hdr_target],
                ..Default::default()
            },
        )?;
        self.hdr_framebuffer = Some(fb);
        Ok(())
    }

    /// Returns true if there are particles to draw this frame.
    pub fn has_pending_draws(&self) -> bool {
        !self.pending_draws.is_empty()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_frame<L>(
        &mut self,
        builder: &mut AutoCommandBufferBuilder<L>,
        emitters: &[PlanktonEmitterFrameData],
        view_proj: &[[f32; 4]; 4],
        camera_right: [f32; 3],
        camera_up: [f32; 3],
        camera_near: f32,
        camera_far: f32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Fast path: no emitters this frame and no pools to evict or drain.
        // Skips profile scope, counter bumps, and the HashMap iteration below.
        if emitters.is_empty() && self.pools.is_empty() {
            self.pending_draws.clear();
            return Ok(());
        }

        crate::profile_scope!("plankton_update_frame");

        self.frame_counter = self.frame_counter.wrapping_add(1);
        self.pending_draws.clear();

        // Track which emitters are active this frame
        let mut active_guids: Vec<uuid::Uuid> = Vec::with_capacity(emitters.len());

        for emitter in emitters {
            let guid = emitter.entity_guid;
            let dt = emitter.delta_time;
            active_guids.push(guid);

            // Update elapsed time from the first emitter's dt
            if active_guids.len() == 1 {
                self.elapsed_time += dt;
            }

            // Create pool if it doesn't exist
            if !self.pools.contains_key(&guid) {
                let capacity = emitter.capacity;
                let pool = PlanktonPool::new(self.allocator.clone(), capacity)?;
                self.init_pass.prepare_pool(
                    self.descriptor_set_allocator.clone(),
                    guid,
                    &pool,
                )?;
                self.emit_pass.prepare_pool(
                    self.descriptor_set_allocator.clone(),
                    guid,
                    &pool,
                )?;
                self.simulate_pass.prepare_pool(
                    self.descriptor_set_allocator.clone(),
                    guid,
                    &pool,
                )?;
                self.render_pass.prepare_pool(
                    self.descriptor_set_allocator.clone(),
                    guid,
                    &pool,
                )?;
                self.pools.insert(guid, pool);
            }

            // Run init pass if not yet initialized
            let pool = self.pools.get_mut(&guid).expect("pool just inserted");
            if !pool.initialized {
                self.init_pass.dispatch(builder, guid, pool.capacity)?;
                pool.initialized = true;
            }

            // Compute emit count from accumulator
            let accumulator = self.emission_accumulators.entry(guid).or_insert(0.0);
            *accumulator += emitter.emission.emission_rate * dt;
            let emit_count = (*accumulator as u32).min(pool.capacity);
            *accumulator -= emit_count as f32;

            // Dispatch emit pass
            if emit_count > 0 {
                let push = EmitPushConstants {
                    emitter_transform: emitter.world_transform,
                    velocity_base_variance: [
                        emitter.emission.velocity_base[0],
                        emitter.emission.velocity_base[1],
                        emitter.emission.velocity_base[2],
                        emitter.emission.velocity_variance,
                    ],
                    color_start: emitter.visual.color_start,
                    color_end: emitter.visual.color_end,
                    size_lifetime: [
                        emitter.visual.size_start,
                        emitter.visual.size_end,
                        emitter.emission.lifetime_min,
                        emitter.emission.lifetime_max,
                    ],
                    shape_params: emitter.emission.shape_params,
                    shape_type: emitter.emission.shape_type,
                    random_seed: self.frame_counter.wrapping_mul(2654435761),
                    dt,
                    emit_count,
                };

                self.emit_pass.dispatch(builder, guid, push, emit_count)?;
            }

            // Dispatch simulate pass (always runs for all live particles)
            let capacity = pool.capacity;
            let sim_push = SimulatePushConstants {
                gravity_and_drag: [
                    emitter.forces.gravity[0],
                    emitter.forces.gravity[1],
                    emitter.forces.gravity[2],
                    emitter.forces.drag,
                ],
                wind_and_turb_strength: [
                    emitter.forces.wind[0],
                    emitter.forces.wind[1],
                    emitter.forces.wind[2],
                    emitter.forces.turbulence_strength,
                ],
                turb_scale_speed_pad: [
                    emitter.forces.turbulence_scale,
                    emitter.forces.turbulence_speed,
                    0.0,
                    0.0,
                ],
                color_start: emitter.visual.color_start,
                color_end: emitter.visual.color_end,
                size_start: emitter.visual.size_start,
                size_end: emitter.visual.size_end,
                delta_time: dt,
                time: self.elapsed_time,
                capacity,
                _sim_pad0: 0,
                _sim_pad1: 0,
                _sim_pad2: 0,
            };

            self.simulate_pass.dispatch(builder, guid, sim_push)?;

            // Queue render draw for the graph execute phase
            let render_push = PlanktonRenderPushConstants {
                view_projection: *view_proj,
                camera_right: [camera_right[0], camera_right[1], camera_right[2], 0.0],
                camera_up: [
                    camera_up[0],
                    camera_up[1],
                    camera_up[2],
                    emitter.visual.soft_fade_distance,
                ],
                camera_near_far_pad: [camera_near, camera_far, 0.0, 0.0],
            };
            self.pending_draws.push((guid, capacity, render_push));

            // Reset absence counter for active emitters
            self.absence_counter.remove(&guid);
        }

        // Increment absence counters for inactive pools and collect evictions
        let mut to_evict: Vec<uuid::Uuid> = Vec::new();
        for guid in self.pools.keys() {
            if !active_guids.contains(guid) {
                let counter = self.absence_counter.entry(*guid).or_insert(0);
                *counter += 1;
                if *counter >= ABSENCE_EVICTION_THRESHOLD {
                    to_evict.push(*guid);
                }
            }
        }

        // Evict stale pools
        for guid in &to_evict {
            self.pools.remove(guid);
            self.absence_counter.remove(guid);
            self.emission_accumulators.remove(guid);
            self.init_pass.remove_pool(guid);
            self.emit_pass.remove_pool(guid);
            self.simulate_pass.remove_pool(guid);
            self.render_pass.remove_pool(guid);
        }

        Ok(())
    }

    /// Execute the plankton billboard render pass into the HDR target.
    /// Called from the render graph execute phase.
    pub fn render_particles<L>(
        &self,
        builder: &mut AutoCommandBufferBuilder<L>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use vulkano::command_buffer::{RenderPassBeginInfo, SubpassBeginInfo, SubpassContents, SubpassEndInfo};
        use vulkano::pipeline::graphics::viewport::{Scissor, Viewport};

        let framebuffer = self
            .hdr_framebuffer
            .as_ref()
            .ok_or("plankton: HDR framebuffer not set")?;

        let extent = framebuffer.extent();
        let viewport = Viewport {
            offset: [0.0, 0.0],
            extent: [extent[0] as f32, extent[1] as f32],
            depth_range: 0.0..=1.0,
        };
        let scissor = Scissor {
            offset: [0, 0],
            extent: [extent[0], extent[1]],
        };

        builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: vec![None], // Load, not clear
                    ..RenderPassBeginInfo::framebuffer(framebuffer.clone())
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )?
            .set_viewport(0, smallvec::smallvec![viewport])?
            .set_scissor(0, smallvec::smallvec![scissor])?;

        self.render_pass.draw(builder, &self.pending_draws)?;

        builder.end_render_pass(SubpassEndInfo::default())?;

        Ok(())
    }
}

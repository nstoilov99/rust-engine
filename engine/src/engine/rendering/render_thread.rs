//! Dedicated render thread with crossbeam channels.
//!
//! The render thread receives `FramePacket`s from the main thread via
//! a bounded(1) channel, and sends `RenderEvent`s back via a bounded(4)
//! response channel. The thread creates its own `DeferredRenderer` during
//! initialization to validate GPU object construction off the main thread.

use crate::engine::rendering::common::gpu_context::GpuContext;
use crate::engine::rendering::frame_packet::{FramePacket, RenderEvent, RenderMode};
use crate::engine::rendering::render_target::RenderTarget;
use crate::engine::rendering::rendering_3d::DeferredRenderer;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use vulkano::image::Image;
use vulkano::swapchain::{acquire_next_image, Surface, Swapchain, SwapchainPresentInfo};
use vulkano::sync::GpuFuture;
use vulkano::{Validated, VulkanError};

/// Swapchain data transferred from the main thread to the render thread.
pub struct SwapchainTransfer {
    pub surface: Arc<Surface>,
    pub swapchain: Arc<Swapchain>,
    pub images: Vec<Arc<Image>>,
}

/// Configuration for spawning the render thread.
pub struct RenderThreadConfig {
    pub gpu_context: Arc<GpuContext>,
    pub render_mode: RenderMode,
    pub initial_dimensions: [u32; 2],
    pub swapchain_transfer: Option<SwapchainTransfer>,
    #[cfg(feature = "editor")]
    pub viewport_dimensions: Option<[u32; 2]>,
    /// Glyph shaper/atlas shared with the main thread's `CrustyGui`
    /// (Phase 16). `None` disables the crusty render pass.
    #[cfg(feature = "editor")]
    pub crusty_text: Option<crate::engine::gui::crusty::SharedTextRenderer>,
}

/// Handle to the render thread, held by the main thread.
pub struct RenderThread {
    sender: Sender<FramePacket>,
    response_receiver: Receiver<RenderEvent>,
    join_handle: Option<JoinHandle<()>>,
}

impl RenderThread {
    /// Spawn the render thread with the given configuration.
    pub fn spawn(config: RenderThreadConfig) -> Self {
        // Depth of 2 matches the 2-slot GPU fence ring — lets the main thread
        // prepare frame N+1 while the render thread is still processing frame N,
        // so CPU work on both threads pipelines cleanly. Depth of 1 forces them
        // to alternate and adds a full ping-pong of sync overhead per frame.
        let (packet_tx, packet_rx) = bounded::<FramePacket>(2);
        let (response_tx, response_rx) = bounded::<RenderEvent>(4);

        let join_handle = thread::Builder::new()
            .name("renderer".into())
            .spawn(move || {
                Self::thread_entry(config, packet_rx, response_tx);
            })
            .expect("failed to spawn render thread");

        Self {
            sender: packet_tx,
            response_receiver: response_rx,
            join_handle: Some(join_handle),
        }
    }

    fn thread_entry(
        config: RenderThreadConfig,
        receiver: Receiver<FramePacket>,
        response: Sender<RenderEvent>,
    ) {
        let gpu = &config.gpu_context;
        let [w, h] = config.initial_dimensions;

        let mut deferred_renderer = match DeferredRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            gpu.memory_allocator.clone(),
            gpu.command_buffer_allocator.clone(),
            gpu.descriptor_set_allocator.clone(),
            w.max(1),
            h.max(1),
        ) {
            Ok(dr) => {
                log::info!("render_thread: DeferredRenderer created successfully");
                dr
            }
            Err(e) => {
                log::error!("render_thread: failed to create DeferredRenderer: {}", e);
                let _ = response.send(RenderEvent::RenderError {
                    message: format!("DeferredRenderer creation failed: {}", e),
                });
                return;
            }
        };

        let has_swapchain = config.swapchain_transfer.is_some();
        let (mut sc_swapchain, mut sc_images, surface) = if let Some(st) = config.swapchain_transfer
        {
            (Some(st.swapchain), Some(st.images), Some(st.surface))
        } else {
            (None, None, None)
        };
        // 3-slot fence ring. Why 3 and not 2: Vulkano's `FenceSignalFuture::Drop`
        // blocks on `fence.wait(None)` if the fence isn't signaled yet. On Windows
        // with DWM, the present fence only signals after the image has been through
        // the compositor (~2–3 vblanks delayed), so with a 2-slot ring we'd drop
        // each fence ~12ms after submission and still block for a few ms waiting on
        // it. 3 slots gives us ~18ms of head-start — fence is always signaled by
        // the time we drop it, so Drop is an instant no-op.
        let mut fence_slots: [Option<Box<dyn GpuFuture>>; 3] = [None, None, None];
        let mut frame_count: u64 = 0;
        let mut needs_recreate = false;

        #[cfg(feature = "editor")]
        let mut viewport_texture = {
            let vp_dims = config.viewport_dimensions.unwrap_or([800, 600]);
            match crate::engine::editor::ViewportTexture::new(
                gpu.device.clone(),
                gpu.memory_allocator.clone(),
                vp_dims[0].max(1),
                vp_dims[1].max(1),
            ) {
                Ok(vt) => {
                    log::info!("render_thread: viewport texture created ({}x{})", vp_dims[0], vp_dims[1]);
                    if let Err(e) =
                        vt.clear(gpu.queue.clone(), gpu.command_buffer_allocator.clone())
                    {
                        log::warn!("render_thread: viewport texture clear failed: {}", e);
                    }
                    Some(vt)
                }
                Err(e) => {
                    log::warn!("render_thread: viewport texture creation failed: {}", e);
                    None
                }
            }
        };

        #[cfg(feature = "editor")]
        let sc_format = sc_swapchain
            .as_ref()
            .map(|sc| sc.image_format())
            .unwrap_or(vulkano::format::Format::B8G8R8A8_SRGB);

        #[cfg(feature = "editor")]
        let mut egui_renderer = {
            match crate::engine::gui::EguiRenderer::new(
                gpu.device.clone(),
                gpu.queue.clone(),
                sc_format,
            ) {
                Ok(r) => {
                    log::info!("render_thread: EguiRenderer created successfully");
                    Some(r)
                }
                Err(e) => {
                    log::warn!("render_thread: EguiRenderer creation failed: {}", e);
                    None
                }
            }
        };

        #[cfg(feature = "editor")]
        let mut crusty_renderer = config.crusty_text.clone().map(|text| {
            log::info!("render_thread: CrustyRenderer created");
            crate::engine::gui::crusty::CrustyRenderer::new(
                gpu.device.clone(),
                gpu.queue.clone(),
                sc_format,
                text,
            )
        });

        #[cfg(feature = "editor")]
        let crusty_icons = crusty_renderer
            .as_mut()
            .map(|r| {
                r.load_hierarchy_icons(
                    gpu.queue.clone(),
                    gpu.memory_allocator.clone(),
                    gpu.command_buffer_allocator.clone(),
                )
            })
            .unwrap_or_default();

        // Crusty draws the viewport image through its own texture registry;
        // register once, re-point the id on every resize below.
        #[cfg(feature = "editor")]
        let crusty_viewport_texture = crusty_renderer.as_mut().and_then(|r| {
            viewport_texture
                .as_ref()
                .map(|vt| r.register_native_texture(vt.image_view()))
        });

        let ready_event = RenderEvent::RenderThreadReady {
            #[cfg(feature = "editor")]
            viewport_texture: viewport_texture.as_ref().map(|vt| vt.image_view()),
            #[cfg(feature = "editor")]
            crusty_icons,
            #[cfg(feature = "editor")]
            crusty_viewport_texture,
        };
        if response.send(ready_event).is_err() {
            return;
        }

        loop {
            // Do NOT scope the blocking recv() — the idle wait would show up as work time
            // in the profiler (e.g. a scope stretching from 0ms through the whole frame).
            let packet = match receiver.recv() {
                Ok(p) => p,
                Err(_) => {
                    log::info!("render_thread: channel disconnected, shutting down");
                    break;
                }
            };
            crate::profile_scope!("render_frame");

            log::trace!("render_thread: received frame {}", packet.frame_number);

            // Upload requested crusty textures (thumbnails) before any early
            // `continue` below can drop the packet — the main-thread cache
            // marks them in-flight and waits for the registered event.
            #[cfg(feature = "editor")]
            if let Some(ref mut cr) = crusty_renderer {
                if !packet.crusty_texture_uploads.is_empty() {
                    let mut registered = Vec::new();
                    for up in &packet.crusty_texture_uploads {
                        match cr.upload_rgba_texture(
                            gpu.queue.clone(),
                            gpu.memory_allocator.clone(),
                            gpu.command_buffer_allocator.clone(),
                            &up.rgba,
                            up.width,
                            up.height,
                        ) {
                            Ok(id) => registered.push((up.id, id)),
                            Err(e) => {
                                log::warn!("render_thread: crusty texture upload failed: {e}")
                            }
                        }
                    }
                    if !registered.is_empty() {
                        let _ = response.send(RenderEvent::CrustyTexturesRegistered(registered));
                    }
                }
                if !packet.crusty_native_registrations.is_empty() {
                    let registered: Vec<_> = packet
                        .crusty_native_registrations
                        .iter()
                        .map(|(key, view)| (key.clone(), cr.register_native_texture(view.clone())))
                        .collect();
                    let _ = response.send(RenderEvent::CrustyNativeRegistered(registered));
                }
                for (id, view) in &packet.crusty_native_updates {
                    cr.update_native_texture(*id, view.clone());
                }
                for id in &packet.crusty_native_removals {
                    cr.remove_native_texture(*id);
                }
            }

            if !has_swapchain {
                continue;
            }

            let swapchain_ref = sc_swapchain.as_ref().unwrap();
            let images_ref = sc_images.as_ref().unwrap();
            let surface_ref = surface.as_ref().unwrap();

            // Wait on the fence slot for this frame (3 slots, round-robin).
            let slot = (frame_count % 3) as usize;
            if let Some(mut prev) = fence_slots[slot].take() {
                prev.cleanup_finished();
            }

            // Handle swapchain recreation
            if needs_recreate {
                match crate::engine::core::swapchain::recreate_swapchain(
                    gpu.device.clone(),
                    surface_ref.clone(),
                    swapchain_ref.clone(),
                ) {
                    Ok((new_sc, new_imgs)) => {
                        if new_imgs.is_empty() {
                            needs_recreate = false;
                            continue;
                        }
                        let dims = new_sc.image_extent();
                        sc_swapchain = Some(new_sc);
                        sc_images = Some(new_imgs);
                        deferred_renderer.clear_framebuffer_cache();
                        #[cfg(feature = "editor")]
                        if let Some(ref mut cr) = crusty_renderer {
                            cr.clear_framebuffer_cache();
                        }
                        needs_recreate = false;

                        if dims[0] > 0 && dims[1] > 0 {
                            if let Err(e) = deferred_renderer.resize(dims[0], dims[1]) {
                                log::error!("render_thread: resize failed: {}", e);
                            }
                        }

                        let _ = response.send(RenderEvent::SwapchainRecreated {
                            dimensions: dims,
                        });
                    }
                    Err(e) => {
                        log::error!("render_thread: swapchain recreation failed: {}", e);
                        needs_recreate = false;
                        continue;
                    }
                }
                continue;
            }

            // Acquire swapchain image
            let (image_index, target_image, acquire_future) = {
                crate::profile_scope!("acquire_image");
                match acquire_next_image(swapchain_ref.clone(), None) {
                    Ok((idx, suboptimal, future)) => {
                        if suboptimal {
                            needs_recreate = true;
                        }
                        (idx, images_ref[idx as usize].clone(), future)
                    }
                    Err(e) => match e {
                        Validated::Error(VulkanError::OutOfDate) => {
                            needs_recreate = true;
                            continue;
                        }
                        _ => {
                            log::error!("render_thread: acquire failed: {:?}", e);
                            continue;
                        }
                    },
                }
            };

            // Handle viewport texture resize (editor only)
            #[cfg(feature = "editor")]
            if let (Some(ref mut vt), Some(vp_dims)) =
                (&mut viewport_texture, packet.viewport_dimensions)
            {
                let [vp_w, vp_h] = vp_dims;
                if vp_w > 0 && vp_h > 0 && (vp_w != vt.width() || vp_h != vt.height()) {
                    match vt.resize(vp_w, vp_h) {
                        Ok(true) => {
                            log::info!(
                                "render_thread: viewport texture resized to {}x{}",
                                vp_w,
                                vp_h
                            );
                            if let Err(e) =
                                vt.clear(gpu.queue.clone(), gpu.command_buffer_allocator.clone())
                            {
                                log::warn!("render_thread: viewport texture clear failed: {}", e);
                            }
                            if let Err(e) = deferred_renderer.resize(vp_w, vp_h) {
                                log::error!("render_thread: deferred resize failed: {}", e);
                            }
                            if let Some(ref mut egui_r) = egui_renderer {
                                if let Some(tex_id) = packet.viewport_texture_id {
                                    egui_r.update_native_texture(tex_id, vt.image_view());
                                }
                            }
                            #[cfg(feature = "editor")]
                            if let (Some(cr), Some(id)) =
                                (crusty_renderer.as_mut(), crusty_viewport_texture)
                            {
                                cr.update_native_texture(id, vt.image_view());
                            }
                            let _ = response.send(RenderEvent::ViewportTextureChanged {
                                texture_id: packet
                                    .viewport_texture_id
                                    .unwrap_or(egui::TextureId::default()),
                                image_view: vt.image_view(),
                            });
                        }
                        Ok(false) => {}
                        Err(e) => {
                            log::error!("render_thread: viewport resize failed: {}", e);
                        }
                    }
                }
            }

            // Process texture bind commands (editor only)
            #[cfg(feature = "editor")]
            if let Some(ref mut egui_r) = egui_renderer {
                for bind in &packet.texture_binds {
                    egui_r.update_native_texture(bind.texture_id, bind.image_view.clone());
                }
                // Ensure the viewport texture in EguiRenderer points to the render
                // thread's actual image (not the main thread's placeholder).
                if let (Some(tex_id), Some(ref vt)) =
                    (packet.viewport_texture_id, &viewport_texture)
                {
                    egui_r.update_native_texture(tex_id, vt.image_view());
                }
            }

            // Record and submit based on render mode
            #[cfg(feature = "editor")]
            let is_editor = packet.render_mode == RenderMode::Editor;
            #[cfg(not(feature = "editor"))]
            let is_editor = false;

            if is_editor {
                #[cfg(feature = "editor")]
                {
                    // Editor mode: render deferred to viewport texture, then egui to swapchain
                    let deferred_cb = if let Some(ref vt) = viewport_texture {
                        crate::profile_scope!("record_deferred");
                        let render_target = RenderTarget::Texture {
                            image: vt.image(),
                        };
                        match deferred_renderer.render(
                            &packet.mesh_data,
                            &packet.shadow_caster_data,
                            &packet.light_data,
                            render_target,
                            packet.grid_visible,
                            packet.view_proj,
                            packet.camera_pos,
                            &packet.debug_draw,
                            &packet.post_processing,
                            &packet.plankton_emitters,
                        ) {
                            Ok(cb) => Some(cb),
                            Err(e) => {
                                log::error!("render_thread: deferred render error: {}", e);
                                None
                            }
                        }
                    } else {
                        None
                    };

                    #[cfg(feature = "editor")]
                    let crusty_target = target_image.clone();

                    let egui_cb = if let (Some(ref mut egui_r), Some(primitives), Some(deltas)) = (
                        &mut egui_renderer,
                        packet.egui_primitives,
                        packet.egui_texture_deltas,
                    ) {
                        crate::profile_scope!("record_egui");
                        let screen_rect = egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(
                                packet.window_dimensions[0] as f32,
                                packet.window_dimensions[1] as f32,
                            ),
                        );
                        match egui_r.render(target_image, primitives, deltas, screen_rect) {
                            Ok(cb) => Some(cb),
                            Err(e) => {
                                log::error!("render_thread: egui render error: {}", e);
                                None
                            }
                        }
                    } else {
                        None
                    };

                    // crusty-gui pass, composited over the egui output (Phase 16
                    // migration — panels move over one at a time).
                    #[cfg(feature = "editor")]
                    let crusty_cb = if let (Some(ref mut crusty_r), Some(paint)) =
                        (&mut crusty_renderer, packet.crusty_paint.as_deref())
                    {
                        crate::profile_scope!("record_crusty");
                        match crusty_r.render(crusty_target, paint, None, None) {
                            Ok(cb) => cb,
                            Err(e) => {
                                log::error!("render_thread: crusty render error: {}", e);
                                None
                            }
                        }
                    } else {
                        None
                    };

                    // Chain and submit whatever command buffers we have.
                    // Deferred-only frames (no UI pass) don't present.
                    let mut ui_cbs: Vec<
                        Arc<vulkano::command_buffer::PrimaryAutoCommandBuffer>,
                    > = Vec::new();
                    if let Some(cb) = egui_cb {
                        ui_cbs.push(cb);
                    }
                    #[cfg(feature = "editor")]
                    if let Some(cb) = crusty_cb {
                        ui_cbs.push(cb);
                    }

                    let submit_result = if ui_cbs.is_empty() {
                        log::trace!("render_thread: editor frame with no UI command buffers");
                        None
                    } else {
                        let mut cbs = deferred_cb.into_iter().collect::<Vec<_>>();
                        // Mesh-editor preview passes must execute before the
                        // GUI pass that samples their render targets.
                        #[cfg(feature = "editor")]
                        cbs.extend(packet.crusty_preview_cbs.iter().cloned());
                        cbs.extend(ui_cbs);

                        let mut future: Option<Box<dyn GpuFuture>> =
                            Some(acquire_future.boxed());
                        for cb in cbs {
                            match future.take().unwrap().then_execute(gpu.queue.clone(), cb) {
                                Ok(f) => future = Some(f.boxed()),
                                Err(e) => {
                                    log::error!("render_thread: editor execute error: {:?}", e);
                                    break;
                                }
                            }
                        }

                        let _submit_guard =
                            crate::engine::rendering::common::gpu_context::lock_queue_submit();
                        if let Some(future) = future {
                            if let Err(e) = future.flush() {
                                log::error!("render_thread: editor flush failed: {:?}", e);
                                unsafe { future.signal_finished() };
                                None
                            } else {
                                unsafe { future.signal_finished() };
                                let present = future
                                    .then_swapchain_present(
                                        gpu.queue.clone(),
                                        SwapchainPresentInfo::swapchain_image_index(
                                            swapchain_ref.clone(),
                                            image_index,
                                        ),
                                    )
                                    .then_signal_fence_and_flush();
                                match present {
                                    Ok(fence) => Some(fence.boxed()),
                                    Err(Validated::Error(VulkanError::OutOfDate)) => {
                                        needs_recreate = true;
                                        None
                                    }
                                    Err(e) => {
                                        log::error!("render_thread: editor present error: {:?}", e);
                                        None
                                    }
                                }
                            }
                        } else {
                            None
                        }
                    };

                    if let Some(fence) = submit_result {
                        fence_slots[slot] = Some(fence);
                    }
                }
            } else {
                // Standalone mode: render deferred directly to swapchain
                let deferred_cb = {
                    crate::profile_scope!("record_deferred");
                    let render_target = RenderTarget::Swapchain {
                        image: target_image,
                    };
                    match deferred_renderer.render(
                        &packet.mesh_data,
                        &packet.shadow_caster_data,
                        &packet.light_data,
                        render_target,
                        packet.grid_visible,
                        packet.view_proj,
                        packet.camera_pos,
                        &packet.debug_draw,
                        &packet.post_processing,
                        &packet.plankton_emitters,
                    ) {
                        Ok(cb) => cb,
                        Err(e) => {
                            log::error!("render_thread: render error: {}", e);
                            continue;
                        }
                    }
                };

                let _submit_guard =
                    crate::engine::rendering::common::gpu_context::lock_queue_submit();
                let future = acquire_future
                    .then_execute(gpu.queue.clone(), deferred_cb)
                    .map(|f| {
                        f.then_swapchain_present(
                            gpu.queue.clone(),
                            SwapchainPresentInfo::swapchain_image_index(
                                swapchain_ref.clone(),
                                image_index,
                            ),
                        )
                        .then_signal_fence_and_flush()
                    });

                match future {
                    Ok(Ok(f)) => {
                        fence_slots[slot] = Some(f.boxed());
                    }
                    Ok(Err(Validated::Error(VulkanError::OutOfDate))) => {
                        needs_recreate = true;
                    }
                    Ok(Err(e)) => {
                        log::error!("render_thread: present error: {:?}", e);
                    }
                    Err(e) => {
                        log::error!("render_thread: execute error: {:?}", e);
                    }
                }
            }

            frame_count += 1;
        }

        // Shutdown: wait for ALL fence slots before dropping GPU state
        for slot in &mut fence_slots {
            if let Some(mut prev) = slot.take() {
                prev.cleanup_finished();
            }
        }
        if has_swapchain {
            let _ = unsafe { gpu.device.wait_idle() };
        }
        log::info!("render_thread: GPU idle, dropping resources");
    }

    /// Block until the render thread sends `RenderThreadReady`.
    /// Returns the event, or an error on timeout / disconnect.
    pub fn wait_for_ready(
        &self,
        timeout: std::time::Duration,
    ) -> Result<RenderEvent, String> {
        self.response_receiver
            .recv_timeout(timeout)
            .map_err(|e| format!("render thread ready wait failed: {}", e))
    }

    /// Send a frame packet to the render thread.
    #[allow(clippy::result_large_err)]
    pub fn send(&self, packet: FramePacket) -> Result<(), crossbeam_channel::SendError<FramePacket>> {
        self.sender.send(packet)
    }

    /// Non-blocking poll for response events from the render thread.
    pub fn poll_events(&self) -> Vec<RenderEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.response_receiver.try_recv() {
            events.push(event);
        }
        events
    }

    /// Shut down the render thread gracefully.
    pub fn shutdown(&mut self) {
        if let Some(handle) = self.join_handle.take() {
            // Replace sender with a disconnected one to signal the render thread
            let (dead_tx, _) = bounded::<FramePacket>(1);
            let old_sender = std::mem::replace(&mut self.sender, dead_tx);
            drop(old_sender);

            match handle.join() {
                Ok(()) => log::info!("render_thread: joined successfully"),
                Err(e) => log::error!("render_thread: panicked during shutdown: {:?}", e),
            }
        }
    }
}

impl Drop for RenderThread {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Try to create a Vulkan-backed RenderThreadConfig.
    /// Returns `None` when the CI / host has no Vulkan driver (headless runner).
    fn try_test_config() -> Option<RenderThreadConfig> {
        use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
        use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
        use vulkano::device::DeviceExtensions;
        use vulkano::instance::{Instance, InstanceCreateInfo};
        use vulkano::memory::allocator::StandardMemoryAllocator;
        use vulkano::VulkanLibrary;

        let library = VulkanLibrary::new().ok()?;
        let instance = Instance::new(library, InstanceCreateInfo::default()).ok()?;
        let physical_device = instance.enumerate_physical_devices().ok()?.next()?;
        let queue_family_index = physical_device
            .queue_family_properties()
            .iter()
            .position(|q| q.queue_flags.intersects(vulkano::device::QueueFlags::GRAPHICS))?
            as u32;
        let (device, mut queues) = vulkano::device::Device::new(
            physical_device,
            vulkano::device::DeviceCreateInfo {
                queue_create_infos: vec![vulkano::device::QueueCreateInfo {
                    queue_family_index,
                    ..Default::default()
                }],
                enabled_extensions: DeviceExtensions {
                    ..DeviceExtensions::empty()
                },
                ..Default::default()
            },
        )
        .ok()?;
        let queue = queues.next()?;

        let gpu = Arc::new(GpuContext {
            device: device.clone(),
            queue,
            memory_allocator: Arc::new(StandardMemoryAllocator::new_default(device.clone())),
            command_buffer_allocator: Arc::new(StandardCommandBufferAllocator::new(
                device.clone(),
                Default::default(),
            )),
            descriptor_set_allocator: Arc::new(StandardDescriptorSetAllocator::new(
                device,
                Default::default(),
            )),
        });

        Some(RenderThreadConfig {
            gpu_context: gpu,
            render_mode: RenderMode::Standalone,
            initial_dimensions: [800, 600],
            swapchain_transfer: None,
            #[cfg(feature = "editor")]
            viewport_dimensions: None,
        })
    }

    #[test]
    fn test_render_thread_spawn_shutdown() {
        let config = match try_test_config() {
            Some(c) => c,
            None => {
                eprintln!("skipping: no Vulkan driver available");
                return;
            }
        };
        let mut rt = RenderThread::spawn(config);
        rt.shutdown();
    }

    #[test]
    fn test_render_thread_ready_handshake() {
        let config = match try_test_config() {
            Some(c) => c,
            None => {
                eprintln!("skipping: no Vulkan driver available");
                return;
            }
        };
        let mut rt = RenderThread::spawn(config);

        let event = rt
            .response_receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("timed out waiting for RenderThreadReady");

        match event {
            RenderEvent::RenderThreadReady { .. } => {}
            _ => panic!("expected RenderThreadReady, got different event"),
        }

        rt.shutdown();
    }
}

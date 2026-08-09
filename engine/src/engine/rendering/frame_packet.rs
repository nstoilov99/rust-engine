//! Frame packet types for cross-thread render data transfer.
//!
//! `FramePacket` is the sole data interface between the main thread
//! (game logic, ECS, crusty-gui layout) and the render thread (command
//! recording, GPU submission). All fields are owned/cloned — no
//! shared mutable state crosses the boundary.

use crate::engine::debug_draw::DebugDrawData;
use crate::engine::rendering::rendering_3d::{
    LightUniformData, MeshRenderData, PostProcessingSettings,
};
use glam::{Mat4, Vec3};
#[cfg(feature = "editor")]
use std::sync::Arc;
#[cfg(feature = "editor")]
use vulkano::image::view::ImageView;

/// The render mode determines which code path the render thread uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Standalone,
    #[cfg(feature = "editor")]
    Editor,
}

/// Emission parameters for a single plankton emitter frame.
#[derive(Debug, Clone)]
pub struct EmissionParameters {
    pub shape_type: u32,
    pub shape_params: [f32; 4],
    pub emission_rate: f32,
    pub burst_count: u32,
    pub burst_interval: f32,
    pub velocity_base: [f32; 3],
    pub velocity_variance: f32,
    pub lifetime_min: f32,
    pub lifetime_max: f32,
}

/// Force parameters for a single plankton emitter (Y-up render space).
#[derive(Debug, Clone)]
pub struct ForceParameters {
    pub gravity: [f32; 3],
    pub drag: f32,
    pub wind: [f32; 3],
    pub turbulence_strength: f32,
    pub turbulence_scale: f32,
    pub turbulence_speed: f32,
}

/// Visual parameters for a single plankton emitter.
#[derive(Debug, Clone)]
pub struct VisualParameters {
    pub size_start: f32,
    pub size_end: f32,
    pub color_start: [f32; 4],
    pub color_end: [f32; 4],
    pub texture_path: String,
    pub soft_fade_distance: f32,
}

/// Emitter flags packed as a bitmask.
#[derive(Debug, Clone, Copy)]
pub struct EmitterFlags {
    pub blend_mode: u32, // 0 = Additive
}

/// Frame data for a single plankton emitter, extracted from ECS.
#[derive(Debug, Clone)]
pub struct PlanktonEmitterFrameData {
    pub entity_guid: uuid::Uuid,
    pub world_transform: [[f32; 4]; 4],
    pub emission: EmissionParameters,
    pub forces: ForceParameters,
    pub visual: VisualParameters,
    pub flags: EmitterFlags,
    pub delta_time: f32,
    pub capacity: u32,
}

/// All data needed by the render thread to produce one frame.
pub struct FramePacket {
    // Scene data
    pub mesh_data: Vec<MeshRenderData>,
    /// All shadow-casting meshes, NOT camera-frustum culled. Off-screen
    /// casters must still render into the shadow map or their shadows
    /// pop in/out as the camera turns.
    pub shadow_caster_data: Vec<MeshRenderData>,
    pub light_data: LightUniformData,
    pub view_proj: Mat4,
    pub camera_pos: Vec3,
    /// Camera near/far clip planes. The render thread must not assume them —
    /// depth-linearizing consumers (plankton soft particles) read these.
    pub camera_near: f32,
    pub camera_far: f32,
    pub grid_visible: bool,
    pub debug_draw: DebugDrawData,
    pub post_processing: PostProcessingSettings,
    pub plankton_emitters: Vec<PlanktonEmitterFrameData>,

    /// crusty-gui paint list — the editor's sole UI pass, and the
    /// standalone HUD overlay when the `hud` feature is on.
    #[cfg(any(feature = "editor", feature = "hud"))]
    pub crusty_paint: Option<Vec<crusty_gui::paint::PaintCmd>>,

    /// Thumbnail pixels the render thread should upload + register as
    /// crusty textures; ids come back via
    /// [`RenderEvent::CrustyTexturesRegistered`].
    #[cfg(feature = "editor")]
    pub crusty_texture_uploads: Vec<CrustyTextureUpload>,

    /// Pre-recorded command buffers (mesh-editor previews) the render
    /// thread executes before the GUI pass, so the preview textures are
    /// rendered and layout-transitioned within the same submission that
    /// samples them.
    #[cfg(feature = "editor")]
    pub crusty_preview_cbs: Vec<Arc<vulkano::command_buffer::PrimaryAutoCommandBuffer>>,

    /// Image views to register as crusty native textures, keyed by an
    /// arbitrary string; ids come back via
    /// [`RenderEvent::CrustyNativeRegistered`].
    #[cfg(feature = "editor")]
    pub crusty_native_registrations: Vec<(String, Arc<ImageView>)>,

    /// Re-point an already-registered crusty native texture at a new image
    /// view (preview texture resized).
    #[cfg(feature = "editor")]
    pub crusty_native_updates: Vec<(crusty_gui::paint::TextureId, Arc<ImageView>)>,

    /// Crusty native textures to drop (mesh editor closed) so the registry
    /// doesn't pin dead render targets.
    #[cfg(feature = "editor")]
    pub crusty_native_removals: Vec<crusty_gui::paint::TextureId>,

    // Render config
    pub render_mode: RenderMode,
    pub window_dimensions: [u32; 2],
    #[cfg(feature = "editor")]
    pub viewport_dimensions: Option<[u32; 2]>,

    // Frame metadata
    pub frame_number: u64,
}

/// RGBA8 pixels for a texture the render thread uploads to the GPU and
/// registers with the crusty renderer (asset-browser thumbnails).
#[cfg(feature = "editor")]
pub struct CrustyTextureUpload {
    pub id: crate::engine::assets::AssetId,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Events sent from the render thread back to the main thread.
pub enum RenderEvent {
    RenderThreadReady {
        #[cfg(feature = "editor")]
        viewport_texture: Option<Arc<ImageView>>,
        /// Hierarchy icon set uploaded by the render thread: SVG file stem
        /// → crusty texture id, for the crusty layout pass on the main
        /// thread. Empty when the crusty renderer is disabled.
        #[cfg(feature = "editor")]
        crusty_icons: std::collections::HashMap<String, crusty_gui::paint::TextureId>,
        /// Crusty texture id the render thread registered for the viewport
        /// render target. Stays valid across resizes (the render thread
        /// re-points it at the new image view).
        #[cfg(feature = "editor")]
        crusty_viewport_texture: Option<crusty_gui::paint::TextureId>,
    },
    SwapchainRecreated {
        dimensions: [u32; 2],
    },
    RenderError {
        message: String,
    },
    /// Asset id → crusty texture id for uploads the render thread finished.
    #[cfg(feature = "editor")]
    CrustyTexturesRegistered(Vec<(crate::engine::assets::AssetId, crusty_gui::paint::TextureId)>),
    /// Key → crusty texture id for native image views the render thread
    /// registered from [`FramePacket::crusty_native_registrations`].
    #[cfg(feature = "editor")]
    CrustyNativeRegistered(Vec<(String, crusty_gui::paint::TextureId)>),
}

impl FramePacket {
    /// Build a standalone-mode frame packet from prepared render data.
    #[allow(clippy::too_many_arguments)]
    pub fn build_standalone(
        mesh_data: Vec<MeshRenderData>,
        shadow_caster_data: Vec<MeshRenderData>,
        light_data: LightUniformData,
        view_proj: Mat4,
        camera_pos: Vec3,
        camera_near_far: (f32, f32),
        grid_visible: bool,
        debug_draw: DebugDrawData,
        window_dimensions: [u32; 2],
        frame_number: u64,
        plankton_emitters: Vec<PlanktonEmitterFrameData>,
    ) -> Self {
        Self {
            mesh_data,
            shadow_caster_data,
            light_data,
            view_proj,
            camera_pos,
            camera_near: camera_near_far.0,
            camera_far: camera_near_far.1,
            grid_visible,
            debug_draw,
            post_processing: PostProcessingSettings::default(),
            plankton_emitters,
            #[cfg(any(feature = "editor", feature = "hud"))]
            crusty_paint: None,
            #[cfg(feature = "editor")]
            crusty_texture_uploads: Vec::new(),
            #[cfg(feature = "editor")]
            crusty_preview_cbs: Vec::new(),
            #[cfg(feature = "editor")]
            crusty_native_registrations: Vec::new(),
            #[cfg(feature = "editor")]
            crusty_native_updates: Vec::new(),
            #[cfg(feature = "editor")]
            crusty_native_removals: Vec::new(),
            render_mode: RenderMode::Standalone,
            window_dimensions,
            #[cfg(feature = "editor")]
            viewport_dimensions: None,
            frame_number,
        }
    }

    /// Build an editor-mode frame packet from prepared render data.
    #[cfg(feature = "editor")]
    #[allow(clippy::too_many_arguments)]
    pub fn build_editor(
        mesh_data: Vec<MeshRenderData>,
        shadow_caster_data: Vec<MeshRenderData>,
        light_data: LightUniformData,
        view_proj: Mat4,
        camera_pos: Vec3,
        camera_near_far: (f32, f32),
        grid_visible: bool,
        debug_draw: DebugDrawData,
        window_dimensions: [u32; 2],
        viewport_dimensions: Option<[u32; 2]>,
        frame_number: u64,
        plankton_emitters: Vec<PlanktonEmitterFrameData>,
    ) -> Self {
        Self {
            mesh_data,
            shadow_caster_data,
            light_data,
            view_proj,
            camera_pos,
            camera_near: camera_near_far.0,
            camera_far: camera_near_far.1,
            grid_visible,
            debug_draw,
            post_processing: PostProcessingSettings::default(),
            plankton_emitters,
            crusty_paint: None,
            crusty_texture_uploads: Vec::new(),
            crusty_preview_cbs: Vec::new(),
            crusty_native_registrations: Vec::new(),
            crusty_native_updates: Vec::new(),
            crusty_native_removals: Vec::new(),
            render_mode: RenderMode::Editor,
            window_dimensions,
            viewport_dimensions,
            frame_number,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn test_frame_packet_is_send() {
        assert_send::<FramePacket>();
    }

    #[test]
    fn test_render_event_is_send() {
        assert_send::<RenderEvent>();
    }
}

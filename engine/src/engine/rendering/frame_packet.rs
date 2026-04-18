//! Frame packet types for cross-thread render data transfer.
//!
//! `FramePacket` is the sole data interface between the main thread
//! (game logic, ECS, egui layout) and the render thread (command
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

/// A command to bind or update an egui texture slot on the render thread.
#[cfg(feature = "editor")]
pub struct TextureBindCommand {
    pub texture_id: egui::TextureId,
    pub image_view: Arc<ImageView>,
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
    pub grid_visible: bool,
    pub debug_draw: DebugDrawData,
    pub post_processing: PostProcessingSettings,
    pub plankton_emitters: Vec<PlanktonEmitterFrameData>,

    // egui data (None until Step 7 wires up the egui split)
    #[cfg(feature = "editor")]
    pub egui_primitives: Option<Vec<egui::ClippedPrimitive>>,
    #[cfg(feature = "editor")]
    pub egui_texture_deltas: Option<egui::TexturesDelta>,

    // Render config
    pub render_mode: RenderMode,
    pub window_dimensions: [u32; 2],
    #[cfg(feature = "editor")]
    pub viewport_dimensions: Option<[u32; 2]>,

    // Frame metadata
    pub frame_number: u64,

    // Texture bind commands (for egui texture slot protocol)
    #[cfg(feature = "editor")]
    pub texture_binds: Vec<TextureBindCommand>,

    /// The egui TextureId assigned to the viewport texture, so the render
    /// thread can update its EguiRenderer cache after a viewport resize.
    #[cfg(feature = "editor")]
    pub viewport_texture_id: Option<egui::TextureId>,
}

/// Events sent from the render thread back to the main thread.
pub enum RenderEvent {
    RenderThreadReady {
        #[cfg(feature = "editor")]
        viewport_texture: Option<Arc<ImageView>>,
    },
    SwapchainRecreated {
        dimensions: [u32; 2],
    },
    #[cfg(feature = "editor")]
    ViewportTextureChanged {
        texture_id: egui::TextureId,
        image_view: Arc<ImageView>,
    },
    RenderError {
        message: String,
    },
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
            grid_visible,
            debug_draw,
            post_processing: PostProcessingSettings::default(),
            plankton_emitters,
            #[cfg(feature = "editor")]
            egui_primitives: None,
            #[cfg(feature = "editor")]
            egui_texture_deltas: None,
            render_mode: RenderMode::Standalone,
            window_dimensions,
            #[cfg(feature = "editor")]
            viewport_dimensions: None,
            frame_number,
            #[cfg(feature = "editor")]
            texture_binds: Vec::new(),
            #[cfg(feature = "editor")]
            viewport_texture_id: None,
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
            grid_visible,
            debug_draw,
            post_processing: PostProcessingSettings::default(),
            plankton_emitters,
            egui_primitives: None,
            egui_texture_deltas: None,
            render_mode: RenderMode::Editor,
            window_dimensions,
            viewport_dimensions,
            frame_number,
            texture_binds: Vec::new(),
            viewport_texture_id: None,
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
